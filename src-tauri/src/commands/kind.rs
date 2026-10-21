//! kind cluster lifecycle: install the isolated binary, list/create/delete
//! clusters, fetch kubeconfig, and load local images — all with the kind
//! binary and its kubeconfig kept inside the app's data directory so nothing
//! leaks into `~/.kube` or the host `PATH`.

use std::collections::HashMap;
use std::process::Stdio;

use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::docker::kind::{
    install_kind as install_kind_binary, kind_status as read_kind_status, KindInstallStatus, KindPaths,
};
use crate::state::AppState;

/// Docker label kind puts on every node container; its value is the cluster name.
const CLUSTER_LABEL: &str = "io.x-k8s.kind.cluster";

#[derive(Clone, serde::Serialize)]
pub struct KindCluster {
    pub name: String,
    /// True when at least one node (control-plane) is running.
    pub running: bool,
    pub node_count: usize,
    pub running_nodes: usize,
    /// Unix timestamp (seconds) of the oldest node, if known.
    pub created: Option<i64>,
    /// True when the isolated kubeconfig for this cluster exists.
    pub has_kubeconfig: bool,
    #[serde(default)]
    pub uptime_seconds: Option<i64>,
    #[serde(default)]
    pub last_started_at: Option<i64>,
    #[serde(default)]
    pub pod_count: Option<u32>,
    #[serde(default)]
    pub running_pod_count: Option<u32>,
    /// Whether the API server answers `/readyz` right now. `None` when the
    /// cluster is stopped. Detail/monitoring is gated on this.
    #[serde(default)]
    pub api_ready: Option<bool>,
    /// Summed one-shot docker stats across node containers: CPU as % of the
    /// host, memory in MiB actually used by the cluster right now.
    #[serde(default)]
    pub cpu_percent: Option<f64>,
    #[serde(default)]
    pub mem_used_mb: Option<f64>,
    /// Node resource caps applied to this cluster (None = uncapped).
    #[serde(default)]
    pub cpu_quota_m: Option<u64>,
    #[serde(default)]
    pub mem_limit_mb: Option<f64>,
}

#[tauri::command]
pub async fn kind_status(state: State<'_, AppState>) -> Result<KindInstallStatus, String> {
    Ok(read_kind_status(&state.kind))
}

/// Downloads and verifies the pinned kind binary. Emits `kind-install-progress`.
#[tauri::command]
pub async fn install_kind(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let paths = &state.kind;
    let handle = app.clone();
    let result = install_kind_binary(paths, move |done, total| {
        let _ = handle.emit(
            "kind-install-progress",
            serde_json::json!({ "done": done, "total": total }),
        );
    })
    .await;
    match result {
        Ok(_) => {
            let _ = app.emit(
                "kind-install-progress",
                serde_json::json!({ "done": -1i64, "total": -1i64, "finished": true }),
            );
            Ok(())
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let _ = app.emit(
                "kind-install-progress",
                serde_json::json!({ "error": msg }),
            );
            Err(msg)
        }
    }
}

/// Lists kind clusters by grouping node containers on the kind label. Works
/// whether or not the kind binary is installed (clusters live in the engine).
#[tauri::command]
pub async fn list_kind_clusters(
    state: State<'_, AppState>,
) -> Result<Vec<KindCluster>, String> {
    // kind targets the bundled engine even when a remote endpoint is active.
    let client = local_client(&state).await?;
    // Docker's filter grammar: {"label": ["key"]} = "has this label key".
    let filters: HashMap<String, Vec<String>> =
        HashMap::from([("label".to_string(), vec![CLUSTER_LABEL.to_string()])]);
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();

    let containers = client
        .docker()
        .list_containers(Some(opts))
        .await
        .map_err(|e| e.to_string())?;

    let mut map: HashMap<String, KindCluster> = HashMap::new();
    for c in containers {
        let labels = c.labels.unwrap_or_default();
        let Some(name) = labels.get(CLUSTER_LABEL).cloned() else {
            continue;
        };
        let running = c
            .state
            .as_ref()
            .map(|s| format!("{:?}", s).to_lowercase())
            .as_deref()
            == Some("running");
        let entry = map.entry(name.clone()).or_insert_with(|| KindCluster {
            name: name.clone(),
            running: false,
            node_count: 0,
            running_nodes: 0,
            created: None,
            has_kubeconfig: false,
            uptime_seconds: None,
            last_started_at: None,
            pod_count: None,
            running_pod_count: None,
            api_ready: None,
            cpu_percent: None,
            mem_used_mb: None,
            cpu_quota_m: None,
            mem_limit_mb: None,
        });
        entry.node_count += 1;
        if running {
            entry.running_nodes += 1;
            entry.running = true;
        }
        let ts = c.created.unwrap_or(0);
        entry.created = Some(match entry.created {
            Some(prev) if prev > 0 && prev < ts => prev,
            _ if ts > 0 => ts,
            _ => entry.created.unwrap_or(0),
        });
    }

    let mut list: Vec<KindCluster> = map.into_values().collect();
    for cluster in list.iter_mut() {
        cluster.has_kubeconfig = kubeconfig_has_context(&state.kind, &cluster.name).await;
        enrich_cluster_card(&client, cluster).await;
    }
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
}

/// Fills the card-level monitoring fields (uptime, pods, live resource usage)
/// of a running cluster from node inspect data, one-shot docker stats, and a
/// single `kubectl get pods` exec. Best-effort: any failure leaves fields
/// `None` so the list still renders without the stats.
async fn enrich_cluster_card(
    client: &crate::docker::client::DockerClient,
    cluster: &mut KindCluster,
) {
    let Ok(ids) = cluster_node_ids(client, &cluster.name).await else {
        return;
    };
    if ids.is_empty() {
        return;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Uptime = now - oldest StartedAt among running nodes; caps from HostConfig.
    let mut started_times: Vec<i64> = Vec::new();
    let mut last_end_times: Vec<i64> = Vec::new();
    let mut cpu_quota_m = 0u64;
    let mut mem_limit_bytes = 0i64;
    for id in &ids {
        let Ok(details) = client
            .docker()
            .inspect_container(id, None::<bollard::query_parameters::InspectContainerOptions>)
            .await
        else {
            continue;
        };
        let Some(state) = details.state else { continue; };
        if state.running.unwrap_or(false) {
            if let Some(started) = state.started_at.as_deref().and_then(parse_docker_ts) {
                started_times.push(started);
            }
        } else if let Some(finished) = state.finished_at.as_deref().and_then(parse_docker_ts) {
            last_end_times.push(finished);
        }
        if let Some(host) = details.host_config {
            if let Some(q) = host.cpu_quota.filter(|q| *q > 0) {
                cpu_quota_m += (q / 100) as u64;
            }
            if let Some(m) = host.memory.filter(|m| *m > 0) {
                mem_limit_bytes += m;
            }
        }
    }
    if let Some(&oldest) = started_times.iter().min() {
        cluster.last_started_at = Some(oldest);
        cluster.uptime_seconds = Some((now - oldest).max(0));
    } else if let Some(&latest_end) = last_end_times.iter().max() {
        // Stopped cluster: surface when it last ran (latest node finish time).
        cluster.last_started_at = Some(latest_end);
    }
    if cpu_quota_m > 0 {
        cluster.cpu_quota_m = Some(cpu_quota_m);
    }
    if mem_limit_bytes > 0 {
        cluster.mem_limit_mb = Some(mem_limit_bytes as f64 / (1024.0 * 1024.0));
    }

    if !cluster.running {
        return;
    }

    // Live usage: one-shot docker stats per node, summed (CPU as % of host).
    let mut cpu_pct = 0.0f64;
    let mut mem_mb = 0.0f64;
    let mut any_stats = false;
    for id in &ids {
        if let Ok(s) = client.container_stats(id).await {
            cpu_pct += s.cpu_percent;
            mem_mb += s.mem_usage as f64 / (1024.0 * 1024.0);
            any_stats = true;
        }
    }
    if any_stats {
        cluster.cpu_percent = Some(cpu_pct);
        cluster.mem_used_mb = Some(mem_mb);
    }

    // Pod counts via the control-plane node's bundled kubectl.
    let cp = format!("{}-control-plane", cluster.name);
    if let Ok(out) = client
        .exec_capture(
            &cp,
            vec![
                "kubectl".into(),
                "--kubeconfig=/etc/kubernetes/admin.conf".into(),
                "get".into(),
                "pods".into(),
                "-A".into(),
                "--no-headers".into(),
            ],
        )
        .await
    {
        let (total, running) = count_pods(&out);
        cluster.pod_count = Some(total);
        cluster.running_pod_count = Some(running);
        // The pods query answering means discovery + core API are serving;
        // /readyz is the authoritative check for the whole control plane.
        cluster.api_ready = Some(
            client
                .exec_capture(
                    &cp,
                    vec![
                        "kubectl".into(),
                        "--kubeconfig=/etc/kubernetes/admin.conf".into(),
                        "get".into(),
                        "--raw".into(),
                        "/readyz".into(),
                    ],
                )
                .await
                .is_ok(),
        );
    } else {
        // API not answering (still booting or unhealthy) — Detail must stay shut.
        cluster.api_ready = Some(false);
    }
}

/// Parses Docker's RFC3339 timestamps ("2026-10-20T01:02:03.123456789Z").
/// Docker's zero value ("0001-01-01T00:00:00Z", i.e. never started/stopped)
/// yields `None`.
fn parse_docker_ts(ts: &str) -> Option<i64> {
    let t = chrono::DateTime::parse_from_rfc3339(ts).ok()?.timestamp();
    (t > 0).then_some(t)
}

/// Counts pods in `kubectl get pods -A --no-headers` output: (total, running).
/// Status is the 4th whitespace-separated column (NS NAME READY STATUS ...).
fn count_pods(out: &str) -> (u32, u32) {
    let mut total = 0u32;
    let mut running = 0u32;
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        total += 1;
        if line.split_whitespace().nth(3) == Some("Running") {
            running += 1;
        }
    }
    (total, running)
}

#[cfg(test)]
mod cluster_card_tests {
    use super::*;

    #[test]
    fn parse_docker_ts_reads_rfc3339_and_rejects_zero_or_garbage() {
        assert_eq!(parse_docker_ts("1970-01-01T00:00:10Z"), Some(10));
        assert_eq!(parse_docker_ts("1970-01-01T00:00:10.500Z"), Some(10));
        assert_eq!(parse_docker_ts("0001-01-01T00:00:00Z"), None);
        assert_eq!(parse_docker_ts(""), None);
        assert_eq!(parse_docker_ts("not-a-date"), None);
    }

    #[test]
    fn count_pods_tallies_total_and_running() {
        let out = "default       nginx-abc     1/1     Running   0          2m\n\
                   kube-system   coredns-xyz   1/1     Running   0          5m\n\
                   default       job-1         0/1     Completed 0          1m\n\
                   default       pending-1     0/1     Pending   0          1m\n\n";
        assert_eq!(count_pods(out), (4, 2));
        assert_eq!(count_pods(""), (0, 0));
    }
}

/// Distinct cluster names currently known to the engine — same label query as
/// [`list_kind_clusters`] but without the per-cluster kubeconfig probe, so the
/// tray's periodic refresh stays cheap.
pub async fn count_kind_clusters(state: &AppState) -> Result<usize, String> {
    let client = local_client(state).await?;
    let filters: HashMap<String, Vec<String>> =
        HashMap::from([("label".to_string(), vec![CLUSTER_LABEL.to_string()])]);
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    let containers = client
        .docker()
        .list_containers(Some(opts))
        .await
        .map_err(|e| e.to_string())?;
    let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in containers {
        if let Some(n) = c.labels.as_ref().and_then(|l| l.get(CLUSTER_LABEL)) {
            names.insert(n.clone());
        }
    }
    Ok(names.len())
}

/// Builds a kind `Command` wired to this app's isolated layout and engine.
fn kind_command(paths: &KindPaths, docker_host: &str) -> Command {
    let mut cmd = Command::new(paths.binary());
    // Keep kubeconfig inside the app dir — never `~/.kube/config`.
    cmd.env("KUBECONFIG", &paths.kubeconfig);
    // Point kind at the engine this app manages (isolated from any host
    // docker context the user may have).
    if !docker_host.is_empty() {
        cmd.env("DOCKER_HOST", docker_host);
    }
    cmd.stdin(Stdio::null());
    cmd.kill_on_drop(true);
    cmd
}

/// Streams a kind subprocess's combined output, echoing each line to `event`.
async fn run_kind_streaming(
    paths: &KindPaths,
    docker_host: &str,
    args: &[&str],
    app: &AppHandle,
    event: &str,
) -> Result<(), String> {
    let mut cmd = kind_command(paths, docker_host);
    cmd.args(args);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!(
                "kind binary not found at {} — install kind first",
                paths.binary().display()
            )
        } else {
            format!("failed to run kind: {e}")
        }
    })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    fn spawn_line_reader<R>(pipe: R, app: AppHandle, event: String) -> tokio::task::JoinHandle<()>
    where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            let mut lines = BufReader::new(pipe).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit(&event, serde_json::json!({ "line": line }));
            }
        })
    }

    let mut readers: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    if let Some(out) = stdout {
        readers.push(spawn_line_reader(out, app.clone(), event.to_string()));
    }
    if let Some(err) = stderr {
        readers.push(spawn_line_reader(err, app.clone(), event.to_string()));
    }

    let status = child.wait().await.map_err(|e| format!("wait kind: {e}"))?;
    for r in readers {
        let _ = r.await;
    }

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "kind {} exited with {}",
            args.first().copied().unwrap_or(""),
            status.code().unwrap_or(-1)
        ))
    }
}

/// kind always rides this app's bundled engine (kubeconfig points at the
/// engine host's localhost API port, which only resolves when kind runs here —
/// never through a remote endpoint).
fn docker_host_for(state: &AppState) -> String {
    format!("unix://{}", state.engine.socket().display())
}

async fn local_client(state: &AppState) -> Result<crate::docker::client::DockerClient, String> {
    crate::docker::client::DockerClient::connect(Some(&state.engine.socket()))
        .map_err(|e| format!("Docker daemon not reachable: {e}"))
}

#[tauri::command]
pub async fn create_kind_cluster(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    image: Option<String>,
    nodes: Option<u32>,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Cluster name is required".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err("Cluster name may only contain letters, digits, '-' and '.'".into());
    }
    let host = docker_host_for(&state);
    let paths = &state.kind;
    if !paths.is_installed() {
        return Err("kind is not installed yet".into());
    }

    let n = nodes.unwrap_or(1).clamp(1, 10);
    let image = image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // The node's containerd state lives on the engine's data root, and when
    // that filesystem reports DT_UNKNOWN for every dirent (d_type-less ext4
    // under fuse-overlayfs — see `ensure_snapshotter_compat`), containerd's
    // overlayfs/erofs snapshotters refuse to load, the CRI plugin never
    // registers, and kubeadm dies with "unknown service
    // runtime.v1.RuntimeService". Bind-mounting a d_type-capable directory
    // over /var/lib/containerd inside every node — the same fix the engine
    // itself uses for its own containerd — keeps image layers and snapshotter
    // state on a filesystem containerd accepts. Per-node dirs, or two nodes
    // would share one content store on the host.
    let dtype_root = crate::docker::engine::dtype_compat_root()
        .ok_or_else(|| {
            "no d_type-capable directory is available for the kind node containerd state"
                .to_string()
        })?;
    let cluster_state = dtype_root.join("kind-nodes").join(&name);
    let mut nodes_yaml = String::new();
    for i in 0..n {
        let role = if i == 0 { "control-plane" } else { "worker" };
        let node_state = cluster_state.join(format!("node{i}"));
        std::fs::create_dir_all(&node_state)
            .map_err(|e| format!("cannot create {}: {e}", node_state.display()))?;
        // Rootless engines may map the node's root to a subuid; 0777 keeps
        // the mount writable either way, and the parent dirs stay private.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&node_state, std::fs::Permissions::from_mode(0o777));
        }
        nodes_yaml.push_str(&format!("  - role: {role}"));
        if let Some(img) = &image {
            nodes_yaml.push_str(&format!("\n    image: {img}"));
        }
        nodes_yaml.push_str("\n    extraMounts:\n");
        nodes_yaml.push_str(&format!("      - hostPath: {}\n", node_state.display()));
        nodes_yaml.push_str("        containerPath: /var/lib/containerd\n");
        nodes_yaml.push_str("        readOnly: false\n");
    }
    let config_yaml = format!(
        "kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\nnodes:\n{nodes_yaml}"
    );
    let config_path = paths
        .kubeconfig
        .parent()
        .map(|d| d.join(format!("cluster-config-{name}.yaml")))
        .ok_or_else(|| "kind data directory is unavailable".to_string())?;
    std::fs::write(&config_path, config_yaml)
        .map_err(|e| format!("failed to write kind cluster config: {e}"))?;

    // The node layout lives in the config file now (`--config` conflicts with
    // `--nodes`/`--image`); `--wait` is CLI-only, so keep it as a flag.
    let args: Vec<String> = vec![
        "create".into(),
        "cluster".into(),
        "--name".into(),
        name.clone(),
        "--config".into(),
        config_path.display().to_string(),
        "--wait".into(),
        "120s".into(),
    ];
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let _ = app.emit(
        "kind-progress",
        serde_json::json!({ "action": "create", "name": name, "line": format!("Creating cluster {name}…") }),
    );
    let result = run_kind_streaming(paths, &host, &arg_refs, &app, "kind-progress")
        .await
        .map_err(|e| {
            format!("create cluster '{name}' failed: {e}")
        });
    if result.is_ok() {
        // Creation "succeeds" as soon as kind exits 0; the cluster is usable
        // because the create command waits for nodes by default.
        crate::tray::notify_kind_event(&app, true, &name);
        // kind ships nodes with on-failure:1 — pin them so a later engine
        // start cannot resurrect a cluster the user stopped.
        let _ = enforce_kind_restart_policy(&state).await;
    }
    result
}

#[tauri::command]
pub async fn delete_kind_cluster(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Cluster name is required".into());
    }
    let host = docker_host_for(&state);
    let paths = &state.kind;
    if !paths.is_installed() {
        // Fall back to removing the node containers directly so a stale
        // cluster can still be cleaned up without the binary.
        let result = delete_cluster_containers(&state, &name).await;
        if result.is_ok() {
            crate::tray::notify_kind_event(&app, false, &name);
            cleanup_cluster_state(&name);
        }
        return result;
    }
    let _ = app.emit(
        "kind-progress",
        serde_json::json!({ "action": "delete", "name": name, "line": format!("Deleting cluster {name}…") }),
    );
    let args = ["delete", "cluster", "--name", name.as_str()];
    let result = run_kind_streaming(paths, &host, &args, &app, "kind-progress").await;
    if result.is_ok() {
        crate::tray::notify_kind_event(&app, false, &name);
        cleanup_cluster_state(&name);
    }
    result
}

/// Removes the d_type-compat bind-mount dirs a created cluster left behind.
/// Best-effort: image layers can be gigabytes, and a failure here must not
/// fail the delete itself (the containers are already gone).
fn cleanup_cluster_state(name: &str) {
    if let Some(root) = crate::docker::engine::dtype_compat_root() {
        let dir = root.join("kind-nodes").join(name);
        if dir.is_dir() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                tracing::warn!("could not clean kind node state {}: {e}", dir.display());
            }
        }
    }
}

async fn delete_cluster_containers(state: &AppState, name: &str) -> Result<(), String> {
    let client = local_client(state).await?;
    let mut filters = HashMap::new();
    // {"label": ["key=value"]} narrows to this cluster's node containers.
    filters.insert(
        "label".to_string(),
        vec![format!("{CLUSTER_LABEL}={name}")],
    );
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    let containers = client
        .docker()
        .list_containers(Some(opts))
        .await
        .map_err(|e| e.to_string())?;
    for c in containers {
        if let Some(id) = c.id {
            let _ = client
                .docker()
                .remove_container(
                    &id,
                    Some(bollard::query_parameters::RemoveContainerOptionsBuilder::default().force(true).build()),
                )
                .await;
        }
    }
    Ok(())
}

/// Starts every node container of a cluster. kind has no native start/stop —
/// nodes are plain containers, so this drives them directly.
#[tauri::command]
pub async fn start_kind_cluster(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    set_cluster_power(&app, &*state, &name, true).await
}

/// Stops every node container of a cluster (state persists; Start resumes it).
#[tauri::command]
pub async fn stop_kind_cluster(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    set_cluster_power(&app, &*state, &name, false).await
}

async fn set_cluster_power(
    app: &AppHandle,
    state: &AppState,
    name: &str,
    start: bool,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Cluster name is required".into());
    }
    if start {
        // Re-apply the cluster's per-node caps so a resumed cluster starts
        // under the same limits (docker keeps them across stop/start, but a
        // user may have edited prefs while the cluster was down).
        let prefs = state
            .config
            .read()
            .await
            .kind_cluster_prefs
            .get(name)
            .cloned()
            .unwrap_or_default();
        if prefs.node_cpu_m > 0 || prefs.node_memory_mb > 0 {
            if let Err(e) = apply_cluster_limits(state, name, &prefs).await {
                tracing::warn!("could not apply node limits for {name}: {e}");
            }
        }
    }
    let client = local_client(state).await?;
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![format!("{CLUSTER_LABEL}={name}")]);
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    let containers = client
        .docker()
        .list_containers(Some(opts))
        .await
        .map_err(|e| e.to_string())?;
    if containers.is_empty() {
        return Err(format!("No node containers found for cluster '{name}'"));
    }

    let action = if start { "start" } else { "stop" };
    let verb = if start { "Starting" } else { "Stopping" };
    let _ = app.emit(
        "kind-progress",
        serde_json::json!({ "action": action, "name": name, "line": format!("{verb} cluster {name}…") }),
    );

    let mut changed = 0usize;
    for c in containers {
        let Some(id) = c.id else { continue };
        let is_running = c
            .state
            .as_ref()
            .map(|s| format!("{:?}", s).to_lowercase())
            .as_deref()
            == Some("running");
        // Docker errors when starting an already-running container (or
        // stopping a stopped one) — only touch nodes in the opposite state.
        if start == is_running {
            continue;
        }
        let res = if start {
            client.start_container(&id).await
        } else {
            client.stop_container(&id).await
        };
        if let Err(e) = res {
            return Err(format!("{verb} cluster '{name}' failed: {e}"));
        }
        changed += 1;
    }

    let summary = if changed == 0 {
        format!(
            "Cluster '{name}' is already {}",
            if start { "running" } else { "stopped" }
        )
    } else {
        format!(
            "{} cluster '{name}' complete ({changed} node{})",
            verb,
            if changed == 1 { "" } else { "s" }
        )
    };
    let _ = app.emit(
        "kind-progress",
        serde_json::json!({ "action": action, "name": name, "line": summary }),
    );
    Ok(())
}

/// Ensures the cluster's context is present in the app-managed kubeconfig and
/// returns that file's path. Uses `kind export kubeconfig` (which MERGES into
/// the file named by KUBECONFIG) instead of stdout + overwrite — overwriting
/// would wipe every other cluster's context from the shared file.
#[tauri::command]
pub async fn get_kind_kubeconfig(
    state: State<'_, AppState>,
    name: String,
) -> Result<String, String> {
    let name = name.trim().to_string();
    let paths = &state.kind;
    if !paths.is_installed() {
        return Err("kind is not installed yet".into());
    }
    if kubeconfig_has_context(paths, &name).await {
        return Ok(paths.kubeconfig.display().to_string());
    }

    if let Some(parent) = paths.kubeconfig.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create kubeconfig dir: {e}"))?;
    }

    // kind_command pins KUBECONFIG to the app file, so the exported context
    // lands there and nowhere near ~/.kube.
    let host = docker_host_for(&state);
    let mut cmd = kind_command(paths, &host);
    cmd.args(["export", "kubeconfig", "--name", &name]);
    let output = cmd
        .output()
        .await
        .map_err(|e| format!("run kind export kubeconfig: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "kind export kubeconfig failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if !kubeconfig_has_context(paths, &name).await {
        return Err(format!(
            "kubeconfig context 'kind-{name}' was not written — is the cluster running?"
        ));
    }
    Ok(paths.kubeconfig.display().to_string())
}

async fn kubeconfig_has_context(paths: &KindPaths, name: &str) -> bool {
    tokio::task::spawn_blocking({
        let p = paths.kubeconfig.clone();
        let needle = format!("kind-{name}");
        move || {
            std::fs::read_to_string(&p)
                .map(|s| s.contains(&needle))
                .unwrap_or(false)
        }
    })
    .await
    .unwrap_or(false)
}

/// Loads a locally-present image into a cluster via `kind load docker-image`.
#[tauri::command]
pub async fn load_image_to_cluster(
    app: AppHandle,
    state: State<'_, AppState>,
    cluster: String,
    image: String,
) -> Result<(), String> {
    let cluster = cluster.trim().to_string();
    let image = image.trim().to_string();
    if cluster.is_empty() || image.is_empty() {
        return Err("Cluster and image are required".into());
    }
    let host = docker_host_for(&state);
    let paths = &state.kind;
    if !paths.is_installed() {
        return Err("kind is not installed yet".into());
    }
    let _ = app.emit(
        "kind-progress",
        serde_json::json!({ "action": "load", "name": cluster, "line": format!("Loading {image} into {cluster}…") }),
    );
    let args = [
        "load",
        "docker-image",
        image.as_str(),
        "--name",
        cluster.as_str(),
    ];
    run_kind_streaming(paths, &host, &args, &app, "kind-progress").await
}

// ==================== Cluster monitoring (kubectl inside the control-plane node) ====================
//
// All monitoring rides `docker exec <cluster>-control-plane kubectl …` with the
// node's own admin kubeconfig — no host kubectl, no network exposure. The JSON
// parsing is factored into pure functions so it unit-tests without a cluster.

/// Container name kind gives the control-plane node of `cluster`.
fn control_plane_node(cluster: &str) -> String {
    format!("{cluster}-control-plane")
}

/// Runs `kubectl <args>` inside the cluster's control-plane node, returning stdout.
async fn kubectl_raw(
    state: &AppState,
    cluster: &str,
    args: &[&str],
) -> Result<String, String> {
    let client = local_client(state).await?;
    let mut cmd = vec![
        "kubectl".to_string(),
        "--kubeconfig=/etc/kubernetes/admin.conf".to_string(),
    ];
    cmd.extend(args.iter().map(|s| s.to_string()));
    client
        .exec_capture(&control_plane_node(cluster), cmd)
        .await
        .map_err(|e| format!("kubectl {args:?}: {e}"))
}

/// Same as [`kubectl_raw`] but parses stdout as JSON. kubectl warnings can land
/// in the merged exec stream ahead of the payload, so a first failed parse
/// retries from the first `{`/`[`.
async fn kubectl_json(
    state: &AppState,
    cluster: &str,
    args: &[&str],
) -> Result<serde_json::Value, String> {
    let out = kubectl_raw(state, cluster, args).await?;
    extract_json(&out).ok_or_else(|| format!("bad kubectl JSON from {args:?}"))
}

/// Parses `out` as JSON, tolerating leading warning lines.
fn extract_json(out: &str) -> Option<serde_json::Value> {
    let trimmed = out.trim();
    if let Ok(v) = serde_json::from_str(trimmed) {
        return Some(v);
    }
    let start = trimmed.find(['{', '['])?;
    serde_json::from_str(&trimmed[start..]).ok()
}

fn rfc3339_to_epoch(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|d| d.timestamp())
}

/// "250m" → 250.0 · "1500000000n" → 1.5 · "2" → 2000.0 (millicores).
fn cpu_to_millicores(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(v) = s.strip_suffix('m') {
        return v.parse().ok();
    }
    if let Some(v) = s.strip_suffix('n') {
        return v.parse::<f64>().ok().map(|v| v / 1e6);
    }
    if let Some(v) = s.strip_suffix('u') {
        return v.parse::<f64>().ok().map(|v| v / 1e3);
    }
    s.parse::<f64>().ok().map(|v| v * 1000.0)
}

/// Kubernetes quantity → bytes. "1927Mi" → 2020…; "2Gi" → 2^31; bare "512" → 512.
fn mem_to_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = if let Some(v) = s.strip_suffix("Ki") {
        (v, 1024f64)
    } else if let Some(v) = s.strip_suffix("Mi") {
        (v, 1024f64.powi(2))
    } else if let Some(v) = s.strip_suffix("Gi") {
        (v, 1024f64.powi(3))
    } else if let Some(v) = s.strip_suffix("Ti") {
        (v, 1024f64.powi(4))
    } else if let Some(v) = s.strip_suffix('G') {
        (v, 1e9)
    } else if let Some(v) = s.strip_suffix('M') {
        (v, 1e6)
    } else if let Some(v) = s.strip_suffix('k') {
        (v, 1e3)
    } else {
        (s, 1.0)
    };
    num.trim().parse::<f64>().ok().map(|v| (v * mult) as u64)
}

// ---- Serialized payloads (camelCase on the wire) ----

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sNodeInfo {
    pub name: String,
    pub ready: bool,
    pub status: String,
    pub roles: Vec<String>,
    pub kubelet_version: String,
    pub os_image: String,
    pub container_runtime: String,
    pub internal_ip: String,
    pub cpu_capacity_m: f64,
    pub mem_capacity_bytes: u64,
    pub pods_capacity: u32,
    pub created: Option<i64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sPodInfo {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub ready_containers: u32,
    pub total_containers: u32,
    pub restarts: u32,
    pub node: String,
    pub images: Vec<String>,
    pub created: Option<i64>,
    pub reason: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sEventInfo {
    #[serde(rename = "type")]
    pub kind: String,
    pub reason: String,
    pub message: String,
    pub object: String,
    pub namespace: String,
    pub count: u32,
    pub last_timestamp: Option<i64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sWorkloadInfo {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub ready: u32,
    pub desired: u32,
    pub available: u32,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sNodeMetric {
    pub name: String,
    pub cpu_m: f64,
    pub mem_bytes: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sPodMetric {
    pub namespace: String,
    pub name: String,
    pub cpu_m: f64,
    pub mem_bytes: u64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sMetrics {
    pub nodes: Vec<K8sNodeMetric>,
    pub pods: Vec<K8sPodMetric>,
    /// Set when `kubectl top` failed (typically: metrics-server not installed).
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sClusterSummary {
    pub git_version: String,
    pub platform: String,
    pub ready: bool,
    pub ready_message: Option<String>,
    pub node_count: usize,
    pub ready_nodes: usize,
    pub namespace_count: usize,
    pub pod_running: usize,
    pub pod_pending: usize,
    pub pod_failed: usize,
    pub pod_succeeded: usize,
    /// Failures of individual sub-queries (pods/nodes/ns/…), surfaced in the
    /// UI so a partial summary explains itself instead of showing zeros.
    pub errors: Vec<String>,
}

// ---- Pure parsers (unit-tested) ----

fn parse_nodes(json: &serde_json::Value) -> Vec<K8sNodeInfo> {
    let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|n| {
            let meta = n.get("metadata")?;
            let name = meta.get("name")?.as_str()?.to_string();
            let created = meta
                .get("creationTimestamp")
                .and_then(|t| t.as_str())
                .and_then(rfc3339_to_epoch);
            let status = n.get("status")?;
            let kubelet = status.get("nodeInfo")?;
            let conds = status
                .get("conditions")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let ready_cond = conds
                .iter()
                .find(|c| c.get("type").and_then(|t| t.as_str()) == Some("Ready"));
            let ready = ready_cond
                .map(|c| c.get("status").and_then(|s| s.as_str()) == Some("True"))
                .unwrap_or(false);
            let status_str = if ready {
                "Ready".to_string()
            } else {
                ready_cond
                    .and_then(|c| c.get("reason").and_then(|r| r.as_str()))
                    .unwrap_or("NotReady")
                    .to_string()
            };
            let roles: Vec<String> = meta
                .get("labels")
                .and_then(|l| l.as_object())
                .map(|labels| {
                    labels
                        .keys()
                        .filter_map(|k| {
                            k.strip_prefix("node-role.kubernetes.io/")
                                .map(|r| {
                                    if r.is_empty() {
                                        "worker".to_string()
                                    } else {
                                        r.to_string()
                                    }
                                })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let cpu_capacity_m = kubelet
                .get("cpuCapacity")
                .and_then(|v| v.as_str())
                .and_then(cpu_to_millicores)
                .unwrap_or(0.0);
            let mem_capacity_bytes = kubelet
                .get("memoryCapacity")
                .and_then(|v| v.as_str())
                .and_then(mem_to_bytes)
                .unwrap_or(0);
            let internal_ip = status
                .get("addresses")
                .and_then(|a| a.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|x| x.get("type").and_then(|t| t.as_str()) == Some("InternalIP"))
                        .and_then(|x| x.get("address"))
                        .and_then(|a| a.as_str())
                })
                .unwrap_or("")
                .to_string();
            Some(K8sNodeInfo {
                name,
                ready,
                status: status_str,
                roles,
                kubelet_version: kubelet
                    .get("kubeletVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                os_image: kubelet
                    .get("osImage")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                container_runtime: kubelet
                    .get("containerRuntimeVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                internal_ip,
                cpu_capacity_m,
                mem_capacity_bytes,
                // kubectl serializes capacity quantities as strings ("110").
                pods_capacity: kubelet
                    .get("podsCapacity")
                    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                    .unwrap_or(0) as u32,
                created,
            })
        })
        .collect()
}

fn parse_pods(json: &serde_json::Value) -> Vec<K8sPodInfo> {
    let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|p| {
            let meta = p.get("metadata")?;
            let name = meta.get("name")?.as_str()?.to_string();
            let namespace = meta
                .get("namespace")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let created = meta
                .get("creationTimestamp")
                .and_then(|t| t.as_str())
                .and_then(rfc3339_to_epoch);
            let status = p.get("status")?;
            let phase = status
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();
            let reason = status
                .get("reason")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let cstatuses = status
                .get("containerStatuses")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let ready_containers = cstatuses
                .iter()
                .filter(|c| c.get("ready").and_then(|r| r.as_bool()) == Some(true))
                .count() as u32;
            let restarts = cstatuses
                .iter()
                .filter_map(|c| c.get("restartCount").and_then(|r| r.as_u64()))
                .sum::<u64>() as u32;
            let mut images: Vec<String> = cstatuses
                .iter()
                .filter_map(|c| c.get("image").and_then(|i| i.as_str()).map(|s| s.to_string()))
                .collect();
            if images.is_empty() {
                images = p
                    .get("spec")
                    .and_then(|s| s.get("containers"))
                    .and_then(|c| c.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|c| c.get("image").and_then(|i| i.as_str()).map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
            }
            let total_containers = p
                .get("spec")
                .and_then(|s| s.get("containers"))
                .and_then(|c| c.as_array())
                .map(|a| a.len() as u32)
                .unwrap_or(images.len() as u32);
            Some(K8sPodInfo {
                name,
                namespace,
                phase,
                ready_containers,
                total_containers,
                restarts,
                node: p
                    .get("spec")
                    .and_then(|s| s.get("nodeName"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string(),
                images,
                created,
                reason,
            })
        })
        .collect()
}

fn parse_events(json: &serde_json::Value) -> Vec<K8sEventInfo> {
    let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|e| {
            let meta = e.get("metadata")?;
            let obj = e.get("involvedObject")?;
            let last_ts = e
                .get("lastTimestamp")
                .or_else(|| e.get("eventTime"))
                .and_then(|t| t.as_str())
                .and_then(rfc3339_to_epoch);
            Some(K8sEventInfo {
                kind: e
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Normal")
                    .to_string(),
                reason: e
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                message: e
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                object: format!(
                    "{} {}",
                    obj.get("kind").and_then(|v| v.as_str()).unwrap_or(""),
                    obj.get("name").and_then(|v| v.as_str()).unwrap_or("")
                )
                .trim()
                .to_string(),
                namespace: meta
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                count: e.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
                last_timestamp: last_ts,
            })
        })
        .collect()
}

fn parse_workloads(json: &serde_json::Value) -> Vec<K8sWorkloadInfo> {
    let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|w| {
            let meta = w.get("metadata")?;
            Some(K8sWorkloadInfo {
                kind: w
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string(),
                name: meta.get("name")?.as_str()?.to_string(),
                namespace: meta
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string(),
                ready: w
                    .get("status")
                    .and_then(|s| s.get("readyReplicas"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                desired: w
                    .get("spec")
                    .and_then(|s| s.get("replicas"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                available: w
                    .get("status")
                    .and_then(|s| s.get("availableReplicas"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
            })
        })
        .collect()
}

fn parse_node_metrics(json: &serde_json::Value) -> Vec<K8sNodeMetric> {
    let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|n| {
            let name = n.get("metadata")?.get("name")?.as_str()?.to_string();
            let usage = n.get("usage")?;
            Some(K8sNodeMetric {
                name,
                cpu_m: usage
                    .get("cpu")
                    .and_then(|v| v.as_str())
                    .and_then(cpu_to_millicores)
                    .unwrap_or(0.0),
                mem_bytes: usage
                    .get("memory")
                    .and_then(|v| v.as_str())
                    .and_then(mem_to_bytes)
                    .unwrap_or(0),
            })
        })
        .collect()
}

fn parse_pod_metrics(json: &serde_json::Value) -> Vec<K8sPodMetric> {
    let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|p| {
            let meta = p.get("metadata")?;
            let name = meta.get("name")?.as_str()?.to_string();
            let namespace = meta
                .get("namespace")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let containers = p
                .get("containers")
                .and_then(|c| c.as_array())
                .cloned()
                .unwrap_or_default();
            let mut cpu_m = 0.0;
            let mut mem_bytes = 0u64;
            for c in &containers {
                let Some(usage) = c.get("usage") else { continue };
                cpu_m += usage
                    .get("cpu")
                    .and_then(|v| v.as_str())
                    .and_then(cpu_to_millicores)
                    .unwrap_or(0.0);
                mem_bytes += usage
                    .get("memory")
                    .and_then(|v| v.as_str())
                    .and_then(mem_to_bytes)
                    .unwrap_or(0);
            }
            Some(K8sPodMetric {
                namespace,
                name,
                cpu_m,
                mem_bytes,
            })
        })
        .collect()
}

// ---- Commands ----

/// Interprets `kubectl get --raw /readyz[?verbose]` output.
/// Success = no failing check; accepts plain "ok", verbose `[+]check ok`
/// lines, and bare lines ending in "ok".
fn readyz_verdict(out: &str) -> (bool, Option<String>) {
    let trimmed = out.trim();
    let failed_line = trimmed
        .lines()
        .find(|l| l.trim_start().starts_with("[-]") || l.contains("fail"));
    if let Some(l) = failed_line {
        return (false, Some(l.trim().to_string()));
    }
    let ok = trimmed.starts_with("ok")
        || trimmed.contains("[+]")
        || trimmed.lines().any(|l| l.trim().ends_with(" ok"));
    if ok {
        (true, None)
    } else {
        (
            false,
            Some(
                trimmed
                    .lines()
                    .find(|l| !l.is_empty())
                    .unwrap_or("not ready")
                    .to_string(),
            ),
        )
    }
}

#[tauri::command]
pub async fn get_cluster_nodes(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<Vec<K8sNodeInfo>, String> {
    let v = kubectl_json(&state, &cluster, &["get", "nodes", "-o", "json"]).await?;
    Ok(parse_nodes(&v))
}

#[tauri::command]
pub async fn get_cluster_pods(
    state: State<'_, AppState>,
    cluster: String,
    namespace: Option<String>,
) -> Result<Vec<K8sPodInfo>, String> {
    let v = kubectl_json(
        &state,
        &cluster,
        &["get", "pods", "--all-namespaces", "-o", "json"],
    )
    .await?;
    let pods = parse_pods(&v);
    Ok(match namespace.filter(|n| !n.is_empty() && n != "all") {
        Some(ns) => pods.into_iter().filter(|p| p.namespace == ns).collect(),
        None => pods,
    })
}

#[tauri::command]
pub async fn get_cluster_events(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<Vec<K8sEventInfo>, String> {
    let v = kubectl_json(
        &state,
        &cluster,
        &[
            "get",
            "events",
            "--all-namespaces",
            "--sort-by=.lastTimestamp",
            "-o",
            "json",
        ],
    )
    .await?;
    let mut events = parse_events(&v);
    events.reverse(); // newest last → newest first for the UI
    Ok(events)
}

#[tauri::command]
pub async fn get_cluster_workloads(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<Vec<K8sWorkloadInfo>, String> {
    let v = kubectl_json(
        &state,
        &cluster,
        &[
            "get",
            "deployments,statefulsets,daemonsets",
            "--all-namespaces",
            "-o",
            "json",
        ],
    )
    .await?;
    Ok(parse_workloads(&v))
}

#[tauri::command]
pub async fn get_cluster_metrics(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<K8sMetrics, String> {
    // Both metric queries are best-effort: without metrics-server the API
    // isn't registered, so a failure is data (reported via `error`), not an
    // error. NOTE: `kubectl top` only prints a table — it has no `-o json`
    // flag at all — so asking it for JSON fails even with a healthy
    // metrics-server. The Metrics API returns the same data as real JSON.
    let nodes_res = kubectl_json(
        &state,
        &cluster,
        &["get", "--raw", "/apis/metrics.k8s.io/v1beta1/nodes"],
    )
    .await;
    let pods_res = kubectl_json(
        &state,
        &cluster,
        &["get", "--raw", "/apis/metrics.k8s.io/v1beta1/pods"],
    )
    .await;
    let error = match (&nodes_res, &pods_res) {
        (Err(a), Err(b)) => Some(format!("{a}; {b}")),
        (Err(a), Ok(_)) => Some(a.clone()),
        (Ok(_), Err(b)) => Some(b.clone()),
        _ => None,
    };
    Ok(K8sMetrics {
        nodes: nodes_res
            .as_ref()
            .map(parse_node_metrics)
            .unwrap_or_default(),
        pods: pods_res
            .as_ref()
            .map(parse_pod_metrics)
            .unwrap_or_default(),
        error,
    })
}

#[tauri::command]
pub async fn get_cluster_summary(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<K8sClusterSummary, String> {
    let ver = kubectl_json(&state, &cluster, &["version", "-o", "json"])
        .await
        .unwrap_or(serde_json::json!({}));
    let server = ver.get("serverVersion").unwrap_or(&ver);
    let git_version = server
        .get("gitVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let platform = server
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // /readyz is plain text: non-verbose = "ok" or a failure list; verbose
    // = one `[+]check ok` / `[-]check failed: …` line per check.
    let (ready, ready_message) = match kubectl_raw(
        &state,
        &cluster,
        &["get", "--raw", "/readyz?verbose"],
    )
    .await
    {
        Ok(out) => readyz_verdict(&out),
        Err(e) => (false, Some(e)),
    };

    let mut errors: Vec<String> = Vec::new();
    let pods = match kubectl_json(
        &state,
        &cluster,
        &["get", "pods", "--all-namespaces", "-o", "json"],
    )
    .await
    {
        Ok(v) => parse_pods(&v),
        Err(e) => {
            errors.push(format!("pods: {e}"));
            vec![]
        }
    };
    let (mut pod_running, mut pod_pending, mut pod_failed, mut pod_succeeded) = (0, 0, 0, 0);
    for p in &pods {
        match p.phase.as_str() {
            "Running" => pod_running += 1,
            "Pending" => pod_pending += 1,
            "Failed" => pod_failed += 1,
            "Succeeded" => pod_succeeded += 1,
            _ => {}
        }
    }

    let nodes = match kubectl_json(&state, &cluster, &["get", "nodes", "-o", "json"]).await {
        Ok(v) => parse_nodes(&v),
        Err(e) => {
            errors.push(format!("nodes: {e}"));
            vec![]
        }
    };
    let ready_nodes = nodes.iter().filter(|n| n.ready).count();

    let namespace_count =
        match kubectl_json(&state, &cluster, &["get", "namespaces", "-o", "json"]).await {
            Ok(v) => v
                .get("items")
                .and_then(|i| i.as_array())
                .map(|a| {
                    a.iter()
                        .filter(|n| {
                            n.get("status")
                                .and_then(|s| s.get("phase"))
                                .and_then(|p| p.as_str())
                                == Some("Active")
                        })
                        .count()
                })
                .unwrap_or(0),
            Err(e) => {
                errors.push(format!("namespaces: {e}"));
                0
            }
        };

    Ok(K8sClusterSummary {
        git_version,
        platform,
        ready,
        ready_message,
        node_count: nodes.len(),
        ready_nodes,
        namespace_count,
        pod_running,
        pod_pending,
        pod_failed,
        pod_succeeded,
        errors,
    })
}

/// Where the upstream metrics-server manifest lives.
const METRICS_SERVER_MANIFEST: &str =
    "https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml";

/// One metrics-server pod as the UI reports it.
#[derive(Clone, serde::Serialize)]
pub struct MetricsPodStatus {
    pub name: String,
    pub phase: String,
    pub ready: bool,
    pub restarts: i64,
    /// `state.waiting.reason` / `state.terminated.reason` when the pod is stuck.
    pub reason: Option<String>,
}

/// Real state of metrics-server in a cluster. `installed` is about the
/// deployment existing at all; `top_works` is about `kubectl top` succeeding —
/// they diverge for the first ~minute after install, which is exactly what a
/// `kubectl top`-only heuristic used to get wrong.
#[derive(Clone, serde::Serialize)]
pub struct MetricsServerStatus {
    pub installed: bool,
    pub desired_replicas: i64,
    pub available_replicas: i64,
    pub ready: bool,
    /// True when the container args carry `--kubelet-insecure-tls` (required on
    /// kind, whose kubelet serving certs are self-signed).
    pub kubelet_insecure_tls: bool,
    pub pods: Vec<MetricsPodStatus>,
    pub top_works: bool,
    /// Human-readable diagnosis when something needs attention.
    pub message: Option<String>,
}

/// True when the deployment's first container already passes `arg`.
fn deployment_has_arg(deployment: &serde_json::Value, arg: &str) -> bool {
    deployment
        .pointer("/spec/template/spec/containers/0/args")
        .and_then(|a| a.as_array())
        .map(|args| args.iter().any(|v| v.as_str() == Some(arg)))
        .unwrap_or(false)
}

fn deployment_available_replicas(deployment: &serde_json::Value) -> i64 {
    deployment
        .pointer("/status/availableReplicas")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

fn deployment_desired_replicas(deployment: &serde_json::Value) -> i64 {
    deployment
        .pointer("/spec/replicas")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
}

/// Parses `kubectl get pods -n kube-system -l k8s-app=metrics-server -o json`.
fn parse_metrics_pods(json: &serde_json::Value) -> Vec<MetricsPodStatus> {
    json.get("items")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .map(|p| {
                    let status = p.get("status");
                    let cs = status
                        .and_then(|s| s.get("containerStatuses"))
                        .and_then(|c| c.as_array());
                    let ready = cs
                        .map(|list| {
                            !list.is_empty()
                                && list
                                    .iter()
                                    .all(|c| c.get("ready").and_then(|r| r.as_bool()).unwrap_or(false))
                        })
                        .unwrap_or(false);
                    let restarts = cs
                        .map(|list| {
                            list.iter()
                                .filter_map(|c| c.get("restartCount").and_then(|r| r.as_i64()))
                                .sum()
                        })
                        .unwrap_or(0);
                    // A stuck pod explains itself in waiting/terminated.reason
                    // (ImagePullBackOff, CrashLoopBackOff, ...).
                    let reason = cs
                        .and_then(|list| list.first())
                        .and_then(|c| c.get("state"))
                        .and_then(|s| {
                            s.pointer("/waiting/reason")
                                .or_else(|| s.pointer("/terminated/reason"))
                        })
                        .and_then(|r| r.as_str())
                        .map(|s| s.to_string());
                    MetricsPodStatus {
                        name: p
                            .pointer("/metadata/name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string(),
                        phase: status
                            .and_then(|s| s.get("phase"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown")
                            .to_string(),
                        ready,
                        restarts,
                        reason,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Turns the raw flags into the one sentence the UI shows (or nothing).
fn diagnose_metrics(
    installed: bool,
    ready: bool,
    kubelet_insecure_tls: bool,
    top_works: bool,
    pods: &[MetricsPodStatus],
) -> Option<String> {
    if !installed {
        return None;
    }
    if let Some(p) = pods
        .iter()
        .find(|p| p.reason.is_some() || p.phase == "Failed")
    {
        let why = p.reason.clone().unwrap_or_else(|| p.phase.clone());
        return Some(format!(
            "metrics-server pod {} is stuck in {why} (restarts: {})",
            p.name, p.restarts
        ));
    }
    if !ready {
        return Some(
            "metrics-server deployment is installed but has no ready replica yet — the pod is still starting."
                .to_string(),
        );
    }
    if !kubelet_insecure_tls {
        return Some(
            "metrics-server is running without --kubelet-insecure-tls; kind's self-signed kubelet certs make every scrape fail. Restart the install to patch it."
                .to_string(),
        );
    }
    if !top_works {
        return Some(
            "metrics-server is running — usage data usually shows up within a minute."
                .to_string(),
        );
    }
    None
}

/// Reads the real metrics-server state of a cluster (deployment, pods, and
/// whether `kubectl top` actually answers).
async fn read_metrics_status(
    state: &AppState,
    cluster: &str,
) -> Result<MetricsServerStatus, String> {
    let deployment = match kubectl_json(
        state,
        cluster,
        &[
            "get",
            "deployment",
            "metrics-server",
            "-n",
            "kube-system",
            "-o",
            "json",
        ],
    )
    .await
    {
        Ok(v) => v,
        Err(e) if e.contains("NotFound") || e.to_lowercase().contains("not found") => {
            return Ok(MetricsServerStatus {
                installed: false,
                desired_replicas: 0,
                available_replicas: 0,
                ready: false,
                kubelet_insecure_tls: false,
                pods: Vec::new(),
                top_works: false,
                message: None,
            });
        }
        Err(e) => return Err(e),
    };

    let pods = kubectl_json(
        state,
        cluster,
        &[
            "get",
            "pods",
            "-n",
            "kube-system",
            "-l",
            "k8s-app=metrics-server",
            "-o",
            "json",
        ],
    )
    .await
    .map(|v| parse_metrics_pods(&v))
    .unwrap_or_default();

    let desired_replicas = deployment_desired_replicas(&deployment);
    let available_replicas = deployment_available_replicas(&deployment);
    let ready = available_replicas > 0 && available_replicas >= desired_replicas;
    let kubelet_insecure_tls = deployment_has_arg(&deployment, "--kubelet-insecure-tls");
    // Measured through the same endpoint the monitoring UI reads, so "top
    // works" and "usage shows up" can never disagree.
    let top_works = kubectl_json(
        state,
        cluster,
        &["get", "--raw", "/apis/metrics.k8s.io/v1beta1/nodes"],
    )
    .await
    .is_ok();
    let message = diagnose_metrics(
        true,
        ready,
        kubelet_insecure_tls,
        top_works,
        &pods,
    );

    Ok(MetricsServerStatus {
        installed: true,
        desired_replicas,
        available_replicas,
        ready,
        kubelet_insecure_tls,
        pods,
        top_works,
        message,
    })
}

/// Real metrics-server state — lets the UI show "installed but starting",
/// "installed but broken", or "working" instead of only "no usage numbers".
#[tauri::command]
pub async fn metrics_server_status(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<MetricsServerStatus, String> {
    read_metrics_status(&state, &cluster).await
}

/// Installs metrics-server the way kind needs it: upstream manifest, the
/// `--kubelet-insecure-tls` flag kind's self-signed kubelets require, a wait
/// for the rollout, then a bounded wait until `kubectl top` actually answers.
/// Idempotent — running it again on an existing install repairs/updates it.
#[tauri::command]
pub async fn install_metrics_server(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<MetricsServerStatus, String> {
    kubectl_raw(
        &state,
        &cluster,
        &["apply", "-f", METRICS_SERVER_MANIFEST],
    )
    .await?;

    // Only patch when the flag is missing: a second `add` would append a
    // duplicate argument on every re-install.
    let deployment = kubectl_json(
        &state,
        &cluster,
        &[
            "get",
            "deployment",
            "metrics-server",
            "-n",
            "kube-system",
            "-o",
            "json",
        ],
    )
    .await?;
    if !deployment_has_arg(&deployment, "--kubelet-insecure-tls") {
        kubectl_raw(
            &state,
            &cluster,
            &[
                "patch",
                "deployment",
                "metrics-server",
                "-n",
                "kube-system",
                "--type=json",
                "-p",
                r#"[{"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--kubelet-insecure-tls"}]"#,
            ],
        )
        .await?;
    }

    // Bounded wait for the rollout; a timeout is not fatal — the status read
    // below reports whatever actually happened (ImagePullBackOff, slow pull…).
    let _ = kubectl_raw(
        &state,
        &cluster,
        &[
            "rollout",
            "status",
            "deployment/metrics-server",
            "-n",
            "kube-system",
            "--timeout=150s",
        ],
    )
    .await;

    // metrics-server needs one scrape cycle before the Metrics API answers.
    for _ in 0..12 {
        if kubectl_json(
            &state,
            &cluster,
            &["get", "--raw", "/apis/metrics.k8s.io/v1beta1/nodes"],
        )
        .await
        .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    read_metrics_status(&state, &cluster).await
}

/// Shells probed, in order, when opening a pod terminal. Absolute paths only —
/// some images have no `PATH` entries but do have `/bin/sh`.
const POD_SHELL_CANDIDATES: &[&str] = &["/bin/sh", "/bin/bash", "/bin/ash", "/busybox/sh"];

/// Detects which shell (if any) exists inside a pod container, by running each
/// candidate with `-c true`. Returns the first working path, or `None` for
/// images with no shell at all (distroless), which is what produced
/// `exec: "sh": executable file not found in $PATH`.
#[tauri::command]
pub async fn detect_pod_shell(
    state: State<'_, AppState>,
    cluster: String,
    namespace: String,
    name: String,
    container: Option<String>,
) -> Result<Option<String>, String> {
    k8s_name_check(&name)?;
    k8s_name_check(&namespace)?;
    if let Some(c) = container.as_deref().filter(|s| !s.is_empty()) {
        k8s_name_check(c)?;
    }

    for shell in POD_SHELL_CANDIDATES {
        let mut args: Vec<&str> = vec!["exec", "-n", &namespace, &name];
        if let Some(c) = container.as_deref().filter(|s| !s.is_empty()) {
            args.extend(["-c", c]);
        }
        args.extend(["--", shell, "-c", "true"]);
        if kubectl_raw(&state, &cluster, &args).await.is_ok() {
            return Ok(Some((*shell).to_string()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod metrics_status_tests {
    use super::*;
    use serde_json::json;

    fn deployment(args: &[&str], available: i64) -> serde_json::Value {
        json!({
            "spec": { "replicas": 1, "template": { "spec": { "containers": [
                { "args": args }
            ] } } },
            "status": { "availableReplicas": available }
        })
    }

    #[test]
    fn deployment_args_detected_without_duplicates() {
        let d = deployment(&["--kubelet-insecure-tls"], 1);
        assert!(deployment_has_arg(&d, "--kubelet-insecure-tls"));
        let d = deployment(&["--metric-resolution=15s"], 1);
        assert!(!deployment_has_arg(&d, "--kubelet-insecure-tls"));
        // Missing args array must not panic.
        assert!(!deployment_has_arg(&json!({}), "--kubelet-insecure-tls"));
    }

    #[test]
    fn replicas_are_read_from_status_and_spec() {
        assert_eq!(deployment_available_replicas(&deployment(&[], 1)), 1);
        assert_eq!(deployment_available_replicas(&deployment(&[], 0)), 0);
        assert_eq!(deployment_desired_replicas(&deployment(&[], 1)), 1);
        assert_eq!(deployment_desired_replicas(&json!({})), 1);
    }

    #[test]
    fn pod_status_captures_ready_state_restarts_and_reason() {
        let pods = json!({ "items": [
            { "metadata": { "name": "ms-1" },
              "status": {
                "phase": "Running",
                "containerStatuses": [
                  { "ready": false, "restartCount": 4,
                    "state": { "waiting": { "reason": "CrashLoopBackOff" } } }
                ]
              } }
        ]});
        let parsed = parse_metrics_pods(&pods);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "ms-1");
        assert!(!parsed[0].ready);
        assert_eq!(parsed[0].restarts, 4);
        assert_eq!(parsed[0].reason.as_deref(), Some("CrashLoopBackOff"));
        assert!(parse_metrics_pods(&json!({})).is_empty());
    }

    #[test]
    fn diagnosis_prefers_the_most_actionable_problem() {
        // Not installed → nothing to say (the UI offers Install).
        assert_eq!(diagnose_metrics(false, false, false, false, &[]), None);
        // A stuck pod explains itself first.
        let stuck = vec![MetricsPodStatus {
            name: "ms".into(),
            phase: "Pending".into(),
            ready: false,
            restarts: 0,
            reason: Some("ImagePullBackOff".into()),
        }];
        let msg = diagnose_metrics(true, false, false, false, &stuck).unwrap();
        assert!(msg.contains("ImagePullBackOff"));
        // Installed, not ready, no pod reason → still starting.
        assert!(diagnose_metrics(true, false, false, false, &[])
            .unwrap()
            .contains("still starting"));
        // Ready but missing the kind flag → the real misconfiguration.
        assert!(diagnose_metrics(true, true, false, false, &[])
            .unwrap()
            .contains("kubelet-insecure-tls"));
        // Ready + flag + no data yet → warming up, not an error.
        assert!(diagnose_metrics(true, true, true, false, &[])
            .unwrap()
            .contains("within a minute"));
        // Fully healthy → no banner.
        assert_eq!(diagnose_metrics(true, true, true, true, &[]), None);
    }
}


// ==================== Cluster prefs: auto-start + per-node caps ====================

use crate::config::KindClusterPrefs;

/// Reads a cluster's persisted prefs (defaults when none were set).
#[tauri::command]
pub async fn get_cluster_prefs(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<KindClusterPrefs, String> {
    Ok(state
        .config
        .read()
        .await
        .kind_cluster_prefs
        .get(cluster.trim())
        .cloned()
        .unwrap_or_default())
}

/// Saves a cluster's prefs and applies the node caps immediately.
#[tauri::command]
pub async fn set_cluster_prefs(
    state: State<'_, AppState>,
    cluster: String,
    prefs: KindClusterPrefs,
) -> Result<(), String> {
    let cluster = cluster.trim().to_string();
    if cluster.is_empty() {
        return Err("Cluster name is required".into());
    }
    {
        let mut cfg = state.config.write().await;
        cfg.kind_cluster_prefs.insert(cluster.clone(), prefs.clone());
        cfg.save().await.map_err(|e| e.to_string())?;
    }
    if prefs.node_cpu_m > 0 || prefs.node_memory_mb > 0 {
        let capped = apply_cluster_limits(&state, &cluster, &prefs).await?;
        tracing::info!("applied node caps to {capped} container(s) of '{cluster}'");
    }
    Ok(())
}

/// CPU/memory-caps every node container of `cluster` via `docker update`.
/// Returns how many containers were capped.
async fn apply_cluster_limits(
    state: &AppState,
    cluster: &str,
    prefs: &KindClusterPrefs,
) -> Result<u32, String> {
    let client = local_client(state).await?;
    let ids = cluster_node_ids(&client, cluster).await?;
    let mut capped = 0u32;
    for id in ids {
        let cpu = Some(prefs.node_cpu_m).filter(|v| *v > 0);
        let mem = Some(prefs.node_memory_mb).filter(|v| *v > 0);
        if client.update_container_caps(&id, cpu, mem).await.is_ok() {
            capped += 1;
        }
    }
    Ok(capped)
}

/// Ids of every (running or stopped) node container of a cluster.
async fn cluster_node_ids(
    client: &crate::docker::client::DockerClient,
    cluster: &str,
) -> Result<Vec<String>, String> {
    let mut filters = HashMap::new();
    filters.insert(
        "label".to_string(),
        vec![format!("{CLUSTER_LABEL}={cluster}")],
    );
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    let list = client
        .docker()
        .list_containers(Some(opts))
        .await
        .map_err(|e| e.to_string())?;
    Ok(list.into_iter().filter_map(|c| c.id).collect())
}

/// Clusters eligible for auto-start, sorted. Both switches must agree: the
/// master `kind_auto_start` and the cluster's own `auto_start`.
fn auto_start_names(cfg: &crate::config::AppConfig) -> Vec<String> {
    if !cfg.kind_auto_start {
        return Vec::new();
    }
    let mut names: Vec<String> = cfg
        .kind_cluster_prefs
        .iter()
        .filter(|(_, p)| p.auto_start)
        .map(|(k, _)| k.clone())
        .collect();
    names.sort();
    names
}

/// Auto-starts every cluster flagged `auto_start` when the engine comes up.
/// Starts are sequential with a configurable pause so N clusters don't hit
/// the host at once; caps are applied before each start.
pub async fn auto_start_kind_clusters(app: AppHandle, state: &AppState) -> Result<u32, String> {
    let (names, delay) = {
        let cfg = state.config.read().await;
        (auto_start_names(&cfg), cfg.kind_auto_start_delay)
    };
    if names.is_empty() {
        return Ok(0);
    }
    let mut started = 0u32;
    for (i, name) in names.iter().enumerate() {
        if i > 0 && delay > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay as u64)).await;
        }
        // Skip clusters whose node containers are gone (deleted outside the app).
        let exists = {
            let Ok(client) = local_client(state).await else { break };
            matches!(cluster_node_ids(&client, name).await, Ok(ids) if !ids.is_empty())
        };
        if !exists {
            continue;
        }
        match set_cluster_power(&app, state, name, true).await {
            Ok(()) => started += 1,
            Err(e) => tracing::warn!("auto-start cluster '{name}' failed: {e}"),
        }
    }
    if started > 0 {
        let _ = app.emit(
            "kind-progress",
            serde_json::json!({
                "action": "auto-start",
                "name": "",
                "line": format!("Auto-started {started} cluster(s)"),
            }),
        );
    }
    Ok(started)
}

/// Pins every kind node container to `restart=no`. kind creates nodes with
/// `on-failure:1`, so Docker otherwise restarts a cluster on its own whenever
/// the engine comes up — bypassing the auto-start switches entirely.
pub async fn enforce_kind_restart_policy(state: &AppState) -> Result<usize, String> {
    let client = local_client(state).await?;
    let containers = client
        .list_containers(true)
        .await
        .map_err(|e| e.to_string())?;
    let mut pinned = 0;
    for c in containers.iter().filter(|c| c.kind_cluster.is_some()) {
        let policy = client
            .inspect_container(&c.id)
            .await
            .map(|d| d.restart_policy)
            .unwrap_or_default();
        if policy != "no" && client.update_restart_policy(&c.id, "no").await.is_ok() {
            pinned += 1;
        }
    }
    Ok(pinned)
}

/// A running node is stopped only when its cluster is not eligible and the
/// container came up as part of this engine launch (restored), not before it.
fn should_stop_restored(eligible: bool, started_at: i64, restored_since: i64) -> bool {
    !eligible && started_at >= restored_since
}

/// Brings the cluster set in line with the auto-start configuration whenever
/// the engine comes up:
///  * node restart policies are pinned to `no` (for the next boot),
///  * clusters that started at/after `restored_since` while not marked
///    auto-start are stopped again (that covers containers Docker restored on
///    engine start; pass `0` on a known stopped→running transition, and the
///    process start time on the first observation after launch so clusters
///    that were already running are left alone),
///  * marked clusters are started staggered, as before.
pub async fn reconcile_kind_auto_start(
    app: AppHandle,
    state: &AppState,
    restored_since: i64,
) -> Result<u32, String> {
    let _ = enforce_kind_restart_policy(state).await;
    // Docker restores containers in parallel right after the socket appears;
    // give that pass a moment so nothing is started again after we look.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    {
        let allowed: std::collections::HashSet<String> = {
            let cfg = state.config.read().await;
            auto_start_names(&cfg).into_iter().collect()
        };
        if let Ok(client) = local_client(state).await {
            if let Ok(containers) = client.list_containers(true).await {
                for c in containers.iter().filter(|c| c.state == "running") {
                    let Some(cluster) = c.kind_cluster.as_ref() else {
                        continue;
                    };
                    if allowed.contains(cluster) {
                        continue;
                    }
                    // Only touch containers this engine launch brought up;
                    // ones that predate the launch were deliberate.
                    let started = client.container_started_at(&c.id).await.unwrap_or(0);
                    if !should_stop_restored(
                        allowed.contains(cluster),
                        started,
                        restored_since,
                    ) {
                        continue;
                    }
                    let _ = client.stop_container(&c.id).await;
                    let _ = app.emit(
                        "kind-progress",
                        serde_json::json!({
                            "action": "auto-start",
                            "name": cluster,
                            "line": format!("Stopped {cluster}: auto-start is off"),
                        }),
                    );
                }
            }
        }
    }

    auto_start_kind_clusters(app, state).await
}

// ==================== Pod & resource management ====================

fn k8s_name_check(s: &str) -> Result<(), String> {
    let s = s.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return Err("invalid name".into());
    }
    Ok(())
}

/// Container logs of a pod (its first container). `tail` lines, default 200.
#[tauri::command]
pub async fn get_pod_logs(
    state: State<'_, AppState>,
    cluster: String,
    namespace: String,
    name: String,
    tail: Option<u32>,
) -> Result<String, String> {
    k8s_name_check(&name)?;
    k8s_name_check(&namespace)?;
    let tail = tail.unwrap_or(200).clamp(10, 5000);
    kubectl_raw(
        &state,
        &cluster,
        &[
            "logs",
            &name,
            "-n",
            &namespace,
            &format!("--tail={tail}"),
        ],
    )
    .await
}

/// Resource kinds `get_resource_yaml` will fetch (singular and plural forms,
/// mirroring what `kubectl get` accepts). Anything else is rejected before the
/// exec so the allowlist stays an auditable constant.
const YAML_RESOURCE_KINDS: &[&str] = &[
    "pod",
    "pods",
    "deployment",
    "deployments",
    "statefulset",
    "statefulsets",
    "daemonset",
    "daemonsets",
    "replicaset",
    "replicasets",
    "job",
    "jobs",
    "cronjob",
    "cronjobs",
    "service",
    "services",
    "configmap",
    "configmaps",
    "secret",
    "secrets",
    "persistentvolumeclaim",
    "persistentvolumeclaims",
    "namespace",
    "namespaces",
    "node",
    "nodes",
    "ingress",
    "ingresses",
];

/// True when `kind` is on the [`YAML_RESOURCE_KINDS`] allowlist.
fn is_yaml_kind(kind: &str) -> bool {
    YAML_RESOURCE_KINDS.contains(&kind.trim().to_lowercase().as_str())
}

/// Full manifest (`-o yaml`) of a single namespaced/cluster-scoped resource.
/// `namespace` is ignored for cluster-scoped kinds (node, namespace).
#[tauri::command]
pub async fn get_resource_yaml(
    state: State<'_, AppState>,
    cluster: String,
    kind: String,
    namespace: Option<String>,
    name: String,
) -> Result<String, String> {
    k8s_name_check(&name)?;
    if !is_yaml_kind(&kind) {
        return Err(format!(
            "resource kind '{kind}' is not supported for YAML viewing"
        ));
    }
    if let Some(ns) = namespace.as_deref().filter(|s| !s.is_empty()) {
        k8s_name_check(ns)?;
    }
    let kind_l = kind.trim().to_lowercase();
    let mut args: Vec<&str> = vec!["get", kind_l.as_str(), &name];
    if let Some(ns) = namespace.as_deref().filter(|s| !s.is_empty()) {
        args.extend(["-n", ns]);
    }
    args.extend(["-o", "yaml"]);
    kubectl_raw(&state, &cluster, &args).await
}

/// Events whose `involvedObject` is the given object (oldest first).
#[tauri::command]
pub async fn get_resource_events(
    state: State<'_, AppState>,
    cluster: String,
    namespace: String,
    name: String,
) -> Result<Vec<K8sEventInfo>, String> {
    k8s_name_check(&name)?;
    k8s_name_check(&namespace)?;
    let selector = format!("involvedObject.name={name}");
    let v = match kubectl_json(
        &state,
        &cluster,
        &[
            "get",
            "events",
            "-n",
            &namespace,
            "--field-selector",
            &selector,
            "--sort-by=.lastTimestamp",
            "-o",
            "json",
        ],
    )
    .await
    {
        Ok(v) => v,
        // Some apiservers reject a field selector on events with
        // "the server could not find the requested resource" (notably while
        // the cluster is still settling). Listing the namespace and filtering
        // here gives the same answer without the server-side selector.
        Err(_) => {
            let mut all = kubectl_json(
                &state,
                &cluster,
                &["get", "events", "-n", &namespace, "-o", "json"],
            )
            .await?;
            if let Some(items) = all.get_mut("items").and_then(|i| i.as_array_mut()) {
                items.retain(|e| {
                    e.get("involvedObject")
                        .and_then(|o| o.get("name"))
                        .and_then(|n| n.as_str())
                        == Some(name.as_str())
                });
            }
            all
        }
    };
    Ok(parse_events(&v))
}

/// Aggregated logs of a workload (`kubectl logs deployment/name` picks one of
/// its pods) across all containers.
#[tauri::command]
pub async fn get_workload_logs(
    state: State<'_, AppState>,
    cluster: String,
    kind: String,
    namespace: String,
    name: String,
    tail: Option<u32>,
) -> Result<String, String> {
    k8s_name_check(&name)?;
    k8s_name_check(&namespace)?;
    let kind_l = kind.trim().to_lowercase();
    if !["deployment", "statefulset", "daemonset"].contains(&kind_l.as_str()) {
        return Err(format!(
            "logs are not available for {kind_l} resources — select a workload"
        ));
    }
    let tail = tail.unwrap_or(300).clamp(10, 5000);
    let target = format!("{kind_l}/{name}");
    kubectl_raw(
        &state,
        &cluster,
        &[
            "logs",
            &target,
            "-n",
            &namespace,
            "--all-containers=true",
            &format!("--tail={tail}"),
        ],
    )
    .await
}

#[cfg(test)]
mod resource_view_tests {
    use super::*;

    #[test]
    fn yaml_kind_allowlist_accepts_singular_and_plural_only() {
        assert!(is_yaml_kind("pod"));
        assert!(is_yaml_kind("pods"));
        assert!(is_yaml_kind("Deployment")); // case-insensitive
        assert!(is_yaml_kind("statefulsets"));
        assert!(is_yaml_kind("persistentvolumeclaims"));
        assert!(!is_yaml_kind("clusterrole"));
        assert!(!is_yaml_kind(""));
        assert!(!is_yaml_kind("pods; rm -rf /"));
    }
}

/// Deletes a pod (the owning controller recreates it unless it's bare).
#[tauri::command]
pub async fn delete_pod(
    state: State<'_, AppState>,
    cluster: String,
    namespace: String,
    name: String,
    force: Option<bool>,
) -> Result<(), String> {
    k8s_name_check(&name)?;
    k8s_name_check(&namespace)?;
    let mut args = vec!["delete", "pod", name.trim(), "-n", namespace.trim()];
    if force.unwrap_or(false) {
        args.push("--force");
        args.push("--grace-period=0");
    }
    kubectl_raw(&state, &cluster, &args).await.map(|_| ())
}

/// Generic delete for services/configmaps/secrets/PVCs/namespaces/workloads.
#[tauri::command]
pub async fn delete_k8s_resource(
    state: State<'_, AppState>,
    cluster: String,
    kind: String,
    namespace: Option<String>,
    name: String,
) -> Result<(), String> {
    k8s_name_check(&name)?;
    let allowed = [
        "service",
        "configmap",
        "secret",
        "persistentvolumeclaim",
        "deployment",
        "statefulset",
        "daemonset",
        "namespace",
    ];
    if !allowed.contains(&kind.as_str()) {
        return Err(format!("kind '{kind}' is not deletable from the UI"));
    }
    let ns_owned = namespace.filter(|n| !n.is_empty() && n != "all");
    if let Some(ns) = &ns_owned {
        k8s_name_check(ns)?;
    }
    let mut args = vec!["delete", kind.trim(), name.trim()];
    if let Some(ns) = &ns_owned {
        args.push("-n");
        args.push(ns.trim());
    }
    kubectl_raw(&state, &cluster, &args).await.map(|_| ())
}

/// Scales a Deployment/StatefulSet to `replicas`.
#[tauri::command]
pub async fn scale_workload(
    state: State<'_, AppState>,
    cluster: String,
    kind: String,
    namespace: String,
    name: String,
    replicas: u32,
) -> Result<(), String> {
    k8s_name_check(&name)?;
    if !["deployment", "statefulset"].contains(&kind.as_str()) {
        return Err("only deployments and statefulsets are scalable".into());
    }
    kubectl_raw(
        &state,
        &cluster,
        &[
            "scale",
            &kind,
            &name,
            "-n",
            &namespace,
            &format!("--replicas={replicas}"),
        ],
    )
    .await
    .map(|_| ())
}

/// Rolling-restarts a workload (`kubectl rollout restart`).
#[tauri::command]
pub async fn restart_workload(
    state: State<'_, AppState>,
    cluster: String,
    kind: String,
    namespace: String,
    name: String,
) -> Result<(), String> {
    k8s_name_check(&name)?;
    kubectl_raw(
        &state,
        &cluster,
        &["rollout", "restart", &kind, &name, "-n", &namespace],
    )
    .await
    .map(|_| ())
}

/// Creates a namespace.
#[tauri::command]
pub async fn create_namespace(
    state: State<'_, AppState>,
    cluster: String,
    name: String,
) -> Result<(), String> {
    k8s_name_check(&name)?;
    kubectl_raw(&state, &cluster, &["create", "namespace", name.trim()])
        .await
        .map(|_| ())
}

// ---- Service / ConfigMap+Secret / PVC listings ----

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sServiceInfo {
    pub name: String,
    pub namespace: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub cluster_ip: String,
    pub ports: String,
    pub created: Option<i64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sConfigInfo {
    pub name: String,
    pub namespace: String,
    /// "ConfigMap" or "Secret".
    pub resource_kind: String,
    pub keys: Vec<String>,
    pub created: Option<i64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct K8sPvcInfo {
    pub name: String,
    pub namespace: String,
    pub status: String,
    pub capacity: String,
    pub access_modes: String,
    pub storage_class: String,
    pub created: Option<i64>,
}

fn parse_services(json: &serde_json::Value) -> Vec<K8sServiceInfo> {
    let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|s| {
            let meta = s.get("metadata")?;
            let ports = s
                .get("spec")
                .and_then(|sp| sp.get("ports"))
                .and_then(|p| p.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|p| {
                            let port = p.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
                            let proto =
                                p.get("protocol").and_then(|v| v.as_str()).unwrap_or("TCP");
                            let node = p
                                .get("nodePort")
                                .and_then(|v| v.as_u64())
                                .map(|np| format!(":{np}"))
                                .unwrap_or_default();
                            format!("{port}{node}/{proto}")
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            Some(K8sServiceInfo {
                name: meta.get("name")?.as_str()?.to_string(),
                namespace: meta
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string(),
                kind: s
                    .get("spec")
                    .and_then(|sp| sp.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("ClusterIP")
                    .to_string(),
                cluster_ip: s
                    .get("spec")
                    .and_then(|sp| sp.get("clusterIP"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                ports,
                created: meta
                    .get("creationTimestamp")
                    .and_then(|t| t.as_str())
                    .and_then(rfc3339_to_epoch),
            })
        })
        .collect()
}

fn parse_config_resources(json: &serde_json::Value, resource_kind: &str) -> Vec<K8sConfigInfo> {
    let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|c| {
            let meta = c.get("metadata")?;
            let keys = c
                .get("data")
                .and_then(|d| d.as_object())
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            Some(K8sConfigInfo {
                name: meta.get("name")?.as_str()?.to_string(),
                namespace: meta
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string(),
                resource_kind: resource_kind.to_string(),
                keys,
                created: meta
                    .get("creationTimestamp")
                    .and_then(|t| t.as_str())
                    .and_then(rfc3339_to_epoch),
            })
        })
        .collect()
}

fn parse_pvcs(json: &serde_json::Value) -> Vec<K8sPvcInfo> {
    let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
        return vec![];
    };
    items
        .iter()
        .filter_map(|p| {
            let meta = p.get("metadata")?;
            let spec = p.get("spec")?;
            let status = p.get("status")?;
            Some(K8sPvcInfo {
                name: meta.get("name")?.as_str()?.to_string(),
                namespace: meta
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string(),
                status: status
                    .get("phase")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string(),
                capacity: status
                    .get("capacity")
                    .and_then(|c| c.get("storage"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                access_modes: spec
                    .get("accessModes")
                    .and_then(|a| a.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default(),
                storage_class: spec
                    .get("storageClassName")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                created: meta
                    .get("creationTimestamp")
                    .and_then(|t| t.as_str())
                    .and_then(rfc3339_to_epoch),
            })
        })
        .collect()
}

#[tauri::command]
pub async fn get_cluster_services(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<Vec<K8sServiceInfo>, String> {
    let v = kubectl_json(
        &state,
        &cluster,
        &["get", "services", "--all-namespaces", "-o", "json"],
    )
    .await?;
    Ok(parse_services(&v))
}

#[tauri::command]
pub async fn get_cluster_config_resources(
    state: State<'_, AppState>,
    cluster: String,
    include_secrets: Option<bool>,
) -> Result<Vec<K8sConfigInfo>, String> {
    let mut out = Vec::new();
    let cm = kubectl_json(
        &state,
        &cluster,
        &["get", "configmaps", "--all-namespaces", "-o", "json"],
    )
    .await?;
    out.extend(parse_config_resources(&cm, "ConfigMap"));
    if include_secrets.unwrap_or(true) {
        let sec = kubectl_json(
            &state,
            &cluster,
            &["get", "secrets", "--all-namespaces", "-o", "json"],
        )
        .await?;
        out.extend(parse_config_resources(&sec, "Secret"));
    }
    Ok(out)
}

#[tauri::command]
pub async fn get_cluster_pvcs(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<Vec<K8sPvcInfo>, String> {
    let v = kubectl_json(
        &state,
        &cluster,
        &[
            "get",
            "persistentvolumeclaims",
            "--all-namespaces",
            "-o",
            "json",
        ],
    )
    .await?;
    Ok(parse_pvcs(&v))
}

#[cfg(test)]
mod monitor_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_json_tolerates_leading_warnings() {
        // Clean payloads parse directly.
        assert_eq!(extract_json(r#"{"items":[]}"#), Some(json!({"items": []})));
        // kubectl warnings merged into the exec stream must not break parsing.
        let noisy = "Warning: v1 PodMetrics is deprecated\n{\"items\":[{\"metadata\":{\"name\":\"p\"}}]}";
        assert_eq!(
            extract_json(noisy),
            Some(json!({"items":[{"metadata":{"name":"p"}}]}))
        );
        assert_eq!(extract_json("no json here"), None);
        assert_eq!(extract_json(""), None);
    }

    #[test]
    fn readyz_verdict_accepts_all_formats() {
        // plain non-verbose success
        assert_eq!(readyz_verdict("ok"), (true, None));
        // verbose success (what kind clusters actually return)
        assert_eq!(
            readyz_verdict("[+]ping ok\n[+]log ok\n[+]etcd ok"),
            (true, None)
        );
        // verbose failure keeps the failing line
        let (r, msg) = readyz_verdict("[+]ping ok\n[-]etcd failed: timeout");
        assert!(!r);
        assert_eq!(msg.as_deref(), Some("[-]etcd failed: timeout"));
        // non-verbose failure list
        let (r, msg) = readyz_verdict("ping failed: connection refused");
        assert!(!r);
        assert!(msg.unwrap().contains("failed"));
        // garbage → not ready, not a panic
        assert!(!readyz_verdict("").0);
    }

    #[test]
    fn cpu_quantity_variants_to_millicores() {
        assert_eq!(cpu_to_millicores("250m"), Some(250.0));
        assert_eq!(cpu_to_millicores("2"), Some(2000.0));
        // 1.5e9 nanocores = 1.5 cores = 1500 millicores.
        assert_eq!(cpu_to_millicores("1500000000n"), Some(1500.0));
        assert_eq!(cpu_to_millicores("junk"), None);
    }

    #[test]
    fn mem_quantity_variants_to_bytes() {
        assert_eq!(mem_to_bytes("1024Ki"), Some(1024 * 1024));
        assert_eq!(mem_to_bytes("1927Mi"), Some(1927 * 1024 * 1024));
        assert_eq!(mem_to_bytes("2Gi"), Some(2 * 1024 * 1024 * 1024));
        assert_eq!(mem_to_bytes("512"), Some(512));
        assert_eq!(mem_to_bytes("junk"), None);
    }

    #[test]
    fn parse_nodes_reads_ready_roles_and_capacity() {
        let v = json!({
            "items": [{
                "metadata": {
                    "name": "demo-control-plane",
                    "creationTimestamp": "2025-10-19T07:00:00Z",
                    "labels": { "node-role.kubernetes.io/control-plane": "" }
                },
                "status": {
                    "addresses": [{ "type": "InternalIP", "address": "172.18.0.2" }],
                    "conditions": [
                        { "type": "Ready", "status": "True", "reason": "KubeletReady" }
                    ],
                    "nodeInfo": {
                        "kubeletVersion": "v1.34.0",
                        "osImage": "Ubuntu 24.04",
                        "containerRuntimeVersion": "containerd://2.0.0",
                        "cpuCapacity": "8",
                        "memoryCapacity": "8036204Ki",
                        "podsCapacity": "110"
                    }
                }
            }]
        });
        let nodes = parse_nodes(&v);
        assert_eq!(nodes.len(), 1);
        let n = &nodes[0];
        assert!(n.ready);
        assert_eq!(n.status, "Ready");
        assert_eq!(n.roles, vec!["control-plane"]);
        assert_eq!(n.cpu_capacity_m, 8000.0);
        assert_eq!(n.mem_capacity_bytes, 8036204 * 1024);
        assert_eq!(n.pods_capacity, 110);
        assert_eq!(n.internal_ip, "172.18.0.2");
        assert_eq!(n.created, Some(1760857200)); // 2025-10-19T07:00:00Z
    }

    #[test]
    fn parse_pods_counts_ready_restarts_and_images() {
        let v = json!({
            "items": [{
                "metadata": { "name": "coredns-abc", "namespace": "kube-system",
                              "creationTimestamp": "2025-10-19T07:01:00Z" },
                "spec": { "nodeName": "demo-control-plane", "containers": [{ "image": "registry.k8s.io/coredns:v1.12" }] },
                "status": {
                    "phase": "Running",
                    "containerStatuses": [
                        { "ready": true, "restartCount": 2, "image": "registry.k8s.io/coredns:v1.12" }
                    ]
                }
            }, {
                "metadata": { "name": "stuck", "namespace": "default" },
                "spec": { "containers": [{ "image": "nginx:1.27" }] },
                "status": { "phase": "Pending", "reason": "Unschedulable" }
            }]
        });
        let pods = parse_pods(&v);
        assert_eq!(pods.len(), 2);
        assert_eq!(pods[0].phase, "Running");
        assert_eq!(pods[0].ready_containers, 1);
        assert_eq!(pods[0].restarts, 2);
        assert_eq!(pods[0].node, "demo-control-plane");
        assert_eq!(pods[1].phase, "Pending");
        assert_eq!(pods[1].reason.as_deref(), Some("Unschedulable"));
        assert_eq!(pods[1].total_containers, 1);
    }

    #[test]
    fn parse_events_marks_warnings_and_counts() {
        let v = json!({
            "items": [{
                "type": "Warning",
                "reason": "FailedScheduling",
                "message": "0/1 nodes are available",
                "involvedObject": { "kind": "Pod", "name": "stuck" },
                "metadata": { "namespace": "default" },
                "count": 5,
                "lastTimestamp": "2025-10-19T07:10:00Z"
            }]
        });
        let events = parse_events(&v);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "Warning");
        assert_eq!(events[0].object, "Pod stuck");
        assert_eq!(events[0].count, 5);
    }

    #[test]
    fn parse_workloads_reads_replica_triplet() {
        let v = json!({
            "items": [{
                "kind": "Deployment",
                "metadata": { "name": "web", "namespace": "prod" },
                "spec": { "replicas": 3 },
                "status": { "readyReplicas": 2, "availableReplicas": 2 }
            }, {
                "kind": "DaemonSet",
                "metadata": { "name": "agent", "namespace": "kube-system" },
                "spec": {},
                "status": { "readyReplicas": 1 }
            }]
        });
        let w = parse_workloads(&v);
        assert_eq!(w.len(), 2);
        assert_eq!((w[0].ready, w[0].desired, w[0].available), (2, 3, 2));
        assert_eq!((w[1].ready, w[1].desired), (1, 0));
    }

    #[test]
    fn parse_services_config_and_pvcs() {
        let svcs = serde_json::json!({
            "items": [{
                "metadata": { "name": "web", "namespace": "prod",
                              "creationTimestamp": "2025-10-19T07:00:00Z" },
                "spec": {
                    "type": "NodePort",
                    "clusterIP": "10.96.0.10",
                    "ports": [{ "port": 80, "protocol": "TCP", "nodePort": 30080 }]
                }
            }]
        });
        let s = parse_services(&svcs);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].kind, "NodePort");
        assert_eq!(s[0].ports, "80:30080/TCP");
        assert_eq!(s[0].cluster_ip, "10.96.0.10");

        let cms = serde_json::json!({
            "items": [{
                "metadata": { "name": "cfg", "namespace": "default" },
                "data": { "KEY_A": "1", "KEY_B": "2" }
            }]
        });
        let c = parse_config_resources(&cms, "ConfigMap");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].keys.len(), 2);
        assert_eq!(c[0].resource_kind, "ConfigMap");

        let pvcs = serde_json::json!({
            "items": [{
                "metadata": { "name": "data", "namespace": "app" },
                "spec": {
                    "storageClassName": "standard",
                    "accessModes": ["ReadWriteOnce"]
                },
                "status": {
                    "phase": "Bound",
                    "capacity": { "storage": "1Gi" }
                }
            }]
        });
        let p = parse_pvcs(&pvcs);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].status, "Bound");
        assert_eq!(p[0].capacity, "1Gi");
        assert_eq!(p[0].storage_class, "standard");
    }

    #[test]
    fn parse_metrics_sums_pod_containers() {
        let nodes = json!({
            "items": [{ "metadata": { "name": "n1" },
                        "usage": { "cpu": "120m", "memory": "512Mi" } }]
        });
        let pods = json!({
            "items": [{
                "metadata": { "name": "p1", "namespace": "ns" },
                "containers": [
                    { "usage": { "cpu": "50m", "memory": "100Mi" } },
                    { "usage": { "cpu": "70m", "memory": "200Mi" } }
                ]
            }]
        });
        let nm = parse_node_metrics(&nodes);
        assert_eq!(nm[0].cpu_m, 120.0);
        assert_eq!(nm[0].mem_bytes, 512 * 1024 * 1024);
        let pm = parse_pod_metrics(&pods);
        assert_eq!(pm[0].cpu_m, 120.0);
        assert_eq!(pm[0].mem_bytes, 300 * 1024 * 1024);
    }
}

    /// Debug: exec_capture against the live engine + cluster (opt-in via
    /// `cargo test -- --ignored`). Not part of the suite — needs a running cluster.
    #[tokio::test]
    #[ignore]
    async fn exec_capture_real_cluster_roundtrip() {
        let sock = std::path::Path::new("/run/user/1000/hsx2dock/docker.sock");
        if !sock.exists() {
            eprintln!("engine socket absent, skipping");
            return;
        }
        let client = crate::docker::client::DockerClient::connect(Some(sock)).unwrap();
        async fn kjson(
            client: &crate::docker::client::DockerClient,
            args: &[&str],
        ) -> serde_json::Value {
            let mut cmd = vec![
                "kubectl".to_string(),
                "--kubeconfig=/etc/kubernetes/admin.conf".to_string(),
            ];
            cmd.extend(args.iter().map(|s| s.to_string()));
            let out = client
                .exec_capture("cluster-control-plane", cmd)
                .await
                .expect("exec");
            serde_json::from_str(out.trim()).expect("json")
        }
        let nodes_v = kjson(&client, &["get", "nodes", "-o", "json"]).await;
        let nodes = parse_nodes(&nodes_v);
        eprintln!(
            "parsed nodes: {} (ready flags: {:?})",
            nodes.len(),
            nodes.iter().map(|n| (n.name.clone(), n.ready)).collect::<Vec<_>>()
        );
        let pods_v = kjson(&client, &["get", "pods", "--all-namespaces", "-o", "json"]).await;
        let pods = parse_pods(&pods_v);
        eprintln!(
            "parsed pods: {} (phases: {:?})",
            pods.len(),
            pods.iter()
                .map(|p| format!("{}/{}", p.namespace, p.phase))
                .collect::<Vec<_>>()
        );
        let ns_v = kjson(&client, &["get", "namespaces", "-o", "json"]).await;
        let active = ns_v["items"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|n| n["status"]["phase"] == "Active")
                    .count()
            })
            .unwrap_or(0);
        eprintln!("parsed active namespaces: {active}");
        let out = client
            .exec_capture(
                "cluster-control-plane",
                vec![
                    "kubectl".to_string(),
                    "--kubeconfig=/etc/kubernetes/admin.conf".to_string(),
                    "get".into(),
                    "--raw".into(),
                    "/readyz?verbose".into(),
                ],
            )
            .await
            .expect("readyz");
        eprintln!(
            "readyz starts_with_ok={} contains_failed={} first_line={:?}",
            out.trim_start().starts_with("ok"),
            out.contains("failed"),
            out.trim().lines().next().unwrap_or("")
        );

        // Hammer: 10 back-to-back poll rounds like the UI does (summary-ish
        // burst: 5 execs per round, overlapping rounds allowed) to catch
        // intermittent failures under load.
        let mut fails = 0;
        for round in 0..10 {
            for args in [
                vec!["version", "-o", "json"],
                vec!["get", "nodes", "-o", "json"],
                vec!["get", "pods", "--all-namespaces", "-o", "json"],
                vec!["get", "namespaces", "-o", "json"],
                vec!["get", "events", "--all-namespaces", "--sort-by=.lastTimestamp", "-o", "json"],
            ] {
                let mut cmd = vec![
                    "kubectl".to_string(),
                    "--kubeconfig=/etc/kubernetes/admin.conf".to_string(),
                ];
                cmd.extend(args.iter().map(|s| s.to_string()));
                if let Err(e) = client.exec_capture("cluster-control-plane", cmd).await {
                    fails += 1;
                    eprintln!("round {round} FAIL {:?} -> {e:#}", args.first());
                }
            }
        }
        eprintln!("hammer: {fails} failures / 50 execs");
    }

#[cfg(test)]
mod auto_start_tests {
    use super::*;
    use crate::config::{AppConfig, KindClusterPrefs};

    fn prefs(auto: bool) -> KindClusterPrefs {
        KindClusterPrefs {
            auto_start: auto,
            ..Default::default()
        }
    }

    #[test]
    fn master_switch_gates_every_cluster() {
        let mut cfg = AppConfig::default();
        cfg.kind_cluster_prefs.insert("a".into(), prefs(true));
        cfg.kind_cluster_prefs.insert("b".into(), prefs(false));

        cfg.kind_auto_start = false;
        assert!(auto_start_names(&cfg).is_empty());

        cfg.kind_auto_start = true;
        assert_eq!(auto_start_names(&cfg), vec!["a".to_string()]);
    }

    #[test]
    fn names_are_sorted_and_unmarked_clusters_skipped() {
        let mut cfg = AppConfig::default();
        cfg.kind_auto_start = true;
        cfg.kind_cluster_prefs.insert("zeta".into(), prefs(true));
        cfg.kind_cluster_prefs.insert("alpha".into(), prefs(true));
        cfg.kind_cluster_prefs.insert("mid".into(), prefs(false));

        assert_eq!(
            auto_start_names(&cfg),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }

    #[test]
    fn restored_ineligible_cluster_is_stopped_but_preexisting_is_kept() {
        // A real stopped→running transition uses threshold 0: every ineligible
        // running cluster goes, regardless of when it started.
        assert!(should_stop_restored(false, 10, 0));
        // First tick after launch only stops containers started at/after it,
        // so a cluster that was already running is left alone.
        assert!(should_stop_restored(false, 100, 100));
        assert!(!should_stop_restored(false, 99, 100));
        // Marked clusters are never stopped by the reconciler.
        assert!(!should_stop_restored(true, 100, 0));
    }
}
