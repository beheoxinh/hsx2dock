use tauri::State;

use crate::docker::models::{
    ContainerDetails, ContainerItem, ContainerStats, ResourceLimits, RunConfig,
};
use crate::state::AppState;

#[tauri::command]
pub async fn list_containers(
    state: State<'_, AppState>,
    all: bool,
) -> Result<Vec<ContainerItem>, String> {
    let client = state.get_client().await?;
    client.list_containers(all).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_container(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client.start_container(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_container(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client.stop_container(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restart_container(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client.restart_container(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pause_container(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client.pause_container(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unpause_container(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client.unpause_container(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_container(
    state: State<'_, AppState>,
    id: String,
    force: bool,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .remove_container(&id, force)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_container(
    state: State<'_, AppState>,
    id: String,
    new_name: String,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .rename_container(&id, &new_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn inspect_container(
    state: State<'_, AppState>,
    id: String,
) -> Result<ContainerDetails, String> {
    let client = state.get_client().await?;
    client.inspect_container(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_container_logs(
    state: State<'_, AppState>,
    id: String,
    tail: String,
) -> Result<String, String> {
    let client = state.get_client().await?;
    client
        .logs_tail(&id, &tail)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_container_stats(
    state: State<'_, AppState>,
    id: String,
) -> Result<ContainerStats, String> {
    let client = state.get_client().await?;
    client.container_stats(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_container(
    state: State<'_, AppState>,
    config: RunConfig,
) -> Result<String, String> {
    config.validate().map_err(|e| e.to_string())?;
    let client = state.get_client().await?;
    let id = client
        .create_container(&config, true)
        .await
        .map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn update_container_resources(
    state: State<'_, AppState>,
    id: String,
    limits: ResourceLimits,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .update_container_limits(&id, &limits)
        .await
        .map_err(|e| e.to_string())
}
