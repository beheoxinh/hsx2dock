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
    }
    list.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(list)
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
    set_cluster_power(&app, &state, &name, true).await
}

/// Stops every node container of a cluster (state persists; Start resumes it).
#[tauri::command]
pub async fn stop_kind_cluster(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    set_cluster_power(&app, &state, &name, false).await
}

async fn set_cluster_power(
    app: &AppHandle,
    state: &State<'_, AppState>,
    name: &str,
    start: bool,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Cluster name is required".into());
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

/// Same as [`kubectl_raw`] but parses stdout as JSON.
async fn kubectl_json(
    state: &AppState,
    cluster: &str,
    args: &[&str],
) -> Result<serde_json::Value, String> {
    let out = kubectl_raw(state, cluster, args).await?;
    serde_json::from_str(out.trim()).map_err(|e| format!("bad kubectl JSON from {args:?}: {e}"))
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
    // Both `kubectl top` queries are best-effort: kind ships no metrics-server
    // by default, so a failure is data (reported via `error`), not an error.
    let nodes_res = kubectl_json(&state, &cluster, &["top", "nodes", "-o", "json"]).await;
    let pods_res =
        kubectl_json(&state, &cluster, &["top", "pods", "--all-namespaces", "-o", "json"]).await;
    let error = match (&nodes_res, &pods_res) {
        (Err(a), Err(b)) => Some(format!("{a}; {b}")),
        (Err(a), Ok(_)) => Some(a.clone()),
        (Ok(_), Err(b)) => Some(b.clone()),
        _ => None,
    };
    Ok(K8sMetrics {
        nodes: nodes_res.as_ref().map(|v| parse_node_metrics(v)).unwrap_or_default(),
        pods: pods_res.as_ref().map(|v| parse_pod_metrics(v)).unwrap_or_default(),
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

    // /readyz is plain text ("ok" or a failure list) — not JSON.
    let (ready, ready_message) = match kubectl_raw(
        &state,
        &cluster,
        &["get", "--raw", "/readyz?verbose"],
    )
    .await
    {
        Ok(out) if out.trim_start().starts_with("ok") => (true, None),
        Ok(out) => (false, Some(out.trim().lines().next().unwrap_or("not ready").to_string())),
        Err(e) => (false, Some(e)),
    };

    let pods = kubectl_json(
        &state,
        &cluster,
        &["get", "pods", "--all-namespaces", "-o", "json"],
    )
    .await
    .map(|v| parse_pods(&v))
    .unwrap_or_default();
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

    let nodes = kubectl_json(&state, &cluster, &["get", "nodes", "-o", "json"])
        .await
        .map(|v| parse_nodes(&v))
        .unwrap_or_default();
    let ready_nodes = nodes.iter().filter(|n| n.ready).count();

    let namespace_count = kubectl_json(&state, &cluster, &["get", "namespaces", "-o", "json"])
        .await
        .map(|v| {
            v.get("items")
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
                .unwrap_or(0)
        })
        .unwrap_or(0);

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
    })
}

/// Installs the upstream metrics-server manifests into the cluster so
/// `kubectl top` (and the monitoring UI's CPU/memory pills) start working.
#[tauri::command]
pub async fn install_metrics_server(
    state: State<'_, AppState>,
    cluster: String,
) -> Result<(), String> {
    kubectl_raw(
        &state,
        &cluster,
        &[
            "apply",
            "-f",
            "https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml",
        ],
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod monitor_tests {
    use super::*;
    use serde_json::json;

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
