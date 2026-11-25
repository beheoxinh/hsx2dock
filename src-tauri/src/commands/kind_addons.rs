//! Addon catalog for kind clusters (ingress-nginx, MetalLB, cert-manager,
//! Kubernetes Dashboard, metrics-server) and ingress exposure through the
//! app's Caddy gateway.
//!
//! Manifests are pinned by URL *and* sha256: the app downloads them, checks
//! the digest, copies them into the control-plane node and applies them with
//! server-side apply (cert-manager's CRDs exceed client-side annotation size).

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use super::kind::{control_plane_node, kubectl_json, kubectl_raw, local_client};
use crate::state::AppState;

pub(crate) struct Addon {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub namespace: &'static str,
    pub deployments: &'static [&'static str],
}

pub(crate) const ADDONS: &[Addon] = &[
    Addon {
        id: "ingress-nginx",
        name: "Ingress NGINX",
        description: "Ingress controller (kind build): serves Ingress objects on the node labelled ingress-ready, reachable from the host through the cluster's ingress ports.",
        version: "v1.15.1",
        url: "https://raw.githubusercontent.com/kubernetes/ingress-nginx/controller-v1.15.1/deploy/static/provider/kind/deploy.yaml",
        sha256: "2a3ae008c8786431115502644e77ab398fdebfb721a5d1195ed3089cde3299df",
        namespace: "ingress-nginx",
        deployments: &["ingress-nginx-controller"],
    },
    Addon {
        id: "metallb",
        name: "MetalLB",
        description: "LoadBalancer Services get an IP from the kind network (pool derived from its subnet), reachable from other clusters.",
        version: "v0.16.2",
        url: "https://raw.githubusercontent.com/metallb/metallb/v0.16.2/config/manifests/metallb-native.yaml",
        sha256: "e5f7c268f114c446dc60aaad71b7cabf861f637743e033e354d7d48592807c6d",
        namespace: "metallb-system",
        deployments: &["controller"],
    },
    Addon {
        id: "cert-manager",
        name: "cert-manager",
        description: "Certificates for Ingress TLS; installs a self-signed ClusterIssuer named selfsigned.",
        version: "v1.21.3",
        url: "https://github.com/cert-manager/cert-manager/releases/download/v1.21.3/cert-manager.yaml",
        sha256: "c04a51295adda2b1a775146751f973e787190778c7f35180861d9678876e5c74",
        namespace: "cert-manager",
        deployments: &["cert-manager", "cert-manager-cainjector", "cert-manager-webhook"],
    },
    Addon {
        id: "dashboard",
        name: "Kubernetes Dashboard",
        description: "Web UI with an admin-user ServiceAccount; open it through a port-forward and sign in with a token.",
        version: "v2.7.0",
        url: "https://raw.githubusercontent.com/kubernetes/dashboard/v2.7.0/aio/deploy/recommended.yaml",
        sha256: "943ae40251f1ba64ef012c53271bf6766d1883dd024093e854567b56443764b8",
        namespace: "kubernetes-dashboard",
        deployments: &["kubernetes-dashboard"],
    },
    Addon {
        id: "metrics-server",
        name: "metrics-server",
        description: "Resource metrics for kubectl top and per-pod CPU/memory (kubelet TLS check relaxed for kind).",
        version: "v0.9.1",
        url: "https://github.com/kubernetes-sigs/metrics-server/releases/download/v0.9.1/components.yaml",
        sha256: "3c7f57cdbee0f57fb0259a150e8f6d6cb4f29d72d1aa5dce8803151dd6d4c2a1",
        namespace: "kube-system",
        deployments: &["metrics-server"],
    },
];

pub(crate) fn find_addon(id: &str) -> Result<&'static Addon, String> {
    ADDONS.iter().find(|a| a.id == id).ok_or_else(|| format!("unknown addon '{id}'"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonStatus {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    /// not-installed | installing | uninstalling | progressing | ready | error
    pub state: String,
    pub message: Option<String>,
    /// Extra info (ingress URL, MetalLB pool …).
    pub detail: Option<String>,
}

/// In-flight operations and last failures, per `cluster/addon`.
type OpMap = HashMap<String, (String, Option<String>)>;

fn ops() -> &'static std::sync::Mutex<OpMap> {
    static OPS: std::sync::OnceLock<std::sync::Mutex<OpMap>> = std::sync::OnceLock::new();
    OPS.get_or_init(Default::default)
}

fn set_op(cluster: &str, addon: &str, state: Option<(&str, Option<String>)>) {
    let mut m = ops().lock().unwrap_or_else(|e| e.into_inner());
    let key = format!("{cluster}/{addon}");
    match state {
        Some((s, msg)) => {
            m.insert(key, (s.to_string(), msg));
        }
        None => {
            m.remove(&key);
        }
    }
}

/// Ready/total of the named deployments in a `kubectl get deploy -o json` list.
pub(crate) fn deployments_ready(list: &serde_json::Value, names: &[&str]) -> (usize, usize) {
    let items = list["items"].as_array().cloned().unwrap_or_default();
    let mut found = 0;
    let mut ready = 0;
    for n in names {
        if let Some(d) = items.iter().find(|d| d["metadata"]["name"].as_str() == Some(n)) {
            found += 1;
            let want = d["spec"]["replicas"].as_i64().unwrap_or(1);
            if d["status"]["availableReplicas"].as_i64().unwrap_or(0) >= want.max(1) {
                ready += 1;
            }
        }
    }
    (ready, found)
}

#[tauri::command]
pub async fn list_cluster_addons(
    state: State<'_, Arc<AppState>>,
    cluster: String,
) -> Result<Vec<AddonStatus>, String> {
    list_cluster_addons_core(&state, &cluster).await
}

pub(crate) async fn list_cluster_addons_core(state: &AppState, cluster: &str) -> Result<Vec<AddonStatus>, String> {
    super::kind::check_cluster_name(cluster)?;
    let deploys = kubectl_json(state, cluster, &["get", "deployments", "-A", "-o", "json"]).await?;
    let ingress_port = ingress_http_port(state, cluster).await;
    let mut out = Vec::new();
    for a in ADDONS {
        let scoped = serde_json::json!({
            "items": deploys["items"].as_array().map(|v| v
                .iter()
                .filter(|d| d["metadata"]["namespace"].as_str() == Some(a.namespace))
                .cloned()
                .collect::<Vec<_>>()).unwrap_or_default()
        });
        let (ready, found) = deployments_ready(&scoped, a.deployments);
        let op = ops().lock().unwrap_or_else(|e| e.into_inner()).get(&format!("{cluster}/{}", a.id)).cloned();
        let (state_s, message) = match op {
            Some((s, m)) => (s, m),
            None if found == 0 => ("not-installed".into(), None),
            None if ready == a.deployments.len() => ("ready".into(), None),
            None => ("progressing".into(), Some(format!("{ready}/{} deployments available", a.deployments.len()))),
        };
        let detail = match (a.id, state_s.as_str()) {
            ("ingress-nginx", "ready") => Some(match ingress_port {
                Some(p) => format!("http://localhost:{p}"),
                None => "No host port maps to 80 on this cluster; ingress is only reachable inside the kind network".into(),
            }),
            _ => None,
        };
        out.push(AddonStatus {
            id: a.id.into(),
            name: a.name.into(),
            description: a.description.into(),
            version: a.version.into(),
            state: state_s,
            message,
            detail,
        });
    }
    Ok(out)
}

fn emit(app: Option<&AppHandle>, cluster: &str, addon: &str, line: &str, done: bool, error: Option<&str>) {
    if let Some(app) = app {
        let _ = app.emit(
            "addon-progress",
            serde_json::json!({ "cluster": cluster, "addon": addon, "line": line, "done": done, "error": error }),
        );
    }
}

/// Downloads (cached on disk by digest) and verifies an addon manifest.
async fn fetch_manifest(a: &Addon) -> Result<Vec<u8>, String> {
    let dir = super::k8s_tools::tool_path(super::k8s_tools::Tool::Kubectl)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("manifests"))
        .ok_or("no data dir")?;
    let cached = dir.join(format!("{}.yaml", a.sha256));
    if let Ok(b) = std::fs::read(&cached) {
        if sha256_hex(&b) == a.sha256 {
            return Ok(b);
        }
    }
    let resp = crate::docker::engine::http_get(a.url).await.map_err(|e| format!("{e:#}"))?;
    let body = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
    let got = sha256_hex(&body);
    if got != a.sha256 {
        return Err(format!("{} manifest checksum mismatch (got {got})", a.name));
    }
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(&cached, &body);
    Ok(body)
}

fn sha256_hex(b: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// Copies `body` into the control-plane node at `path` (base64 in chunks that
/// stay under the kernel's per-argument limit).
pub(crate) async fn write_node_file(
    state: &AppState,
    cluster: &str,
    path: &str,
    body: &[u8],
) -> Result<(), String> {
    use base64::Engine as _;
    let client = local_client(state).await?;
    let node = control_plane_node(cluster);
    let b64 = base64::engine::general_purpose::STANDARD.encode(body);
    let tmp = format!("{path}.b64");
    client
        .exec_capture(&node, vec!["sh".into(), "-c".into(), format!(": > '{tmp}'")])
        .await
        .map_err(|e| e.to_string())?;
    for chunk in b64.as_bytes().chunks(96 * 1024) {
        let chunk = std::str::from_utf8(chunk).map_err(|e| e.to_string())?;
        client
            .exec_capture(&node, vec!["sh".into(), "-c".into(), format!("printf %s '{chunk}' >> '{tmp}'")])
            .await
            .map_err(|e| e.to_string())?;
    }
    client
        .exec_capture(
            &node,
            vec!["sh".into(), "-c".into(), format!("base64 -d '{tmp}' > '{path}' && rm -f '{tmp}'")],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn install_cluster_addon(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    cluster: String,
    addon: String,
) -> Result<(), String> {
    start_addon_op(state.inner().clone(), Some(app), cluster, addon, true)
}

#[tauri::command]
pub async fn uninstall_cluster_addon(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    cluster: String,
    addon: String,
) -> Result<(), String> {
    start_addon_op(state.inner().clone(), Some(app), cluster, addon, false)
}

/// Kicks off an install/uninstall in the background (they take minutes);
/// progress goes out on `addon-progress`, status via `list_cluster_addons`.
pub(crate) fn start_addon_op(
    state: Arc<AppState>,
    app: Option<AppHandle>,
    cluster: String,
    addon: String,
    install: bool,
) -> Result<(), String> {
    super::kind::check_cluster_name(&cluster)?;
    let a = find_addon(&addon)?;
    {
        let m = ops().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((s, _)) = m.get(&format!("{cluster}/{addon}")) {
            if s == "installing" || s == "uninstalling" {
                return Err(format!("{} is already {s}", a.name));
            }
        }
    }
    set_op(&cluster, a.id, Some((if install { "installing" } else { "uninstalling" }, None)));
    tauri::async_runtime::spawn(async move {
        let r = if install {
            install_addon(&state, app.as_ref(), &cluster, a).await
        } else {
            uninstall_addon(&state, app.as_ref(), &cluster, a).await
        };
        match r {
            Ok(()) => {
                set_op(&cluster, a.id, None);
                emit(app.as_ref(), &cluster, a.id, "Done", true, None);
            }
            Err(e) => {
                set_op(&cluster, a.id, Some(("error", Some(e.clone()))));
                emit(app.as_ref(), &cluster, a.id, &e, true, Some(&e));
            }
        }
    });
    Ok(())
}

/// Same as [`start_addon_op`] but waits for the result (MCP).
pub(crate) async fn addon_op_wait(
    state: &AppState,
    app: Option<&AppHandle>,
    cluster: &str,
    addon: &str,
    install: bool,
) -> Result<(), String> {
    super::kind::check_cluster_name(cluster)?;
    let a = find_addon(addon)?;
    {
        let m = ops().lock().unwrap_or_else(|e| e.into_inner());
        if let Some((s, _)) = m.get(&format!("{cluster}/{addon}")) {
            if s == "installing" || s == "uninstalling" {
                return Err(format!("{} is already {s}", a.name));
            }
        }
    }
    set_op(cluster, a.id, Some((if install { "installing" } else { "uninstalling" }, None)));
    let r = if install {
        install_addon(state, app, cluster, a).await
    } else {
        uninstall_addon(state, app, cluster, a).await
    };
    match &r {
        Ok(()) => set_op(cluster, a.id, None),
        Err(e) => set_op(cluster, a.id, Some(("error", Some(e.clone())))),
    }
    emit(app, cluster, a.id, r.as_ref().err().map(String::as_str).unwrap_or("Done"), true, r.as_ref().err().map(String::as_str));
    r
}

pub(crate) async fn install_addon(
    state: &AppState,
    app: Option<&AppHandle>,
    cluster: &str,
    a: &Addon,
) -> Result<(), String> {
    let log = |l: &str| emit(app, cluster, a.id, l, false, None);
    if a.id == "ingress-nginx" {
        let labelled = kubectl_raw(state, cluster, &["get", "nodes", "-l", "ingress-ready=true", "-o", "name"]).await?;
        if labelled.trim().is_empty() {
            log("No node is labelled ingress-ready=true; labelling the control plane");
            let node = control_plane_node(cluster);
            kubectl_raw(state, cluster, &["label", "node", &node, "ingress-ready=true", "--overwrite"]).await?;
        }
        if ingress_http_port(state, cluster).await.is_none() {
            log("Note: this cluster maps no host port to 80, so ingress is reachable only inside the kind network (create the cluster with Ingress enabled for host access)");
        }
    }
    log(&format!("Downloading {} {}…", a.name, a.version));
    let body = fetch_manifest(a).await?;
    let path = format!("/tmp/hsx2dock-addon-{}.yaml", a.id);
    write_node_file(state, cluster, &path, &body).await?;
    log("Applying manifest…");
    let out = kubectl_raw(
        state,
        cluster,
        &["apply", "--server-side", "--force-conflicts", "-f", &path, "--timeout=120s"],
    )
    .await?;
    log(&format!("{} objects applied", out.lines().count()));
    if a.id == "metrics-server" {
        let patch = r#"[{"op":"add","path":"/spec/template/spec/containers/0/args/-","value":"--kubelet-insecure-tls"}]"#;
        let args = kubectl_json(state, cluster, &["get", "deploy", "metrics-server", "-n", "kube-system", "-o", "json"]).await?;
        let has = args["spec"]["template"]["spec"]["containers"][0]["args"]
            .as_array()
            .is_some_and(|v| v.iter().any(|x| x.as_str() == Some("--kubelet-insecure-tls")));
        if !has {
            kubectl_raw(state, cluster, &["patch", "deploy", "metrics-server", "-n", "kube-system", "--type=json", "-p", patch]).await?;
        }
    }
    if a.id == "ingress-nginx" {
        // The kind manifest no longer pins the controller to the node that
        // owns the host port mappings; without this it may land on a worker
        // and the host's ingress ports lead nowhere.
        let patch = r#"{"spec":{"template":{"spec":{"nodeSelector":{"ingress-ready":"true"}}}}}"#;
        kubectl_raw(
            state,
            cluster,
            &["patch", "deployment", "ingress-nginx-controller", "-n", a.namespace, "--type=strategic", "-p", patch],
        )
        .await?;
    }
    for d in a.deployments {
        log(&format!("Waiting for {d}…"));
        // `rollout status`, not `wait --for=condition=Available`: a 1-replica
        // deployment with maxUnavailable 1 is "Available" with zero pods.
        kubectl_raw(
            state,
            cluster,
            &["rollout", "status", "-n", a.namespace, &format!("deployment/{d}"), "--timeout=300s"],
        )
        .await?;
    }
    match a.id {
        "metallb" => {
            log("Configuring the address pool");
            let range = assign_metallb_pool(state, cluster).await?;
            log(&format!("Address pool {range}"));
        }
        "cert-manager" => {
            log("Creating the self-signed ClusterIssuer");
            apply_with_retry(state, cluster, SELFSIGNED_ISSUER).await?;
        }
        "dashboard" => {
            log("Creating the admin-user ServiceAccount");
            apply_with_retry(state, cluster, DASHBOARD_ADMIN).await?;
        }
        _ => {}
    }
    Ok(())
}

/// Webhooks of a freshly installed addon may need a few seconds to serve.
async fn apply_with_retry(state: &AppState, cluster: &str, yaml: &str) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..12 {
        match super::kind::apply_yaml_core(state, cluster, yaml).await {
            Ok(_) => return Ok(()),
            Err(e) => last = e,
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
    Err(last)
}

pub(crate) async fn uninstall_addon(
    state: &AppState,
    app: Option<&AppHandle>,
    cluster: &str,
    a: &Addon,
) -> Result<(), String> {
    let log = |l: &str| emit(app, cluster, a.id, l, false, None);
    match a.id {
        "metallb" => {
            let _ = kubectl_raw(state, cluster, &["delete", "ipaddresspools.metallb.io,l2advertisements.metallb.io", "--all", "-n", "metallb-system", "--ignore-not-found"]).await;
        }
        "cert-manager" => {
            let _ = kubectl_raw(state, cluster, &["delete", "clusterissuer", "selfsigned", "--ignore-not-found"]).await;
        }
        "dashboard" => {
            let _ = kubectl_raw(state, cluster, &["delete", "clusterrolebinding", "admin-user", "--ignore-not-found"]).await;
        }
        _ => {}
    }
    log(&format!("Removing {}…", a.name));
    let body = fetch_manifest(a).await?;
    let path = format!("/tmp/hsx2dock-addon-{}.yaml", a.id);
    write_node_file(state, cluster, &path, &body).await?;
    kubectl_raw(state, cluster, &["delete", "-f", &path, "--ignore-not-found", "--wait=true", "--timeout=180s"]).await?;
    Ok(())
}

const SELFSIGNED_ISSUER: &str = "apiVersion: cert-manager.io/v1\nkind: ClusterIssuer\nmetadata:\n  name: selfsigned\nspec:\n  selfSigned: {}\n";

const DASHBOARD_ADMIN: &str = "apiVersion: v1\nkind: ServiceAccount\nmetadata:\n  name: admin-user\n  namespace: kubernetes-dashboard\n---\napiVersion: rbac.authorization.k8s.io/v1\nkind: ClusterRoleBinding\nmetadata:\n  name: admin-user\nroleRef:\n  apiGroup: rbac.authorization.k8s.io\n  kind: ClusterRole\n  name: cluster-admin\nsubjects:\n- kind: ServiceAccount\n  name: admin-user\n  namespace: kubernetes-dashboard\n";

pub(crate) fn metallb_pool_yaml(from: &str, to: &str) -> String {
    format!(
        "apiVersion: metallb.io/v1beta1\nkind: IPAddressPool\nmetadata:\n  name: kind-pool\n  namespace: metallb-system\nspec:\n  addresses:\n  - {from}-{to}\n---\napiVersion: metallb.io/v1beta1\nkind: L2Advertisement\nmetadata:\n  name: kind-l2\n  namespace: metallb-system\nspec:\n  ipAddressPools:\n  - kind-pool\n"
    )
}

/// MetalLB pools are 16-address blocks in the last /24 of the kind network's
/// IPv4 subnet; block `k` (1..=15) is `.16k–.16k+15`. Every cluster shares
/// that network, so each one gets its own block — two clusters announcing the
/// same address would fight over ARP.
pub(crate) fn metallb_range(subnet: &str, block: u8) -> Result<(String, String), String> {
    if !(1..=METALLB_BLOCKS).contains(&block) {
        return Err(format!("MetalLB block {block} is out of range"));
    }
    let (ip, len) = subnet.split_once('/').ok_or("bad subnet")?;
    let ip: std::net::Ipv4Addr = ip.parse().map_err(|_| format!("not an IPv4 subnet: {subnet}"))?;
    let len: u32 = len.parse().map_err(|_| "bad prefix")?;
    if len > 24 {
        return Err(format!("kind network {subnet} is too small for a MetalLB pool"));
    }
    let base = u32::from(ip) & (u32::MAX << (32 - len));
    let last24 = (base | !(u32::MAX << (32 - len))) & !0xFF;
    let first = u32::from(block) * 16;
    let a = std::net::Ipv4Addr::from(last24 | first);
    let b = std::net::Ipv4Addr::from(last24 | (first + 15));
    Ok((a.to_string(), b.to_string()))
}

const METALLB_BLOCKS: u8 = 15;

/// Block index of a pool's first address (`172.18.255.48` → 3), if it is one
/// of ours.
fn metallb_block_of(addr: &str) -> Option<u8> {
    let first = addr.split(['-', '/']).next()?.trim();
    let ip: std::net::Ipv4Addr = first.parse().ok()?;
    let last = ip.octets()[3];
    (last.is_multiple_of(16) && last >= 16).then_some(last / 16)
}

/// First block no other running cluster's MetalLB pool uses.
pub(crate) fn pick_metallb_block(used: &std::collections::HashSet<u8>) -> Option<u8> {
    (1..=METALLB_BLOCKS).find(|b| !used.contains(b))
}

/// Blocks taken by the other running clusters' `kind-pool`.
async fn used_metallb_blocks(state: &AppState, except: &str) -> std::collections::HashSet<u8> {
    let mut used = std::collections::HashSet::new();
    let Ok(clusters) = super::kind::list_kind_clusters_core(state).await else {
        return used;
    };
    for c in clusters.iter().filter(|c| c.running && c.name != except) {
        let Ok(v) = kubectl_json(
            state,
            &c.name,
            &["get", "ipaddresspools.metallb.io", "-n", "metallb-system", "-o", "json"],
        )
        .await
        else {
            continue;
        };
        for item in v["items"].as_array().into_iter().flatten() {
            for a in item["spec"]["addresses"].as_array().into_iter().flatten() {
                if let Some(b) = a.as_str().and_then(metallb_block_of) {
                    used.insert(b);
                }
            }
        }
    }
    used
}

/// Points the cluster's MetalLB pool at a block no other cluster uses and
/// returns the range. Also used after a clone, which inherits its source's
/// pool.
pub(crate) async fn assign_metallb_pool(state: &AppState, cluster: &str) -> Result<String, String> {
    let used = used_metallb_blocks(state, cluster).await;
    let block = pick_metallb_block(&used)
        .ok_or("every MetalLB address block of the kind network is taken by other clusters")?;
    let (from, to) = metallb_range(&kind_subnet(state).await?, block)?;
    apply_with_retry(state, cluster, &metallb_pool_yaml(&from, &to)).await?;
    Ok(format!("{from}-{to}"))
}

pub(crate) async fn kind_subnet(state: &AppState) -> Result<String, String> {
    let client = local_client(state).await?;
    let net = client
        .docker()
        .inspect_network("kind", None::<bollard::query_parameters::InspectNetworkOptions>)
        .await
        .map_err(|e| format!("inspect kind network: {e}"))?;
    net.ipam
        .and_then(|i| i.config)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| c.subnet)
        .find(|s| s.contains('.'))
        .ok_or_else(|| "the kind network has no IPv4 subnet".into())
}

/// Host port mapped to container port 80 on this cluster's nodes.
pub(crate) async fn ingress_http_port(state: &AppState, cluster: &str) -> Option<u16> {
    let clusters = super::kind::list_kind_clusters_core(state).await.ok()?;
    let c = clusters.into_iter().find(|c| c.name == cluster)?;
    c.published_ports.iter().find(|p| p.container_port == 80).map(|p| p.host_port)
}

#[tauri::command]
pub async fn dashboard_token(state: State<'_, Arc<AppState>>, cluster: String) -> Result<String, String> {
    dashboard_token_core(&state, &cluster).await
}

pub(crate) async fn dashboard_token_core(state: &AppState, cluster: &str) -> Result<String, String> {
    super::kind::check_cluster_name(cluster)?;
    let out = kubectl_raw(
        state,
        cluster,
        &["-n", "kubernetes-dashboard", "create", "token", "admin-user", "--duration=24h"],
    )
    .await?;
    Ok(out.trim().lines().last().unwrap_or_default().to_string())
}

// ------------------------------------------------------------- ingresses

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngressInfo {
    pub namespace: String,
    pub name: String,
    pub class: Option<String>,
    pub hosts: Vec<String>,
    pub tls: bool,
    /// One per host: how to open it from this machine.
    pub urls: Vec<IngressUrl>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngressUrl {
    pub host: String,
    pub url: Option<String>,
    /// The host has a gateway route to this cluster.
    pub exposed: bool,
    pub note: Option<String>,
}

/// (namespace, name, class, hosts, tls)
pub(crate) type ParsedIngress = (String, String, Option<String>, Vec<String>, bool);

pub(crate) fn parse_ingresses(v: &serde_json::Value) -> Vec<ParsedIngress> {
    v["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|i| {
                    let mut hosts: Vec<String> = i["spec"]["rules"]
                        .as_array()
                        .map(|r| r.iter().map(|x| x["host"].as_str().unwrap_or("").to_string()).collect())
                        .unwrap_or_default();
                    hosts.dedup();
                    (
                        i["metadata"]["namespace"].as_str().unwrap_or("").to_string(),
                        i["metadata"]["name"].as_str().unwrap_or("").to_string(),
                        i["spec"]["ingressClassName"].as_str().map(str::to_string),
                        hosts,
                        i["spec"]["tls"].as_array().is_some_and(|t| !t.is_empty()),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn route_group(cluster: &str) -> String {
    format!("k8s:{cluster}")
}

#[tauri::command]
pub async fn list_cluster_ingresses(
    state: State<'_, Arc<AppState>>,
    cluster: String,
) -> Result<Vec<IngressInfo>, String> {
    list_cluster_ingresses_core(&state, &cluster).await
}

pub(crate) async fn list_cluster_ingresses_core(state: &AppState, cluster: &str) -> Result<Vec<IngressInfo>, String> {
    super::kind::check_cluster_name(cluster)?;
    let v = kubectl_json(state, cluster, &["get", "ingress", "-A", "-o", "json"]).await?;
    let port = ingress_http_port(state, cluster).await;
    let (routes, gw_http) = {
        let cfg = state.config.read().await;
        (cfg.gateway_routes.clone(), cfg.gateway_http_port)
    };
    let group = route_group(cluster);
    Ok(parse_ingresses(&v)
        .into_iter()
        .map(|(namespace, name, class, hosts, tls)| {
            let urls = hosts
                .iter()
                .map(|h| {
                    let exposed = routes.iter().any(|r| r.enabled && r.domain == *h && r.group_name.as_deref() == Some(&group));
                    let (url, note) = if exposed {
                        let p = if gw_http == 80 { String::new() } else { format!(":{gw_http}") };
                        (Some(format!("http://{h}{p}")), None)
                    } else if h.is_empty() || h == "localhost" {
                        (port.map(|p| format!("http://localhost:{p}")), None)
                    } else {
                        (
                            None,
                            Some(match port {
                                Some(p) => format!("Expose it through the gateway, or send Host: {h} to localhost:{p}"),
                                None => "The cluster has no host port for ingress".into(),
                            }),
                        )
                    };
                    IngressUrl { host: h.clone(), url, exposed, note }
                })
                .collect();
            IngressInfo { namespace, name, class, hosts, tls, urls }
        })
        .collect())
}

#[tauri::command]
pub async fn expose_ingress_host(
    state: State<'_, Arc<AppState>>,
    cluster: String,
    host: String,
    expose: bool,
) -> Result<(), String> {
    expose_ingress_host_core(&state, &cluster, &host, expose).await
}

/// Adds (or removes) a gateway route `host → 127.0.0.1:<ingress port>`; the
/// gateway also writes the hosts entry. Caddy keeps the Host header, so
/// ingress-nginx routes it like any other request.
pub(crate) async fn expose_ingress_host_core(
    state: &AppState,
    cluster: &str,
    host: &str,
    expose: bool,
) -> Result<(), String> {
    super::kind::check_cluster_name(cluster)?;
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() || host.contains('*') || host.contains('/') || host.contains(' ') {
        return Err("A concrete host name is required (wildcards cannot go in the hosts file)".into());
    }
    let group = route_group(cluster);
    let mut routes = state.config.read().await.gateway_routes.clone();
    routes.retain(|r| !(r.domain == host && r.group_name.as_deref() == Some(&group)));
    if expose {
        let port = ingress_http_port(state, cluster)
            .await
            .ok_or("This cluster maps no host port to 80; create it with Ingress enabled")?;
        if routes.iter().any(|r| r.domain == host && r.path.as_deref().unwrap_or("").is_empty()) {
            return Err(format!("{host} already has a gateway route"));
        }
        routes.push(crate::docker::caddy::GatewayRoute {
            id: format!("k8s-{cluster}-{host}"),
            domain: host.clone(),
            target: format!("127.0.0.1:{port}"),
            path: None,
            strip_prefix: None,
            container_name: None,
            group_name: Some(group),
            enabled: true,
            auto_detected: false,
        });
    }
    super::gateway::save_gateway_routes_core(state, routes).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_entries_are_complete() {
        for a in ADDONS {
            assert_eq!(a.sha256.len(), 64, "{}", a.id);
            assert!(a.url.starts_with("https://") && a.url.contains(a.version.trim_start_matches('v')), "{}", a.id);
            assert!(!a.deployments.is_empty());
        }
        assert!(find_addon("nope").is_err());
    }

    #[test]
    fn readiness_counts_only_named_deployments() {
        let v = serde_json::json!({"items": [
            {"metadata": {"name": "a"}, "spec": {"replicas": 1}, "status": {"availableReplicas": 1}},
            {"metadata": {"name": "b"}, "spec": {"replicas": 2}, "status": {"availableReplicas": 1}},
            {"metadata": {"name": "x"}, "spec": {"replicas": 1}, "status": {}}
        ]});
        assert_eq!(deployments_ready(&v, &["a", "b", "c"]), (1, 2));
    }

    #[test]
    fn metallb_pool_sits_at_the_end_of_the_kind_subnet() {
        assert_eq!(metallb_range("172.18.0.0/16", 1).unwrap(), ("172.18.255.16".into(), "172.18.255.31".into()));
        assert_eq!(metallb_range("10.89.3.0/24", 15).unwrap(), ("10.89.3.240".into(), "10.89.3.255".into()));
        assert!(metallb_range("172.18.0.0/16", 0).is_err());
        assert!(metallb_range("172.18.0.0/16", 16).is_err());
        assert!(metallb_range("10.0.0.0/28", 1).is_err());
        assert!(metallb_range("fc00::/64", 1).is_err());
        assert_eq!(metallb_block_of("172.18.255.48-172.18.255.63"), Some(3));
        assert_eq!(metallb_block_of("172.18.255.200-172.18.255.250"), None);
        let used: std::collections::HashSet<u8> = [1, 2].into_iter().collect();
        assert_eq!(pick_metallb_block(&used), Some(3));
        let all: std::collections::HashSet<u8> = (1..=15).collect();
        assert_eq!(pick_metallb_block(&all), None);
        assert!(metallb_pool_yaml("a", "b").contains("- a-b"));
    }

    #[test]
    fn ingresses_are_parsed() {
        let v = serde_json::json!({"items": [{
            "metadata": {"namespace": "default", "name": "web"},
            "spec": {"ingressClassName": "nginx", "rules": [{"host": "web.local"}, {"host": "web.local"}], "tls": [{}]}
        }]});
        let p = parse_ingresses(&v);
        assert_eq!(p[0].3, vec!["web.local".to_string()]);
        assert!(p[0].4);
        assert_eq!(p[0].2.as_deref(), Some("nginx"));
    }
}
