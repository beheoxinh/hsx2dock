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

/// Runs one lifecycle call while the container is claimed in the ops registry,
/// so a single-container action cannot race a group pass or a double click.
async fn claimed_lifecycle<Fut>(state: &AppState, id: &str, op: &str, call: Fut) -> Result<(), String>
where
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let claim = state
        .ops
        .claim_guard(id, op, None)
        .ok_or_else(|| "This container is already being started, stopped or restarted".to_string())?;
    claim.running();
    let result = call.await;
    claim.finish(result.clone());
    result
}

#[tauri::command]
pub async fn start_container(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    claimed_lifecycle(&state, &id, "start", async {
        let client = state.get_client().await?;
        client.start_container(&id).await.map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn stop_container(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    claimed_lifecycle(&state, &id, "stop", async {
        let client = state.get_client().await?;
        client.stop_container(&id).await.map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn restart_container(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    claimed_lifecycle(&state, &id, "restart", async {
        let client = state.get_client().await?;
        client
            .restart_container(&id)
            .await
            .map_err(|e| e.to_string())
    })
    .await
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
    rename_container_core(&state, &id, &new_name).await
}

/// Renames a container and carries its name-based references along.
///
/// Gateway routes map to containers by name, and group members may be stored
/// by name too, so a rename silently orphaned both: the route showed as
/// unmapped and the container fell out of its group.
pub(crate) async fn rename_container_core(
    state: &AppState,
    id: &str,
    new_name: &str,
) -> Result<(), String> {
    let client = state.get_client().await?;
    // Read the old name first: without it the routes and groups that refer to
    // the container by name cannot be found, so do not rename at all.
    let old = client
        .inspect_container(id)
        .await
        .map_err(|e| format!("Cannot read the container before renaming it: {e}"))?;
    client
        .rename_container(id, new_name)
        .await
        .map_err(|e| e.to_string())?;
    let old_name = old.name.trim_start_matches('/').to_string();
    let new_name = new_name.trim_start_matches('/');
    if old_name.is_empty() || old_name == new_name {
        return Ok(());
    }

    let mut cfg = state.config.write().await;
    let mut next = cfg.clone();
    let mut routes_changed = false;
    for r in next.gateway_routes.iter_mut() {
        if r.container_name.as_deref().map(|n| n.trim_start_matches('/')) == Some(old_name.as_str()) {
            r.container_name = Some(new_name.to_string());
            routes_changed = true;
        }
    }
    let mut groups_changed = false;
    for g in next.container_groups.iter_mut() {
        for m in g.container_ids.iter_mut() {
            if m.trim_start_matches('/') == old_name {
                *m = new_name.to_string();
                groups_changed = true;
            }
        }
    }
    if !routes_changed && !groups_changed {
        return Ok(());
    }
    // The rename itself succeeded; a failed bookkeeping save is reported so
    // the user knows the routes/groups still use the old name.
    next.save()
        .await
        .map_err(|e| format!("Renamed, but updating routes/groups failed: {e}"))?;
    *cfg = next;
    if groups_changed {
        state
            .ops
            .emit("container-groups-changed", &cfg.container_groups);
    }
    Ok(())
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
    timestamps: Option<bool>,
) -> Result<String, String> {
    let client = state.get_client().await?;
    client
        .logs_tail(&id, &tail, timestamps.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_container_stats(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<ContainerStats, String> {
    let client = state.get_client().await?;
    state
        .container_stats(&client, &id)
        .await
        .map_err(|e| e.to_string())
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

/// Fills the settings the edit form does not show from the container being
/// replaced. The form only sends image, name, command, ports, env, mounts,
/// network, restart and limits; everything else used to be dropped on
/// recreate — compose labels (the container left its project), entrypoint,
/// user, workdir, healthcheck, privileged/tty.
fn inherit_unmanaged_settings(config: &mut RunConfig, details: &crate::docker::models::ContainerDetails) {
    if config.entrypoint.is_empty() {
        config.entrypoint = details.entrypoint.clone();
    }
    if config.labels.is_empty() {
        config.labels = details.labels.clone();
    }
    // Docker defaults the hostname to the short id; carrying that over would
    // name the new container after the old one.
    if config.hostname.trim().is_empty()
        && !details.hostname.is_empty()
        && !details.id.starts_with(&details.hostname)
    {
        config.hostname = details.hostname.clone();
    }
    if config.workdir.trim().is_empty() {
        config.workdir = details.workdir.clone();
    }
    if config.user.trim().is_empty() {
        config.user = details.user.clone();
    }
    if !config.healthcheck.is_enabled() {
        config.healthcheck = details.healthcheck.clone();
    }
    if config.restart_max_retries.trim().is_empty()
        && config.restart_policy == crate::docker::models::RestartPolicy::OnFailure
        && details.restart_max_retries.trim().parse::<u32>().is_ok_and(|n| n > 0)
    {
        config.restart_max_retries = details.restart_max_retries.clone();
    }
    config.privileged |= details.privileged;
    config.tty |= details.tty;
    config.stdin_open |= details.stdin_open;
    config.read_only_rootfs |= details.read_only_rootfs;
}

/// Core function that recreates a container from an updated [`RunConfig`].
///
/// Workflow:
/// 1. Inspects the original container to check it exists and ensure it is not a kind node.
/// 2. If the config has no name, retains the original container's name.
/// 3. Validates the updated configuration.
/// 4. Finds which container group the original container belonged to, if any.
/// 5. Stops the original and renames it aside.
/// 6. Creates and starts the new container; on failure removes it and puts
///    the original back (name and running state). Only on success is the
///    original removed, keeping its anonymous volumes on disk.
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
    inherit_unmanaged_settings(&mut config, &details);

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

    // The old container is kept (renamed aside) until the new one runs. It
    // used to be force-removed first — with its anonymous volumes — so a bad
    // image or a port clash left the user with no container at all.
    let was_running = matches!(details.state.as_str(), "running" | "restarting" | "paused");
    let original_name = details.name.trim_start_matches('/').to_string();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let parked_name = format!("{original_name}-old-{stamp}");
    if was_running {
        client
            .stop_container(&details.id)
            .await
            .map_err(|e| format!("Could not stop the container before recreating it: {e}"))?;
    }
    if let Err(e) = client.rename_container(&details.id, &parked_name).await {
        if was_running {
            let _ = client.start_container(&details.id).await;
        }
        return Err(format!("Could not set the old container aside: {e}"));
    }

    // Puts the original back the way it was after a failed attempt.
    let restore = |reason: String| {
        let client = client.clone();
        let old_id = details.id.clone();
        let original_name = original_name.clone();
        async move {
            let mut msg = reason;
            if let Err(e) = client.rename_container(&old_id, &original_name).await {
                msg.push_str(&format!(" Restoring the original name also failed: {e}."));
            }
            if was_running {
                if let Err(e) = client.start_container(&old_id).await {
                    msg.push_str(&format!(" Restarting the original container failed: {e}."));
                }
            }
            msg
        }
    };

    // Created stopped (even for `detach`) so a start failure leaves us the
    // id to clean up.
    let mut create_cfg = config.clone();
    create_cfg.detach = false;
    let new_id = match client.create_container(&create_cfg, false).await {
        Ok(nid) => nid,
        Err(e) => {
            return Err(restore(format!(
                "Creating the new container failed: {e}. The original container was kept."
            ))
            .await);
        }
    };
    if let Err(e) = client.start_container(&new_id).await {
        let _ = client.remove_container(&new_id, true).await;
        return Err(restore(format!(
            "The new container failed to start: {e}. The original container was kept."
        ))
        .await);
    }
    // Anonymous volumes of the old container stay on disk (unused) rather
    // than being deleted with it.
    if let Err(e) = client.remove_container_with(&details.id, true, false).await {
        tracing::warn!("recreate: could not remove the old container {parked_name}: {e}");
    }

    // If the old container belonged to a group, update that group's container_ids
    if let Some((group_id, pos)) = old_group_info {
        let mut cfg = state.config.write().await;
        if let Some(group) = cfg.container_groups.iter_mut().find(|g| g.id == group_id) {
            if pos < group.container_ids.len() {
                group.container_ids[pos] = new_id.clone();
            } else {
                group.container_ids.push(new_id.clone());
            }
            if let Err(e) = cfg.save().await {
                tracing::error!("recreate: could not save group membership for {group_id}: {e}");
            }
            state
                .ops
                .emit("container-groups-changed", &cfg.container_groups);
        }
    }

    Ok(new_id)
}
