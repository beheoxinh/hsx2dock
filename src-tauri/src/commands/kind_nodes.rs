//! Adding and removing worker nodes of a running kind cluster. kind itself
//! only shapes a cluster at create time, so a new worker is built the way kind
//! builds one — a privileged node container cloned from an existing node's
//! settings — and joined with a fresh `kubeadm token create --print-join-command`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use super::kind::{control_plane_node, kubectl_raw, local_client};
use crate::docker::kind::CLUSTER_LABEL;
use crate::state::AppState;

const ROLE_LABEL: &str = "io.x-k8s.kind.role";
const CONTAINERD_PATH: &str = "/var/lib/containerd";

/// kind names workers `<c>-worker`, `<c>-worker2`, … — the first free one.
pub(crate) fn next_worker_name(cluster: &str, existing: &HashSet<String>) -> String {
    let first = format!("{cluster}-worker");
    if !existing.contains(&first) {
        return first;
    }
    (2..)
        .map(|i| format!("{cluster}-worker{i}"))
        .find(|n| !existing.contains(n))
        .unwrap_or(first)
}

/// First `node<i>` state dir index not used yet.
pub(crate) fn next_state_index(used: &HashSet<usize>) -> usize {
    (0..).find(|i| !used.contains(i)).unwrap_or(0)
}

/// The template's binds with the containerd state bind swapped for `state`.
pub(crate) fn rewrite_binds(binds: &[String], state: &str) -> Vec<String> {
    let mut out: Vec<String> = binds
        .iter()
        .filter(|b| {
            let mut parts = b.split(':');
            let _src = parts.next();
            parts.next() != Some(CONTAINERD_PATH)
        })
        .cloned()
        .collect();
    out.push(format!("{state}:{CONTAINERD_PATH}"));
    out
}

/// `kubeadm join …` printed by the control plane, made safe for a kind node.
pub(crate) fn join_script(join_cmd: &str) -> Result<String, String> {
    let line = join_cmd
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("kubeadm join "))
        .ok_or_else(|| format!("unexpected join command output: {join_cmd}"))?;
    if line.contains(['\'', '"', ';', '&', '|', '`', '$']) {
        return Err("join command contains unexpected characters".into());
    }
    // kind skips preflight for joins too (swap, kernel config checks fail in
    // a container).
    Ok(format!("{line} --skip-phases=preflight --v=2"))
}

fn progress(app: Option<&AppHandle>, cluster: &str, line: impl Into<String>) {
    if let Some(app) = app {
        let _ = app.emit(
            "kind-progress",
            serde_json::json!({ "action": "node", "name": cluster, "line": line.into() }),
        );
    }
}

#[tauri::command]
pub async fn add_cluster_worker(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    cluster: String,
) -> Result<String, String> {
    add_cluster_worker_core(&state, Some(&app), &cluster).await
}

/// Creates and joins one more worker; returns its node name.
pub(crate) async fn add_cluster_worker_core(
    state: &AppState,
    app: Option<&AppHandle>,
    cluster: &str,
) -> Result<String, String> {
    super::kind::check_cluster_name(cluster)?;
    let client = local_client(state).await?;
    let docker = client.docker();

    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![format!("{CLUSTER_LABEL}={cluster}")]);
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    let nodes = docker.list_containers(Some(opts)).await.map_err(|e| e.to_string())?;
    if nodes.is_empty() {
        return Err(format!("cluster '{cluster}' does not exist"));
    }
    if nodes.len() >= super::kind_spec::MAX_NODES as usize {
        return Err(format!("A cluster can have at most {} nodes", super::kind_spec::MAX_NODES));
    }
    let names: HashSet<String> = nodes
        .iter()
        .flat_map(|n| n.names.clone().unwrap_or_default())
        .map(|n| n.trim_start_matches('/').to_string())
        .collect();
    let cp = control_plane_node(cluster);
    // Clone a worker when there is one (same image, mounts, devices), else
    // the control plane.
    let template = nodes
        .iter()
        .find(|n| n.labels.as_ref().and_then(|l| l.get(ROLE_LABEL)).map(String::as_str) == Some("worker"))
        .or_else(|| nodes.iter().find(|n| n.names.iter().flatten().any(|x| x.trim_start_matches('/') == cp)))
        .and_then(|n| n.id.clone())
        .ok_or("no node to copy settings from")?;
    let tpl = docker.inspect_container(&template, None).await.map_err(|e| e.to_string())?;
    if !tpl.state.as_ref().and_then(|s| s.running).unwrap_or(false) {
        return Err("Start the cluster before adding a node".into());
    }
    let tpl_config = tpl.config.clone().unwrap_or_default();
    let tpl_host = tpl.host_config.clone().unwrap_or_default();

    let name = next_worker_name(cluster, &names);
    // State dir: same parent as the template's containerd bind, next index.
    let tpl_state = tpl_host
        .binds
        .iter()
        .flatten()
        .find_map(|b| {
            let mut p = b.split(':');
            let src = p.next()?;
            (p.next() == Some(CONTAINERD_PATH)).then(|| std::path::PathBuf::from(src))
        })
        .ok_or("the cluster has no app-managed node state mount")?;
    let state_parent = tpl_state.parent().ok_or("bad node state path")?.to_path_buf();
    let used: HashSet<usize> = std::fs::read_dir(&state_parent)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().to_string_lossy().strip_prefix("node")?.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    let node_state = state_parent.join(format!("node{}", next_state_index(&used)));
    std::fs::create_dir_all(&node_state).map_err(|e| format!("cannot create {}: {e}", node_state.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&node_state, std::fs::Permissions::from_mode(0o777));
    }

    progress(app, cluster, format!("Creating node {name}…"));
    let mut labels = tpl_config.labels.clone().unwrap_or_default();
    labels.insert(ROLE_LABEL.into(), "worker".into());
    let binds = rewrite_binds(tpl_host.binds.as_deref().unwrap_or_default(), &node_state.display().to_string());
    let body = bollard::models::ContainerCreateBody {
        image: tpl_config.image.clone(),
        hostname: Some(name.clone()),
        env: tpl_config.env.clone(),
        labels: Some(labels),
        tty: Some(true),
        entrypoint: tpl_config.entrypoint.clone(),
        cmd: tpl_config.cmd.clone(),
        stop_signal: tpl_config.stop_signal.clone(),
        volumes: Some(vec!["/var".to_string()]),
        host_config: Some(bollard::models::HostConfig {
            privileged: tpl_host.privileged,
            security_opt: tpl_host.security_opt.clone(),
            tmpfs: tpl_host.tmpfs.clone(),
            binds: Some(binds),
            cgroupns_mode: tpl_host.cgroupns_mode,
            devices: tpl_host.devices.clone(),
            init: tpl_host.init,
            network_mode: Some("kind".into()),
            restart_policy: Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::NO),
                maximum_retry_count: None,
            }),
            nano_cpus: tpl_host.nano_cpus,
            memory: tpl_host.memory,
            ..Default::default()
        }),
        ..Default::default()
    };
    let opts = bollard::query_parameters::CreateContainerOptionsBuilder::default().name(&name).build();
    let fail = |e: String| async {
        let _ = docker
            .remove_container(
                &name,
                Some(bollard::query_parameters::RemoveContainerOptionsBuilder::default().force(true).build()),
            )
            .await;
        super::kind::remove_node_state(cluster, &node_state);
        e
    };
    if let Err(e) = docker.create_container(Some(opts), body).await {
        return Err(fail(format!("create {name}: {e}")).await);
    }
    if let Err(e) = docker
        .start_container(&name, None::<bollard::query_parameters::StartContainerOptions>)
        .await
    {
        return Err(fail(format!("start {name}: {e}")).await);
    }

    // Containerd up (the entrypoint imports the bundled images first).
    progress(app, cluster, "Waiting for the node to boot…");
    let mut ready = false;
    for _ in 0..90 {
        let out = client
            .exec_capture(
                &name,
                vec!["sh".into(), "-c".into(), "test -S /run/containerd/containerd.sock && systemctl is-active containerd".into()],
            )
            .await
            .unwrap_or_default();
        if out.trim() == "active" {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    if !ready {
        return Err(fail(format!("{name} did not boot within 3 minutes")).await);
    }
    // Same containerd settings as its siblings (registry config_path etc.).
    let tpl_toml = client
        .exec_capture(&template, vec!["cat".into(), "/etc/containerd/config.toml".into()])
        .await
        .unwrap_or_default();
    if tpl_toml.contains("config_path") {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(tpl_toml.as_bytes());
        let script = format!(
            "echo '{b64}' | base64 -d > /etc/containerd/config.toml && systemctl restart containerd"
        );
        let _ = client.exec_capture(&name, vec!["sh".into(), "-c".into(), script]).await;
        if super::kind_registry::node_wired(&client, &template).await {
            let _ = client
                .exec_capture(&name, vec!["sh".into(), "-c".into(), super::kind_registry::node_hosts_script()])
                .await;
        }
    }

    progress(app, cluster, "Joining the cluster…");
    let join = match kubectl_join_command(&client, cluster).await {
        Ok(j) => j,
        Err(e) => return Err(fail(e).await),
    };
    // The trailing words only size the exec timeout (`exec_budget` reads
    // them); `sh -c` sees them as $0/$1 and ignores them.
    let out = client
        .exec_capture(
            &name,
            vec!["sh".into(), "-c".into(), join, "create".into(), "--timeout=240".into()],
        )
        .await;
    if let Err(e) = out {
        return Err(fail(format!("kubeadm join failed: {e:#}")).await);
    }
    let _ = super::kind::enforce_kind_restart_policy(state).await;
    progress(app, cluster, format!("Node {name} joined"));
    super::kind::invalidate_kind_clusters_cache().await;
    Ok(name)
}

async fn kubectl_join_command(
    client: &crate::docker::client::DockerClient,
    cluster: &str,
) -> Result<String, String> {
    let out = client
        .exec_capture(
            &control_plane_node(cluster),
            vec!["kubeadm".into(), "token".into(), "create".into(), "--ttl=15m".into(), "--print-join-command".into()],
        )
        .await
        .map_err(|e| format!("kubeadm token create: {e:#}"))?;
    join_script(&out)
}

#[tauri::command]
pub async fn remove_cluster_node(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    cluster: String,
    node: String,
) -> Result<(), String> {
    remove_cluster_node_core(&state, Some(&app), &cluster, &node).await
}

/// Drains and deletes a worker, then removes its container and state.
pub(crate) async fn remove_cluster_node_core(
    state: &AppState,
    app: Option<&AppHandle>,
    cluster: &str,
    node: &str,
) -> Result<(), String> {
    super::kind::check_cluster_name(cluster)?;
    let client = local_client(state).await?;
    let info = client
        .docker()
        .inspect_container(node, None)
        .await
        .map_err(|_| format!("node '{node}' does not exist"))?;
    let labels = info.config.as_ref().and_then(|c| c.labels.clone()).unwrap_or_default();
    if labels.get(CLUSTER_LABEL).map(String::as_str) != Some(cluster) {
        return Err(format!("'{node}' is not a node of cluster '{cluster}'"));
    }
    if labels.get(ROLE_LABEL).map(String::as_str) != Some("worker") {
        return Err("Only worker nodes can be removed; control planes hold etcd".into());
    }
    let state_dir = info
        .host_config
        .as_ref()
        .and_then(|h| h.binds.clone())
        .unwrap_or_default()
        .into_iter()
        .find_map(|b| {
            let mut p = b.split(':');
            let src = p.next()?.to_string();
            (p.next() == Some(CONTAINERD_PATH)).then(|| std::path::PathBuf::from(src))
        });
    progress(app, cluster, format!("Draining {node}…"));
    // Best effort: a stopped cluster cannot drain, and the node goes anyway.
    let _ = kubectl_raw(
        state,
        cluster,
        &["drain", node, "--ignore-daemonsets", "--delete-emptydir-data", "--force", "--timeout=90s"],
    )
    .await;
    let _ = kubectl_raw(state, cluster, &["delete", "node", node, "--ignore-not-found", "--timeout=30s"]).await;
    client
        .docker()
        .remove_container(
            node,
            Some(
                bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                    .force(true)
                    .v(true)
                    .build(),
            ),
        )
        .await
        .map_err(|e| format!("remove {node}: {e}"))?;
    if let Some(dir) = state_dir {
        super::kind::remove_node_state(cluster, &dir);
    }
    progress(app, cluster, format!("Node {node} removed"));
    super::kind::invalidate_kind_clusters_cache().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_names_follow_kind() {
        let mut have = HashSet::new();
        assert_eq!(next_worker_name("dev", &have), "dev-worker");
        have.insert("dev-worker".to_string());
        have.insert("dev-worker2".to_string());
        assert_eq!(next_worker_name("dev", &have), "dev-worker3");
        assert_eq!(next_state_index(&[0, 1, 3].into_iter().collect()), 2);
    }

    #[test]
    fn binds_swap_only_the_containerd_state() {
        let b = rewrite_binds(
            &["/lib/modules:/lib/modules:ro".into(), "/s/node0:/var/lib/containerd".into()],
            "/s/node2",
        );
        assert_eq!(b, ["/lib/modules:/lib/modules:ro", "/s/node2:/var/lib/containerd"]);
    }

    #[test]
    fn join_script_is_checked_and_skips_preflight() {
        let out = "W1125 warning\nkubeadm join dev-control-plane:6443 --token abc.def --discovery-token-ca-cert-hash sha256:00ff \n";
        let s = join_script(out).unwrap();
        assert!(s.starts_with("kubeadm join dev-control-plane:6443 --token abc.def"));
        assert!(s.ends_with("--skip-phases=preflight --v=2"));
        assert!(join_script("kubeadm join x; rm -rf /").is_err());
        assert!(join_script("error").is_err());
    }
}
