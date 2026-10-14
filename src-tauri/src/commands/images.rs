use futures_util::StreamExt;
use tauri::{AppHandle, Emitter, State};

use crate::docker::models::{ImageHistoryEntry, ImageItem};
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

    let mut stream = docker.create_image(Some(opts), None, None);
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
