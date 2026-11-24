//! Container groups: user-defined sets of containers that start and stop
//! together, optionally auto-started when the engine comes up.
//!
//! Bulk power actions run with bounded concurrency and report every member
//! through [`crate::ops::OpsRegistry`], so the UI shows which containers are
//! queued or starting instead of freezing until the whole group is done.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use tauri::State;

use crate::config::ContainerGroup;
use crate::docker::client::DockerClient;
use crate::docker::models::ContainerItem;
use crate::ops::BULK_CONCURRENCY;
use crate::state::AppState;

/// Synthetic id of the bucket holding every ungrouped container.
pub const ISOLATED_GROUP_ID: &str = "__isolated__";

/// Upper bound for one `docker start`. Starting is normally sub-second; a
/// container stuck in a slow mount or network setup must not pin a slot forever.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// `docker stop` waits the container's own grace period (10s by default)
/// before SIGKILL, so this leaves room for a generous custom one.
const STOP_TIMEOUT: Duration = Duration::from_secs(45);

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
///
/// Restart-policy reconciliation used to run inline here — one inspect per
/// container on the engine — so every drag, rename or toggle waited on a full
/// engine pass. It is now queued in the background and coalesced.
pub(crate) async fn save_container_groups_core(
    state: &Arc<AppState>,
    groups: Vec<ContainerGroup>,
) -> Result<(), String> {
    validate_groups(&groups)?;
    {
        let mut cfg = state.config.write().await;
        cfg.container_groups = groups;
        cfg.save().await.map_err(|e| e.to_string())?;
    }
    schedule_restart_policy_sync(state.clone());
    Ok(())
}

/// Queues one background reconciliation pass. Saves that arrive while a pass
/// is queued share it; the pass reads the groups as they are when it runs.
pub fn schedule_restart_policy_sync(state: Arc<AppState>) {
    if state.restart_policy_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    crate::runtime::spawn(async move {
        let _guard = state.restart_policy_lock.lock().await;
        state.restart_policy_pending.store(false, Ordering::Release);
        let groups = state.config.read().await.container_groups.clone();
        if let Err(e) = enforce_groups_restart_policy(&state, &groups).await {
            tracing::warn!("restart policy sync failed: {e}");
        }
    });
}

/// True when `token` (a stored group member) names `c`.
fn member_matches(c: &ContainerItem, token: &str) -> bool {
    let token = token.trim_start_matches('/');
    token == c.id || token == c.short_id || token == c.name
}

/// The group a non-compose container belongs to, if any.
fn custom_group_of<'a>(groups: &'a [ContainerGroup], c: &ContainerItem) -> Option<&'a ContainerGroup> {
    groups.iter().find(|g| {
        !g.id.starts_with("compose:")
            && g.id != ISOLATED_GROUP_ID
            && g.container_ids.iter().any(|t| member_matches(c, t))
    })
}

/// Whether the group that owns `c` has auto-start on. Kind nodes return `None`:
/// their lifecycle belongs to the Kubernetes supervisor.
fn auto_start_for(groups: &[ContainerGroup], allowed: &HashSet<&str>, c: &ContainerItem) -> Option<bool> {
    if c.kind_cluster.is_some() {
        return None;
    }
    Some(if let Some(project) = c.compose_project.as_deref() {
        allowed.contains(format!("compose:{project}").as_str())
    } else if let Some(g) = custom_group_of(groups, c) {
        allowed.contains(g.id.as_str())
    } else {
        allowed.contains(ISOLATED_GROUP_ID)
    })
}

/// Enforces Docker restart policy (`"no"` vs `"unless-stopped"`) based on group auto-start settings.
/// When auto-start is OFF, setting restart policy to `"no"` prevents dockerd from resurrecting
/// containers independently upon engine restart.
///
/// Inspects run with bounded concurrency, and a container whose policy was
/// already seen at the target value is skipped without an inspect.
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

    let allowed: HashSet<&str> = groups
        .iter()
        .filter(|g| g.auto_start)
        .map(|g| g.id.as_str())
        .collect();

    let work: Vec<(String, &'static str)> = {
        let seen = state.restart_policy_seen.lock().map_err(|e| e.to_string())?;
        containers
            .iter()
            .filter_map(|c| {
                let on = auto_start_for(groups, &allowed, c)?;
                let target = if on { "unless-stopped" } else { "no" };
                (seen.get(&c.id).map(String::as_str) != Some(target)).then(|| (c.id.clone(), target))
            })
            .collect()
    };
    if work.is_empty() {
        return Ok(());
    }

    // Futures are built up front (not in a stream closure) so the spawned
    // task's future stays `Send` for every lifetime.
    let futs: Vec<_> = work
        .into_iter()
        .map(|(id, target)| {
            let client = client.clone();
            async move {
                let current = match client.inspect_container(&id).await {
                    Ok(d) => d.restart_policy,
                    Err(_) => return (id, None),
                };
                if current == target {
                    return (id, Some(current));
                }
                match client.update_restart_policy(&id, target).await {
                    Ok(()) => (id, Some(target.to_string())),
                    Err(_) => (id, Some(current)),
                }
            }
        })
        .collect();
    let results: Vec<(String, Option<String>)> =
        stream::iter(futs).buffer_unordered(8).collect().await;

    if let Ok(mut seen) = state.restart_policy_seen.lock() {
        // Ids of removed containers would otherwise accumulate forever.
        let live: HashSet<&str> = containers.iter().map(|c| c.id.as_str()).collect();
        seen.retain(|id, _| live.contains(id.as_str()));
        for (id, policy) in results {
            if let Some(p) = policy {
                seen.insert(id, p);
            }
        }
    }
    Ok(())
}

/// Starts every stopped member of a group.
///
/// Returns as soon as the pass is scheduled; progress arrives as
/// `container-op` / `group-op` events and the final summary on the last
/// `group-op`. Fails up front only when the group is already busy.
#[tauri::command]
pub async fn start_container_group(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<String, String> {
    spawn_group_power(state.inner().clone(), id, true)
}

/// Stops every running member of a group; same contract as start.
#[tauri::command]
pub async fn stop_container_group(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<String, String> {
    spawn_group_power(state.inner().clone(), id, false)
}

fn spawn_group_power(state: Arc<AppState>, id: String, start: bool) -> Result<String, String> {
    if state.ops.group_is_busy(&id) {
        return Err("This group is already being started or stopped".to_string());
    }
    crate::runtime::spawn(async move {
        if let Err(e) = set_group_power(&state, &id, start).await {
            tracing::warn!("group {id} power change failed: {e}");
        }
    });
    Ok("scheduled".to_string())
}

/// Lists ops in flight so a freshly mounted page can restore its spinners.
#[tauri::command]
pub async fn list_container_ops(
    state: State<'_, Arc<AppState>>,
) -> Result<crate::ops::OpsSnapshot, String> {
    Ok(state.ops.snapshot())
}

/// App-free, awaited form of a group start (tray, MCP).
pub(crate) async fn start_container_group_core(
    state: &AppState,
    id: &str,
) -> Result<String, String> {
    set_group_power(state, id, true).await
}

/// App-free, awaited form of a group stop (tray, MCP).
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

/// Members of a saved group (or of the Isolated bucket), resolved against the
/// live list and de-duplicated, in the group's order.
async fn resolve_members(
    state: &AppState,
    live: &[ContainerItem],
    id: &str,
) -> Result<Vec<ContainerItem>, String> {
    let cfg = state.config.read().await;
    if id == ISOLATED_GROUP_ID {
        return Ok(live
            .iter()
            .filter(|c| {
                // Kind nodes are clusters, not loose containers: starting
                // them here would boot a cluster the user switched off.
                c.compose_project.is_none()
                    && c.kind_cluster.is_none()
                    && custom_group_of(&cfg.container_groups, c).is_none()
            })
            .cloned()
            .collect());
    }
    let group = cfg
        .container_groups
        .iter()
        .find(|g| g.id == id)
        .ok_or_else(|| format!("Group '{id}' no longer exists"))?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for token in &group.container_ids {
        if let Some(c) = live.iter().find(|c| member_matches(c, token)) {
            if c.kind_cluster.is_none() && seen.insert(c.id.clone()) {
                out.push(c.clone());
            }
        }
    }
    if out.is_empty() && !group.container_ids.is_empty() {
        return Err("None of the group's containers exist any more".to_string());
    }
    Ok(out)
}

/// One lifecycle call with a deadline. A paused container cannot be started,
/// only unpaused, so "start" does the right thing for it.
async fn power_one(client: &DockerClient, c: &ContainerItem, start: bool) -> Result<(), String> {
    let fut = async {
        if start {
            if c.state == "paused" {
                client.unpause_container(&c.id).await
            } else {
                client.start_container(&c.id).await
            }
        } else {
            client.stop_container(&c.id).await
        }
    };
    let limit = if start { START_TIMEOUT } else { STOP_TIMEOUT };
    match tokio::time::timeout(limit, fut).await {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(_) => Err(format!("timed out after {}s", limit.as_secs())),
    }
}

/// Starts or stops `targets` with bounded concurrency, reporting each one.
/// Returns `(done, failures)`; containers another operation already owns are
/// counted in `busy`.
async fn power_many(
    state: &AppState,
    client: &DockerClient,
    group: Option<&str>,
    targets: Vec<ContainerItem>,
    start: bool,
) -> (usize, usize, Vec<String>) {
    let op = if start { "start" } else { "stop" };
    let mut busy = 0usize;
    let claimed: Vec<ContainerItem> = targets
        .into_iter()
        .filter(|c| {
            let ok = state.ops.claim(&c.id, op, group);
            if !ok {
                busy += 1;
            }
            ok
        })
        .collect();

    // Issued in group order so earlier members (a database, say) get their
    // start call first, but never more than BULK_CONCURRENCY at a time.
    let futs: Vec<_> = claimed
        .into_iter()
        .map(|c| async move {
            state.ops.running(&c.id);
            let r = power_one(client, &c, start).await;
            state.ops.finish(&c.id, r.clone());
            if let Some(g) = group {
                state.ops.group_progress(g);
            }
            (c.name, r)
        })
        .collect();
    let results: Vec<(String, Result<(), String>)> = stream::iter(futs)
        .buffer_unordered(BULK_CONCURRENCY)
        .collect()
        .await;

    let mut done = 0usize;
    let mut errors = Vec::new();
    for (name, r) in results {
        match r {
            Ok(()) => done += 1,
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    (done, busy, errors)
}

pub(crate) async fn set_group_power(
    state: &AppState,
    id: &str,
    start: bool,
) -> Result<String, String> {
    let op = if start { "start" } else { "stop" };

    if let Some(project) = id.strip_prefix("compose:") {
        // Compose owns its containers, so the project CLI does the work; the
        // members are still marked busy so their cards show the spinner.
        let members: Vec<String> = match state.get_client().await {
            Ok(client) => client
                .list_containers(true)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|c| c.compose_project.as_deref() == Some(project) && c.running != start)
                .map(|c| c.id)
                .collect(),
            Err(_) => Vec::new(),
        };
        if !state.ops.begin_group(id, op, members.len().max(1)) {
            return Err("This project is already being started or stopped".to_string());
        }
        let claimed: Vec<String> = members
            .into_iter()
            .filter(|cid| state.ops.claim(cid, op, Some(id)))
            .collect();
        for cid in &claimed {
            state.ops.running(cid);
        }
        let result = crate::commands::compose::compose_project_action_core(project, op).await;
        for cid in &claimed {
            state.ops.finish(cid, result.as_ref().map(|_| ()).map_err(Clone::clone));
        }
        state.ops.end_group(id, &result);
        return result;
    }

    if state.ops.group_is_busy(id) {
        return Err("This group is already being started or stopped".to_string());
    }

    // Whether this pass owns the group entry. Only the owner may end it: a
    // pass refused because another one is running must not clear that one.
    let began = std::sync::atomic::AtomicBool::new(false);
    let run = async {
        let client = state.get_client().await?;
        let live = client
            .list_containers(true)
            .await
            .map_err(|e| e.to_string())?;
        let members = resolve_members(state, &live, id).await?;
        if members.is_empty() {
            return Err("The group has no containers".to_string());
        }
        let total = members.len();
        let targets: Vec<ContainerItem> = members
            .into_iter()
            .filter(|c| if start { !c.running } else { c.running || c.state == "paused" || c.state == "restarting" })
            .collect();
        let skipped = total - targets.len();
        if !state.ops.begin_group(id, op, targets.len()) {
            return Err("This group is already being started or stopped".to_string());
        }
        began.store(true, std::sync::atomic::Ordering::Relaxed);
        let (done, busy, errors) = power_many(state, &client, Some(id), targets, start).await;
        let verb = if start { "started" } else { "stopped" };
        let mut summary = format!("{done} {verb}, {} skipped", skipped + busy);
        if !errors.is_empty() {
            summary.push_str(&format!("; failed: {}", errors.join("; ")));
        }
        if done == 0 && skipped + busy == 0 && !errors.is_empty() {
            return Err(summary);
        }
        Ok(summary)
    };
    let result = run.await;
    if began.load(std::sync::atomic::Ordering::Relaxed) {
        state.ops.end_group(id, &result);
    } else if result.is_err() && state.ops.begin_group(id, op, 0) {
        // Failed before the pass began (engine down, empty group): the caller
        // already got "scheduled", so the failure still has to reach the UI.
        state.ops.end_group(id, &result);
    }
    result
}

/// Reconciles container auto-start when the Docker engine starts up:
/// 1. Stops any Compose projects or isolated/custom groups that Docker daemon
///    auto-started via their container restart policy (unless_stopped / always)
///    if their auto_start toggle is OFF.
/// 2. Starts all groups/compose projects that have auto_start turned ON.
///
/// Both passes run with bounded concurrency and report through the ops
/// registry, so a large engine no longer takes one container at a time.
pub async fn auto_start_groups(state: &AppState, _restored_since: i64) {
    // Give dockerd a brief moment to finish its internal container restore pass
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let groups: Vec<ContainerGroup> = state.config.read().await.container_groups.clone();
    let allowed: HashSet<&str> = groups
        .iter()
        .filter(|g| g.auto_start)
        .map(|g| g.id.as_str())
        .collect();

    // A fresh engine may hold containers changed behind the app's back, so
    // the policy pass starts from a clean slate here.
    if let Ok(mut seen) = state.restart_policy_seen.lock() {
        seen.clear();
    }
    {
        let _guard = state.restart_policy_lock.lock().await;
        let _ = enforce_groups_restart_policy(state, &groups).await;
    }

    if let Ok(client) = state.get_client().await {
        if let Ok(containers) = client.list_containers(true).await {
            let to_stop: Vec<ContainerItem> = containers
                .into_iter()
                .filter(|c| c.state == "running")
                .filter(|c| auto_start_for(&groups, &allowed, c) == Some(false))
                .collect();
            if !to_stop.is_empty() {
                tracing::info!("auto-start: stopping {} containers whose group is off", to_stop.len());
                let (_, _, errors) = power_many(state, &client, None, to_stop, false).await;
                for e in errors {
                    tracing::warn!("auto-start stop failed: {e}");
                }
            }
        }
    }

    // Isolated first is arbitrary; groups run one after another so the
    // overall concurrency stays at BULK_CONCURRENCY.
    let mut ids: Vec<String> = allowed.iter().map(|s| s.to_string()).collect();
    ids.sort();
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
