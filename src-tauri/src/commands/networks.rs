use std::sync::Arc;
use tauri::State;

use crate::docker::models::NetworkItem;
use crate::state::AppState;

#[tauri::command]
pub async fn list_networks(state: State<'_, Arc<AppState>>) -> Result<Vec<NetworkItem>, String> {
    list_networks_core(&state).await
}

/// App-free core of [`list_networks`], shared with the MCP server.
pub(crate) async fn list_networks_core(state: &AppState) -> Result<Vec<NetworkItem>, String> {
    let client = state.get_client().await?;
    client.list_networks().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_network(
    state: State<'_, Arc<AppState>>,
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
pub async fn remove_network(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    remove_network_core(&state, &id).await
}

/// App-free core of [`remove_network`].
pub(crate) async fn remove_network_core(state: &AppState, id: &str) -> Result<(), String> {
    let client = state.get_client().await?;
    let networks = client.list_networks().await.map_err(|e| e.to_string())?;
    if networks
        .iter()
        .any(|n| n.protected && (n.id == id || n.name == id))
    {
        return Err(
            "The 'kind' network carries every cluster node. Remove the cluster \
             from the Kubernetes page instead."
                .to_string(),
        );
    }
    client.remove_network(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prune_networks(state: State<'_, Arc<AppState>>) -> Result<u64, String> {
    prune_networks_core(&state).await
}

/// App-free core of [`prune_networks`].
pub(crate) async fn prune_networks_core(state: &AppState) -> Result<u64, String> {
    let client = state.get_client().await?;
    client.prune_networks().await.map_err(|e| e.to_string())
}
