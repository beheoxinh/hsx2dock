//! Cluster snapshots, restore and clone.
//!
//! A kind node keeps its state in three places: the container rootfs (`/etc`,
//! `/kind`), an anonymous `/var` volume (etcd, kubelet) and the app's
//! containerd bind mount under `/var/lib/containerd`. A snapshot is
//! `docker commit` of every node (rootfs) plus one tar per node of `/var`,
//! containerd state included. Tars are made and unpacked by a short-lived
//! helper container (the node image has GNU tar) so files owned by the
//! engine's sub-uids keep their owners.
//!
//! Restore recreates the nodes from the snapshot images and refills `/var`.
//! Clone does the same under a new cluster name and API port. Node names
//! inside Kubernetes stay those of the source (each container's hostname is
//! kept), and the API endpoint is renamed to the clone's control-plane
//! container so workers never reach the source cluster. A changed node IP is
//! handled by kind's own entrypoint (it rewrites manifests and the API cert).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::kind::{check_cluster_name, control_plane_node, local_client};
use crate::docker::kind::CLUSTER_LABEL;
use crate::state::AppState;

const ROLE_LABEL: &str = "io.x-k8s.kind.role";
const CONTAINERD_PATH: &str = "/var/lib/containerd";
const SNAP_REPO: &str = "hsx2dock-snap";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotNode {
    pub name: String,
    pub role: String,
    pub image: String,
    pub config: bollard::models::ContainerConfig,
    pub host_config: bollard::models::HostConfig,
    /// Fixed address on the kind network (clones get one), re-applied on an
    /// in-place restore.
    #[serde(default)]
    pub ipv4: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMeta {
    pub id: String,
    pub cluster: String,
    pub created: String,
    #[serde(default)]
    pub note: Option<String>,
    pub node_image: String,
    pub nodes: Vec<SnapshotNode>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub id: String,
    pub cluster: String,
    pub created: String,
    pub note: Option<String>,
    pub node_image: String,
    pub nodes: Vec<String>,
    pub size_bytes: u64,
}

fn snapshots_root() -> PathBuf {
    crate::docker::engine::dtype_compat_root()
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(crate::migrate::APP_DIR)
        })
        .join("kind-snapshots")
}

pub(crate) fn check_snapshot_id(id: &str) -> Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-');
    if ok { Ok(()) } else { Err(format!("invalid snapshot id '{id}'")) }
}

fn snapshot_dir(cluster: &str, id: &str) -> Result<PathBuf, String> {
    check_cluster_name(cluster)?;
    check_snapshot_id(id)?;
    Ok(snapshots_root().join(cluster).join(id))
}

/// `<cluster>-worker2` → `worker2`.
pub(crate) fn node_suffix<'a>(cluster: &str, node: &'a str) -> Result<&'a str, String> {
    node.strip_prefix(cluster)
        .and_then(|s| s.strip_prefix('-'))
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .ok_or_else(|| format!("unexpected node name '{node}'"))
}

pub(crate) fn snapshot_image(cluster: &str, suffix: &str, id: &str) -> String {
    format!("{SNAP_REPO}/{cluster}.{suffix}:{id}")
}

/// Binds with the containerd state swapped for `state` (others kept).
fn with_state_bind(binds: &[String], state: &Path) -> Vec<String> {
    super::kind_nodes::rewrite_binds(binds, &state.display().to_string())
}

/// Port bindings for a clone: the API port gets `api_port`; other host
/// ports are kept only when nothing else uses them.
pub(crate) fn clone_port_bindings(
    bindings: &bollard::models::PortMap,
    api_port: u16,
    is_free: impl Fn(u16) -> bool,
) -> (bollard::models::PortMap, Vec<String>) {
    let mut out = HashMap::new();
    let mut dropped = Vec::new();
    for (cport, list) in bindings {
        let Some(list) = list else { continue };
        let mut kept = Vec::new();
        for b in list {
            let mut b = b.clone();
            if cport == "6443/tcp" {
                b.host_port = Some(api_port.to_string());
                kept.push(b);
                continue;
            }
            match b.host_port.as_deref().and_then(|p| p.parse::<u16>().ok()) {
                Some(p) if is_free(p) => kept.push(b),
                Some(p) => dropped.push(format!("host port {p} → {cport} is in use and was not mapped")),
                None => {}
            }
        }
        if !kept.is_empty() {
            out.insert(cport.clone(), Some(kept));
        }
    }
    (out, dropped)
}

/// Shell script run on a clone's control plane: point the API endpoint at
/// the clone's container name, add it to the cert SANs and re-issue the cert.
pub(crate) fn clone_cp_script(old: &str, new: &str) -> String {
    let (o, n) = (format!("{old}-control-plane:6443"), format!("{new}-control-plane:6443"));
    let n_host = format!("{new}-control-plane");
    format!(
        "set -e\n\
         sed -i 's#{o}#{n}#g' /kind/kubeadm.conf /etc/kubernetes/*.conf\n\
         grep -q '^  - {n_host}$' /kind/kubeadm.conf || sed -i 's#^  certSANs:$#  certSANs:\\n  - {n_host}#' /kind/kubeadm.conf\n\
         rm -f /etc/kubernetes/pki/apiserver.crt /etc/kubernetes/pki/apiserver.key\n\
         kubeadm init phase certs apiserver --config /kind/kubeadm.conf\n\
         crictl ps -q --name kube-apiserver | xargs -r crictl stop >/dev/null 2>&1 || true\n\
         systemctl restart kubelet\n"
    )
}

pub(crate) fn clone_cm_script(old: &str, new: &str) -> String {
    let (o, n) = (format!("{old}-control-plane:6443"), format!("{new}-control-plane:6443"));
    // Configmaps first, then every DaemonSet/Deployment still naming the old
    // endpoint (kindnet carries it in its env). Worker nodes resolve the old
    // name through Docker DNS to the *source* cluster, so nothing may keep it.
    format!(
        r#"K='kubectl --kubeconfig=/etc/kubernetes/admin.conf'
for x in kube-system/kube-proxy kube-public/cluster-info kube-system/kubeadm-config; do
  ns=${{x%%/*}}; cm=${{x#*/}}
  $K -n $ns get cm $cm -o yaml 2>/dev/null | sed 's#{o}#{n}#g' | $K replace -f - >/dev/null || true
done
$K get ds,deploy -A -o jsonpath='{{range .items[*]}}{{.kind}}/{{.metadata.namespace}}/{{.metadata.name}}{{"\n"}}{{end}}' 2>/dev/null |
while IFS=/ read -r kind ns name; do
  y=$($K -n "$ns" get "$kind" "$name" -o yaml 2>/dev/null) || continue
  case "$y" in *'{o}'*) printf '%s\n' "$y" | sed 's#{o}#{n}#g' | $K replace -f - >/dev/null || true;; esac
done
$K -n kube-system rollout restart ds kube-proxy >/dev/null 2>&1 || true
"#
    )
}

pub(crate) fn clone_worker_script(old: &str, new: &str) -> String {
    let (o, n) = (format!("{old}-control-plane:6443"), format!("{new}-control-plane:6443"));
    format!(
        "sed -i 's#{o}#{n}#g' /etc/kubernetes/kubelet.conf /kind/kubeadm.conf 2>/dev/null; systemctl restart kubelet\n"
    )
}

fn progress(app: Option<&AppHandle>, cluster: &str, line: &str, done: bool, error: Option<&str>) {
    if let Some(app) = app {
        let _ = app.emit(
            "snapshot-progress",
            serde_json::json!({ "cluster": cluster, "line": line, "done": done, "error": error }),
        );
    }
}

struct Node {
    id: String,
    name: String,
    role: String,
    running: bool,
}

async fn cluster_nodes(docker: &bollard::Docker, cluster: &str) -> Result<Vec<Node>, String> {
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![format!("{CLUSTER_LABEL}={cluster}")]);
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    let list = docker.list_containers(Some(opts)).await.map_err(|e| e.to_string())?;
    let mut out: Vec<Node> = list
        .into_iter()
        .filter_map(|c| {
            Some(Node {
                id: c.id?,
                name: c.names?.first()?.trim_start_matches('/').to_string(),
                role: c.labels.and_then(|l| l.get(ROLE_LABEL).cloned()).unwrap_or_else(|| "worker".into()),
                running: c.state.map(|s| s.to_string() == "running").unwrap_or(false),
            })
        })
        .collect();
    out.sort_by_key(|n| (n.role != "control-plane", n.name.clone()));
    Ok(out)
}

/// Runs `script` in a throwaway container from `image` that shares the
/// volumes of `volumes_from` and sees `snap_dir` at `/snap`.
async fn run_helper(
    docker: &bollard::Docker,
    image: &str,
    volumes_from: &str,
    snap_dir: &Path,
    script: String,
) -> Result<(), String> {
    let name = format!("hsx2dock-snaphelper-{}", uuid::Uuid::new_v4().simple());
    let body = bollard::models::ContainerCreateBody {
        image: Some(image.to_string()),
        entrypoint: Some(vec!["sh".into(), "-c".into()]),
        cmd: Some(vec![script]),
        labels: Some(HashMap::from([("hsx2dock.helper".to_string(), "snapshot".to_string())])),
        host_config: Some(bollard::models::HostConfig {
            volumes_from: Some(vec![volumes_from.to_string()]),
            binds: Some(vec![format!("{}:/snap", snap_dir.display())]),
            network_mode: Some("none".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let opts = bollard::query_parameters::CreateContainerOptionsBuilder::default().name(&name).build();
    docker.create_container(Some(opts), body).await.map_err(|e| format!("helper: {e}"))?;
    let remove = || async {
        let _ = docker
            .remove_container(
                &name,
                Some(bollard::query_parameters::RemoveContainerOptionsBuilder::default().force(true).v(true).build()),
            )
            .await;
    };
    if let Err(e) = docker.start_container(&name, None::<bollard::query_parameters::StartContainerOptions>).await {
        remove().await;
        return Err(format!("helper: {e}"));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3600);
    let code = loop {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        let info = match docker.inspect_container(&name, None).await {
            Ok(i) => i,
            Err(e) => {
                remove().await;
                return Err(e.to_string());
            }
        };
        let st = info.state.unwrap_or_default();
        if !st.running.unwrap_or(false) {
            break st.exit_code.unwrap_or(-1);
        }
        if std::time::Instant::now() > deadline {
            remove().await;
            return Err("helper timed out".into());
        }
    };
    remove().await;
    if code != 0 {
        let err = std::fs::read_to_string(snap_dir.join(".helper.err")).unwrap_or_default();
        return Err(format!("helper exited with {code}: {}", err.trim().lines().rev().take(5).collect::<Vec<_>>().join(" | ")));
    }
    let _ = std::fs::remove_file(snap_dir.join(".helper.err"));
    Ok(())
}

fn dir_size(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

// ------------------------------------------------------------- commands

#[tauri::command]
pub fn list_cluster_snapshots(cluster: Option<String>) -> Result<Vec<SnapshotInfo>, String> {
    list_cluster_snapshots_core(cluster.as_deref())
}

pub(crate) fn list_cluster_snapshots_core(cluster: Option<&str>) -> Result<Vec<SnapshotInfo>, String> {
    let root = snapshots_root();
    let clusters: Vec<String> = match cluster {
        Some(c) => {
            check_cluster_name(c)?;
            vec![c.to_string()]
        }
        None => std::fs::read_dir(&root)
            .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default(),
    };
    let mut out = Vec::new();
    for c in clusters {
        let Ok(rd) = std::fs::read_dir(root.join(&c)) else { continue };
        for e in rd.flatten() {
            let Ok(text) = std::fs::read_to_string(e.path().join("meta.json")) else { continue };
            let Ok(meta) = serde_json::from_str::<SnapshotMeta>(&text) else { continue };
            out.push(SnapshotInfo {
                size_bytes: dir_size(&e.path()),
                id: meta.id,
                cluster: meta.cluster,
                created: meta.created,
                note: meta.note,
                node_image: meta.node_image,
                nodes: meta.nodes.into_iter().map(|n| n.name).collect(),
            });
        }
    }
    out.sort_by(|a, b| b.created.cmp(&a.created));
    Ok(out)
}

fn load_meta(cluster: &str, id: &str) -> Result<(PathBuf, SnapshotMeta), String> {
    let dir = snapshot_dir(cluster, id)?;
    let text = std::fs::read_to_string(dir.join("meta.json")).map_err(|_| format!("snapshot {cluster}/{id} not found"))?;
    let meta: SnapshotMeta = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok((dir, meta))
}

/// Serialises snapshot work per cluster.
fn busy() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static B: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    B.get_or_init(Default::default)
}
struct Busy(String);
impl Busy {
    fn claim(key: &str) -> Result<Self, String> {
        let mut b = busy().lock().unwrap_or_else(|e| e.into_inner());
        if !b.insert(key.to_string()) {
            return Err(format!("a snapshot operation on {key} is already running"));
        }
        Ok(Self(key.to_string()))
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        busy().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

#[tauri::command]
pub async fn create_cluster_snapshot(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    cluster: String,
    note: Option<String>,
) -> Result<String, String> {
    let r = create_cluster_snapshot_core(&state, Some(&app), &cluster, note).await;
    match &r {
        Ok(id) => progress(Some(&app), &cluster, &format!("Snapshot {id} saved"), true, None),
        Err(e) => progress(Some(&app), &cluster, e, true, Some(e)),
    }
    r
}

/// Stops the cluster if needed, snapshots every node, starts it again.
pub(crate) async fn create_cluster_snapshot_core(
    state: &AppState,
    app: Option<&AppHandle>,
    cluster: &str,
    note: Option<String>,
) -> Result<String, String> {
    check_cluster_name(cluster)?;
    let _busy = Busy::claim(cluster)?;
    let client = local_client(state).await?;
    let docker = client.docker();
    let nodes = cluster_nodes(docker, cluster).await?;
    if nodes.is_empty() {
        return Err(format!("cluster '{cluster}' does not exist"));
    }
    let was_running = nodes.iter().any(|n| n.running);
    if was_running {
        progress(app, cluster, "Stopping the cluster for a consistent snapshot…", false, None);
        super::kind::set_cluster_power(app, state, cluster, false).await?;
    }
    let id = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let dir = snapshot_dir(cluster, &id)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = async {
        let mut metas = Vec::new();
        let mut node_image = String::new();
        for n in &nodes {
            let info = docker.inspect_container(&n.id, None).await.map_err(|e| e.to_string())?;
            let config = info.config.clone().unwrap_or_default();
            let host_config = info.host_config.clone().unwrap_or_default();
            let ipv4 = info
                .network_settings
                .as_ref()
                .and_then(|n| n.networks.as_ref())
                .and_then(|n| n.get("kind"))
                .and_then(|e| e.ipam_config.as_ref())
                .and_then(|c| c.ipv4_address.clone())
                .filter(|a| !a.is_empty());
            if node_image.is_empty() {
                node_image = config.image.clone().unwrap_or_default();
            }
            let suffix = node_suffix(cluster, &n.name)?;
            let image = snapshot_image(cluster, suffix, &id);
            progress(app, cluster, &format!("Committing {}…", n.name), false, None);
            let opts = bollard::query_parameters::CommitContainerOptionsBuilder::default()
                .container(&n.id)
                .repo(&format!("{SNAP_REPO}/{cluster}.{suffix}"))
                .tag(&id)
                .comment("hsx2dock cluster snapshot")
                .pause(false)
                .build();
            docker
                .commit_container(opts, bollard::models::ContainerConfig::default())
                .await
                .map_err(|e| format!("commit {}: {e}", n.name))?;
            progress(app, cluster, &format!("Archiving /var of {}…", n.name), false, None);
            run_helper(
                docker,
                &image,
                &n.id,
                &dir,
                format!("tar -C /var --numeric-owner -cpf /snap/{suffix}.tar . 2>/snap/.helper.err"),
            )
            .await?;
            metas.push(SnapshotNode { name: n.name.clone(), role: n.role.clone(), image, config, host_config, ipv4 });
        }
        let meta = SnapshotMeta {
            id: id.clone(),
            cluster: cluster.to_string(),
            created: chrono::Local::now().to_rfc3339(),
            note: note.filter(|s| !s.trim().is_empty()),
            node_image,
            nodes: metas,
        };
        std::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&meta).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    }
    .await;
    if let Err(e) = &result {
        let _ = delete_snapshot_files(state, cluster, &id).await;
        if was_running {
            let _ = super::kind::set_cluster_power(app, state, cluster, true).await;
        }
        return Err(e.clone());
    }
    if was_running {
        progress(app, cluster, "Starting the cluster again…", false, None);
        super::kind::set_cluster_power(app, state, cluster, true).await?;
    }
    Ok(id)
}

#[tauri::command]
pub async fn delete_cluster_snapshot(state: State<'_, Arc<AppState>>, cluster: String, id: String) -> Result<(), String> {
    delete_cluster_snapshot_core(&state, &cluster, &id).await
}

pub(crate) async fn delete_cluster_snapshot_core(state: &AppState, cluster: &str, id: &str) -> Result<(), String> {
    let _busy = Busy::claim(cluster)?;
    load_meta(cluster, id)?;
    delete_snapshot_files(state, cluster, id).await
}

async fn delete_snapshot_files(state: &AppState, cluster: &str, id: &str) -> Result<(), String> {
    let dir = snapshot_dir(cluster, id)?;
    let client = local_client(state).await?;
    let images: Vec<String> = load_meta(cluster, id)
        .map(|(_, m)| m.nodes.into_iter().map(|n| n.image).collect())
        .unwrap_or_else(|_| {
            // Partial snapshot without meta: images follow the naming scheme.
            Vec::new()
        });
    for img in images {
        if let Err(e) = client
            .docker()
            // Restored and cloned nodes run from these images; forcing only
            // drops the tag, the layers stay until those nodes are gone.
            .remove_image(
                &img,
                Some(bollard::query_parameters::RemoveImageOptionsBuilder::default().force(true).build()),
                None,
            )
            .await
        {
            let msg = e.to_string();
            if !msg.contains("No such image") && !msg.contains("404") {
                return Err(format!("remove {img}: {msg}"));
            }
        }
    }
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRequest {
    pub cluster: String,
    pub id: String,
    /// Clone into a new cluster with this name instead of restoring in place.
    #[serde(default)]
    pub new_name: Option<String>,
}

#[tauri::command]
pub async fn restore_cluster_snapshot(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    req: RestoreRequest,
) -> Result<String, String> {
    let target = req.new_name.clone().unwrap_or_else(|| req.cluster.clone());
    let r = restore_cluster_snapshot_core(&state, Some(&app), req).await;
    match &r {
        Ok(m) => progress(Some(&app), &target, m, true, None),
        Err(e) => progress(Some(&app), &target, e, true, Some(e)),
    }
    r
}

/// Restores a snapshot over its cluster (replacing the current nodes), or
/// clones it into `new_name`. Returns a summary line.
pub(crate) async fn restore_cluster_snapshot_core(
    state: &AppState,
    app: Option<&AppHandle>,
    req: RestoreRequest,
) -> Result<String, String> {
    let (dir, meta) = load_meta(&req.cluster, &req.id)?;
    let target = req.new_name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| req.cluster.clone());
    check_cluster_name(&target)?;
    let cloning = target != meta.cluster;
    let _busy = Busy::claim(&target)?;
    let client = local_client(state).await?;
    let docker = client.docker();
    let existing = cluster_nodes(docker, &target).await?;
    if cloning {
        if !existing.is_empty() {
            return Err(format!("cluster '{target}' already exists"));
        }
        let cps = meta.nodes.iter().filter(|n| n.role == "control-plane").count();
        if cps != 1 {
            return Err("Only single control-plane clusters can be cloned (restore an HA snapshot in place instead)".into());
        }
    } else if existing.iter().any(|n| n.running) {
        progress(app, &target, "Stopping the cluster…", false, None);
        super::kind::set_cluster_power(app, state, &target, false).await?;
    }
    for n in &meta.nodes {
        let tar = dir.join(format!("{}.tar", node_suffix(&meta.cluster, &n.name)?));
        if !tar.is_file() {
            return Err(format!("snapshot is incomplete: {} is missing", tar.display()));
        }
    }
    if !cloning {
        super::kind_portforward::stop_cluster_forwards(&target);
        for n in &existing {
            progress(app, &target, &format!("Removing current node {}…", n.name), false, None);
            docker
                .remove_container(
                    &n.id,
                    Some(bollard::query_parameters::RemoveContainerOptionsBuilder::default().force(true).v(true).build()),
                )
                .await
                .map_err(|e| format!("remove {}: {e}", n.name))?;
        }
    }

    let state_root = crate::docker::engine::dtype_compat_root()
        .ok_or("no d_type-capable directory for node state")?
        .join("kind-nodes")
        .join(&target);
    let api_port = if cloning {
        let taken = super::kind_access::kind_published_ports(state).await;
        Some(super::kind_access::pick_free_api_port(&state.kind, &taken).ok_or("no free API server port")?)
    } else {
        None
    };
    let published = if cloning { super::kind_access::kind_published_ports(state).await } else { Default::default() };
    let mut notes = Vec::new();
    let mut created: Vec<String> = Vec::new();
    // Docker's network DNS answers for every container's hostname, so a clone
    // that kept its source's hostnames would make `<src>-control-plane`
    // resolve to both clusters. Clones get their own hostnames and fixed
    // addresses instead; /etc/hosts maps the old node names (which stay the
    // Kubernetes node names) to the clone's own nodes.
    let clone_ips: Vec<String> = if cloning {
        let subnet = super::kind_addons::kind_subnet(state).await?;
        let used = kind_network_addresses(docker).await;
        pick_static_ips(&subnet, &used, meta.nodes.len())
            .ok_or("no free addresses left on the kind network for the clone")?
            .into_iter()
            .map(|ip| ip.to_string())
            .collect()
    } else {
        Vec::new()
    };
    let clone_hosts: Vec<String> = if cloning { clone_extra_hosts(&meta.nodes, &clone_ips) } else { Vec::new() };

    let build = async {
        for (i, n) in meta.nodes.iter().enumerate() {
            let suffix = node_suffix(&meta.cluster, &n.name)?;
            let name = format!("{target}-{suffix}");
            // Clones get fresh state dirs; a restore reuses the node's own.
            let node_state = if cloning {
                state_root.join(format!("node{i}"))
            } else {
                n.host_config
                    .binds
                    .iter()
                    .flatten()
                    .find_map(|b| {
                        let mut p = b.split(':');
                        let src = p.next()?;
                        (p.next() == Some(CONTAINERD_PATH)).then(|| PathBuf::from(src))
                    })
                    .unwrap_or_else(|| state_root.join(format!("node{i}")))
            };
            std::fs::create_dir_all(&node_state).map_err(|e| format!("{}: {e}", node_state.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&node_state, std::fs::Permissions::from_mode(0o777));
            }
            let mut labels = n.config.labels.clone().unwrap_or_default();
            labels.insert(CLUSTER_LABEL.into(), target.clone());
            let mut host = n.host_config.clone();
            host.binds = Some(with_state_bind(host.binds.as_deref().unwrap_or_default(), &node_state));
            host.network_mode = Some("kind".into());
            host.restart_policy = Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::NO),
                maximum_retry_count: None,
            });
            if let (Some(api), Some(pb)) = (api_port, host.port_bindings.as_ref()) {
                let (pb, dropped) = clone_port_bindings(pb, api, |p| {
                    !published.contains(&p) && super::kind_access::host_port_is_free(p)
                });
                host.port_bindings = Some(pb);
                notes.extend(dropped);
            }
            let exposed = host.port_bindings.as_ref().map(|pb| pb.keys().cloned().collect());
            let ipv4 = if cloning { clone_ips.get(i).cloned() } else { n.ipv4.clone() };
            if cloning {
                host.extra_hosts = Some(clone_hosts.clone());
            }
            let networking = ipv4.as_ref().map(|ip| bollard::models::NetworkingConfig {
                endpoints_config: Some(HashMap::from([(
                    "kind".to_string(),
                    bollard::models::EndpointSettings {
                        ipam_config: Some(bollard::models::EndpointIpamConfig {
                            ipv4_address: Some(ip.clone()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )])),
            });
            let body = bollard::models::ContainerCreateBody {
                image: Some(n.image.clone()),
                // A restore keeps the node's hostname; a clone takes its
                // container name (see `clone_ips`).
                hostname: if cloning { Some(name.clone()) } else { n.config.hostname.clone() },
                networking_config: networking,
                env: n.config.env.clone(),
                labels: Some(labels),
                tty: Some(true),
                entrypoint: n.config.entrypoint.clone(),
                cmd: n.config.cmd.clone(),
                stop_signal: n.config.stop_signal.clone(),
                exposed_ports: exposed,
                volumes: Some(vec!["/var".to_string()]),
                host_config: Some(host),
                ..Default::default()
            };
            progress(app, &target, &format!("Creating {name}…"), false, None);
            let opts = bollard::query_parameters::CreateContainerOptionsBuilder::default().name(&name).build();
            docker.create_container(Some(opts), body).await.map_err(|e| format!("create {name}: {e}"))?;
            created.push(name.clone());
            progress(app, &target, &format!("Restoring /var of {name}…"), false, None);
            run_helper(
                docker,
                &n.image,
                &name,
                &dir,
                restore_var_script(suffix, cloning.then(|| node_k8s_name(n))),
            )
            .await?;
        }
        Ok::<(), String>(())
    }
    .await;
    if let Err(e) = build {
        for c in &created {
            let _ = docker
                .remove_container(
                    c,
                    Some(bollard::query_parameters::RemoveContainerOptionsBuilder::default().force(true).v(true).build()),
                )
                .await;
        }
        if cloning {
            super::kind::cleanup_cluster_state(&target, &[]);
        }
        return Err(e);
    }

    super::kind::invalidate_kind_clusters_cache().await;
    progress(app, &target, "Starting nodes…", false, None);
    super::kind::set_cluster_power(app, state, &target, true).await?;

    if cloning {
        let cp = control_plane_node(&target);
        wait_containerd(&client, &cp).await?;
        progress(app, &target, "Re-issuing the API server certificate for the clone…", false, None);
        client
            .exec_capture(
                &cp,
                vec!["bash".into(), "-c".into(), clone_cp_script(&meta.cluster, &target), "create".into(), "--timeout=150".into()],
            )
            .await
            .map_err(|e| format!("clone fix-up: {e:#}"))?;
        progress(app, &target, "Waiting for the API server…", false, None);
        wait_api(&client, &cp).await?;
        let _ = client
            .exec_capture(&cp, vec!["bash".into(), "-c".into(), clone_cm_script(&meta.cluster, &target)])
            .await;
        for c in created.iter().filter(|c| **c != cp) {
            if wait_containerd(&client, c).await.is_ok() {
                let _ = client
                    .exec_capture(c, vec!["bash".into(), "-c".into(), clone_worker_script(&meta.cluster, &target)])
                    .await;
            }
        }
    }
    // A clone inherits its source's MetalLB pool, and an older snapshot may
    // carry a block another cluster took since: both would announce the same
    // addresses on the shared kind network.
    let cp = control_plane_node(&target);
    if wait_api(&client, &cp).await.is_ok() {
        let has_pool = super::kind::kubectl_json(
            state,
            &target,
            &["get", "ipaddresspools.metallb.io", "-n", "metallb-system", "-o", "json"],
        )
        .await
        .map(|v| v["items"].as_array().is_some_and(|a| !a.is_empty()))
        .unwrap_or(false);
        if has_pool {
            progress(app, &target, "Giving MetalLB its own address block…", false, None);
            if let Err(e) = super::kind_addons::assign_metallb_pool(state, &target).await {
                tracing::warn!("restore {target}: MetalLB pool: {e}");
            }
        }
    }
    let _ = super::kind::enforce_kind_restart_policy(state).await;
    super::kind::invalidate_kind_clusters_cache().await;
    let mut msg = if cloning {
        format!(
            "Cloned {}/{} into {target} (API port {})",
            meta.cluster,
            meta.id,
            api_port.map(|p| p.to_string()).unwrap_or_default()
        )
    } else {
        format!("Restored {target} to snapshot {}", meta.id)
    };
    if !notes.is_empty() {
        msg.push_str(&format!(" — {}", notes.join("; ")));
    }
    Ok(msg)
}

async fn wait_containerd(client: &crate::docker::client::DockerClient, node: &str) -> Result<(), String> {
    for _ in 0..90 {
        let out = client
            .exec_capture(node, vec!["sh".into(), "-c".into(), "systemctl is-active containerd kubelet | head -1".into()])
            .await
            .unwrap_or_default();
        if out.trim() == "active" {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Err(format!("{node} did not boot within 3 minutes"))
}

async fn wait_api(client: &crate::docker::client::DockerClient, cp: &str) -> Result<(), String> {
    for _ in 0..90 {
        let out = client
            .exec_capture(
                cp,
                vec![
                    "sh".into(),
                    "-c".into(),
                    "kubectl --kubeconfig=/etc/kubernetes/admin.conf get --raw /readyz 2>/dev/null || true".into(),
                ],
            )
            .await
            .unwrap_or_default();
        if out.trim() == "ok" {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Err("the API server did not become ready within 3 minutes".into())
}

/// Kubernetes node name of a snapshot node: its hostname (a clone's
/// kubelet carries the original via `--hostname-override`, read back here
/// from the saved extra hosts).
fn node_k8s_name(n: &SnapshotNode) -> String {
    let own = n.config.hostname.clone().unwrap_or_else(|| n.name.clone());
    match &n.ipv4 {
        // A node of a clone: its original name maps to its own address.
        Some(ip) => n
            .host_config
            .extra_hosts
            .iter()
            .flatten()
            .find_map(|h| h.split_once(':').filter(|(_, a)| a == ip).map(|(name, _)| name.to_string()))
            .unwrap_or(own),
        None => own,
    }
}

/// `/etc/hosts` entries for a clone: every name a node answered to (its
/// hostname and the Kubernetes node name) → the clone node's new address.
pub(crate) fn clone_extra_hosts(nodes: &[SnapshotNode], ips: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (n, ip) in nodes.iter().zip(ips) {
        let mut names = vec![n.config.hostname.clone().unwrap_or_else(|| n.name.clone()), node_k8s_name(n)];
        names.dedup();
        for name in names {
            let e = format!("{name}:{ip}");
            if !out.contains(&e) {
                out.push(e);
            }
        }
    }
    out
}

/// Helper script that replaces a node's /var with the archived one. For a
/// clone it pins the kubelet to the original node name, since the container
/// hostname changes.
pub(crate) fn restore_var_script(suffix: &str, keep_node_name: Option<String>) -> String {
    let mut s = format!(
        "find /var -mindepth 1 -delete 2>/dev/null; tar -C /var --numeric-owner -xpf /snap/{suffix}.tar 2>/snap/.helper.err"
    );
    if let Some(node) = keep_node_name {
        s.push_str(&format!(
            " && f=/var/lib/kubelet/kubeadm-flags.env && {{ grep -q -- --hostname-override $f || \
             sed -i 's#^KUBELET_KUBEADM_ARGS=\"#KUBELET_KUBEADM_ARGS=\"--hostname-override={node} #' $f; }} 2>>/snap/.helper.err"
        ));
    }
    s
}

/// Every IPv4 address a container on the kind network holds or has pinned,
/// running or not (a stopped clone keeps its fixed address).
async fn kind_network_addresses(docker: &bollard::Docker) -> std::collections::HashSet<std::net::Ipv4Addr> {
    let mut used = std::collections::HashSet::new();
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default().all(true).build();
    for c in docker.list_containers(Some(opts)).await.unwrap_or_default() {
        let Some(ep) = c.network_settings.and_then(|n| n.networks).and_then(|mut n| n.remove("kind")) else {
            continue;
        };
        let fixed = ep.ipam_config.and_then(|c| c.ipv4_address);
        for a in [ep.ip_address, fixed].into_iter().flatten() {
            if let Ok(ip) = a.split('/').next().unwrap_or("").parse() {
                used.insert(ip);
            }
        }
    }
    used
}

/// `n` free addresses for clone nodes, from the top of the kind subnet
/// downwards, where Docker's sequential allocator rarely reaches. The last
/// /24 is left to MetalLB (`kind_addons::metallb_range`).
pub(crate) fn pick_static_ips(
    subnet: &str,
    used: &std::collections::HashSet<std::net::Ipv4Addr>,
    n: usize,
) -> Option<Vec<std::net::Ipv4Addr>> {
    let (ip, len) = subnet.split_once('/')?;
    let ip: std::net::Ipv4Addr = ip.parse().ok()?;
    let len: u32 = len.parse().ok()?;
    if !(8..=24).contains(&len) {
        return None;
    }
    let base = u32::from(ip) & (u32::MAX << (32 - len));
    let last24 = (base | !(u32::MAX << (32 - len))) & !0xFF;
    // Below the MetalLB area: the /24 before it, or `.2–.15` of a /24.
    let (hi, lo) = if len <= 23 { (last24 - 2, last24 - 255) } else { (last24 | 15, last24 | 2) };
    let out: Vec<std::net::Ipv4Addr> = (lo..=hi)
        .rev()
        .map(std::net::Ipv4Addr::from)
        .filter(|a| !used.contains(a) && a.octets()[3] != 0 && a.octets()[3] != 255)
        .take(n)
        .collect();
    (out.len() == n).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_images() {
        assert_eq!(node_suffix("dev", "dev-control-plane").unwrap(), "control-plane");
        assert_eq!(node_suffix("dev", "dev-worker2").unwrap(), "worker2");
        assert!(node_suffix("dev", "other-worker").is_err());
        assert!(node_suffix("dev", "dev-").is_err());
        assert_eq!(snapshot_image("dev", "worker", "20261125-101500"), "hsx2dock-snap/dev.worker:20261125-101500");
        assert!(check_snapshot_id("20261125-101500").is_ok());
        assert!(check_snapshot_id("../x").is_err());
    }

    #[test]
    fn clone_ports_remap_api_and_drop_busy() {
        let pb: bollard::models::PortMap = HashMap::from([
            (
                "6443/tcp".to_string(),
                Some(vec![bollard::models::PortBinding { host_ip: Some("127.0.0.1".into()), host_port: Some("16443".into()) }]),
            ),
            (
                "80/tcp".to_string(),
                Some(vec![bollard::models::PortBinding { host_ip: None, host_port: Some("18080".into()) }]),
            ),
            (
                "443/tcp".to_string(),
                Some(vec![bollard::models::PortBinding { host_ip: None, host_port: Some("18443".into()) }]),
            ),
        ]);
        let (out, dropped) = clone_port_bindings(&pb, 16444, |p| p == 18443);
        assert_eq!(out["6443/tcp"].as_ref().unwrap()[0].host_port.as_deref(), Some("16444"));
        assert_eq!(out["6443/tcp"].as_ref().unwrap()[0].host_ip.as_deref(), Some("127.0.0.1"));
        assert!(!out.contains_key("80/tcp"));
        assert!(out.contains_key("443/tcp"));
        assert_eq!(dropped.len(), 1);
    }

    #[test]
    fn clone_scripts_rename_only_the_endpoint() {
        let s = clone_cp_script("dev", "dev2");
        assert!(s.contains("s#dev-control-plane:6443#dev2-control-plane:6443#g"));
        assert!(s.contains("  - dev2-control-plane"));
        assert!(s.contains("kubeadm init phase certs apiserver"));
        assert!(clone_worker_script("dev", "dev2").contains("/etc/kubernetes/kubelet.conf"));
        let cm = clone_cm_script("dev", "dev2");
        assert!(cm.contains("kube-public/cluster-info"));
        // kindnet carries the endpoint in its env; worker nodes resolve the
        // old name to the source cluster, so every workload must be swept.
        assert!(cm.contains("get ds,deploy -A"));
        assert!(cm.contains("{range .items[*]}{.kind}/"));
        assert!(cm.contains("*'dev-control-plane:6443'*"));
    }
    #[test]
    fn clone_addresses_sit_below_the_metallb_block() {
        let used: std::collections::HashSet<std::net::Ipv4Addr> = ["172.18.254.254".parse().unwrap()].into_iter().collect();
        let ips = pick_static_ips("172.18.0.0/16", &used, 2).unwrap();
        assert_eq!(ips, vec!["172.18.254.253".parse::<std::net::Ipv4Addr>().unwrap(), "172.18.254.252".parse().unwrap()]);
        let small = pick_static_ips("10.89.3.0/24", &Default::default(), 1).unwrap();
        assert_eq!(small, vec!["10.89.3.15".parse::<std::net::Ipv4Addr>().unwrap()]);
        assert!(pick_static_ips("10.0.0.0/28", &Default::default(), 1).is_none());
        assert!(pick_static_ips("10.89.3.0/24", &Default::default(), 20).is_none());
    }

    fn snap_node(name: &str, ipv4: Option<&str>, hosts: &[&str]) -> SnapshotNode {
        SnapshotNode {
            name: name.into(),
            role: "worker".into(),
            image: "img".into(),
            config: bollard::models::ContainerConfig { hostname: Some(name.into()), ..Default::default() },
            host_config: bollard::models::HostConfig {
                extra_hosts: (!hosts.is_empty()).then(|| hosts.iter().map(|h| h.to_string()).collect()),
                ..Default::default()
            },
            ipv4: ipv4.map(str::to_string),
        }
    }

    #[test]
    fn clone_hosts_map_old_node_names_to_new_addresses() {
        let nodes = [snap_node("dev-control-plane", None, &[]), snap_node("dev-worker", None, &[])];
        let ips = ["172.18.254.254".to_string(), "172.18.254.253".to_string()];
        assert_eq!(
            clone_extra_hosts(&nodes, &ips),
            vec!["dev-control-plane:172.18.254.254", "dev-worker:172.18.254.253"]
        );
        // Cloning a clone: the node is `dev2-worker` but Kubernetes knows it
        // as `dev-worker`; both names follow it.
        let n = snap_node("dev2-worker", Some("172.18.254.253"), &["dev-control-plane:172.18.254.254", "dev-worker:172.18.254.253"]);
        assert_eq!(node_k8s_name(&n), "dev-worker");
        assert_eq!(clone_extra_hosts(&[n], &["172.18.254.250".into()]), vec!["dev2-worker:172.18.254.250", "dev-worker:172.18.254.250"]);
        let s = restore_var_script("worker", Some("dev-worker".into()));
        assert!(s.contains("--hostname-override=dev-worker "));
        assert!(!restore_var_script("worker", None).contains("hostname-override"));
    }

}
