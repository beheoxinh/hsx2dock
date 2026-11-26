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
    remove_image_core(&state, &id, force).await
}

/// App-free core of [`remove_image`], shared with the MCP server so an agent
/// cannot delete a kind node image the UI protects.
pub(crate) async fn remove_image_core(state: &AppState, id: &str, force: bool) -> Result<(), String> {
    let id = id.to_string();
    let client = state.get_client().await?;
    let tags = client
        .image_repo_tags(&id)
        .await
        .map_err(|e| e.to_string())?;
    let configured = state.config.read().await.kind_default_image.clone();
    // Tags cover official and configured node images; container-derived IDs
    // cover retagged or digest-only ones.
    // Fail closed: if the kind nodes cannot be listed, the image may be one
    // a cluster needs, so it is refused rather than removed unchecked.
    let kind_ids = client.kind_node_image_ids().await.map_err(|e| {
        format!("Could not check whether this image is a kind node image, so it was not removed: {e}")
    })?;
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

/// App-free core of [`pull_image`]. Resolves only once the pull has finished,
/// failed or stalled, so the caller gets the real outcome. Without an
/// `AppHandle` the progress lines are dropped — the MCP server has no window
/// to emit to.
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
    let Some(_slot) = PullSlot::claim(&image) else {
        return Ok(());
    };
    let client = state.get_client().await?;
    let docker = client.docker();
    let opts = bollard::query_parameters::CreateImageOptionsBuilder::default()
        .from_image(image.as_str())
        .build();
    let creds = saved_credentials(state, &registry_host_for_image(&image)).await;
    let mut stream = docker.create_image(Some(opts), None, creds);
    let app_handle = app.cloned();

    // Byte-level progress arrives hundreds of times a second across layers;
    // the UI only needs a few frames.
    let mut last_tick = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let outcome: Result<(), String> = loop {
        let info = match tokio::time::timeout(STALL_TIMEOUT, stream.next()).await {
            Err(_) => break Err(stalled_message("pull")),
            Ok(None) => break Ok(()),
            Ok(Some(Err(e))) => break Err(e.to_string()),
            Ok(Some(Ok(info))) => info,
        };
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
            image: image.clone(),
            status: info.status.unwrap_or_default(),
            progress: progress_str,
            current,
            total,
            done: false,
            error: None,
        };
        emit_progress(&app_handle, "pull-progress", payload);
    };

    let finished = PullProgressPayload {
        image: image.clone(),
        status: if outcome.is_ok() { "Complete" } else { "Error" }.to_string(),
        progress: None,
        current: None,
        total: None,
        done: true,
        error: outcome.as_ref().err().cloned(),
    };
    emit_progress(&app_handle, "pull-progress", finished);
    outcome
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
mod registry_and_preview_tests {
    use super::*;
    use crate::docker::models::BuildArg;

    #[test]
    fn docker_hub_spellings_share_one_auth_key() {
        let hub = build_auth_key("docker.io");
        assert_eq!(build_auth_key("index.docker.io"), hub);
        assert_eq!(build_auth_key("Docker.IO/"), hub);
        assert_eq!(build_auth_key("registry-1.docker.io"), hub);
        assert_eq!(
            build_auth_key(&registry_host_for_image("index.docker.io/library/nginx")),
            hub
        );
        assert_eq!(build_auth_key("https://GHCR.io/"), "ghcr.io");
    }

    #[test]
    fn build_preview_shows_keys_and_hides_values() {
        let req = BuildOptions {
            context_dir: "/ctx".into(),
            dockerfile: "Dockerfile".into(),
            tag: "local/app:dev".into(),
            build_args: vec![
                BuildArg {
                    key: "NPM_TOKEN".into(),
                    value: "s3cr3t-value".into(),
                },
                BuildArg {
                    key: "EMPTY".into(),
                    value: String::new(),
                },
            ],
            ..Default::default()
        };
        let preview = masked_for_preview(req.clone()).command_preview();
        assert!(!preview.contains("s3cr3t-value"), "{preview}");
        assert!(preview.contains("NPM_TOKEN="), "{preview}");
        assert!(preview.contains("EMPTY="), "{preview}");
        // The real request is untouched; only the preview copy is masked.
        assert_eq!(req.build_args[0].value, "s3cr3t-value");
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

/// App-free core of [`push_image`]. Resolves once the push has finished,
/// failed or stalled.
pub(crate) async fn push_image_core(
    state: &AppState,
    app: Option<&AppHandle>,
    image: String,
) -> Result<(), String> {
    // Like pull, an untagged name would push every tag of the repository.
    let name = with_default_tag(image.trim());
    if name.is_empty() {
        return Err("Image name is required to push".into());
    }
    let host = registry_host_for_image(&name);
    // The app's own local registry takes anonymous pushes.
    let creds = if super::kind_registry::is_local_registry_host(&host) {
        None
    } else {
        Some(saved_credentials(state, &host).await.ok_or_else(|| {
            format!("No saved login for {host} — add one under Settings → Registries before pushing")
        })?)
    };

    let client = state.get_client().await?;
    let docker = client.docker();
    let mut stream = docker.push_image(
        name.as_str(),
        None::<bollard::query_parameters::PushImageOptions>,
        creds,
    );

    let app_handle = app.cloned();
    let outcome: Result<(), String> = loop {
        let info = match tokio::time::timeout(STALL_TIMEOUT, stream.next()).await {
            Err(_) => break Err(stalled_message("push")),
            Ok(None) => break Ok(()),
            Ok(Some(Err(e))) => break Err(e.to_string()),
            Ok(Some(Ok(info))) => info,
        };
        if let Some(message) = info.error_detail.as_ref().and_then(|e| e.message.clone()) {
            break Err(message);
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
            image: name.clone(),
            status: info.status.clone().unwrap_or_else(|| "Pushing".to_string()),
            progress: progress_str,
            current,
            total,
            done: false,
            error: None,
        };
        emit_progress(&app_handle, "push-progress", payload);
    };

    let finished = PushProgressPayload {
        image: name.clone(),
        status: if outcome.is_ok() { "Complete" } else { "Error" }.to_string(),
        progress: None,
        current: None,
        total: None,
        done: true,
        error: outcome.as_ref().err().cloned(),
    };
    emit_progress(&app_handle, "push-progress", finished);
    outcome
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

/// How long a pull, push or build may go without a single event before it is
/// stopped. A stream that never ends otherwise held the UI's busy flag forever.
const STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

fn stalled_message(what: &str) -> String {
    format!(
        "No progress from the registry for {} minutes, so the {what} was stopped.",
        STALL_TIMEOUT.as_secs() / 60
    )
}

/// Key for one registry in the build's X-Registry-Config: the bare host, as
/// the docker CLI writes it, except Docker Hub's legacy index address. Hosts
/// are normalised first, so `Docker.IO/` and `docker.io` share one key.
fn build_auth_key(host: &str) -> String {
    let host = crate::commands::registries::normalize_host(host);
    if matches!(host.as_str(), "docker.io" | "index.docker.io" | "registry-1.docker.io") {
        crate::config::server_address_for_host("docker.io")
    } else {
        host
    }
}

/// The saved login for the registry `host` names. Matched on the same key as
/// the build's registry map, so `index.docker.io/...` finds the Docker Hub login.
pub(crate) async fn saved_credentials(
    state: &AppState,
    host: &str,
) -> Option<bollard::auth::DockerCredentials> {
    let wanted = build_auth_key(host);
    let cfg = state.config.read().await;
    cfg.registries
        .iter()
        .find(|r| build_auth_key(&r.host) == wanted)
        .map(|r| r.credentials())
}

/// Clears the build-running flag on every exit, a panic included: a stuck flag
/// rejected every later build until the app restarted.
struct BuildRelease(Arc<std::sync::atomic::AtomicBool>);

impl Drop for BuildRelease {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Clones of build-argument values are secrets more often than not, so the
/// command preview shows the keys and hides the values.
fn masked_for_preview(mut req: BuildOptions) -> BuildOptions {
    for arg in req.build_args.iter_mut() {
        if !arg.value.is_empty() {
            arg.value = "***".to_string();
        }
    }
    req
}

#[tauri::command]
pub async fn build_image(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    req: BuildOptions,
) -> Result<(), String> {
    // Refusals the form can act on come back synchronously, before the modal
    // switches to its progress view. The build itself runs detached and
    // reports through `build-progress`, which the modal listens to.
    req.validate()?;
    if state
        .build_running
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Err("A build is already in progress".to_string());
    }
    let state = state.inner().clone();
    tokio::spawn(async move {
        // The outcome already reached the modal as a build-progress event.
        let _ = build_image_core(&state, Some(&app), req).await;
    });
    Ok(())
}

/// App-free core of [`build_image`]. Resolves once the build has succeeded,
/// failed or been cancelled, so MCP callers get the real result. Progress is
/// also emitted for the window when there is one.
pub(crate) async fn build_image_core(
    state: &AppState,
    app: Option<&AppHandle>,
    req: BuildOptions,
) -> Result<(), String> {
    // Validate up front so a bad path fails before anything starts.
    req.validate()?;

    let claimed = state
        .build_running
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        )
        .is_ok();
    if !claimed {
        return Err("A build is already in progress".to_string());
    }
    let _release = BuildRelease(state.build_running.clone());

    let app_handle = app.cloned();
    let tag = req.tag.trim().to_string();
    let cancel = state.build_cancel.clone();
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let starting = BuildProgressPayload {
        done: false,
        ..build_final_payload(
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
        )
    };
    emit_progress(&app_handle, "build-progress", starting);

    let result = run_build(state, &req, &tag, &app_handle, &cancel).await;

    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        emit_progress(
            &app_handle,
            "build-progress",
            build_final_payload(
                &tag,
                "Cancelled",
                Some("Build cancelled.".to_string()),
                None,
                true,
                None,
            ),
        );
        return Err("Build cancelled.".to_string());
    }
    match result {
        Ok(image_id) => {
            emit_progress(
                &app_handle,
                "build-progress",
                build_final_payload(
                    &tag,
                    "Succeeded",
                    Some("Build succeeded.".to_string()),
                    None,
                    false,
                    image_id,
                ),
            );
            Ok(())
        }
        Err(e) => {
            emit_progress(
                &app_handle,
                "build-progress",
                build_final_payload(&tag, "Failed", None, Some(e.clone()), false, None),
            );
            Err(e)
        }
    }
}

/// Runs the engine build with the saved registry logins attached. Saved logins
/// go to the daemon as X-Registry-Config, like `docker build` does: a `FROM`
/// on a private registry failed with "unauthorized" even though the user had
/// logged in through the app.
async fn run_build(
    state: &AppState,
    req: &BuildOptions,
    tag: &str,
    app: &Option<AppHandle>,
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<Option<String>, String> {
    let client = state.get_client().await?;
    let registry_auth = {
        let cfg = state.config.read().await;
        let map: std::collections::HashMap<String, bollard::auth::DockerCredentials> = cfg
            .registries
            .iter()
            .map(|r| (build_auth_key(&r.host), r.credentials()))
            .collect();
        (!map.is_empty()).then_some(map)
    };
    client
        .build_image(req, cancel, registry_auth, |step: BuildStep| {
            let payload = build_step_payload(tag, &step, false, false, None, None);
            emit_progress(app, "build-progress", payload);
        })
        .await
        .map_err(|e| e.to_string())
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
    Ok(masked_for_preview(req).command_preview())
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
