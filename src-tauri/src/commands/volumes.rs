use std::collections::HashMap;
use tauri::State;

use crate::docker::models::VolumeItem;
use crate::state::AppState;

#[tauri::command]
pub async fn list_volumes(state: State<'_, AppState>) -> Result<Vec<VolumeItem>, String> {
    let client = state.get_client().await?;
    client.list_volumes().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_volume(
    state: State<'_, AppState>,
    name: String,
    driver: String,
    driver_opts: HashMap<String, String>,
    labels: HashMap<String, String>,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .create_volume(&name, &driver, driver_opts, labels)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_volume(
    state: State<'_, AppState>,
    name: String,
    force: bool,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .remove_volume(&name, force)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prune_volumes(state: State<'_, AppState>) -> Result<u64, String> {
    let client = state.get_client().await?;
    client.prune_volumes().await.map_err(|e| e.to_string())
}
