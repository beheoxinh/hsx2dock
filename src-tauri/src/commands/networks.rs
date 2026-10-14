use tauri::State;

use crate::docker::models::NetworkItem;
use crate::state::AppState;

#[tauri::command]
pub async fn list_networks(state: State<'_, AppState>) -> Result<Vec<NetworkItem>, String> {
    let client = state.get_client().await?;
    client.list_networks().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_network(
    state: State<'_, AppState>,
    name: String,
    driver: String,
    subnet: Option<String>,
    gateway: Option<String>,
    internal: Option<bool>,
    attachable: Option<bool>,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .create_network(
            &name,
            &driver,
            subnet.as_deref().unwrap_or_default(),
            gateway.as_deref().unwrap_or_default(),
            internal.unwrap_or(false),
            attachable.unwrap_or(true),
            std::collections::HashMap::new(),
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_network(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client.remove_network(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prune_networks(state: State<'_, AppState>) -> Result<u64, String> {
    let client = state.get_client().await?;
    client.prune_networks().await.map_err(|e| e.to_string())
}
