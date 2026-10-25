use std::sync::Arc;
use tauri::State;

use crate::docker::models::{
    ContainerDetails, ContainerItem, ContainerStats, ResourceLimits, RunConfig,
};
use crate::state::AppState;

#[tauri::command]
pub async fn list_containers(
    state: State<'_, Arc<AppState>>,
    all: bool,
) -> Result<Vec<ContainerItem>, String> {
    let client = state.get_client().await?;
    client.list_containers(all).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_container(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client.start_container(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_container(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client.stop_container(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restart_container(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .restart_container(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pause_container(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client.pause_container(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unpause_container(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .unpause_container(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_container(
    state: State<'_, Arc<AppState>>,
    id: String,
    force: bool,
) -> Result<(), String> {
    let client = state.get_client().await?;
    // kind node containers *are* the cluster: deleting one silently destroys
    // user data. Cluster lifecycle is owned by the Kubernetes page.
    let details = client
        .inspect_container(&id)
        .await
        .map_err(|e| e.to_string())?;
    if details
        .labels
        .iter()
        .any(|label| label.key == crate::docker::kind::CLUSTER_LABEL)
    {
        return Err(
            "This container is a kind cluster node. Use the Kubernetes page to \
             start, stop or delete the cluster."
                .to_string(),
        );
    }
    client
        .remove_container(&id, force)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_container(
    state: State<'_, Arc<AppState>>,
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
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<ContainerDetails, String> {
    let client = state.get_client().await?;
    client
        .inspect_container(&id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_container_logs(
    state: State<'_, Arc<AppState>>,
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
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<ContainerStats, String> {
    let client = state.get_client().await?;
    client.container_stats(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_container_sizes(
    state: State<'_, Arc<AppState>>,
) -> Result<std::collections::HashMap<String, u64>, String> {
    let client = state.get_client().await?;
    client.container_sizes().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_container(
    state: State<'_, Arc<AppState>>,
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
    state: State<'_, Arc<AppState>>,
    id: String,
    limits: ResourceLimits,
) -> Result<(), String> {
    update_container_resources_core(&state, &id, limits).await
}

pub(crate) async fn update_container_resources_core(
    state: &AppState,
    id: &str,
    limits: ResourceLimits,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .update_container_limits(id, &limits)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recreate_container(
    state: State<'_, Arc<AppState>>,
    id: String,
    config: RunConfig,
) -> Result<String, String> {
    recreate_container_core(&state, &id, config).await
}

/// Core function that recreates a container from an updated [`RunConfig`].
///
/// Workflow:
/// 1. Inspects the original container to check it exists and ensure it is not a kind node.
/// 2. If the config has no name, retains the original container's name.
/// 3. Validates the updated configuration.
/// 4. Finds which container group the original container belonged to, if any.
/// 5. Removes the original container (forced, so running ones stop cleanly first).
/// 6. Creates and starts the new container with the updated configuration.
/// 7. Re-attaches the new container ID to the original group if applicable.
pub(crate) async fn recreate_container_core(
    state: &AppState,
    id: &str,
    mut config: RunConfig,
) -> Result<String, String> {
    let client = state.get_client().await?;
    let details = client
        .inspect_container(id)
        .await
        .map_err(|e| format!("Cannot inspect container to recreate: {e}"))?;

    if details
        .labels
        .iter()
        .any(|label| label.key == crate::docker::kind::CLUSTER_LABEL)
    {
        return Err(
            "This container is a kind cluster node. It cannot be recreated \
             through container edit."
                .to_string(),
        );
    }

    if config.name.trim().is_empty() {
        config.name = details.name.clone();
    }

    config.validate().map_err(|e| e.to_string())?;

    // Check if the old container was in a container group so we can preserve membership
    let old_group_info: Option<(String, usize)> = {
        let cfg = state.config.read().await;
        cfg.container_groups.iter().find_map(|g| {
            g.container_ids
                .iter()
                .position(|cid| cid == id || cid == &details.id || cid == &details.short_id)
                .map(|pos| (g.id.clone(), pos))
        })
    };

    // Remove the old container cleanly with force = true
    client
        .remove_container(id, true)
        .await
        .map_err(|e| format!("Failed to stop and remove previous container: {e}"))?;

    // Create and start the new container with the modified config
    let new_id = match client.create_container(&config, true).await {
        Ok(nid) => nid,
        Err(e) => {
            return Err(format!(
                "Old container was removed, but creating new container failed: {e}. \
                 Check image availability and port conflicts."
            ));
        }
    };

    // If the old container belonged to a group, update that group's container_ids
    if let Some((group_id, pos)) = old_group_info {
        let mut cfg = state.config.write().await;
        if let Some(group) = cfg.container_groups.iter_mut().find(|g| g.id == group_id) {
            if pos < group.container_ids.len() {
                group.container_ids[pos] = new_id.clone();
            } else {
                group.container_ids.push(new_id.clone());
            }
            let _ = cfg.save().await;
        }
    }

    Ok(new_id)
}
