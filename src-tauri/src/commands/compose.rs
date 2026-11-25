//! Compose projects: discovery (settings file, pinned files, container
//! labels), lifecycle with streamed progress, per-service actions, and the
//! compose/.env editors.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::docker::compose::{self, ComposeModel, DownOptions, LineSink, Target, UpOptions};
use crate::state::AppState;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ComposeProjectInfo {
    pub name: String,
    /// The compose file, when known (empty for label-only projects whose
    /// file is gone or was never recorded).
    pub path: String,
    /// One row per container.
    pub services: Vec<compose::ComposeService>,
    /// Where the project came from: `settings`, `pinned` or `labels`.
    #[serde(default)]
    pub source: String,
    /// Saved in the app's project list.
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub working_dir: String,
    /// Services as the file declares them (every profile included).
    #[serde(default)]
    pub declared: Vec<compose::ServiceSpec>,
    #[serde(default)]
    pub profiles: Vec<String>,
    /// The project's default `.env`, when it exists.
    #[serde(default)]
    pub env_file: String,
    /// Why the file could not be read or resolved, if it could not.
    #[serde(default)]
    pub config_error: String,
}

/// One streamed output line of a running compose operation.
#[derive(Clone, Serialize)]
struct ProgressLine {
    op_id: String,
    line: String,
}

/// A sink that forwards each line as a `compose-progress` event, or none
/// when the caller did not ask for progress.
fn progress_sink(app: &AppHandle, op_id: Option<String>) -> Option<LineSink> {
    let op_id = op_id.filter(|id| !id.is_empty())?;
    let app = app.clone();
    Some(Box::new(move |line: &str| {
        let _ = app.emit(
            "compose-progress",
            ProgressLine { op_id: op_id.clone(), line: line.to_string() },
        );
    }))
}

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

type ModelKey = (PathBuf, Option<SystemTime>, Option<SystemTime>);

/// `compose config` costs a CLI round trip, so each file's model is kept
/// until the file or its `.env` changes.
type ModelCache = HashMap<PathBuf, (ModelKey, Result<ComposeModel, String>)>;
static MODEL_CACHE: LazyLock<Mutex<ModelCache>> = LazyLock::new(Default::default);

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

async fn cached_model(file: &Path) -> Result<ComposeModel, String> {
    let env = compose::env_path(file, None).ok();
    let key: ModelKey = (file.to_path_buf(), mtime(file), env.as_deref().and_then(mtime));
    if let Some((k, v)) = MODEL_CACHE.lock().ok().and_then(|c| c.get(file).cloned()) {
        if k == key {
            return v;
        }
    }
    let v = compose::config(file, None).await.map_err(err);
    if let Ok(mut c) = MODEL_CACHE.lock() {
        c.insert(file.to_path_buf(), (key, v.clone()));
    }
    v
}

fn dir_name(file: &Path) -> String {
    file.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("default")
        .to_string()
}

/// Fill in what the compose file says about a project.
async fn describe_file(file: &Path, info: &mut ComposeProjectInfo) {
    info.path = file.display().to_string();
    info.working_dir = file.parent().map(|p| p.display().to_string()).unwrap_or_default();
    if let Ok(env) = compose::env_path(file, None) {
        if env.is_file() {
            info.env_file = env.display().to_string();
        }
    }
    match cached_model(file).await {
        Ok(model) => {
            if info.name.is_empty() {
                info.name = if model.name.is_empty() { dir_name(file) } else { model.name.clone() };
            }
            info.declared = model.services;
            info.profiles = model.profiles;
        }
        Err(e) => {
            if info.name.is_empty() {
                info.name = dir_name(file);
            }
            info.config_error = e;
        }
    }
}

#[tauri::command]
pub async fn list_compose_projects(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ComposeProjectInfo>, String> {
    list_compose_projects_core(&state).await
}

/// App-free core of [`list_compose_projects`], shared with the MCP server.
pub(crate) async fn list_compose_projects_core(
    state: &AppState,
) -> Result<Vec<ComposeProjectInfo>, String> {
    let (configured, pinned) = {
        let cfg = state.config.read().await;
        (cfg.compose_file.clone(), cfg.compose_projects.clone())
    };

    // Containers carrying compose labels, grouped by project.
    let client = state.get_client().await.ok();
    let mut by_label: BTreeMap<String, ComposeProjectInfo> = BTreeMap::new();
    if let Some(client) = &client {
        if let Ok(containers) = client.list_containers(true).await {
            for p in projects_from_containers(&containers) {
                by_label.insert(p.name.clone(), p);
            }
        }
    }

    let mut projects: Vec<ComposeProjectInfo> = Vec::new();
    let mut files: Vec<(String, &str)> = Vec::new();
    if !configured.trim().is_empty() {
        files.push((configured.clone(), "settings"));
    }
    files.extend(pinned.iter().map(|p| (p.clone(), "pinned")));

    let mut seen_files: Vec<PathBuf> = Vec::new();
    for (raw, source) in files {
        let mut info = ComposeProjectInfo { source: source.into(), ..Default::default() };
        match compose::resolve_compose_file(&raw) {
            Ok(file) => {
                if seen_files.contains(&file) {
                    continue;
                }
                seen_files.push(file.clone());
                describe_file(&file, &mut info).await;
            }
            Err(e) => {
                // A pinned file that vanished stays listed so it can be
                // unpinned; a stale settings entry is just skipped.
                if source != "pinned" {
                    continue;
                }
                info.path = raw.clone();
                info.name = dir_name(Path::new(&raw));
                info.config_error = err(e);
            }
        }
        info.pinned = pinned.contains(&info.path) || pinned.contains(&raw);
        if projects.iter().any(|p| p.name == info.name) {
            continue;
        }
        if let Some(labelled) = by_label.remove(&info.name) {
            info.services = labelled.services;
        }
        projects.push(info);
    }

    // Projects started from anywhere else (a terminal, an older session).
    for (_, mut info) in by_label {
        info.source = "labels".into();
        if let Some(client) = &client {
            let file = compose_config_file(client, &info.services).await;
            if let Ok(f) = compose::resolve_compose_file(&file) {
                info.pinned = pinned.contains(&f.display().to_string());
                // `info.name` is already the label's project name and wins.
                describe_file(&f, &mut info).await;
            } else {
                info.path = file;
            }
        }
        projects.push(info);
    }
    Ok(projects)
}

/// Groups containers carrying a compose project label into projects, one
/// service row per container, sorted by project then service.
fn projects_from_containers(
    containers: &[crate::docker::models::ContainerItem],
) -> Vec<ComposeProjectInfo> {
    let mut by_project: BTreeMap<String, Vec<compose::ComposeService>> = Default::default();
    for c in containers {
        let Some(project) = c.compose_project.as_ref().filter(|p| !p.is_empty()) else {
            continue;
        };
        let health = ["unhealthy", "healthy", "health: starting"]
            .into_iter()
            .find(|h| c.status.contains(&format!("({h})")))
            .map(|h| h.trim_start_matches("health: ").to_string())
            .unwrap_or_default();
        by_project.entry(project.clone()).or_default().push(compose::ComposeService {
            id: c.id.clone(),
            name: c.name.clone(),
            service: c.compose_service.clone().unwrap_or_default(),
            image: c.image.clone(),
            state: c.state.clone(),
            health,
            ports: c.ports.clone(),
        });
    }
    by_project
        .into_iter()
        .map(|(name, mut services)| {
            services.sort_by(|a, b| a.service.cmp(&b.service).then(a.name.cmp(&b.name)));
            ComposeProjectInfo { name, services, ..Default::default() }
        })
        .collect()
}

/// The compose file a project was started from, read from one of its
/// containers (`com.docker.compose.project.config_files`, first entry).
async fn compose_config_file(
    client: &crate::docker::client::DockerClient,
    services: &[compose::ComposeService],
) -> String {
    let Some(first) = services.first() else {
        return String::new();
    };
    client
        .docker()
        .inspect_container(&first.id, None)
        .await
        .ok()
        .and_then(|d| d.config?.labels?.get("com.docker.compose.project.config_files").cloned())
        .and_then(|files| files.split(',').next().map(str::to_string))
        .unwrap_or_default()
}

/// Add a compose file (or a directory holding one) to the saved projects.
/// Returns the resolved file path.
#[tauri::command]
pub async fn compose_pin_project(
    state: State<'_, Arc<AppState>>,
    file_path: String,
) -> Result<String, String> {
    compose_pin_core(&state, &file_path, true).await
}

#[tauri::command]
pub async fn compose_unpin_project(
    state: State<'_, Arc<AppState>>,
    file_path: String,
) -> Result<String, String> {
    compose_pin_core(&state, &file_path, false).await
}

pub(crate) async fn compose_pin_core(
    state: &AppState,
    file_path: &str,
    pin: bool,
) -> Result<String, String> {
    let resolved = if pin {
        let f = compose::resolve_compose_file(file_path).map_err(err)?;
        // Refuse files compose itself cannot read, so the list only holds
        // real projects.
        compose::config(&f, None).await.map_err(err)?;
        f.display().to_string()
    } else {
        file_path.trim().to_string()
    };
    let key = resolved.clone();
    state
        .update_config(move |cfg| {
            cfg.compose_projects.retain(|p| p != &key);
            if pin {
                cfg.compose_projects.push(key);
            }
            Ok(())
        })
        .await?;
    Ok(resolved)
}

/// Compose files under `dir` (default depth 3, at most 6).
#[tauri::command]
pub async fn compose_scan_dir(dir: String, depth: Option<u32>) -> Result<Vec<String>, String> {
    compose_scan_core(&dir, depth).await
}

pub(crate) async fn compose_scan_core(dir: &str, depth: Option<u32>) -> Result<Vec<String>, String> {
    let trimmed = dir.trim();
    let root = if let Some(rest) = trimmed.strip_prefix("~/") {
        dirs::home_dir().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(trimmed))
    } else if trimmed == "~" {
        dirs::home_dir().unwrap_or_default()
    } else {
        PathBuf::from(trimmed)
    };
    if !root.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }
    let depth = depth.unwrap_or(3).min(6) as usize;
    tokio::task::spawn_blocking(move || compose::scan_dir(&root, depth))
        .await
        .map_err(|e| e.to_string())
        .map(|found| found.into_iter().map(|p| p.display().to_string()).collect())
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// `up` options as the UI and MCP send them.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UpRequest {
    pub profiles: Vec<String>,
    /// Env file relative to the project directory (default: compose's own).
    pub env_file: Option<String>,
    #[serde(flatten)]
    pub options: UpOptions,
}

fn file_target(file_path: &str, env_file: Option<&str>, profiles: Vec<String>) -> Result<Target, String> {
    let file = compose::resolve_compose_file(file_path).map_err(err)?;
    let env_file = match env_file.map(str::trim).filter(|e| !e.is_empty()) {
        Some(name) => Some(compose::env_path(&file, Some(name)).map_err(err)?),
        None => None,
    };
    Ok(Target { file: Some(file), env_file, profiles, ..Default::default() })
}

#[tauri::command]
pub async fn compose_up(
    app: AppHandle,
    file_path: String,
    options: Option<UpRequest>,
    op_id: Option<String>,
) -> Result<String, String> {
    compose_up_core(&file_path, options.unwrap_or_default(), progress_sink(&app, op_id)).await
}

/// App-free core of [`compose_up`], shared with the MCP server.
pub(crate) async fn compose_up_core(
    file_path: &str,
    req: UpRequest,
    sink: Option<LineSink>,
) -> Result<String, String> {
    let target = file_target(file_path, req.env_file.as_deref(), req.profiles)?;
    let verb = compose::up_args(&req.options).map_err(err)?;
    compose::run(target, verb, sink).await.map_err(err)
}

#[tauri::command]
pub async fn compose_down(
    app: AppHandle,
    file_path: String,
    options: Option<DownOptions>,
    op_id: Option<String>,
) -> Result<String, String> {
    compose_down_core(&file_path, options.unwrap_or_default(), progress_sink(&app, op_id)).await
}

/// App-free core of [`compose_down`], shared with the MCP server.
pub(crate) async fn compose_down_core(
    file_path: &str,
    opts: DownOptions,
    sink: Option<LineSink>,
) -> Result<String, String> {
    // Every profile, so services started under one are removed too.
    let target = file_target(file_path, None, vec!["*".into()])?;
    compose::run(target, compose::down_args(&opts), sink).await.map_err(err)
}

/// Start/stop/restart/up/recreate/scale one service. `file_path` is needed
/// for up, recreate and scale; the others work by project name.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn compose_service_action(
    app: AppHandle,
    project: String,
    file_path: Option<String>,
    service: String,
    action: String,
    replicas: Option<u32>,
    op_id: Option<String>,
) -> Result<String, String> {
    compose_service_action_core(
        &project,
        file_path.as_deref(),
        &service,
        &action,
        replicas,
        progress_sink(&app, op_id),
    )
    .await
}

pub(crate) async fn compose_service_action_core(
    project: &str,
    file_path: Option<&str>,
    service: &str,
    action: &str,
    replicas: Option<u32>,
    sink: Option<LineSink>,
) -> Result<String, String> {
    let verb = compose::service_action_args(action, service, replicas).map_err(err)?;
    let file = file_path.map(str::trim).filter(|f| !f.is_empty());
    let target = match file {
        Some(f) => {
            let mut t = file_target(f, None, Vec::new())?;
            // A service behind a profile has to be enabled to be addressed.
            if let Ok(model) = cached_model(t.file.as_deref().unwrap_or(Path::new(""))).await {
                if let Some(spec) = model.services.iter().find(|s| s.name == service) {
                    t.profiles = spec.profiles.clone();
                }
            }
            if !project.trim().is_empty() {
                t.project = Some(project.trim().to_string());
            }
            t
        }
        None if compose::service_action_needs_file(action) => {
            return Err(format!("'{action}' needs the project's compose file"));
        }
        None => Target::project(project.trim()),
    };
    compose::run(target, verb, sink).await.map_err(err)
}

#[tauri::command]
pub async fn compose_logs(
    file_path: String,
    tail: String,
    services: Option<Vec<String>>,
) -> Result<String, String> {
    let tail_n: usize = tail.parse().unwrap_or(100);
    compose_logs_core(&file_path, tail_n, &services.unwrap_or_default()).await
}

/// App-free core of [`compose_logs`], shared with the MCP server.
pub(crate) async fn compose_logs_core(
    file_path: &str,
    tail: usize,
    services: &[String],
) -> Result<String, String> {
    let file = compose::resolve_compose_file(file_path).map_err(err)?;
    compose::logs(&file, tail.min(5000), services).await.map_err(err)
}

/// Lifecycle for a compose project addressed by name only (`compose -p X
/// start|stop|restart`) — this is what the Containers page's project headers
/// use, and it needs no compose file because the containers carry the
/// project's labels.
#[tauri::command]
pub async fn compose_project_action(
    app: AppHandle,
    project: String,
    action: String,
    op_id: Option<String>,
) -> Result<String, String> {
    compose::project_action_streaming(&project, &action, progress_sink(&app, op_id))
        .await
        .map_err(err)
}

/// App-free core of [`compose_project_action`], shared with the MCP server.
pub(crate) async fn compose_project_action_core(
    project: &str,
    action: &str,
) -> Result<String, String> {
    compose::project_action(project, action).await.map_err(err)
}

/// Open the project's directory in the desktop file manager.
#[tauri::command]
pub async fn compose_open_folder(file_path: String) -> Result<(), String> {
    let file = compose::resolve_compose_file(&file_path).map_err(err)?;
    let dir = file.parent().ok_or("compose file has no parent directory")?;
    std::process::Command::new("xdg-open")
        .arg(dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut child| {
            // Reap it so no zombie is left behind.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        })
        .map_err(|e| format!("cannot open {}: {e}", dir.display()))
}

// ---------------------------------------------------------------------------
// Editors
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn compose_read_file(file_path: String) -> Result<String, String> {
    let file = compose::resolve_compose_file(&file_path).map_err(err)?;
    compose::read_text(&file)
        .map_err(err)?
        .ok_or_else(|| format!("compose file not found: {}", file.display()))
}

#[derive(Clone, Serialize)]
pub struct ComposeValidation {
    pub ok: bool,
    pub error: String,
    pub model: Option<ComposeModel>,
}

/// Validate `content` (or, when absent, the file on disk) with
/// `compose config`, as if it were the project's compose file.
#[tauri::command]
pub async fn compose_validate(
    file_path: String,
    content: Option<String>,
    env_file: Option<String>,
) -> Result<ComposeValidation, String> {
    compose_validate_core(&file_path, content, env_file.as_deref()).await
}

pub(crate) async fn compose_validate_core(
    file_path: &str,
    content: Option<String>,
    env_file: Option<&str>,
) -> Result<ComposeValidation, String> {
    let file = compose::resolve_compose_file(file_path).map_err(err)?;
    let env = match env_file.map(str::trim).filter(|e| !e.is_empty()) {
        Some(name) => Some(compose::env_path(&file, Some(name)).map_err(err)?),
        None => None,
    };
    let text = match content {
        Some(t) => t,
        None => compose::read_text(&file).map_err(err)?.unwrap_or_default(),
    };
    Ok(match compose::validate_text(&file, text, env).await {
        Ok(model) => ComposeValidation { ok: true, error: String::new(), model: Some(model) },
        Err(e) => ComposeValidation { ok: false, error: err(e), model: None },
    })
}

/// Save the compose file, refusing when it changed on disk since `original`
/// was read, and when the new text does not validate.
#[tauri::command]
pub async fn compose_save_file(
    file_path: String,
    content: String,
    original: String,
) -> Result<(), String> {
    let file = compose::resolve_compose_file(&file_path).map_err(err)?;
    let check = compose::validate_text(&file, content.clone(), None).await;
    if let Err(e) = check {
        return Err(format!("not saved, the file does not validate: {}", err(e)));
    }
    compose::write_text_checked(&file, &content, Some(&original)).map_err(err)
}

#[derive(Clone, Serialize)]
pub struct EnvFileInfo {
    pub path: String,
    pub exists: bool,
    pub content: String,
}

#[tauri::command]
pub async fn compose_read_env(
    file_path: String,
    env_file: Option<String>,
) -> Result<EnvFileInfo, String> {
    let file = compose::resolve_compose_file(&file_path).map_err(err)?;
    let path = compose::env_path(&file, env_file.as_deref()).map_err(err)?;
    let content = compose::read_text(&path).map_err(err)?;
    Ok(EnvFileInfo {
        path: path.display().to_string(),
        exists: content.is_some(),
        content: content.unwrap_or_default(),
    })
}

/// Save an env file of the project. `original` is what was read (`None` when
/// the file did not exist), so concurrent edits are not overwritten.
#[tauri::command]
pub async fn compose_save_env(
    file_path: String,
    env_file: Option<String>,
    content: String,
    original: Option<String>,
) -> Result<(), String> {
    let file = compose::resolve_compose_file(&file_path).map_err(err)?;
    let path = compose::env_path(&file, env_file.as_deref()).map_err(err)?;
    compose::write_text_checked(&path, &content, original.as_deref()).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::models::ContainerItem;

    fn item(name: &str, project: Option<&str>, service: &str, status: &str) -> ContainerItem {
        ContainerItem {
            id: format!("id-{name}"),
            name: name.into(),
            state: "running".into(),
            status: status.into(),
            compose_project: project.map(str::to_string),
            compose_service: Some(service.into()),
            ..Default::default()
        }
    }

    #[test]
    fn label_projects_group_containers_and_read_health() {
        let list = vec![
            item("b-api", Some("shop"), "api", "Up 2 minutes (healthy)"),
            item("lone", None, "x", "Up"),
            item("b-db", Some("shop"), "db", "Up 1 minute (health: starting)"),
            item("c-web", Some("blog"), "web", "Up 3 minutes (unhealthy)"),
        ];
        let projects = projects_from_containers(&list);
        let names: Vec<_> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["blog", "shop"]);
        assert_eq!(projects[0].services[0].health, "unhealthy");
        let shop: Vec<_> = projects[1]
            .services
            .iter()
            .map(|s| (s.service.as_str(), s.health.as_str()))
            .collect();
        assert_eq!(shop, [("api", "healthy"), ("db", "starting")]);
    }

    #[test]
    fn up_request_reads_flat_options() {
        let req: UpRequest = serde_json::from_value(serde_json::json!({
            "profiles": ["tools"],
            "env_file": "dev.env",
            "force_recreate": true,
            "services": ["web"]
        }))
        .unwrap();
        assert_eq!(req.profiles, ["tools"]);
        assert_eq!(req.env_file.as_deref(), Some("dev.env"));
        assert!(req.options.force_recreate);
        assert!(!req.options.remove_orphans);
        assert_eq!(req.options.services, ["web"]);
    }
}
