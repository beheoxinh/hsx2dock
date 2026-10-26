//! Container groups: user-defined sets of containers that start and stop
//! together, optionally auto-started when the engine comes up.

use std::collections::HashSet;

use std::sync::Arc;
use tauri::State;

use crate::config::ContainerGroup;
use crate::state::AppState;

/// All saved container groups, in config order.
#[tauri::command]
pub async fn list_container_groups(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ContainerGroup>, String> {
    list_container_groups_core(&state).await
}

/// App-free core of [`list_container_groups`], shared with the MCP server.
pub(crate) async fn list_container_groups_core(
    state: &AppState,
) -> Result<Vec<ContainerGroup>, String> {
    Ok(state.config.read().await.container_groups.clone())
}

/// Replaces the whole group list — the UI edits groups as a set and saves
/// the result, which keeps renames/removals/reorders in one round trip.
#[tauri::command]
pub async fn save_container_groups(
    state: State<'_, Arc<AppState>>,
    groups: Vec<ContainerGroup>,
) -> Result<(), String> {
    save_container_groups_core(&state, groups).await
}

/// App-free core of [`save_container_groups`].
pub(crate) async fn save_container_groups_core(
    state: &AppState,
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
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<String, String> {
    start_container_group_core(&state, &id).await
}

/// App-free core of [`start_container_group`].
pub(crate) async fn start_container_group_core(
    state: &AppState,
    id: &str,
) -> Result<String, String> {
    set_group_power(state, id, true).await
}

/// Stops every running member of a group; same summary contract as start.
#[tauri::command]
pub async fn stop_container_group(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<String, String> {
    stop_container_group_core(&state, &id).await
}

/// App-free core of [`stop_container_group`].
pub(crate) async fn stop_container_group_core(
    state: &AppState,
    id: &str,
) -> Result<String, String> {
    set_group_power(state, id, false).await
}

fn validate_groups(groups: &[ContainerGroup]) -> Result<(), String> {
    let mut names = HashSet::new();
    let mut ids = HashSet::new();
    // A container lives in exactly one group (like a file in one folder), so a
    // member claimed twice is rejected rather than silently duplicated.
    let mut members: HashSet<&str> = HashSet::new();
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
        for member in &g.container_ids {
            let member = member.trim();
            if member.is_empty() {
                return Err(format!("Group '{name}' has an empty member id"));
            }
            if !members.insert(member) {
                return Err(format!(
                    "Container '{member}' is listed in more than one group"
                ));
            }
        }
    }
    Ok(())
}

pub(crate) async fn set_group_power(
    state: &AppState,
    id: &str,
    start: bool,
) -> Result<String, String> {
    if let Some(project) = id.strip_prefix("compose:") {
        let action = if start { "start" } else { "stop" };
        return crate::commands::compose::compose_project_action_core(project, action).await;
    }

    let client = state.get_client().await?;
    let live = client
        .list_containers(true)
        .await
        .map_err(|e| e.to_string())?;

    let members: Vec<String> = if id == "__isolated__" {
        let cfg = state.config.read().await;
        let mut grouped_tokens = std::collections::HashSet::new();
        for g in &cfg.container_groups {
            if !g.id.starts_with("compose:") && g.id != "__isolated__" {
                for token in &g.container_ids {
                    grouped_tokens.insert(token.as_str());
                }
            }
        }
        live.iter()
            .filter(|c| {
                c.compose_project.is_none()
                    && !grouped_tokens.contains(c.id.as_str())
                    && !grouped_tokens.contains(c.short_id.as_str())
                    && !grouped_tokens.contains(c.name.as_str())
            })
            .map(|c| c.id.clone())
            .collect()
    } else {
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

    let mut done = 0usize;
    let mut skipped = 0usize;
    let mut missing = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for member in &members {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn group(id: &str, name: &str, members: &[&str]) -> ContainerGroup {
        ContainerGroup {
            id: id.to_string(),
            name: name.to_string(),
            container_ids: members.iter().map(|m| m.to_string()).collect(),
            auto_start: false,
        }
    }

    #[test]
    fn empty_group_is_valid() {
        // Creating an empty group is the normal flow: members arrive by drag.
        assert!(validate_groups(&[group("g1", "Empty", &[])]).is_ok());
    }

    #[test]
    fn group_names_must_be_unique_and_present() {
        assert!(validate_groups(&[group("g1", "", &[])]).is_err());
        assert!(validate_groups(&[group("g1", "A", &[]), group("g2", " a ", &[])]).is_err());
        assert!(validate_groups(&[group("g1", "A", &[]), group("g1", "B", &[])]).is_err());
    }

    #[test]
    fn a_container_cannot_belong_to_two_groups() {
        let err =
            validate_groups(&[group("g1", "A", &["abc"]), group("g2", "B", &["abc"])]).unwrap_err();
        assert!(err.contains("more than one group"), "{err}");
        // The same container listed twice inside one group is fine to reject
        // too — it is still one membership claimed twice.
        assert!(validate_groups(&[group("g1", "A", &["abc", "abc"])]).is_err());
    }

    #[test]
    fn distinct_members_across_groups_are_accepted() {
        assert!(validate_groups(&[
            group("g1", "A", &["one"]),
            group("g2", "B", &["two", "three"]),
        ])
        .is_ok());
    }
}
