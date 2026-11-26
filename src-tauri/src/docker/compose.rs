//! Thin wrapper around the `docker compose` CLI plugin.
//!
//! bollard speaks the Engine API, which has no first-class Compose concept,
//! so project lifecycle (`up`/`down`/`ps`/`logs`/`config`) shells out to the
//! bundled `docker` CLI against the app's own engine
//! (see [`super::engine::docker_command`]).
//! Every blocking call runs inside `spawn_blocking` so neither the GTK main
//! loop nor the Tokio async workers stall.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One service row of `docker compose ps --format json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeService {
    pub id: String,
    pub name: String,
    pub service: String,
    pub image: String,
    pub state: String,
    pub health: String,
    pub ports: String,
}

/// One port a declared service publishes (`published` may be empty or a
/// range when the file leaves the host side to the engine).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortSpec {
    pub published: String,
    pub target: u64,
    pub protocol: String,
}

/// A service as the compose file declares it (from `compose config`), so
/// services that have no container yet still show up with their wiring.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub image: String,
    pub build: bool,
    pub depends_on: Vec<String>,
    pub ports: Vec<PortSpec>,
    pub profiles: Vec<String>,
    pub healthcheck: bool,
    pub replicas: Option<u64>,
    pub container_name: String,
    /// Declares `develop.watch` rules, so `compose watch` has work to do.
    #[serde(default)]
    pub watch: bool,
}

/// The parts of a project's resolved configuration the app works with.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeModel {
    pub name: String,
    pub services: Vec<ServiceSpec>,
    pub profiles: Vec<String>,
}

/// Receives each output line of a long-running compose call as it arrives.
pub type LineSink = Box<dyn Fn(&str) + Send>;

/// File names `docker compose` itself looks for, in its order of preference.
pub const COMPOSE_FILE_NAMES: [&str; 4] = [
    "compose.yaml",
    "compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];

/// Upper bound for up/down/start/stop/restart (image pulls included).
const LIFECYCLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);
/// Compose and env files larger than this are not something to edit here.
const MAX_EDIT_BYTES: u64 = 2 * 1024 * 1024;
/// Output lines kept for the command's return value (the sink sees all).
const MAX_KEPT_LINES: usize = 4000;

/// Probe for a usable Compose implementation via the bundled engine.
pub fn is_available() -> bool {
    let paths = crate::docker::engine::EnginePaths::new();
    crate::docker::engine::docker_command_resolved(&paths)
        .arg("compose")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Invocation
// ---------------------------------------------------------------------------

/// Which project a compose call addresses, and with what global options.
#[derive(Debug, Clone, Default)]
pub struct Target {
    /// `-f FILE`; when absent the project is addressed by name only.
    pub file: Option<PathBuf>,
    /// More `-f FILE`s merged on top of `file` (an override file).
    pub extra_files: Vec<PathBuf>,
    /// `-p NAME`.
    pub project: Option<String>,
    /// `--env-file PATH`.
    pub env_file: Option<PathBuf>,
    /// `--profile NAME`, repeated.
    pub profiles: Vec<String>,
}

impl Target {
    pub fn file(file: &Path) -> Self {
        Self { file: Some(file.to_path_buf()), ..Default::default() }
    }

    pub fn project(name: &str) -> Self {
        Self { project: Some(name.to_string()), ..Default::default() }
    }

    pub fn global_args(&self) -> anyhow::Result<Vec<String>> {
        let mut args: Vec<String> = ["compose", "--ansi", "never", "--progress", "plain"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        if let Some(f) = &self.file {
            if !f.is_file() {
                bail!("compose file not found: {}", f.display());
            }
            args.push("-f".into());
            args.push(f.display().to_string());
            for extra in &self.extra_files {
                if !extra.is_file() {
                    bail!("compose file not found: {}", extra.display());
                }
                args.push("-f".into());
                args.push(extra.display().to_string());
            }
        }
        if let Some(p) = &self.project {
            check_project_name(p)?;
            args.push("-p".into());
            args.push(p.clone());
        }
        if self.file.is_none() && self.project.is_none() {
            bail!("a compose file or project name is required");
        }
        if let Some(e) = &self.env_file {
            if !e.is_file() {
                bail!("env file not found: {}", e.display());
            }
            args.push("--env-file".into());
            args.push(e.display().to_string());
        }
        for p in &self.profiles {
            check_profile_name(p)?;
            args.push("--profile".into());
            args.push(p.clone());
        }
        Ok(args)
    }
}

fn valid_token(s: &str, allow_upper: bool) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return false };
    let ok_char = |c: char| {
        c.is_ascii_digit()
            || c.is_ascii_lowercase()
            || (allow_upper && c.is_ascii_uppercase())
    };
    ok_char(first) && chars.all(|c| ok_char(c) || matches!(c, '_' | '-' | '.')) && s.len() <= 128
}

/// Compose project names: lowercase letters, digits, `_` and `-`.
pub fn check_project_name(name: &str) -> anyhow::Result<()> {
    if !valid_token(name, false) || name.contains('.') {
        bail!("invalid compose project name '{name}'");
    }
    Ok(())
}

pub fn check_service_name(name: &str) -> anyhow::Result<()> {
    if !valid_token(name, true) {
        bail!("invalid service name '{name}'");
    }
    Ok(())
}

fn check_profile_name(name: &str) -> anyhow::Result<()> {
    if name != "*" && !valid_token(name, true) {
        bail!("invalid profile name '{name}'");
    }
    Ok(())
}

/// Options for `up`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpOptions {
    pub force_recreate: bool,
    pub remove_orphans: bool,
    /// Only these services (and their dependencies); empty = all.
    pub services: Vec<String>,
}

/// Options for `down`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownOptions {
    pub remove_volumes: bool,
    pub remove_orphans: bool,
}

pub fn up_args(o: &UpOptions) -> anyhow::Result<Vec<String>> {
    let mut a = vec!["up".to_string(), "-d".to_string()];
    if o.force_recreate {
        a.push("--force-recreate".into());
    }
    if o.remove_orphans {
        a.push("--remove-orphans".into());
    }
    for s in &o.services {
        check_service_name(s)?;
        a.push(s.clone());
    }
    Ok(a)
}

pub fn down_args(o: &DownOptions) -> Vec<String> {
    let mut a = vec!["down".to_string()];
    if o.remove_volumes {
        a.push("-v".into());
    }
    if o.remove_orphans {
        a.push("--remove-orphans".into());
    }
    a
}

/// Options for `pull`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PullOptions {
    /// Only these services; empty = every service with an image.
    pub services: Vec<String>,
    /// Keep going when one image cannot be pulled.
    pub ignore_failures: bool,
}

/// Options for `build`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildOptions {
    /// Only these services; empty = every service with a `build:` section.
    pub services: Vec<String>,
    pub no_cache: bool,
    /// Always pull newer base images.
    pub pull: bool,
}

pub fn pull_args(o: &PullOptions) -> anyhow::Result<Vec<String>> {
    let mut a = vec!["pull".to_string()];
    if o.ignore_failures {
        a.push("--ignore-pull-failures".into());
    }
    for s in &o.services {
        check_service_name(s)?;
        a.push(s.clone());
    }
    Ok(a)
}

pub fn build_args(o: &BuildOptions) -> anyhow::Result<Vec<String>> {
    let mut a = vec!["build".to_string()];
    if o.no_cache {
        a.push("--no-cache".into());
    }
    if o.pull {
        a.push("--pull".into());
    }
    for s in &o.services {
        check_service_name(s)?;
        a.push(s.clone());
    }
    Ok(a)
}

/// Per-service verbs. `start`/`stop`/`restart` work by project name alone;
/// `up`, `recreate` and `scale` need the compose file.
pub const SERVICE_ACTIONS: [&str; 6] = ["start", "stop", "restart", "up", "recreate", "scale"];

pub fn service_action_needs_file(action: &str) -> bool {
    matches!(action, "up" | "recreate" | "scale")
}

pub fn service_action_args(
    action: &str,
    service: &str,
    replicas: Option<u32>,
) -> anyhow::Result<Vec<String>> {
    check_service_name(service)?;
    let svc = service.to_string();
    let args: Vec<String> = match action {
        "start" | "stop" | "restart" => vec![action.into(), svc],
        "up" => vec!["up".into(), "-d".into(), svc],
        "recreate" => vec![
            "up".into(),
            "-d".into(),
            "--no-deps".into(),
            "--force-recreate".into(),
            svc,
        ],
        "scale" => {
            let n = replicas.context("scale needs a replica count")?;
            if n > 100 {
                bail!("at most 100 replicas");
            }
            vec![
                "up".into(),
                "-d".into(),
                "--no-deps".into(),
                "--no-recreate".into(),
                "--scale".into(),
                format!("{svc}={n}"),
                svc,
            ]
        }
        other => bail!("unsupported service action '{other}'"),
    };
    Ok(args)
}

/// Run one compose invocation (global options from `target`, then `verb`).
/// With a `sink` every output line is passed on as it arrives.
pub async fn run(
    target: Target,
    verb: Vec<String>,
    sink: Option<LineSink>,
) -> anyhow::Result<String> {
    let limit = match verb.first().map(String::as_str) {
        Some("ps" | "logs" | "config") => QUERY_TIMEOUT,
        _ => LIFECYCLE_TIMEOUT,
    };
    tokio::task::spawn_blocking(move || {
        let mut args = target.global_args()?;
        args.extend(verb);
        run_args(&args, limit, sink, None)
    })
    .await
    .context("compose task join")?
}

fn run_args(
    args: &[String],
    limit: Duration,
    sink: Option<LineSink>,
    stdin: Option<String>,
) -> anyhow::Result<String> {
    let paths = crate::docker::engine::EnginePaths::new();
    let mut cmd = crate::docker::engine::docker_command_resolved(&paths);
    cmd.args(args);
    if sink.is_none() && stdin.is_none() {
        let out = crate::procutil::output_with_timeout(cmd, limit, "docker compose")
            .context("failed to run `docker compose`")?;
        return finish_output("compose", out);
    }
    run_streaming(cmd, limit, sink, stdin)
}

/// Like `output_with_timeout`, but hands each line to `sink` as it arrives
/// and can feed `stdin` (used for `-f -` validation).
fn run_streaming(
    mut cmd: std::process::Command,
    limit: Duration,
    sink: Option<LineSink>,
    stdin: Option<String>,
) -> anyhow::Result<String> {
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("failed to run `docker compose`")?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = pipe.write_all(text.as_bytes());
        });
    }
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let pump = |pipe: Option<Box<dyn Read + Send>>, tx: std::sync::mpsc::Sender<String>| {
        std::thread::spawn(move || {
            if let Some(p) = pipe {
                for line in BufReader::new(p).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            }
        })
    };
    let t_out = pump(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), tx.clone());
    let t_err = pump(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>), tx);
    let deadline = Instant::now() + limit;
    let mut lines: std::collections::VecDeque<String> = Default::default();
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                if let Some(s) = &sink {
                    s(&line);
                }
                if lines.len() == MAX_KEPT_LINES {
                    lines.pop_front();
                }
                lines.push_back(line);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("`docker compose` did not finish within {}s and was stopped", limit.as_secs());
        }
    }
    let status = child.wait().context("waiting for `docker compose`")?;
    let _ = t_out.join();
    let _ = t_err.join();
    let combined = Vec::from(lines).join("\n");
    if status.success() {
        Ok(combined)
    } else {
        bail!("compose failed: {}", tail_lines(&combined, 5))
    }
}

fn tail_lines(text: &str, n: usize) -> String {
    let tail: Vec<&str> = text.lines().rev().take(n).collect();
    tail.into_iter().rev().collect::<Vec<_>>().join("\n")
}

fn finish_output(label: &str, out: std::process::Output) -> anyhow::Result<String> {
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !stderr.trim().is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }
    if out.status.success() {
        Ok(combined)
    } else {
        bail!("{label} failed: {}", tail_lines(&combined, 5))
    }
}

// ---------------------------------------------------------------------------
// Lifecycle shorthands
// ---------------------------------------------------------------------------

/// Bring a project up in detached mode, returning the CLI's combined output.
pub async fn up(file: &Path) -> anyhow::Result<String> {
    run(Target::file(file), up_args(&UpOptions::default())?, None).await
}

/// Tear a project down, returning the CLI's combined output.
pub async fn down(file: &Path) -> anyhow::Result<String> {
    run(Target::file(file), down_args(&DownOptions::default()), None).await
}

/// Run a project-scoped compose verb by *project name* (`compose -p NAME`),
/// which works without the compose file as long as the project's containers
/// carry the usual compose labels — this is how Docker Desktop drives
/// Start/Stop/Restart of a compose project from the Containers view.
pub async fn project_action(project: &str, subcommand: &str) -> anyhow::Result<String> {
    project_action_streaming(project, subcommand, None).await
}

/// [`project_action`] with every output line also sent to `sink`.
pub async fn project_action_streaming(
    project: &str,
    subcommand: &str,
    sink: Option<LineSink>,
) -> anyhow::Result<String> {
    let project = project.trim();
    if project.is_empty() {
        bail!("project name is required");
    }
    if !matches!(subcommand, "start" | "stop" | "restart" | "down") {
        bail!("unsupported compose project action '{subcommand}'");
    }
    run(Target::project(project), vec![subcommand.to_string()], sink).await
}

/// List the project's services via `ps --format json`.
pub async fn ps(file: &Path) -> anyhow::Result<Vec<ComposeService>> {
    let out = run(
        Target::file(file),
        vec!["ps".into(), "--all".into(), "--format".into(), "json".into()],
        None,
    )
    .await?;
    Ok(parse_ps_output(&out))
}

/// Fetch the project's combined logs (last `tail` lines), optionally only
/// for some services.
pub async fn logs(file: &Path, tail: usize, services: &[String]) -> anyhow::Result<String> {
    let mut verb = vec!["logs".to_string(), "--no-color".into(), format!("--tail={tail}")];
    for s in services {
        check_service_name(s)?;
        verb.push(s.clone());
    }
    run(Target::file(file), verb, None).await
}

/// The project's resolved model, every profile included.
pub async fn config(file: &Path, env_file: Option<PathBuf>) -> anyhow::Result<ComposeModel> {
    let target = Target {
        file: Some(file.to_path_buf()),
        extra_files: override_file_for(file).into_iter().collect(),
        env_file,
        profiles: vec!["*".into()],
        ..Default::default()
    };
    config_of(target).await
}

/// [`config`] for an explicit target (files, env file, profiles).
pub async fn config_of(mut target: Target) -> anyhow::Result<ComposeModel> {
    if target.profiles.is_empty() {
        target.profiles = vec!["*".into()];
    }
    let out = run(target, vec!["config".into(), "--format".into(), "json".into()], None).await?;
    parse_config(&out)
}

/// The merged configuration as compose resolves it (YAML, every profile, the
/// override file and `.env` interpolation applied), for read-only display.
pub async fn merged_yaml(mut target: Target) -> anyhow::Result<String> {
    target.profiles = vec!["*".into()];
    let out = run(target, vec!["config".into()], None).await?;
    // Warnings (obsolete `version:` …) come first on stderr; keep the YAML.
    Ok(match out.find("name:") {
        Some(i) => out[i..].to_string(),
        None => out,
    })
}

/// Override files compose itself merges when it is run without `-f`, in its
/// order of preference. Only a standard-named base file gets one.
const OVERRIDE_FILE_NAMES: [&str; 4] = [
    "compose.override.yaml",
    "compose.override.yml",
    "docker-compose.override.yaml",
    "docker-compose.override.yml",
];

/// The override file next to `file` that `docker compose` would merge by
/// default. The app always passes `-f`, which turns that default off, so it
/// adds the override back explicitly.
pub fn override_file_for(file: &Path) -> Option<PathBuf> {
    let name = file.file_name()?.to_str()?;
    if !COMPOSE_FILE_NAMES.contains(&name) {
        return None;
    }
    let dir = file.parent()?;
    OVERRIDE_FILE_NAMES.iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// Validate unsaved compose text as if it were `file` (same project
/// directory, so relative paths and `.env` resolve the same way). The text
/// goes through stdin; nothing is written next to the user's file.
pub async fn validate_text(
    file: &Path,
    text: String,
    env_file: Option<PathBuf>,
) -> anyhow::Result<ComposeModel> {
    validate_in_stack(file, text, env_file, &[]).await
}

/// Validate unsaved text of `file` as part of `stack` (every `-f` in merge
/// order, `file` among them), so an override file is checked together with
/// the base it extends. An empty stack means `file` alone.
pub async fn validate_in_stack(
    file: &Path,
    text: String,
    env_file: Option<PathBuf>,
    stack: &[PathBuf],
) -> anyhow::Result<ComposeModel> {
    let base = stack.first().map(PathBuf::as_path).unwrap_or(file);
    let dir = base
        .parent()
        .context("compose file has no parent directory")?
        .to_path_buf();
    let files: Vec<String> = if stack.is_empty() {
        vec!["-".to_string()]
    } else {
        stack
            .iter()
            .map(|f| if f == file { "-".to_string() } else { f.display().to_string() })
            .collect()
    };
    tokio::task::spawn_blocking(move || {
        let mut args: Vec<String> = ["compose", "--ansi", "never", "--project-directory"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.push(dir.display().to_string());
        for f in files {
            args.extend(["-f".to_string(), f]);
        }
        if let Some(e) = env_file {
            args.push("--env-file".into());
            args.push(e.display().to_string());
        }
        args.extend(["--profile", "*", "config", "--format", "json"].map(String::from));
        let out = run_args(&args, QUERY_TIMEOUT, None, Some(text))?;
        // stderr warnings (e.g. obsolete `version:`) precede the JSON body.
        let json_start = out.find('{').unwrap_or(0);
        parse_config(&out[json_start..])
    })
    .await
    .context("compose validate task join")?
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn str_field(v: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(s) = v.get(key).and_then(Value::as_str) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

fn ports_summary(v: &Value) -> String {
    let Some(publishers) = v.get("Publishers").and_then(Value::as_array) else {
        return String::new();
    };
    let mut parts = Vec::new();
    for p in publishers {
        let target = p
            .get("TargetPort")
            .and_then(Value::as_u64)
            .map(|n| n.to_string())
            .unwrap_or_default();
        let published = p
            .get("PublishedPort")
            .and_then(Value::as_u64)
            .map(|n| n.to_string())
            .unwrap_or_default();
        let proto = p
            .get("Protocol")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if target.is_empty() {
            continue;
        }
        let part = if published.is_empty() || published == "0" {
            format!("{target}/{proto}")
        } else {
            format!("{published}->{target}/{proto}")
        };
        // IPv4 and IPv6 bindings of one port are listed twice.
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    parts.join(", ")
}

fn parse_service(v: &Value) -> ComposeService {
    ComposeService {
        id: str_field(v, &["ID", "Id", "id"]),
        name: str_field(v, &["Name", "name"]),
        service: str_field(v, &["Service", "service"]),
        image: str_field(v, &["Image", "image"]),
        state: str_field(v, &["State", "state"]),
        health: str_field(v, &["Health", "health"]),
        ports: ports_summary(v),
    }
}

/// Parse `docker compose ps --format json` output.
///
/// Newer plugin versions print one JSON array; older ones print one JSON
/// object per line. Both shapes (and empty output) are accepted so the UI
/// survives plugin skew.
pub fn parse_ps_output(out: &str) -> Vec<ComposeService> {
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(trimmed) {
        return items.iter().map(parse_service).collect();
    }
    trimmed
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .map(|v| parse_service(&v))
        .collect()
}

fn parse_port(v: &Value) -> Option<PortSpec> {
    match v {
        Value::Object(_) => {
            let target = v.get("target").and_then(|t| {
                t.as_u64().or_else(|| t.as_str().and_then(|s| s.parse().ok()))
            })?;
            let published = match v.get("published") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => String::new(),
            };
            let protocol = v.get("protocol").and_then(Value::as_str).unwrap_or("tcp").to_string();
            Some(PortSpec { published, target, protocol })
        }
        // Short syntax: "[ip:]published:target[/proto]" or "target".
        Value::String(s) => {
            let (spec, protocol) = match s.split_once('/') {
                Some((a, p)) => (a, p.to_string()),
                None => (s.as_str(), "tcp".to_string()),
            };
            let parts: Vec<&str> = spec.rsplitn(3, ':').collect();
            let target = parts.first()?.parse().ok()?;
            let published = parts.get(1).map(|p| p.to_string()).unwrap_or_default();
            Some(PortSpec { published, target, protocol })
        }
        Value::Number(n) => Some(PortSpec {
            published: String::new(),
            target: n.as_u64()?,
            protocol: "tcp".into(),
        }),
        _ => None,
    }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(items)) => {
            items.iter().filter_map(|i| i.as_str().map(str::to_string)).collect()
        }
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Parse `docker compose config --format json`.
pub fn parse_config(json: &str) -> anyhow::Result<ComposeModel> {
    let v: Value = serde_json::from_str(json.trim()).context("unreadable compose config")?;
    let name = v.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    let mut services = Vec::new();
    let mut profiles: Vec<String> = Vec::new();
    if let Some(map) = v.get("services").and_then(Value::as_object) {
        for (svc_name, s) in map {
            let mut depends_on = string_list(s.get("depends_on"));
            depends_on.sort();
            let ports = s
                .get("ports")
                .and_then(Value::as_array)
                .map(|ps| ps.iter().filter_map(parse_port).collect())
                .unwrap_or_default();
            let svc_profiles = string_list(s.get("profiles"));
            for p in &svc_profiles {
                if !profiles.contains(p) {
                    profiles.push(p.clone());
                }
            }
            let healthcheck = s
                .get("healthcheck")
                .map(|h| h.is_object() && h.get("disable").and_then(Value::as_bool) != Some(true))
                .unwrap_or(false);
            let replicas = s
                .get("deploy")
                .and_then(|d| d.get("replicas"))
                .and_then(Value::as_u64)
                .or_else(|| s.get("scale").and_then(Value::as_u64));
            services.push(ServiceSpec {
                name: svc_name.clone(),
                image: str_field(s, &["image"]),
                build: s.get("build").is_some_and(|b| !b.is_null()),
                depends_on,
                ports,
                profiles: svc_profiles,
                healthcheck,
                replicas,
                container_name: str_field(s, &["container_name"]),
                watch: s
                    .get("develop")
                    .and_then(|d| d.get("watch"))
                    .and_then(Value::as_array)
                    .is_some_and(|w| !w.is_empty()),
            });
        }
    }
    services.sort_by(|a, b| a.name.cmp(&b.name));
    profiles.sort();
    Ok(ComposeModel { name, services, profiles })
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

fn expand_home(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

/// The compose file inside `dir`, by compose's own lookup order.
pub fn compose_file_in(dir: &Path) -> Option<PathBuf> {
    COMPOSE_FILE_NAMES.iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// Resolve a compose file path the way the UI stores it: expand `~`, accept a
/// project directory (picking its compose file) and reject anything else that
/// is not a regular file.
pub fn resolve_compose_file(raw: &str) -> anyhow::Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("no compose file selected");
    }
    let expanded = expand_home(trimmed);
    if expanded.is_dir() {
        return compose_file_in(&expanded)
            .with_context(|| format!("no compose file in {}", expanded.display()));
    }
    if !expanded.is_file() {
        bail!("compose file not found: {}", expanded.display());
    }
    Ok(expanded)
}

/// Directories never worth descending into when looking for projects.
const SCAN_SKIP: [&str; 9] = [
    "node_modules",
    "target",
    "vendor",
    "dist",
    "build",
    "__pycache__",
    "venv",
    "site-packages",
    "proc",
];
const SCAN_MAX_RESULTS: usize = 200;
const SCAN_MAX_DIRS: usize = 20_000;

/// Compose files under `root`, at most `max_depth` directories deep. Hidden
/// directories, dependency/build trees and symlinks are skipped.
pub fn scan_dir(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut visited = 0usize;
    while let Some((dir, depth)) = queue.pop_front() {
        visited += 1;
        if visited > SCAN_MAX_DIRS || found.len() >= SCAN_MAX_RESULTS {
            break;
        }
        if let Some(f) = compose_file_in(&dir) {
            found.push(f);
        }
        if depth >= max_depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut subdirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                !name.starts_with('.') && !SCAN_SKIP.contains(&name.as_ref())
            })
            .map(|e| e.path())
            .collect();
        subdirs.sort();
        queue.extend(subdirs.into_iter().map(|d| (d, depth + 1)));
    }
    found
}

/// The env file of a project: `name` (default `.env`) relative to the
/// compose file's directory. Absolute paths and `..` are refused so the
/// editor cannot be pointed at arbitrary files.
pub fn env_path(compose_file: &Path, name: Option<&str>) -> anyhow::Result<PathBuf> {
    let name = name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(".env");
    let rel = Path::new(name);
    if rel.is_absolute()
        || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        bail!("env file must be a path inside the project directory");
    }
    let dir = compose_file.parent().context("compose file has no parent directory")?;
    Ok(dir.join(rel))
}

/// Read a text file for editing (size-capped). A missing file reads as `None`.
pub fn read_text(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
        Ok(m) if m.len() > MAX_EDIT_BYTES => bail!("{} is too large to edit here", path.display()),
        Ok(_) => {}
    }
    std::fs::read_to_string(path)
        .map(Some)
        .with_context(|| format!("cannot read {}", path.display()))
}

/// Replace `path` with `content`, but only if it still holds `expected`
/// (`None` = must not exist yet), so an edit made elsewhere in the meantime
/// is never silently overwritten. Written to a sibling temp file and renamed
/// into place, keeping the original permissions.
pub fn write_text_checked(path: &Path, content: &str, expected: Option<&str>) -> anyhow::Result<()> {
    if content.len() as u64 > MAX_EDIT_BYTES {
        bail!("content is too large");
    }
    let current = read_text(path)?;
    if current.as_deref() != expected {
        bail!("{} changed on disk since it was opened; reload it first", path.display());
    }
    let dir = path.parent().context("file has no parent directory")?;
    let file_name = path.file_name().context("file has no name")?.to_string_lossy();
    let tmp = dir.join(format!(".{file_name}.hsx2dock-{}.tmp", std::process::id()));
    std::fs::write(&tmp, content).with_context(|| format!("cannot write {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("cannot replace {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARRAY_JSON: &str = r#"[
        {"ID":"abc123","Name":"web-proj-web-1","Service":"web","Image":"nginx:latest","State":"running","Health":"","Publishers":[{"URL":"0.0.0.0","TargetPort":80,"PublishedPort":8080,"Protocol":"tcp"},{"URL":"::","TargetPort":80,"PublishedPort":8080,"Protocol":"tcp"}]},
        {"ID":"def456","Name":"web-proj-db-1","Service":"db","Image":"postgres:16","State":"exited","Health":"","Publishers":[]}
    ]"#;

    #[test]
    fn parses_json_array_shape() {
        let services = parse_ps_output(ARRAY_JSON);
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].service, "web");
        assert_eq!(services[0].state, "running");
        assert_eq!(services[0].ports, "8080->80/tcp");
        assert_eq!(services[1].ports, "");
    }

    #[test]
    fn parses_json_lines_shape() {
        let lines = ARRAY_JSON
            .trim_start_matches('[')
            .trim_end_matches(']')
            .replace("},\n        {", "}\n{");
        let services = parse_ps_output(&lines);
        assert_eq!(services.len(), 2);
        assert_eq!(services[1].service, "db");
        assert_eq!(services[1].image, "postgres:16");
    }

    #[test]
    fn empty_output_is_empty_list() {
        assert!(parse_ps_output("").is_empty());
        assert!(parse_ps_output("   \n  ").is_empty());
    }

    #[test]
    fn missing_fields_default_to_empty() {
        let services = parse_ps_output(r#"[{"Service":"worker"}]"#);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "");
        assert_eq!(services[0].ports, "");
    }

    #[test]
    fn unpublished_ports_render_without_arrow() {
        let services = parse_ps_output(
            r#"[{"Service":"w","Publishers":[{"TargetPort":53,"PublishedPort":0,"Protocol":"udp"}]}]"#,
        );
        assert_eq!(services[0].ports, "53/udp");
    }

    #[test]
    fn resolve_rejects_blank_and_missing() {
        assert!(resolve_compose_file("").is_err());
        assert!(resolve_compose_file("/definitely/not/a/compose.yml").is_err());
    }

    const CONFIG_JSON: &str = r#"{
      "name": "cdemo",
      "services": {
        "web": {"image": "nginx:alpine", "depends_on": {"api": {"condition": "service_started"}},
                "ports": [{"mode": "ingress", "target": 80, "published": "18181", "protocol": "tcp"}]},
        "api": {"image": "busybox", "depends_on": {"db": {}}, "healthcheck": {"test": ["CMD", "true"]}},
        "db": {"build": {"context": "."}, "deploy": {"replicas": 2}, "ports": ["127.0.0.1:5432:5432", "53/udp"]},
        "debug": {"image": "busybox", "profiles": ["tools", "debug"], "healthcheck": {"disable": true}}
      }
    }"#;

    #[test]
    fn parses_config_model() {
        let m = parse_config(CONFIG_JSON).unwrap();
        assert_eq!(m.name, "cdemo");
        let names: Vec<_> = m.services.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["api", "db", "debug", "web"]);
        assert_eq!(m.profiles, ["debug", "tools"]);
        let api = &m.services[0];
        assert_eq!(api.depends_on, ["db"]);
        assert!(api.healthcheck);
        let db = &m.services[1];
        assert!(db.build);
        assert_eq!(db.replicas, Some(2));
        assert_eq!(
            db.ports,
            [
                PortSpec { published: "5432".into(), target: 5432, protocol: "tcp".into() },
                PortSpec { published: "".into(), target: 53, protocol: "udp".into() },
            ]
        );
        assert!(!m.services[2].healthcheck);
        assert_eq!(m.services[3].ports[0].published, "18181");
    }

    #[test]
    fn builds_global_and_verb_args() {
        let t = Target {
            project: Some("shop".into()),
            profiles: vec!["tools".into()],
            ..Default::default()
        };
        let a = t.global_args().unwrap();
        assert_eq!(&a[5..], ["-p", "shop", "--profile", "tools"]);
        assert!(Target::default().global_args().is_err());
        assert!(Target::project("Bad Name").global_args().is_err());
        assert!(Target { project: Some("x".into()), profiles: vec!["a;b".into()], ..Default::default() }
            .global_args()
            .is_err());

        let up = up_args(&UpOptions {
            force_recreate: true,
            remove_orphans: true,
            services: vec!["web".into()],
        })
        .unwrap();
        assert_eq!(up, ["up", "-d", "--force-recreate", "--remove-orphans", "web"]);
        assert!(up_args(&UpOptions { services: vec!["-rf".into()], ..Default::default() }).is_err());
        assert_eq!(
            down_args(&DownOptions { remove_volumes: true, remove_orphans: false }),
            ["down", "-v"]
        );
    }

    #[test]
    fn service_actions_map_to_compose_verbs() {
        assert_eq!(service_action_args("restart", "api", None).unwrap(), ["restart", "api"]);
        assert_eq!(
            service_action_args("recreate", "api", None).unwrap(),
            ["up", "-d", "--no-deps", "--force-recreate", "api"]
        );
        assert_eq!(
            service_action_args("scale", "api", Some(3)).unwrap(),
            ["up", "-d", "--no-deps", "--no-recreate", "--scale", "api=3", "api"]
        );
        assert!(service_action_args("scale", "api", None).is_err());
        assert!(service_action_args("scale", "api", Some(500)).is_err());
        assert!(service_action_args("rm", "api", None).is_err());
        assert!(service_action_args("stop", "--all", None).is_err());
        assert!(service_action_needs_file("scale"));
        assert!(!service_action_needs_file("stop"));
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hsx2dock-compose-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn env_path_stays_inside_project() {
        let f = Path::new("/srv/app/compose.yaml");
        assert_eq!(env_path(f, None).unwrap(), Path::new("/srv/app/.env"));
        assert_eq!(env_path(f, Some("cfg/dev.env")).unwrap(), Path::new("/srv/app/cfg/dev.env"));
        assert!(env_path(f, Some("../x.env")).is_err());
        assert!(env_path(f, Some("/etc/passwd")).is_err());
    }

    #[test]
    fn scan_finds_nested_projects_and_skips_noise() {
        let root = tmp_dir("scan");
        for d in ["a", "b/c", "node_modules/x", ".hidden"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("a/compose.yaml"), "services: {}").unwrap();
        std::fs::write(root.join("b/c/docker-compose.yml"), "services: {}").unwrap();
        std::fs::write(root.join("node_modules/x/compose.yaml"), "").unwrap();
        std::fs::write(root.join(".hidden/compose.yaml"), "").unwrap();
        let found = scan_dir(&root, 3);
        let rel: Vec<_> = found
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
            .collect();
        assert_eq!(rel, ["a/compose.yaml", "b/c/docker-compose.yml"]);
        assert_eq!(scan_dir(&root, 1).len(), 1);
        assert_eq!(resolve_compose_file(&root.join("a").display().to_string()).unwrap(), root.join("a/compose.yaml"));
    }

    #[test]
    fn checked_write_refuses_stale_content() {
        let dir = tmp_dir("write");
        let f = dir.join("compose.yaml");
        let _ = std::fs::remove_file(&f);
        write_text_checked(&f, "one", None).unwrap();
        assert!(write_text_checked(&f, "two", None).is_err());
        assert!(write_text_checked(&f, "two", Some("zero")).is_err());
        write_text_checked(&f, "two", Some("one")).unwrap();
        assert_eq!(read_text(&f).unwrap().as_deref(), Some("two"));
        assert_eq!(read_text(&dir.join("missing")).unwrap(), None);
    }
}
