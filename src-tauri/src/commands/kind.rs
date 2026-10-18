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
