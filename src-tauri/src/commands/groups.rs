//! Container groups: user-defined sets of containers that start and stop
//! together, optionally auto-started when the engine comes up.

use std::collections::HashSet;

use tauri::State;

use crate::config::ContainerGroup;
use crate::state::AppState;

/// All saved container groups, in config order.
#[tauri::command]
pub async fn list_container_groups(
    state: State<'_, AppState>,
) -> Result<Vec<ContainerGroup>, String> {
    Ok(state.config.read().await.container_groups.clone())
}

/// Replaces the whole group list — the UI edits groups as a set and saves
/// the result, which keeps renames/removals/reorders in one round trip.
#[tauri::command]
pub async fn save_container_groups(
    state: State<'_, AppState>,
    groups: Vec<ContainerGroup>,
) -> Result<(), String> {
    validate_groups(&groups)?;
    let mut cfg = state.config.write().await;
    cfg.container_groups = groups;
    cfg.save().await.map_err(|e| e.to_string())
}

/// Starts every stopped member of a group. Returns a one-line summary
/// (`"2 started, 1 already running, 0 missing"`); errors only when nothing
/// could be started.
#[tauri::command]
pub async fn start_container_group(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    set_group_power(&state, &id, true).await
}

/// Stops every running member of a group; same summary contract as start.
#[tauri::command]
pub async fn stop_container_group(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    set_group_power(&state, &id, false).await
}

fn validate_groups(groups: &[ContainerGroup]) -> Result<(), String> {
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    for g in groups {
        let name = g.name.trim();
        if name.is_empty() {
            return Err("Every group needs a name".to_string());
        }
        if !names.insert(name.to_lowercase()) {
            return Err(format!("Duplicate group name '{name}'"));
        }
        let id = g.id.trim();
        if id.is_empty() {
            return Err(format!("Group '{name}' has no id"));
        }
        if !ids.insert(id.to_string()) {
            return Err(format!("Duplicate group id '{id}'"));
        }
    }
    Ok(())
}

async fn set_group_power(
    state: &AppState,
    id: &str,
    start: bool,
) -> Result<String, String> {
    let members = {
        let cfg = state.config.read().await;
        cfg.container_groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.container_ids.clone())
            .ok_or_else(|| format!("Group '{id}' no longer exists"))?
    };
    if members.is_empty() {
        return Err("The group has no containers".to_string());
    }
    let client = state.get_client().await?;
    let live = client
        .list_containers(true)
        .await
        .map_err(|e| e.to_string())?;
    let mut done = 0usize;
    let mut skipped = 0usize;
    let mut missing = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for member in &members {
        // Members are stored by id, but accepting names/short ids keeps
        // groups working across engine data migrations.
        let Some(item) = live
            .iter()
            .find(|c| c.id == *member || c.short_id == *member || c.name == *member)
        else {
            missing += 1;
            continue;
        };
        if item.running == start {
            skipped += 1;
            continue;
        }
        let op = if start {
            client.start_container(&item.id).await
        } else {
            client.stop_container(&item.id).await
        };
        match op {
            Ok(()) => done += 1,
            Err(e) => errors.push(format!("{}: {e}", item.name)),
        }
    }
    let verb = if start { "started" } else { "stopped" };
    let mut summary = format!("{done} {verb}, {skipped} skipped, {missing} missing");
    if !errors.is_empty() {
        summary.push_str(&format!("; failed: {}", errors.join("; ")));
    }
    if done == 0 && skipped == 0 && !errors.is_empty() {
        return Err(summary);
    }
    Ok(summary)
}

/// Starts every group flagged `auto_start` once the engine is up (app open,
/// manual engine start). Failures are logged, never propagated — a missing
/// container must not break startup.
pub async fn auto_start_groups(state: &AppState) {
    let ids: Vec<String> = {
        let cfg = state.config.read().await;
        cfg.container_groups
            .iter()
            .filter(|g| g.auto_start)
            .map(|g| g.id.clone())
            .collect()
    };
    if ids.is_empty() {
        return;
    }
    for id in ids {
        match set_group_power(state, &id, true).await {
            Ok(summary) => tracing::info!("auto-start group {id}: {summary}"),
            Err(e) => tracing::warn!("auto-start group {id} failed: {e}"),
        }
    }
}
