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

/// How long a container's applied restart policy is trusted before the
/// enforcement pass inspects it again.
const RESTART_POLICY_RECHECK: std::time::Duration = std::time::Duration::from_secs(300);

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
        // Save a copy first: on a write error memory must keep the old
        // groups, or the next unrelated save would persist the rejected ones.
        let mut next = cfg.clone();
        next.container_groups = groups;
        next.save().await.map_err(|e| e.to_string())?;
        *cfg = next;
        // Emitted under the write lock so listeners see saves in commit order.
        // MCP edits reach the open window through this too; before, the UI
        // kept its stale copy and its next save silently undid them.
        state
            .ops
            .emit("container-groups-changed", &cfg.container_groups);
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
                // A remembered policy is re-checked after a while: `docker
                // update --restart` from a terminal changes it behind the app.
                let known = seen.get(&c.id).is_some_and(|(p, at)| {
                    p == target && at.elapsed() < RESTART_POLICY_RECHECK
                });
                (!known).then(|| (c.id.clone(), target))
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
                if policy_satisfies(&current, target) {
                    // Remembered as the target so the next pass skips it.
                    return (id, Some(target.to_string()));
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
        let now = std::time::Instant::now();
        for (id, policy) in results {
            if let Some(p) = policy {
                seen.insert(id, (p, now));
            }
        }
    }
    Ok(())
}

/// Whether a container's current restart policy already does what the group
/// switch asks. Only the resurrect-on-boot policies are the app's to change:
/// `on-failure[:N]` is a crash-recovery choice the user made (the boot pass
/// stops it if the engine brings it back), and `always` already resurrects.
fn policy_satisfies(current: &str, target: &str) -> bool {
    let current = current.trim();
    if current.starts_with("on-failure") {
        return true;
    }
    match target {
        "unless-stopped" => matches!(current, "unless-stopped" | "always"),
        _ => matches!(current, "" | "no"),
    }
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
        Err(_) => {
            // The daemon keeps carrying out the call after we stop waiting, so
            // a timeout is not proof of failure. Report what it actually reached.
            if let Ok(d) = client.inspect_container(&c.id).await {
                let reached = if start {
                    d.state == "running"
                } else {
                    matches!(d.state.as_str(), "exited" | "created")
                };
                if reached {
                    return Ok(());
                }
            }
            Err(format!("timed out after {}s", limit.as_secs()))
        }
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
    let claimed: Vec<(ContainerItem, crate::ops::ContainerClaim<'_>)> = targets
        .into_iter()
        .filter_map(|c| match state.ops.claim_guard(&c.id, op, group) {
            Some(claim) => Some((c, claim)),
            None => {
                busy += 1;
                None
            }
        })
        .collect();

    // Issued in group order so earlier members (a database, say) get their
    // start call first, but never more than BULK_CONCURRENCY at a time.
    let futs: Vec<_> = claimed
        .into_iter()
        .map(|(c, claim)| async move {
            claim.running();
            let r = power_one(client, &c, start).await;
            claim.finish(r.clone());
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
        // The CLI reports no per-container progress: total 0 makes the UI
        // show "Starting…" instead of a counter stuck at 0/N.
        let Some(pass) = state.ops.begin_group_guard(id, op, 0) else {
            return Err("This project is already being started or stopped".to_string());
        };
        let claimed: Vec<crate::ops::ContainerClaim<'_>> = members
            .iter()
            .filter_map(|cid| state.ops.claim_guard(cid, op, Some(id)))
            .collect();
        for claim in &claimed {
            claim.running();
        }
        let result = crate::commands::compose::compose_project_action_core(project, op).await;
        for claim in claimed {
            claim.finish(result.as_ref().map(|_| ()).map_err(Clone::clone));
        }
        pass.end(&result);
        return result;
    }

    if state.ops.group_is_busy(id) {
        return Err("This group is already being started or stopped".to_string());
    }

    // The guard owns the group entry: only the pass that began it may end it,
    // and a dropped pass ends it as interrupted instead of leaving it busy.
    let mut pass: Option<crate::ops::GroupClaim<'_>> = None;
    let run = async {
        let client = state.get_client().await?;
        let live = client
            .list_containers(true)
            .await
            .map_err(|e| e.to_string())?;
        let members = resolve_members(state, &live, id).await?;
        if members.is_empty() {
            // Not a failure: an empty group (or Isolated with everything
            // grouped) with auto-start on used to toast an error every boot.
            return Ok("nothing to do: the group has no containers".to_string());
        }
        let total = members.len();
        let targets: Vec<ContainerItem> = members
            .into_iter()
            .filter(|c| if start { !c.running } else { c.running || c.state == "paused" || c.state == "restarting" })
            .collect();
        let skipped = total - targets.len();
        let Some(claim) = state.ops.begin_group_guard(id, op, targets.len()) else {
            return Err("This group is already being started or stopped".to_string());
        };
        pass = Some(claim);
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
    if let Some(claim) = pass {
        claim.end(&result);
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
pub async fn auto_start_groups(state: &AppState, restored_since: i64) {
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

    let mut live_projects: HashSet<String> = HashSet::new();
    if let Ok(client) = state.get_client().await {
        if let Ok(containers) = client.list_containers(true).await {
            live_projects = containers
                .iter()
                .filter_map(|c| c.compose_project.clone())
                .collect();
            let candidates: Vec<ContainerItem> = containers
                .into_iter()
                .filter(|c| c.state == "running")
                .filter(|c| auto_start_for(&groups, &allowed, c) == Some(false))
                .collect();
            // Only what Docker restored during this engine start is undone.
            // A container already running before the app opened (the engine
            // outlived the app, or belongs to another owner) was started on
            // purpose and is left alone.
            let to_stop: Vec<ContainerItem> = if restored_since > 0 {
                let checks: Vec<_> = candidates
                    .into_iter()
                    .map(|c| {
                        let client = client.clone();
                        async move {
                            let started = client.container_started_at(&c.id).await.unwrap_or(0);
                            (started >= restored_since).then_some(c)
                        }
                    })
                    .collect();
                stream::iter(checks)
                    .buffer_unordered(BULK_CONCURRENCY)
                    .filter_map(|c| async move { c })
                    .collect()
                    .await
            } else {
                candidates
            };
            if !to_stop.is_empty() {
                tracing::info!("auto-start: stopping {} containers whose group is off", to_stop.len());
                // One summarised failure instead of a toast per container.
                const PASS_ID: &str = "__autostart__";
                if state.ops.begin_group(PASS_ID, "stop", to_stop.len()) {
                    let (done, busy, errors) =
                        power_many(state, &client, Some(PASS_ID), to_stop, false).await;
                    let mut summary = format!("{done} stopped, {busy} skipped");
                    if !errors.is_empty() {
                        summary.push_str(&format!("; failed: {}", errors.join("; ")));
                    }
                    state.ops.end_group(PASS_ID, &Ok(summary));
                }
            }
        }
    }

    // Isolated first is arbitrary; groups run one after another so the
    // overall concurrency stays at BULK_CONCURRENCY.
    let mut ids: Vec<String> = allowed
        .iter()
        .filter(|id| match id.strip_prefix("compose:") {
            // A saved switch for a project that no longer exists would fail
            // (and toast) on every boot.
            Some(project) => live_projects.contains(project),
            None => true,
        })
        .map(|s| s.to_string())
        .collect();
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

    #[test]
    fn user_restart_policies_are_kept() {
        assert!(policy_satisfies("on-failure", "no"));
        assert!(policy_satisfies("on-failure", "unless-stopped"));
        assert!(policy_satisfies("always", "unless-stopped"));
        assert!(!policy_satisfies("always", "no"));
        assert!(!policy_satisfies("no", "unless-stopped"));
        assert!(policy_satisfies("", "no"));
    }

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
