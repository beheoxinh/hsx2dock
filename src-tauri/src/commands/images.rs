use futures_util::StreamExt;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::docker::models::{
    registry_host_for_image, BuildContextInfo, BuildOptions, BuildStep, ImageHistoryEntry,
    ImageItem,
};
use crate::state::AppState;

/// Emits a progress event when there is a window to emit to.
///
/// The MCP server runs these same cores with no `AppHandle`; progress is a
/// UI affordance there, not part of the operation's contract.
fn emit_progress<P: serde::Serialize + Clone>(app: &Option<AppHandle>, event: &str, payload: P) {
    if let Some(app) = app {
        let _ = app.emit(event, payload);
    }
}

#[tauri::command]
pub async fn list_images(state: State<'_, Arc<AppState>>) -> Result<Vec<ImageItem>, String> {
    let client = state.get_client().await?;
    // kind's node images are core to this app: keep them out of the Images
    // page so they cannot be selected for removal.
    let configured = state.config.read().await.kind_default_image.clone();
    client
        .list_images_visible(Some(configured.as_str()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_image(
    state: State<'_, Arc<AppState>>,
    id: String,
    force: bool,
) -> Result<(), String> {
    let client = state.get_client().await?;
    let tags = client
        .image_repo_tags(&id)
        .await
        .map_err(|e| e.to_string())?;
    let configured = state.config.read().await.kind_default_image.clone();
    // Tags cover official and configured node images; container-derived IDs
    // cover retagged or digest-only ones.
    let kind_ids = client.kind_node_image_ids().await.unwrap_or_default();
    let normalised_id = id.trim_start_matches("sha256:");
    if crate::docker::kind::is_kind_image(&tags, Some(configured.as_str()))
        || kind_ids.contains(normalised_id)
    {
        return Err(
            "This image is a kind node image. Removing it would break every cluster \
             that uses it, so the app keeps it protected."
                .to_string(),
        );
    }
    client
        .remove_image(&id, force)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn tag_image(
    state: State<'_, Arc<AppState>>,
    id: String,
    repo: String,
    tag: String,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .tag_image(&id, &repo, &tag)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prune_images(
    state: State<'_, Arc<AppState>>,
    dangling_only: Option<bool>,
) -> Result<(u64, u64), String> {
    let client = state.get_client().await?;
    let configured = state.config.read().await.kind_default_image.clone();
    client
        .prune_images_mode(dangling_only.unwrap_or(false), Some(configured.as_str()))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_image_history(
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<Vec<ImageHistoryEntry>, String> {
    let client = state.get_client().await?;
    client.image_history(&name).await.map_err(|e| e.to_string())
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub name: String,
    pub description: String,
    pub star_count: u64,
}

#[tauri::command]
pub async fn search_hub(
    state: State<'_, Arc<AppState>>,
    term: String,
) -> Result<Vec<SearchResult>, String> {
    let client = state.get_client().await?;
    let res = client
        .search_images(&term)
        .await
        .map_err(|e| e.to_string())?;
    Ok(res
        .into_iter()
        .map(|(name, description, star_count)| SearchResult {
            name,
            description,
            star_count,
        })
        .collect())
}

#[derive(Clone, serde::Serialize)]
pub struct PullProgressPayload {
    pub image: String,
    pub status: String,
    pub progress: Option<String>,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub done: bool,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn pull_image(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    image: String,
) -> Result<(), String> {
    pull_image_core(&state, Some(&app), image).await
}

/// App-free core of [`pull_image`]. Without an `AppHandle` the progress lines
/// are dropped — the MCP server has no window to emit to.
pub(crate) async fn pull_image_core(
    state: &AppState,
    app: Option<&AppHandle>,
    image: String,
) -> Result<(), String> {
    let image = with_default_tag(image.trim());
    if image.is_empty() {
        return Err("Image name is required".to_string());
    }
    // A second click on Pull (or the same image from MCP) used to start a
    // parallel stream whose progress interleaved with the first one's.
    let Some(slot) = PullSlot::claim(&image) else {
        return Ok(());
    };
    let client = state.get_client().await?;
    let docker = client.docker();
    let opts = bollard::query_parameters::CreateImageOptionsBuilder::default()
        .from_image(image.as_str())
        .build();

    // Attach the saved login for this image's registry, if there is one.
    let creds = {
        let cfg = state.config.read().await;
        let host = registry_host_for_image(&image);
        cfg.registries
            .iter()
            .find(|r| r.host == host)
            .map(|r| r.credentials())
    };

    let mut stream = docker.create_image(Some(opts), None, creds);
    let app_handle = app.cloned();
    let img_name = image.clone();

    tokio::spawn(async move {
        let _slot = slot;
        // Byte-level progress arrives hundreds of times a second across
        // layers; the UI only needs a few frames.
        let mut last_tick = std::time::Instant::now() - std::time::Duration::from_secs(1);
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(info) => {
                    let is_tick = info
                        .progress_detail
                        .as_ref()
                        .is_some_and(|d| d.current.is_some());
                    if is_tick {
                        if last_tick.elapsed() < std::time::Duration::from_millis(100) {
                            continue;
                        }
                        last_tick = std::time::Instant::now();
                    }
                    let (current, total) = if let Some(ref detail) = info.progress_detail {
                        (
                            detail.current.map(|v| v as u64),
                            detail.total.map(|v| v as u64),
                        )
                    } else {
                        (None, None)
                    };
                    let progress_str = match (current, total) {
                        (Some(c), Some(t)) => Some(format!("{c}/{t}")),
                        _ => info.id.clone(),
                    };
                    let payload = PullProgressPayload {
                        image: img_name.clone(),
                        status: info.status.unwrap_or_default(),
                        progress: progress_str,
                        current,
                        total,
                        done: false,
                        error: None,
                    };
                    emit_progress(&app_handle, "pull-progress", payload);
                }
                Err(e) => {
                    let payload = PullProgressPayload {
                        image: img_name.clone(),
                        status: "Error".to_string(),
                        progress: None,
                        current: None,
                        total: None,
                        done: true,
                        error: Some(e.to_string()),
                    };
                    emit_progress(&app_handle, "pull-progress", payload);
                    return;
                }
            }
        }
        emit_progress(
            &app_handle,
            "pull-progress",
            PullProgressPayload {
                image: img_name,
                status: "Complete".to_string(),
                progress: None,
                current: None,
                total: None,
                done: true,
                error: None,
            },
        );
    });

    Ok(())
}

/// The Engine API pulls *every* tag of a repository when `fromImage` has no
/// tag, unlike `docker pull`, which defaults to `latest`.
fn with_default_tag(image: &str) -> String {
    if image.is_empty() || image.contains('@') {
        return image.to_string();
    }
    let last = image.rsplit('/').next().unwrap_or(image);
    if last.contains(':') {
        image.to_string()
    } else {
        format!("{image}:latest")
    }
}

/// Marks an image as being pulled for as long as the value lives.
struct PullSlot(String);

fn pulls_in_flight() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static SET: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    SET.get_or_init(Default::default)
}

impl PullSlot {
    fn claim(image: &str) -> Option<Self> {
        let mut set = pulls_in_flight().lock().ok()?;
        set.insert(image.to_string()).then(|| Self(image.to_string()))
    }
}

impl Drop for PullSlot {
    fn drop(&mut self) {
        if let Ok(mut set) = pulls_in_flight().lock() {
            set.remove(&self.0);
        }
    }
}

#[cfg(test)]
mod pull_tests {
    use super::*;

    #[test]
    fn untagged_images_default_to_latest() {
        assert_eq!(with_default_tag("nginx"), "nginx:latest");
        assert_eq!(with_default_tag("library/nginx"), "library/nginx:latest");
        assert_eq!(with_default_tag("localhost:5000/app"), "localhost:5000/app:latest");
        assert_eq!(with_default_tag("nginx:1.27"), "nginx:1.27");
        assert_eq!(with_default_tag("localhost:5000/app:v2"), "localhost:5000/app:v2");
        assert_eq!(with_default_tag("nginx@sha256:abc"), "nginx@sha256:abc");
    }

    #[test]
    fn a_pull_slot_is_exclusive_until_dropped() {
        let a = PullSlot::claim("x:1").unwrap();
        assert!(PullSlot::claim("x:1").is_none());
        drop(a);
        assert!(PullSlot::claim("x:1").is_some());
    }
}

#[derive(Clone, serde::Serialize)]
pub struct PushProgressPayload {
    pub image: String,
    pub status: String,
    pub progress: Option<String>,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub done: bool,
    pub error: Option<String>,
}

/// Pushes an image to the registry its name points at, using the saved login
/// for that registry. Streams progress on the `push-progress` event.
#[tauri::command]
pub async fn push_image(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    image: String,
) -> Result<(), String> {
    push_image_core(&state, Some(&app), image).await
}

/// App-free core of [`push_image`].
pub(crate) async fn push_image_core(
    state: &AppState,
    app: Option<&AppHandle>,
    image: String,
) -> Result<(), String> {
    let name = image.trim().to_string();
    if name.is_empty() {
        return Err("Image name is required to push".into());
    }
    let host = registry_host_for_image(&name);
    let creds = {
        let cfg = state.config.read().await;
        cfg.registries
            .iter()
            .find(|r| r.host == host)
            .map(|r| r.credentials())
    };
    let creds = creds.ok_or_else(|| {
        format!("No saved login for {host} — add one under Settings → Registries before pushing")
    })?;

    let client = state.get_client().await?;
    let docker = client.docker();
    let mut stream = docker.push_image(
        name.as_str(),
        None::<bollard::query_parameters::PushImageOptions>,
        Some(creds),
    );

    let app_handle = app.cloned();
    let img_name = name.clone();
    tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(info) => {
                    if let Some(message) =
                        info.error_detail.as_ref().and_then(|e| e.message.clone())
                    {
                        let payload = PushProgressPayload {
                            image: img_name.clone(),
                            status: "Error".to_string(),
                            progress: None,
                            current: None,
                            total: None,
                            done: true,
                            error: Some(message),
                        };
                        emit_progress(&app_handle, "push-progress", payload);
                        return;
                    }
                    let (current, total) = if let Some(ref detail) = info.progress_detail {
                        (
                            detail.current.map(|v| v as u64),
                            detail.total.map(|v| v as u64),
                        )
                    } else {
                        (None, None)
                    };
                    let progress_str = match (current, total) {
                        (Some(c), Some(t)) => Some(format!("{c}/{t}")),
                        _ => None,
                    };
                    let payload = PushProgressPayload {
                        image: img_name.clone(),
                        status: info.status.clone().unwrap_or_else(|| "Pushing".to_string()),
                        progress: progress_str,
                        current,
                        total,
                        done: false,
                        error: None,
                    };
                    emit_progress(&app_handle, "push-progress", payload);
                }
                Err(e) => {
                    let payload = PushProgressPayload {
                        image: img_name.clone(),
                        status: "Error".to_string(),
                        progress: None,
                        current: None,
                        total: None,
                        done: true,
                        error: Some(e.to_string()),
                    };
                    emit_progress(&app_handle, "push-progress", payload);
                    return;
                }
            }
        }
        emit_progress(
            &app_handle,
            "push-progress",
            PushProgressPayload {
                image: img_name,
                status: "Complete".to_string(),
                progress: None,
                current: None,
                total: None,
                done: true,
                error: None,
            },
        );
    });

    Ok(())
}

#[derive(Clone, serde::Serialize)]
pub struct BuildProgressPayload {
    pub tag: String,
    pub status: String,
    pub log: Option<String>,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub done: bool,
    pub cancelled: bool,
    pub image_id: Option<String>,
}

fn build_step_payload(
    tag: &str,
    step: &BuildStep,
    done: bool,
    cancelled: bool,
    error: Option<String>,
    image_id: Option<String>,
) -> BuildProgressPayload {
    BuildProgressPayload {
        tag: tag.to_string(),
        status: step.status.clone(),
        log: step.log.clone(),
        current: step.current,
        total: step.total,
        error,
        done,
        cancelled,
        image_id,
    }
}

fn build_final_payload(
    tag: &str,
    status: &str,
    log: Option<String>,
    error: Option<String>,
    cancelled: bool,
    image_id: Option<String>,
) -> BuildProgressPayload {
    BuildProgressPayload {
        tag: tag.to_string(),
        status: status.to_string(),
        log,
        current: None,
        total: None,
        error,
        done: true,
        cancelled,
        image_id,
    }
}

/// Key for one registry in the build's X-Registry-Config: the bare host, as
/// the docker CLI writes it, except Docker Hub's legacy index address.
fn build_auth_key(host: &str) -> String {
    let host = host.trim().trim_end_matches('/');
    let host = host
        .strip_prefix("https://")
        .or_else(|| host.strip_prefix("http://"))
        .unwrap_or(host);
    if matches!(host, "docker.io" | "index.docker.io" | "registry-1.docker.io") {
        crate::config::server_address_for_host("docker.io")
    } else {
        host.to_string()
    }
}

#[tauri::command]
pub async fn build_image(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    req: BuildOptions,
) -> Result<(), String> {
    build_image_core(&state, Some(&app), req).await
}

/// App-free core of [`build_image`]. Without an `AppHandle` the step lines are
/// dropped; the build itself is identical.
pub(crate) async fn build_image_core(
    state: &AppState,
    app: Option<&AppHandle>,
    req: BuildOptions,
) -> Result<(), String> {
    // Validate up front so a bad path fails synchronously, before the modal
    // switches to the progress view.
    req.validate()?;

    let already = state
        .build_running
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        )
        .is_err();
    if already {
        return Err("A build is already in progress".to_string());
    }

    let client = match state.get_client().await {
        Ok(client) => client,
        Err(e) => {
            state
                .build_running
                .store(false, std::sync::atomic::Ordering::Relaxed);
            return Err(e);
        }
    };

    // Saved registry logins go to the daemon as X-Registry-Config, like
    // `docker build` does: a `FROM` on a private registry failed with
    // "unauthorized" even though the user had logged in through the app.
    let registry_auth = {
        let cfg = state.config.read().await;
        let map: std::collections::HashMap<String, bollard::auth::DockerCredentials> = cfg
            .registries
            .iter()
            .map(|r| (build_auth_key(&r.host), r.credentials()))
            .collect();
        (!map.is_empty()).then_some(map)
    };
    let cancel = state.build_cancel.clone();
    let running = state.build_running.clone();
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let app_handle = app.cloned();
    let tag = req.tag.trim().to_string();

    tokio::spawn(async move {
        // Cleared on every exit, a panic included: a stuck flag rejected every
        // later build until the app restarted.
        struct Release(Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Release {
            fn drop(&mut self) {
                self.0.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let _release = Release(running);
        let starting = build_final_payload(
            &tag,
            "Starting",
            Some(format!(
                "docker build -f {} -t {}",
                req.dockerfile.trim(),
                tag
            )),
            None,
            false,
            None,
        );
        let starting = BuildProgressPayload {
            done: false,
            ..starting
        };
        emit_progress(&app_handle, "build-progress", starting);

        let result = client
            .build_image(&req, &cancel, registry_auth, |step: BuildStep| {
                let payload = build_step_payload(&tag, &step, false, false, None, None);
                emit_progress(&app_handle, "build-progress", payload);
            })
            .await;

        let cancelled = cancel.load(std::sync::atomic::Ordering::Relaxed);
        let final_payload = if cancelled {
            build_final_payload(
                &tag,
                "Cancelled",
                Some("Build cancelled.".to_string()),
                None,
                true,
                None,
            )
        } else {
            match result {
                Ok(image_id) => build_final_payload(
                    &tag,
                    "Succeeded",
                    Some("Build succeeded.".to_string()),
                    None,
                    false,
                    image_id,
                ),
                Err(e) => {
                    build_final_payload(&tag, "Failed", None, Some(e.to_string()), false, None)
                }
            }
        };
        emit_progress(&app_handle, "build-progress", final_payload);
    });

    Ok(())
}

#[tauri::command]
pub async fn cancel_image_build(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    if !state
        .build_running
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Err("No build in progress".to_string());
    }
    state
        .build_cancel
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// The `docker build …` line that mirrors `req`, so the form preview can never
/// drift from what the app actually runs.
#[tauri::command]
pub async fn preview_build_command(req: BuildOptions) -> Result<String, String> {
    preview_build_command_core(req)
}

/// App-free core of [`preview_build_command`].
pub(crate) fn preview_build_command_core(req: BuildOptions) -> Result<String, String> {
    Ok(req.command_preview())
}

/// Inspects a build context so the UI can validate it and offer the
/// Dockerfiles it found without a native folder picker.
#[tauri::command]
pub async fn validate_build_context(path: String) -> Result<BuildContextInfo, String> {
    validate_build_context_core(&path)
}

/// App-free core of [`validate_build_context`].
pub(crate) fn validate_build_context_core(path: &str) -> Result<BuildContextInfo, String> {
    let mut info = BuildContextInfo {
        exists: false,
        is_dir: false,
        entries: Vec::new(),
        dockerfiles: Vec::new(),
        suggested_tag: String::new(),
    };
    let raw = path.trim();
    if raw.is_empty() {
        return Ok(info);
    }
    let context = std::path::PathBuf::from(raw);
    if !context.exists() {
        return Ok(info);
    }
    info.exists = true;
    info.is_dir = context.is_dir();
    if !info.is_dir {
        return Ok(info);
    }

    let mut entries: Vec<String> = match std::fs::read_dir(&context) {
        Ok(read) => read
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    entries.sort();
    // Dockerfiles are picked before the preview list is cut, so a large
    // directory cannot hide them past entry 50.
    info.dockerfiles = entries
        .iter()
        .filter(|name| {
            let lower = name.to_lowercase();
            lower.starts_with("dockerfile") || lower.ends_with(".dockerfile")
        })
        .cloned()
        .collect();
    entries.truncate(50);
    info.entries = entries;

    let dir_name = context
        .canonicalize()
        .ok()
        .and_then(|c| c.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "context".to_string());
    info.suggested_tag = format!("local/{}-dev", dir_name);
    Ok(info)
}
