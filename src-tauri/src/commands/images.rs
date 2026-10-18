use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, State};

use crate::docker::models::{
    registry_host_for_image, BuildContextInfo, BuildOptions, BuildStep, ImageHistoryEntry, ImageItem,
};
use crate::state::AppState;

#[tauri::command]
pub async fn list_images(state: State<'_, AppState>) -> Result<Vec<ImageItem>, String> {
    let client = state.get_client().await?;
    client.list_images().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_image(
    state: State<'_, AppState>,
    id: String,
    force: bool,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client.remove_image(&id, force).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn tag_image(
    state: State<'_, AppState>,
    id: String,
    repo: String,
    tag: String,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client.tag_image(&id, &repo, &tag).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prune_images(state: State<'_, AppState>) -> Result<(u64, u64), String> {
    let client = state.get_client().await?;
    client.prune_images().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_image_history(
    state: State<'_, AppState>,
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
    state: State<'_, AppState>,
    term: String,
) -> Result<Vec<SearchResult>, String> {
    let client = state.get_client().await?;
    let res = client.search_images(&term).await.map_err(|e| e.to_string())?;
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
    state: State<'_, AppState>,
    image: String,
) -> Result<(), String> {
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
    let app_handle = app.clone();
    let img_name = image.clone();

    tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(info) => {
                    let (current, total) = if let Some(ref detail) = info.progress_detail {
                        (detail.current.map(|v| v as u64), detail.total.map(|v| v as u64))
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
                    let _ = app_handle.emit("pull-progress", payload);
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
                    let _ = app_handle.emit("pull-progress", payload);
                    return;
                }
            }
        }
        let _ = app_handle.emit(
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
    state: State<'_, AppState>,
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

    let app_handle = app.clone();
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
                        let _ = app_handle.emit("push-progress", payload);
                        return;
                    }
                    let (current, total) = if let Some(ref detail) = info.progress_detail {
                        (detail.current.map(|v| v as u64), detail.total.map(|v| v as u64))
                    } else {
                        (None, None)
                    };
                    let progress_str = match (current, total) {
                        (Some(c), Some(t)) => Some(format!("{c}/{t}")),
                        _ => None,
                    };
                    let payload = PushProgressPayload {
                        image: img_name.clone(),
                        status: info
                            .status
                            .clone()
                            .unwrap_or_else(|| "Pushing".to_string()),
                        progress: progress_str,
                        current,
                        total,
                        done: false,
                        error: None,
                    };
                    let _ = app_handle.emit("push-progress", payload);
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
                    let _ = app_handle.emit("push-progress", payload);
                    return;
                }
            }
        }
        let _ = app_handle.emit(
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

#[tauri::command]
pub async fn build_image(
    app: AppHandle,
    state: State<'_, AppState>,
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
            state.build_running.store(false, std::sync::atomic::Ordering::Relaxed);
            return Err(e);
        }
    };

    let cancel = state.build_cancel.clone();
    let running = state.build_running.clone();
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);

    let app_handle = app.clone();
    let tag = req.tag.trim().to_string();

    tokio::spawn(async move {
        let starting = build_final_payload(
            &tag,
            "Starting",
            Some(format!("docker build -f {} -t {}", req.dockerfile.trim(), tag)),
            None,
            false,
            None,
        );
        let starting = BuildProgressPayload {
            done: false,
            ..starting
        };
        let _ = app_handle.emit("build-progress", starting);

        let result = client
            .build_image(&req, &cancel, |step: BuildStep| {
                let payload = build_step_payload(&tag, &step, false, false, None, None);
                let _ = app_handle.emit("build-progress", payload);
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
                Err(e) => build_final_payload(
                    &tag,
                    "Failed",
                    None,
                    Some(e.to_string()),
                    false,
                    None,
                ),
            }
        };
        let _ = app_handle.emit("build-progress", final_payload);
        running.store(false, std::sync::atomic::Ordering::Relaxed);
    });

    Ok(())
}

#[tauri::command]
pub async fn cancel_image_build(state: State<'_, AppState>) -> Result<(), String> {
    if !state.build_running.load(std::sync::atomic::Ordering::Relaxed) {
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
    Ok(req.command_preview())
}

/// Inspects a build context so the UI can validate it and offer the
/// Dockerfiles it found without a native folder picker.
#[tauri::command]
pub async fn validate_build_context(path: String) -> Result<BuildContextInfo, String> {
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
    entries.truncate(50);
    info.dockerfiles = entries
        .iter()
        .filter(|name| name.to_lowercase().starts_with("dockerfile"))
        .cloned()
        .collect();
    info.entries = entries;

    let dir_name = context
        .canonicalize()
        .ok()
        .and_then(|c| c.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "context".to_string());
    info.suggested_tag = format!("local/{}-dev", dir_name);
    Ok(info)
}
