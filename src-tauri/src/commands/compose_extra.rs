//! Compose extras: pull/build, "image has a newer version" checks, merged
//! config, healthcheck details, project volumes (size, backup, restore),
//! export/import of a project, new projects from a template, and turning a
//! group of plain containers into a compose file.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::commands::compose::{cached_model, err, file_target, progress_sink};
use crate::docker::compose::{self, BuildOptions, PullOptions};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Pull and build
// ---------------------------------------------------------------------------

/// Profiles the named services sit behind: a service behind a profile can
/// only be addressed with that profile enabled.
async fn profiles_for(file: &Path, services: &[String]) -> Vec<String> {
    let Ok(model) = cached_model(file).await else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for spec in model.services.iter().filter(|s| services.contains(&s.name)) {
        for p in &spec.profiles {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
    }
    out
}

#[tauri::command]
pub async fn compose_pull(
    app: AppHandle,
    file_path: String,
    options: Option<PullOptions>,
    op_id: Option<String>,
) -> Result<String, String> {
    let opts = options.unwrap_or_default();
    let mut target = file_target(&file_path, None, Vec::new())?;
    if let Some(f) = target.file.clone() {
        target.profiles = profiles_for(&f, &opts.services).await;
    }
    let verb = compose::pull_args(&opts).map_err(err)?;
    let out = compose::run(target, verb, progress_sink(&app, op_id)).await.map_err(err)?;
    // What was just pulled is the newest there is.
    forget_image_checks();
    Ok(out)
}

#[tauri::command]
pub async fn compose_build(
    app: AppHandle,
    file_path: String,
    options: Option<BuildOptions>,
    op_id: Option<String>,
) -> Result<String, String> {
    let opts = options.unwrap_or_default();
    let mut target = file_target(&file_path, None, Vec::new())?;
    if let Some(f) = target.file.clone() {
        target.profiles = profiles_for(&f, &opts.services).await;
    }
    let verb = compose::build_args(&opts).map_err(err)?;
    compose::run(target, verb, progress_sink(&app, op_id)).await.map_err(err)
}

// ---------------------------------------------------------------------------
// Image update check
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImageUpdate {
    pub service: String,
    pub image: String,
    /// `update` (registry has a different digest), `current`, `missing`
    /// (not pulled yet), `local` (built here, nothing to compare) or `error`.
    pub status: String,
    pub local_digest: String,
    pub remote_digest: String,
    pub error: String,
}

/// Registry answers are reused this long: a project page refreshes often,
/// and registries (Docker Hub above all) rate-limit anonymous lookups.
const REMOTE_TTL: Duration = Duration::from_secs(15 * 60);
const REMOTE_TIMEOUT: Duration = Duration::from_secs(20);

type RemoteCache = HashMap<String, (Instant, Result<String, String>)>;
static REMOTE_DIGESTS: LazyLock<Mutex<RemoteCache>> = LazyLock::new(Default::default);

fn forget_image_checks() {
    if let Ok(mut c) = REMOTE_DIGESTS.lock() {
        c.clear();
    }
}

/// The registry's current digest for `image` (cached).
async fn remote_digest(state: &AppState, docker: &bollard::Docker, image: &str, force: bool) -> Result<String, String> {
    if !force {
        if let Some((at, v)) = REMOTE_DIGESTS.lock().ok().and_then(|c| c.get(image).cloned()) {
            if at.elapsed() < REMOTE_TTL {
                return v;
            }
        }
    }
    let host = crate::docker::models::registry_host_for_image(image);
    let creds = crate::commands::images::saved_credentials(state, &host).await;
    let v = match tokio::time::timeout(REMOTE_TIMEOUT, docker.inspect_registry_image(image, creds)).await {
        Err(_) => Err("registry did not answer in time".to_string()),
        Ok(Err(e)) => Err(short_registry_error(&e.to_string())),
        Ok(Ok(d)) => d.descriptor.digest.ok_or_else(|| "registry sent no digest".to_string()),
    };
    if let Ok(mut c) = REMOTE_DIGESTS.lock() {
        // Expired answers are never read again; dropping them here keeps the
        // map bounded by the images checked recently, not every image ever.
        let now = Instant::now();
        prune_remote_cache(&mut c, now);
        c.insert(image.to_string(), (now, v.clone()));
    }
    v
}

/// Removes registry answers older than [`REMOTE_TTL`] at `now`.
fn prune_remote_cache(c: &mut RemoteCache, now: Instant) {
    c.retain(|_, (at, _)| now.saturating_duration_since(*at) < REMOTE_TTL);
}

fn short_registry_error(e: &str) -> String {
    let lower = e.to_ascii_lowercase();
    if lower.contains("unauthorized") || lower.contains("denied") || lower.contains("401") {
        "registry refused the lookup (private image? add a login in Settings › Registries)".into()
    } else if lower.contains("not found") || lower.contains("manifest unknown") {
        "image not found in the registry".into()
    } else if lower.contains("toomanyrequests") || lower.contains("429") {
        "registry rate limit reached, try later".into()
    } else {
        e.lines().next().unwrap_or(e).chars().take(200).collect()
    }
}

/// Compares the digest a local image was pulled with to the registry's.
pub fn compare_digests(repo_digests: &[String], remote: &str) -> bool {
    repo_digests.iter().any(|d| d.rsplit_once('@').map(|(_, h)| h) == Some(remote))
}

/// For each service with an image: does the registry hold a newer one?
#[tauri::command]
pub async fn compose_image_updates(
    state: State<'_, Arc<AppState>>,
    file_path: String,
    force: Option<bool>,
) -> Result<Vec<ImageUpdate>, String> {
    let file = compose::resolve_compose_file(&file_path).map_err(err)?;
    let model = cached_model(&file).await?;
    let client = state.get_client().await?;
    let docker = client.docker().clone();
    let force = force.unwrap_or(false);
    let mut out = Vec::new();
    for spec in &model.services {
        if spec.image.is_empty() {
            continue;
        }
        let mut row = ImageUpdate {
            service: spec.name.clone(),
            image: spec.image.clone(),
            status: String::new(),
            local_digest: String::new(),
            remote_digest: String::new(),
            error: String::new(),
        };
        if spec.build {
            // Built here from the project's Dockerfile: no registry copy.
            row.status = "local".into();
            out.push(row);
            continue;
        }
        let local = match docker.inspect_image(&spec.image).await {
            Ok(i) => i.repo_digests.unwrap_or_default(),
            Err(_) => {
                row.status = "missing".into();
                out.push(row);
                continue;
            }
        };
        row.local_digest = local
            .first()
            .and_then(|d| d.rsplit_once('@').map(|(_, h)| h.to_string()))
            .unwrap_or_default();
        if local.is_empty() {
            row.status = "local".into();
            out.push(row);
            continue;
        }
        match remote_digest(&state, &docker, &spec.image, force).await {
            Ok(remote) => {
                row.status = if compare_digests(&local, &remote) { "current" } else { "update" }.into();
                row.remote_digest = remote;
            }
            Err(e) => {
                row.status = "error".into();
                row.error = e;
            }
        }
        out.push(row);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Merged config
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn compose_merged_config(file_path: String, env_file: Option<String>) -> Result<String, String> {
    let target = file_target(&file_path, env_file.as_deref(), Vec::new())?;
    compose::merged_yaml(target).await.map_err(err)
}

// ---------------------------------------------------------------------------
// Healthcheck details
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
pub struct HealthRun {
    pub start: String,
    pub end: String,
    pub exit_code: i64,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct HealthDetails {
    /// `healthy`, `unhealthy`, `starting`, or empty without a healthcheck.
    pub status: String,
    pub failing_streak: i64,
    /// The check command as configured (`CMD-SHELL curl …`).
    pub test: Vec<String>,
    pub interval_ms: i64,
    pub timeout_ms: i64,
    pub retries: i64,
    /// Most recent first.
    pub runs: Vec<HealthRun>,
}

#[tauri::command]
pub async fn container_health(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<HealthDetails, String> {
    let client = state.get_client().await?;
    let info = client
        .docker()
        .inspect_container(&id, None)
        .await
        .map_err(|e| e.to_string())?;
    let mut d = HealthDetails::default();
    if let Some(hc) = info.config.and_then(|c| c.healthcheck) {
        d.test = hc.test.unwrap_or_default();
        d.interval_ms = hc.interval.unwrap_or(0) / 1_000_000;
        d.timeout_ms = hc.timeout.unwrap_or(0) / 1_000_000;
        d.retries = hc.retries.unwrap_or(0);
    }
    if let Some(h) = info.state.and_then(|s| s.health) {
        d.status = h
            .status
            .and_then(|s| serde_json::to_value(s).ok())
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        d.failing_streak = h.failing_streak.unwrap_or(0);
        let mut runs: Vec<HealthRun> = h
            .log
            .unwrap_or_default()
            .into_iter()
            .map(|r| HealthRun {
                start: r.start.map(|t| t.to_string()).unwrap_or_default(),
                end: r.end.map(|t| t.to_string()).unwrap_or_default(),
                exit_code: r.exit_code.unwrap_or(-1),
                output: r.output.unwrap_or_default().trim_end().chars().take(4000).collect(),
            })
            .collect();
        runs.reverse();
        d.runs = runs;
    }
    Ok(d)
}

// ---------------------------------------------------------------------------
// Project volumes: size, backup, restore
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ProjectVolume {
    pub name: String,
    pub driver: String,
    pub size: Option<u64>,
    /// Containers (any state) that mount it.
    pub used_by: Vec<String>,
    /// Running containers that mount it: restore refuses while there are any.
    pub running_users: Vec<String>,
}

/// A container as the volume grouping sees it: its name, whether it runs, and
/// the named volumes it mounts (bind mounts have no name and are left out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MountedContainer {
    pub name: String,
    pub running: bool,
    pub volumes: Vec<String>,
}

/// Groups containers by the volumes they mount: volume name -> (every
/// container mounting it, the running ones). Pure, so the grouping is tested
/// without dockerd.
pub(crate) fn group_volume_users(containers: &[MountedContainer]) -> HashMap<String, (Vec<String>, Vec<String>)> {
    let mut out: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    for c in containers {
        // A container that mounts one volume twice is still one user.
        let unique: BTreeSet<&String> = c.volumes.iter().collect();
        for v in unique {
            let entry = out.entry(v.clone()).or_default();
            entry.0.push(c.name.clone());
            if c.running {
                entry.1.push(c.name.clone());
            }
        }
    }
    out
}

/// One `list_containers(all)` call, reduced to [`MountedContainer`]s.
async fn list_mounted(docker: &bollard::Docker) -> Result<Vec<MountedContainer>, String> {
    use bollard::query_parameters::ListContainersOptionsBuilder;
    let opts = ListContainersOptionsBuilder::default().all(true).build();
    let list = docker.list_containers(Some(opts)).await.map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|c| MountedContainer {
            name: c
                .names
                .and_then(|n| n.first().cloned())
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_string(),
            running: c.state.map(|s| format!("{s:?}").eq_ignore_ascii_case("running")).unwrap_or(false),
            volumes: c.mounts.unwrap_or_default().into_iter().filter_map(|m| m.name).collect(),
        })
        .collect())
}

/// Users of one volume: (every container mounting it, the running ones).
async fn volume_users(docker: &bollard::Docker, volume: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mounted = list_mounted(docker).await?;
    Ok(group_volume_users(&mounted).remove(volume).unwrap_or_default())
}

#[tauri::command]
pub async fn compose_project_volumes(
    state: State<'_, Arc<AppState>>,
    project: String,
) -> Result<Vec<ProjectVolume>, String> {
    compose::check_project_name(project.trim()).map_err(err)?;
    let client = state.get_client().await?;
    let docker = client.docker().clone();
    let mut filters = HashMap::new();
    filters.insert(
        "label".to_string(),
        vec![format!("com.docker.compose.project={}", project.trim())],
    );
    let opts = bollard::query_parameters::ListVolumesOptionsBuilder::default()
        .filters(&filters)
        .build();
    let listed = docker.list_volumes(Some(opts)).await.map_err(|e| e.to_string())?;
    let names: Vec<(String, String)> = listed
        .volumes
        .unwrap_or_default()
        .into_iter()
        .map(|v| (v.name, v.driver))
        .collect();
    // Sizes come from the shared (cached) volume walk.
    let sizes: HashMap<String, Option<u64>> = crate::commands::volumes::list_volumes_core(&state)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|v| (v.name, v.size))
        .collect();
    // One container listing for all volumes (was one per volume).
    let users = group_volume_users(&list_mounted(&docker).await.unwrap_or_default());
    let mut out = Vec::new();
    for (name, driver) in names {
        let (used_by, running_users) = users.get(&name).cloned().unwrap_or_default();
        out.push(ProjectVolume {
            size: sizes.get(&name).copied().flatten(),
            name,
            driver,
            used_by,
            running_users,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Small image the backup/restore helper container runs.
const HELPER_IMAGE: &str = "busybox:1.36";
const BACKUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);

fn expand_home(raw: &str) -> PathBuf {
    let t = raw.trim();
    if let Some(rest) = t.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest);
        }
    } else if t == "~" {
        return dirs::home_dir().unwrap_or_default();
    }
    PathBuf::from(t)
}

/// Where backups and exports go when the user names no folder.
pub fn default_backup_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("hsx2dock-backups")
}

fn check_volume_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.len() <= 255
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !name.starts_with(['.', '-']);
    if ok { Ok(()) } else { Err(format!("invalid volume name '{name}'")) }
}

fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Runs the helper container through the bundled CLI (which also pulls the
/// helper image on first use), streaming its output to `sink`.
async fn run_helper(args: Vec<String>, sink: Option<compose::LineSink>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let paths = crate::docker::engine::EnginePaths::new();
        let mut cmd = crate::docker::engine::docker_command_resolved(&paths);
        cmd.args(&args);
        if let Some(s) = &sink {
            s(&format!("$ docker {}", args.join(" ")));
        }
        let out = crate::procutil::output_with_timeout(cmd, BACKUP_TIMEOUT, "docker run")
            .map_err(|e| e.to_string())?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if let Some(s) = &sink {
            for l in text.lines() {
                s(l);
            }
        }
        if out.status.success() {
            Ok(text)
        } else {
            Err(format!("helper container failed: {}", text.lines().rev().take(3).collect::<Vec<_>>().join(" | ")))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Archives a volume to `<dest>/<volume>-<time>.tar.gz`. Returns the file.
#[tauri::command]
pub async fn volume_backup(
    app: AppHandle,
    volume: String,
    dest_dir: Option<String>,
    op_id: Option<String>,
) -> Result<String, String> {
    check_volume_name(&volume)?;
    let dir = match dest_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => expand_home(d),
        None => default_backup_dir().join("volumes"),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    let file = format!("{volume}-{}.tar.gz", timestamp());
    let args: Vec<String> = vec![
        "run".into(),
        "--rm".into(),
        "--network".into(),
        "none".into(),
        "-v".into(),
        format!("{volume}:/data:ro"),
        "-v".into(),
        format!("{}:/backup", dir.display()),
        HELPER_IMAGE.into(),
        "tar".into(),
        "czf".into(),
        format!("/backup/{file}"),
        "-C".into(),
        "/data".into(),
        ".".into(),
    ];
    run_helper(args, progress_sink(&app, op_id)).await?;
    Ok(dir.join(file).display().to_string())
}

/// Replaces a volume's content with an archive made by [`volume_backup`].
/// Refused while a running container uses the volume.
#[tauri::command]
pub async fn volume_restore(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    volume: String,
    archive: String,
    op_id: Option<String>,
) -> Result<(), String> {
    check_volume_name(&volume)?;
    let archive = expand_home(&archive);
    if !archive.is_file() {
        return Err(format!("archive not found: {}", archive.display()));
    }
    let archive = archive.canonicalize().map_err(|e| e.to_string())?;
    let (dir, name) = match (archive.parent(), archive.file_name().and_then(|n| n.to_str())) {
        (Some(d), Some(n)) => (d.to_path_buf(), n.to_string()),
        _ => return Err("archive path has no folder or name".into()),
    };
    if name.contains('\'') {
        return Err("archive name must not contain quotes".into());
    }
    let client = state.get_client().await?;
    // A failed listing must stop the restore: an unknown running set is not "none running".
    let (_, running) = volume_users(client.docker(), &volume).await?;
    if !running.is_empty() {
        return Err(format!("stop the containers using {volume} first: {}", running.join(", ")));
    }
    let script = format!(
        "find /data -mindepth 1 -maxdepth 1 -exec rm -rf {{}} + && tar xzf '/backup/{name}' -C /data"
    );
    let args: Vec<String> = vec![
        "run".into(),
        "--rm".into(),
        "--network".into(),
        "none".into(),
        "-v".into(),
        format!("{volume}:/data"),
        "-v".into(),
        format!("{}:/backup:ro", dir.display()),
        HELPER_IMAGE.into(),
        "sh".into(),
        "-c".into(),
        script,
    ];
    run_helper(args, progress_sink(&app, op_id)).await.map(|_| ())
}

// ---------------------------------------------------------------------------
// Export / import
// ---------------------------------------------------------------------------

const MANIFEST: &str = "hsx2dock-project.json";
const MAX_IMPORT_FILES: usize = 20;
const MAX_IMPORT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExportManifest {
    pub name: String,
    /// The base compose file name inside the archive.
    pub compose_file: String,
    pub files: Vec<String>,
    pub env_masked: bool,
}

/// Env keys whose values are treated as secrets when exporting.
pub fn is_secret_key(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    ["PASS", "SECRET", "TOKEN", "KEY", "PRIVATE", "CREDENTIAL", "AUTH"]
        .iter()
        .any(|w| k.contains(w))
}

/// Blanks the value of every secret-looking `KEY=value` line, keeping
/// comments and layout.
pub fn mask_env(text: &str) -> String {
    text.lines()
        .map(|line| {
            let t = line.trim_start();
            if t.starts_with('#') {
                return line.to_string();
            }
            let body = t.strip_prefix("export ").unwrap_or(t);
            match body.split_once('=') {
                Some((k, _)) if is_secret_key(k.trim()) => {
                    let indent = &line[..line.len() - t.len()];
                    let export = if t.starts_with("export ") { "export " } else { "" };
                    format!("{indent}{export}{}=", k.trim_end())
                }
                _ => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if text.ends_with('\n') { "\n" } else { "" }
}

/// `KEY=value` pairs of an env file (comments and blank lines skipped;
/// surrounding quotes removed). Later duplicates win, as in compose.
pub fn env_entries(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let body = t.strip_prefix("export ").unwrap_or(t);
        let Some((k, v)) = body.split_once('=') else { continue };
        let k = k.trim().to_string();
        let v = v.trim();
        let v = if v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\''))) {
            v[1..v.len() - 1].to_string()
        } else {
            v.to_string()
        };
        out.retain(|(x, _)| x != &k);
        out.push((k, v));
    }
    out
}

fn env_value(v: &str) -> String {
    if v.is_empty() || v.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:@,+%".contains(c)) {
        v.to_string()
    } else {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
    }
}

/// Sets and removes keys in an env file, keeping comments, order and the
/// lines it does not touch. New keys are appended.
pub fn env_apply(text: &str, set: &BTreeMap<String, String>, unset: &[String]) -> Result<String, String> {
    for k in set.keys().chain(unset.iter()) {
        let ok = !k.is_empty()
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !k.starts_with(|c: char| c.is_ascii_digit());
        if !ok {
            return Err(format!("invalid env key '{k}'"));
        }
    }
    let mut done: Vec<&String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        let body = t.strip_prefix("export ").unwrap_or(t);
        let key = if t.starts_with('#') { None } else { body.split_once('=').map(|(k, _)| k.trim().to_string()) };
        match key {
            Some(k) if unset.contains(&k) => continue,
            Some(k) => match set.get_key_value(&k) {
                Some((key, v)) => {
                    let export = if t.starts_with("export ") { "export " } else { "" };
                    lines.push(format!("{export}{k}={}", env_value(v)));
                    done.push(key);
                }
                None => lines.push(line.to_string()),
            },
            None => lines.push(line.to_string()),
        }
    }
    for (k, v) in set {
        if !done.contains(&k) {
            lines.push(format!("{k}={}", env_value(v)));
        }
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

fn add_entry<W: std::io::Write>(tar: &mut tar::Builder<W>, name: &str, data: &[u8]) -> Result<(), String> {
    let mut h = tar::Header::new_gnu();
    h.set_size(data.len() as u64);
    h.set_mode(0o644);
    h.set_mtime(chrono::Utc::now().timestamp().max(0) as u64);
    h.set_cksum();
    tar.append_data(&mut h, name, data).map_err(|e| e.to_string())
}

/// Packs the compose file, its override and (optionally) `.env` into
/// `<dest>/<project>-<time>.tar.gz`. Returns the archive path.
#[tauri::command]
pub async fn compose_export(
    file_path: String,
    dest_dir: Option<String>,
    include_env: bool,
    mask_secrets: bool,
) -> Result<String, String> {
    let file = compose::resolve_compose_file(&file_path).map_err(err)?;
    let name = cached_model(&file)
        .await
        .ok()
        .map(|m| m.name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            file.parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "project".into())
        });
    let dir = match dest_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => expand_home(d),
        None => default_backup_dir().join("projects"),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let base_name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    entries.push((base_name.clone(), std::fs::read(&file).map_err(|e| e.to_string())?));
    if let Some(o) = compose::override_file_for(&file) {
        let n = o.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        entries.push((n, std::fs::read(&o).map_err(|e| e.to_string())?));
    }
    if include_env {
        if let Ok(env) = compose::env_path(&file, None) {
            if let Ok(Some(text)) = compose::read_text(&env) {
                let text = if mask_secrets { mask_env(&text) } else { text };
                entries.push((".env".into(), text.into_bytes()));
            }
        }
    }
    let manifest = ExportManifest {
        name: name.clone(),
        compose_file: base_name,
        files: entries.iter().map(|(n, _)| n.clone()).collect(),
        env_masked: include_env && mask_secrets,
    };
    let out_path = dir.join(format!("{name}-{}.tar.gz", timestamp()));
    let f = std::fs::File::create(&out_path).map_err(|e| format!("cannot write {}: {e}", out_path.display()))?;
    let gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    add_entry(&mut tar, MANIFEST, &serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?)?;
    for (n, data) in &entries {
        add_entry(&mut tar, n, data)?;
    }
    tar.into_inner()
        .and_then(|gz| gz.finish())
        .map_err(|e| format!("cannot finish the archive: {e}"))?;
    Ok(out_path.display().to_string())
}

/// A file name an import may create: one plain component.
fn safe_entry_name(name: &str) -> bool {
    let p = Path::new(name);
    let mut comps = p.components();
    matches!(comps.next(), Some(std::path::Component::Normal(_))) && comps.next().is_none() && !name.contains('\0')
}

/// Reads an export archive: (manifest, files). Only plain files with simple
/// names are accepted; anything else rejects the whole archive.
/// Files of an export: (name, bytes).
pub type ExportFiles = Vec<(String, Vec<u8>)>;

pub fn read_export(archive: &Path) -> Result<(ExportManifest, ExportFiles), String> {
    let f = std::fs::File::open(archive).map_err(|e| format!("cannot open {}: {e}", archive.display()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    let mut manifest: Option<ExportManifest> = None;
    let mut files = Vec::new();
    for entry in tar.entries().map_err(|e| format!("not a project export: {e}"))? {
        let mut entry = entry.map_err(|e| format!("damaged archive: {e}"))?;
        if !entry.header().entry_type().is_file() {
            return Err("the archive holds something other than plain files".into());
        }
        let name = entry.path().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
        if !safe_entry_name(&name) {
            return Err(format!("unsafe file name in archive: {name}"));
        }
        if entry.size() > MAX_IMPORT_BYTES {
            return Err(format!("{name} is too large"));
        }
        let mut data = Vec::new();
        entry
            .by_ref()
            .take(MAX_IMPORT_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        if name == MANIFEST {
            manifest = Some(serde_json::from_slice(&data).map_err(|e| format!("bad manifest: {e}"))?);
        } else {
            files.push((name, data));
        }
        if files.len() > MAX_IMPORT_FILES {
            return Err("too many files in the archive".into());
        }
    }
    let manifest = manifest.ok_or("not a project export (no manifest)")?;
    if !files.iter().any(|(n, _)| n == &manifest.compose_file) {
        return Err("the archive is missing its compose file".into());
    }
    Ok((manifest, files))
}

/// Unpacks an export into `<dest_dir>/<project>` (which must not hold a
/// compose project already) and pins it. Returns the compose file.
#[tauri::command]
pub async fn compose_import(
    state: State<'_, Arc<AppState>>,
    archive: String,
    dest_dir: String,
) -> Result<String, String> {
    let (manifest, files) = read_export(&expand_home(&archive))?;
    let folder = if manifest.name.is_empty() { "project".to_string() } else { manifest.name.clone() };
    if !safe_entry_name(&folder) {
        return Err("bad project name in manifest".into());
    }
    let dir = expand_home(&dest_dir).join(&folder);
    if compose::compose_file_in(&dir).is_some() {
        return Err(format!("{} already holds a compose project", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for (name, data) in &files {
        let p = dir.join(name);
        if p.exists() {
            return Err(format!("{} already exists", p.display()));
        }
        std::fs::write(&p, data).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
    }
    let compose_file = dir.join(&manifest.compose_file);
    crate::commands::compose::compose_pin_core(&state, &compose_file.display().to_string(), true).await
}

// ---------------------------------------------------------------------------
// New project (template) and group → compose
// ---------------------------------------------------------------------------

/// Writes a new `compose.yaml` (and optional `.env`) into `dir`, validates it
/// and pins it. Refuses a folder that already holds a compose file.
#[tauri::command]
pub async fn compose_create_project(
    state: State<'_, Arc<AppState>>,
    dir: String,
    content: String,
    env: Option<String>,
) -> Result<String, String> {
    let dir = expand_home(&dir);
    if dir.as_os_str().is_empty() {
        return Err("choose a folder".into());
    }
    if compose::compose_file_in(&dir).is_some() {
        return Err(format!("{} already holds a compose file", dir.display()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let file = dir.join("compose.yaml");
    let env_text = env.filter(|e| !e.trim().is_empty());
    let env_path = dir.join(".env");
    if env_text.is_some() && env_path.exists() {
        return Err(format!("{} already exists", env_path.display()));
    }
    // `.env` first: the compose file may interpolate from it.
    if let Some(e) = &env_text {
        compose::write_text_checked(&env_path, e, None).map_err(err)?;
    }
    if let Err(e) = compose::validate_text(&file, content.clone(), None).await {
        if env_text.is_some() {
            let _ = std::fs::remove_file(&env_path);
        }
        return Err(format!("the file does not validate: {}", err(e)));
    }
    compose::write_text_checked(&file, &content, None).map_err(err)?;
    crate::commands::compose::compose_pin_core(&state, &file.display().to_string(), true).await
}

/// A YAML double-quoted scalar (JSON string syntax is valid YAML).
fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// A compose service name from a container name.
pub fn service_name(container: &str) -> String {
    let mut s: String = container
        .trim_start_matches('/')
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    s = s.trim_matches('-').to_string();
    if s.is_empty() || !s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
        s = format!("svc-{s}");
    }
    s
}

/// What the generator needs from one container.
#[derive(Debug, Clone, Default)]
pub struct ContainerShape {
    pub name: String,
    pub image: String,
    pub command: Vec<String>,
    pub entrypoint: Vec<String>,
    pub env: Vec<String>,
    /// "host_ip:host_port:container_port/proto" style, already formatted.
    pub ports: Vec<String>,
    /// (source or volume name, target, read_only, is_named_volume)
    pub mounts: Vec<(String, String, bool, bool)>,
    pub restart: String,
    pub networks: Vec<String>,
    pub labels: BTreeMap<String, String>,
    pub working_dir: String,
    pub user: String,
}

/// Renders a compose file for `containers`. Named volumes and user networks
/// are declared `external`, so the project reuses the existing data and
/// networks instead of creating empty ones.
pub fn render_compose(project: &str, containers: &[ContainerShape]) -> String {
    let mut out = String::new();
    out.push_str(&format!("name: {}\n\nservices:\n", q(project)));
    let mut volumes: Vec<String> = Vec::new();
    let mut networks: Vec<String> = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for c in containers {
        let mut svc = service_name(&c.name);
        while used.contains(&svc) {
            svc.push_str("-2");
        }
        used.push(svc.clone());
        out.push_str(&format!("  {svc}:\n    image: {}\n", q(&c.image)));
        out.push_str(&format!("    container_name: {}\n", q(c.name.trim_start_matches('/'))));
        let list = |out: &mut String, key: &str, items: &[String]| {
            if !items.is_empty() {
                out.push_str(&format!("    {key}: [{}]\n", items.iter().map(|i| q(i)).collect::<Vec<_>>().join(", ")));
            }
        };
        list(&mut out, "entrypoint", &c.entrypoint);
        list(&mut out, "command", &c.command);
        if !c.working_dir.is_empty() {
            out.push_str(&format!("    working_dir: {}\n", q(&c.working_dir)));
        }
        if !c.user.is_empty() {
            out.push_str(&format!("    user: {}\n", q(&c.user)));
        }
        if !c.restart.is_empty() && c.restart != "no" {
            out.push_str(&format!("    restart: {}\n", q(&c.restart)));
        }
        if !c.env.is_empty() {
            out.push_str("    environment:\n");
            for e in &c.env {
                out.push_str(&format!("      - {}\n", q(e)));
            }
        }
        if !c.ports.is_empty() {
            out.push_str("    ports:\n");
            for p in &c.ports {
                out.push_str(&format!("      - {}\n", q(p)));
            }
        }
        if !c.mounts.is_empty() {
            out.push_str("    volumes:\n");
            for (src, dst, ro, named) in &c.mounts {
                if *named && !volumes.contains(src) {
                    volumes.push(src.clone());
                }
                let spec = format!("{src}:{dst}{}", if *ro { ":ro" } else { "" });
                out.push_str(&format!("      - {}\n", q(&spec)));
            }
        }
        if !c.networks.is_empty() {
            out.push_str("    networks:\n");
            for n in &c.networks {
                if !networks.contains(n) {
                    networks.push(n.clone());
                }
                out.push_str(&format!("      - {}\n", q(n)));
            }
        }
        if !c.labels.is_empty() {
            out.push_str("    labels:\n");
            for (k, v) in &c.labels {
                out.push_str(&format!("      {}: {}\n", q(k), q(v)));
            }
        }
    }
    if !volumes.is_empty() {
        out.push_str("\nvolumes:\n");
        for v in &volumes {
            out.push_str(&format!("  {}:\n    external: true\n", q(v)));
        }
    }
    if !networks.is_empty() {
        out.push_str("\nnetworks:\n");
        for n in &networks {
            out.push_str(&format!("  {}:\n    external: true\n", q(n)));
        }
    }
    out
}

/// Reads containers into [`ContainerShape`]s, leaving out what the image
/// already sets (env, command, labels) so the file stays short.
async fn shapes(docker: &bollard::Docker, ids: &[String]) -> Result<Vec<ContainerShape>, String> {
    let mut out = Vec::new();
    for id in ids {
        let c = docker.inspect_container(id, None).await.map_err(|e| format!("{id}: {e}"))?;
        let cfg = c.config.clone().unwrap_or_default();
        let image = cfg.image.clone().unwrap_or_default();
        let img_cfg = docker.inspect_image(&image).await.ok().and_then(|i| i.config).unwrap_or_default();
        let img_env = img_cfg.env.unwrap_or_default();
        let img_labels = img_cfg.labels.unwrap_or_default();
        let mut shape = ContainerShape {
            name: c.name.clone().unwrap_or_default().trim_start_matches('/').to_string(),
            image,
            ..Default::default()
        };
        let cmd = cfg.cmd.clone().unwrap_or_default();
        if cmd != img_cfg.cmd.unwrap_or_default() {
            shape.command = cmd;
        }
        let ep = cfg.entrypoint.clone().unwrap_or_default();
        if ep != img_cfg.entrypoint.unwrap_or_default() {
            shape.entrypoint = ep;
        }
        shape.env = cfg.env.clone().unwrap_or_default().into_iter().filter(|e| !img_env.contains(e)).collect();
        let wd = cfg.working_dir.clone().unwrap_or_default();
        if wd != img_cfg.working_dir.unwrap_or_default() {
            shape.working_dir = wd;
        }
        let user = cfg.user.clone().unwrap_or_default();
        if user != img_cfg.user.unwrap_or_default() {
            shape.user = user;
        }
        shape.labels = cfg
            .labels
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|(k, v)| !k.starts_with("com.docker.") && !k.starts_with("org.opencontainers.") && img_labels.get(k) != Some(v))
            .collect();
        if let Some(hc) = &c.host_config {
            if let Some(rp) = hc.restart_policy.as_ref().and_then(|r| r.name) {
                shape.restart = serde_json::to_value(rp).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            }
            let mut ports: Vec<String> = Vec::new();
            for (cport, binds) in hc.port_bindings.clone().unwrap_or_default() {
                for b in binds.unwrap_or_default() {
                    let host = b.host_port.unwrap_or_default();
                    if host.is_empty() {
                        continue;
                    }
                    let ip = b.host_ip.unwrap_or_default();
                    let (num, proto) = cport.split_once('/').unwrap_or((cport.as_str(), "tcp"));
                    let ip_part = if ip.is_empty() || ip == "0.0.0.0" || ip == "::" { String::new() } else { format!("{ip}:") };
                    let suffix = if proto == "tcp" { String::new() } else { format!("/{proto}") };
                    let spec = format!("{ip_part}{host}:{num}{suffix}");
                    if !ports.contains(&spec) {
                        ports.push(spec);
                    }
                }
            }
            ports.sort();
            shape.ports = ports;
        }
        for m in c.mounts.clone().unwrap_or_default() {
            let kind = m.typ.and_then(|t| serde_json::to_value(t).ok()).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            let dst = m.destination.unwrap_or_default();
            let ro = m.rw == Some(false);
            match kind.as_str() {
                "volume" => {
                    if let Some(n) = m.name.filter(|n| !n.is_empty()) {
                        // Anonymous volumes (64-hex names) start fresh instead.
                        if !(n.len() == 64 && n.chars().all(|c| c.is_ascii_hexdigit())) {
                            shape.mounts.push((n, dst, ro, true));
                        }
                    }
                }
                "bind" => {
                    if let Some(src) = m.source.filter(|s| !s.is_empty()) {
                        shape.mounts.push((src, dst, ro, false));
                    }
                }
                _ => {}
            }
        }
        if let Some(nets) = c.network_settings.as_ref().and_then(|n| n.networks.clone()) {
            let mut names: Vec<String> = nets
                .into_keys()
                .filter(|n| !matches!(n.as_str(), "bridge" | "host" | "none"))
                .collect();
            names.sort();
            shape.networks = names;
        }
        out.push(shape);
    }
    Ok(out)
}

/// A compose file describing the given containers, for review before it is
/// saved as a project.
#[tauri::command]
pub async fn compose_from_containers(
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    project: String,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err("no containers selected".into());
    }
    compose::check_project_name(project.trim()).map_err(err)?;
    let client = state.get_client().await?;
    let shapes = shapes(client.docker(), &ids).await?;
    Ok(render_compose(project.trim(), &shapes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_compare_on_the_hash() {
        let local = vec!["nginx@sha256:aaa".to_string(), "docker.io/library/nginx@sha256:bbb".to_string()];
        assert!(compare_digests(&local, "sha256:bbb"));
        assert!(!compare_digests(&local, "sha256:ccc"));
        assert!(!compare_digests(&[], "sha256:aaa"));
    }

    #[test]
    fn env_masking_blanks_secrets_only() {
        let text = "# db\nDB_PASSWORD=hunter2\nexport API_TOKEN=abc\nPORT=8080\n  SECRET_KEY = x\n";
        let masked = mask_env(text);
        assert_eq!(masked, "# db\nDB_PASSWORD=\nexport API_TOKEN=\nPORT=8080\n  SECRET_KEY=\n");
        assert!(is_secret_key("aws_secret_access_key"));
        assert!(!is_secret_key("PORT"));
    }

    #[test]
    fn remote_cache_drops_expired_answers_only() {
        let base = Instant::now();
        let mut c: RemoteCache = HashMap::new();
        c.insert("old".into(), (base, Ok("sha256:a".into())));
        c.insert("fresh".into(), (base + REMOTE_TTL, Err("x".into())));
        prune_remote_cache(&mut c, base + REMOTE_TTL + Duration::from_secs(1));
        assert!(!c.contains_key("old"));
        assert!(c.contains_key("fresh"));
    }

    #[test]
    fn env_apply_keeps_layout() {
        let text = "# app\nexport A=1\nB=\"x y\"\nC=3\n";
        let mut set = BTreeMap::new();
        set.insert("A".to_string(), "2".to_string());
        set.insert("D".to_string(), "has space".to_string());
        let out = env_apply(text, &set, &["C".to_string()]).unwrap();
        assert_eq!(out, "# app\nexport A=2\nB=\"x y\"\nD=\"has space\"\n");
        assert_eq!(
            env_entries(&out),
            vec![("A".into(), "2".into()), ("B".into(), "x y".into()), ("D".into(), "has space".into())]
        );
        assert!(env_apply("", &BTreeMap::from([("1X".to_string(), String::new())]), &[]).is_err());
    }

    #[test]
    fn import_names_must_be_plain() {
        assert!(safe_entry_name("compose.yaml"));
        assert!(safe_entry_name(".env"));
        assert!(!safe_entry_name("../x"));
        assert!(!safe_entry_name("/etc/passwd"));
        assert!(!safe_entry_name("a/b"));
        assert!(!safe_entry_name(""));
    }

    #[test]
    fn export_round_trips_through_read_export() {
        let dir = std::env::temp_dir().join(format!("hsx2dock-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.tar.gz");
        let f = std::fs::File::create(&path).unwrap();
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(f, flate2::Compression::default()));
        let m = ExportManifest { name: "shop".into(), compose_file: "compose.yaml".into(), files: vec!["compose.yaml".into()], env_masked: false };
        add_entry(&mut tar, MANIFEST, &serde_json::to_vec(&m).unwrap()).unwrap();
        add_entry(&mut tar, "compose.yaml", b"services: {}\n").unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let (got, files) = read_export(&path).unwrap();
        assert_eq!(got, m);
        assert_eq!(files, vec![("compose.yaml".to_string(), b"services: {}\n".to_vec())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn service_names_are_compose_safe() {
        assert_eq!(service_name("/My.App_1"), "my-app_1");
        assert_eq!(service_name("_x"), "svc-_x");
    }

    #[test]
    fn renders_external_volumes_and_networks() {
        let c = ContainerShape {
            name: "web".into(),
            image: "nginx:1.27".into(),
            env: vec!["A=1".into()],
            ports: vec!["8080:80".into()],
            mounts: vec![("data".into(), "/usr/share/nginx/html".into(), true, true)],
            restart: "unless-stopped".into(),
            networks: vec!["shared".into()],
            ..Default::default()
        };
        let y = render_compose("site", &[c]);
        assert!(y.starts_with("name: \"site\"\n"));
        assert!(y.contains("  web:\n    image: \"nginx:1.27\"\n"));
        assert!(y.contains("      - \"data:/usr/share/nginx/html:ro\"\n"));
        assert!(y.contains("    restart: \"unless-stopped\"\n"));
        assert!(y.contains("\nvolumes:\n  \"data\":\n    external: true\n"));
        assert!(y.contains("\nnetworks:\n  \"shared\":\n    external: true\n"));
    }

    fn mounted(name: &str, running: bool, volumes: &[&str]) -> MountedContainer {
        MountedContainer {
            name: name.into(),
            running,
            volumes: volumes.iter().map(|v| v.to_string()).collect(),
        }
    }

    #[test]
    fn volume_users_group_all_and_running_users_per_volume() {
        let containers = vec![
            mounted("db", true, &["pgdata"]),
            mounted("backup", false, &["pgdata", "dumps"]),
            mounted("web", true, &[]),
            mounted("twice", true, &["dumps", "dumps"]),
        ];
        let g = group_volume_users(&containers);
        assert_eq!(g["pgdata"], (vec!["db".to_string(), "backup".to_string()], vec!["db".to_string()]));
        // Mounted twice by one container: listed once.
        assert_eq!(g["dumps"], (vec!["backup".to_string(), "twice".to_string()], vec!["twice".to_string()]));
        // Unmounted volumes have no entry; callers default to empty lists.
        assert!(!g.contains_key("nothing"));
        assert!(group_volume_users(&[]).is_empty());
    }
}
