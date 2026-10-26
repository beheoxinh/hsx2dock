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
    {
        let mut cfg = state.config.write().await;
        cfg.container_groups = groups.clone();
        cfg.save().await.map_err(|e| e.to_string())?;
    }
    // Synchronize container restart policies with groups' auto_start setting
    let _ = enforce_groups_restart_policy(state, &groups).await;
    Ok(())
}

/// Enforces Docker restart policy (`"no"` vs `"unless-stopped"`) based on group auto-start settings.
/// When auto-start is OFF, setting restart policy to `"no"` prevents dockerd from resurrecting
/// containers independently upon engine restart.
pub async fn enforce_groups_restart_policy(
    state: &AppState,
    groups: &[ContainerGroup],
) -> Result<(), String> {
    let Ok(client) = state.get_client().await else {
        return Ok(());
    };
    let Ok(containers) = client.list_containers(true).await else {
        return Ok(());
    };

    let allowed_groups: std::collections::HashSet<&str> = groups
        .iter()
        .filter(|g| g.auto_start)
        .map(|g| g.id.as_str())
        .collect();

    for c in &containers {
        // Skip kind cluster nodes (managed separately by kind supervisor)
        if c.kind_cluster.is_some() {
            continue;
        }

        let is_auto_start_on = if let Some(project) = c.compose_project.as_deref() {
            let group_id = format!("compose:{project}");
            allowed_groups.contains(group_id.as_str())
        } else if let Some(g) = groups.iter().find(|g| {
            !g.id.starts_with("compose:")
                && g.id != "__isolated__"
                && g.container_ids.iter().any(|token| {
                    token == &c.id || token == &c.short_id || token == &c.name
                })
        }) {
            allowed_groups.contains(g.id.as_str())
        } else {
            // Isolated container
            allowed_groups.contains("__isolated__")
        };

        let target_policy = if is_auto_start_on {
            "unless-stopped"
        } else {
            "no"
        };

        if let Ok(details) = client.inspect_container(&c.id).await {
            if details.restart_policy != target_policy {
                let _ = client.update_restart_policy(&c.id, target_policy).await;
            }
        }
    }

    Ok(())
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

/// Reconciles container auto-start when the Docker engine starts up:
/// 1. Stops any Compose projects or isolated/custom groups that Docker daemon
///    auto-started via their container restart policy (unless_stopped / always)
///    if their auto_start toggle is OFF.
/// 2. Starts all groups/compose projects that have auto_start turned ON.
pub async fn auto_start_groups(state: &AppState, _restored_since: i64) {
    // Give dockerd a brief moment to finish its internal container restore pass
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let (allowed_groups, all_configured_groups) = {
        let cfg = state.config.read().await;
        let allowed: std::collections::HashSet<String> = cfg
            .container_groups
            .iter()
            .filter(|g| g.auto_start)
            .map(|g| g.id.clone())
            .collect();
        let all: Vec<crate::config::ContainerGroup> = cfg.container_groups.clone();
        (allowed, all)
    };

    // First enforce restart policies on all containers so dockerd won't resurrect
    // containers whose auto-start toggle is off in future restarts
    let _ = enforce_groups_restart_policy(state, &all_configured_groups).await;

    if let Ok(client) = state.get_client().await {
        if let Ok(containers) = client.list_containers(true).await {
            // Find all running containers and stop those whose auto-start is OFF
            for c in containers.iter().filter(|c| c.state == "running") {
                // Ignore kind cluster nodes (managed by reconcile_kind_auto_start)
                if c.kind_cluster.is_some() {
                    continue;
                }

                // Check if this container belongs to a Compose project
                if let Some(project) = c.compose_project.as_deref() {
                    let group_id = format!("compose:{project}");
                    // If compose project auto_start is not enabled (either toggle is off or not in config),
                    // stop this container
                    if !allowed_groups.contains(&group_id) {
                        tracing::info!("Stopping container {} of compose project {}: auto-start is OFF", c.name, project);
                        let _ = client.stop_container(&c.id).await;
                    }
                    continue;
                }

                // Check if this container belongs to a custom group
                let custom_group = all_configured_groups.iter().find(|g| {
                    !g.id.starts_with("compose:")
                        && g.id != "__isolated__"
                        && g.container_ids.iter().any(|token| {
                            token == &c.id || token == &c.short_id || token == &c.name
                        })
                });

                if let Some(g) = custom_group {
                    if !allowed_groups.contains(&g.id) {
                        tracing::info!("Stopping container {} of group {}: auto-start is OFF", c.name, g.name);
                        let _ = client.stop_container(&c.id).await;
                    }
                    continue;
                }

                // Otherwise, this container is isolated (ungrouped)
                if !allowed_groups.contains("__isolated__") {
                    tracing::info!("Stopping isolated container {}: auto-start is OFF", c.name);
                    let _ = client.stop_container(&c.id).await;
                }
            }
        }
    }

    // Now start all groups that have auto_start turned ON
    for id in allowed_groups {
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
