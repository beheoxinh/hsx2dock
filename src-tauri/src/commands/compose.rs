use std::path::Path;
use tauri::State;

use crate::docker::compose;
use crate::state::AppState;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ComposeProjectInfo {
    pub name: String,
    pub path: String,
    pub services: Vec<compose::ComposeService>,
}

#[tauri::command]
pub async fn list_compose_projects(
    state: State<'_, AppState>,
) -> Result<Vec<ComposeProjectInfo>, String> {
    let config = state.config.read().await;
    let file = compose::resolve_compose_file(&config.compose_file);

    if let Ok(f) = file {
        if let Ok(services) = compose::ps(&f).await {
            let name = f
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("default")
                .to_string();
            return Ok(vec![ComposeProjectInfo {
                name,
                path: f.display().to_string(),
                services,
            }]);
        }
    }
    Ok(vec![])
}

#[tauri::command]
pub async fn compose_up(_state: State<'_, AppState>, file_path: String) -> Result<String, String> {
    compose::up(Path::new(&file_path))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn compose_down(
    _state: State<'_, AppState>,
    file_path: String,
) -> Result<String, String> {
    compose::down(Path::new(&file_path))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn compose_logs(
    _state: State<'_, AppState>,
    file_path: String,
    tail: String,
) -> Result<String, String> {
    let tail_n: usize = tail.parse().unwrap_or(100);
    compose::logs(Path::new(&file_path), tail_n)
        .await
        .map_err(|e| e.to_string())
}

/// Lifecycle for a compose project addressed by name only (`compose -p X
/// start|stop|restart`) — this is what the Containers page's project headers
/// use, and it needs no compose file because the containers carry the
/// project's labels.
#[tauri::command]
pub async fn compose_project_action(
    project: String,
    action: String,
) -> Result<String, String> {
    compose::project_action(&project, &action)
        .await
        .map_err(|e| format!("{e:#}"))
}
