use std::path::Path;
use std::sync::Arc;
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
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ComposeProjectInfo>, String> {
    list_compose_projects_core(&state).await
}

/// App-free core of [`list_compose_projects`], shared with the MCP server.
pub(crate) async fn list_compose_projects_core(
    state: &AppState,
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
pub async fn compose_up(
    _state: State<'_, Arc<AppState>>,
    file_path: String,
) -> Result<String, String> {
    compose_lifecycle_core(&file_path, true).await
}

/// App-free core of [`compose_up`]/[`compose_down`], shared with the MCP server.
pub(crate) async fn compose_lifecycle_core(file_path: &str, up: bool) -> Result<String, String> {
    let path = Path::new(file_path);
    if up {
        compose::up(path).await
    } else {
        compose::down(path).await
    }
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn compose_down(
    _state: State<'_, Arc<AppState>>,
    file_path: String,
) -> Result<String, String> {
    compose_lifecycle_core(&file_path, false).await
}

#[tauri::command]
pub async fn compose_logs(
    _state: State<'_, Arc<AppState>>,
    file_path: String,
    tail: String,
) -> Result<String, String> {
    let tail_n: usize = tail.parse().unwrap_or(100);
    compose_logs_core(&file_path, tail_n).await
}

/// App-free core of [`compose_logs`], shared with the MCP server.
pub(crate) async fn compose_logs_core(file_path: &str, tail: usize) -> Result<String, String> {
    compose::logs(Path::new(file_path), tail)
        .await
        .map_err(|e| e.to_string())
}

/// Lifecycle for a compose project addressed by name only (`compose -p X
/// start|stop|restart`) — this is what the Containers page's project headers
/// use, and it needs no compose file because the containers carry the
/// project's labels.
#[tauri::command]
pub async fn compose_project_action(project: String, action: String) -> Result<String, String> {
    compose_project_action_core(&project, &action).await
}

/// App-free core of [`compose_project_action`], shared with the MCP server.
pub(crate) async fn compose_project_action_core(
    project: &str,
    action: &str,
) -> Result<String, String> {
    compose::project_action(project, action)
        .await
        .map_err(|e| format!("{e:#}"))
}
