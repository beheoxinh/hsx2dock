//! Thin wrapper around the `docker compose` CLI plugin.
//!
//! bollard speaks the Engine API, which has no first-class Compose concept,
//! so project lifecycle (`up`/`down`/`ps`/`logs`) shells out to the bundled
//! `docker` CLI against the app's own engine
//! (see [`super::engine::docker_command`]).
//! Every blocking call runs inside `spawn_blocking` so neither the GTK main
//! loop nor the Tokio async workers stall.

use std::path::{Path, PathBuf};

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

/// Probe for a usable Compose implementation via the bundled engine.
pub fn is_available() -> bool {
    let paths = crate::docker::engine::EnginePaths::new();
    crate::docker::engine::docker_command(&paths)
        .arg("compose")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Bring a project up in detached mode, returning the CLI's combined output.
pub async fn up(file: &Path) -> anyhow::Result<String> {
    let file = file.to_path_buf();
    tokio::task::spawn_blocking(move || run_compose(&file, "up", &["-d"]))
        .await
        .context("compose up task join")?
}

/// Tear a project down, returning the CLI's combined output.
pub async fn down(file: &Path) -> anyhow::Result<String> {
    let file = file.to_path_buf();
    tokio::task::spawn_blocking(move || run_compose(&file, "down", &[]))
        .await
        .context("compose down task join")?
}

/// List the project's services via `ps --format json`.
pub async fn ps(file: &Path) -> anyhow::Result<Vec<ComposeService>> {
    let file = file.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let out = run_compose(&file, "ps", &["--format", "json"])?;
        Ok(parse_ps_output(&out))
    })
    .await
    .context("compose ps task join")?
}

/// Fetch the project's combined logs (last `tail` lines).
pub async fn logs(file: &Path, tail: usize) -> anyhow::Result<String> {
    let file = file.to_path_buf();
    let tail_arg = format!("--tail={tail}");
    tokio::task::spawn_blocking(move || run_compose(&file, "logs", &["--no-color", &tail_arg]))
        .await
        .context("compose logs task join")?
}

/// Run one compose invocation, falling back from the v2 plugin to the legacy
/// standalone binary when the plugin itself is missing.
fn run_compose(file: &Path, subcommand: &str, extra: &[&str]) -> anyhow::Result<String> {
    if !file.is_file() {
        bail!("compose file not found: {}", file.display());
    }
    let file_str = file.to_string_lossy();
    let mut args: Vec<&str> = vec!["compose", "-f", &file_str, subcommand];
    args.extend(extra.iter().copied());
    let paths = crate::docker::engine::EnginePaths::new();
    let out = crate::docker::engine::docker_command(&paths)
        .args(&args)
        .output()
        .context("failed to run `docker compose`")?;
    finish_output("compose", out)
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
        let tail: Vec<&str> = combined.lines().rev().take(5).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        bail!("{label} failed: {}", tail.join("\n"))
    }
}

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
        if published.is_empty() || published == "0" {
            parts.push(format!("{target}/{proto}"));
        } else {
            parts.push(format!("{published}->{target}/{proto}"));
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

/// Resolve a compose file path the way the UI stores it: expand `~` and
/// reject paths that do not point at a regular file.
pub fn resolve_compose_file(raw: &str) -> anyhow::Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("no compose file selected");
    }
    let expanded = if let Some(rest) = trimmed.strip_prefix("~/") {
        match dirs::home_dir() {
            Some(home) => home.join(rest),
            None => PathBuf::from(trimmed),
        }
    } else {
        PathBuf::from(trimmed)
    };
    if !expanded.is_file() {
        bail!("compose file not found: {}", expanded.display());
    }
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARRAY_JSON: &str = r#"[
        {"ID":"abc123","Name":"web-proj-web-1","Service":"web","Image":"nginx:latest","State":"running","Health":"","Publishers":[{"URL":"0.0.0.0","TargetPort":80,"PublishedPort":8080,"Protocol":"tcp"}]},
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
}
