//! Advanced cluster creation: the wizard's [`ClusterSpec`], its validation,
//! the kind config it renders to, and [`finalize_config`], which forces the
//! parts every cluster needs into any config — generated or hand-edited — so
//! a user edit in the preview can never drop the containerd state mounts or
//! the host-reachable API port.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use yaml_rust2::yaml::Hash;
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

/// Most nodes one cluster may have (control planes + workers).
pub(crate) const MAX_NODES: u32 = 10;

/// Container path of every node's containerd state mount.
const CONTAINERD_PATH: &str = "/var/lib/containerd";

/// Where node containerd reads per-registry `hosts.toml` files.
pub(crate) const CERTS_D: &str = "/etc/containerd/certs.d";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    pub container_port: u16,
    pub host_port: u16,
    #[serde(default = "default_tcp")]
    pub protocol: String,
}

fn default_tcp() -> String {
    "TCP".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostMount {
    pub host_path: String,
    pub container_path: String,
    #[serde(default)]
    pub read_only: bool,
}

/// Everything the create wizard can choose.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct ClusterSpec {
    /// `kindest/node:vX.Y.Z[@sha256:…]`; None = kind's default image.
    pub image: Option<String>,
    pub control_planes: u32,
    pub workers: u32,
    /// Map the ingress ports on the first control plane and label it
    /// `ingress-ready=true` (what the kind ingress-nginx manifest expects).
    pub ingress: bool,
    pub ingress_http_port: u16,
    pub ingress_https_port: u16,
    /// Extra host → first-control-plane port mappings.
    pub port_mappings: Vec<PortMapping>,
    /// Host directories mounted into every node.
    pub mounts: Vec<HostMount>,
    pub pod_subnet: Option<String>,
    pub service_subnet: Option<String>,
    /// `Name=true|false` per entry.
    pub feature_gates: Vec<String>,
    /// Wire the shared local registry into the nodes' containerd.
    pub local_registry: bool,
}

impl Default for ClusterSpec {
    fn default() -> Self {
        Self {
            image: None,
            control_planes: 1,
            workers: 0,
            ingress: false,
            ingress_http_port: 18080,
            ingress_https_port: 18443,
            port_mappings: Vec::new(),
            mounts: Vec::new(),
            pod_subnet: None,
            service_subnet: None,
            feature_gates: Vec::new(),
            local_registry: false,
        }
    }
}

impl ClusterSpec {
    /// The old two-field create (image + node count): one control plane, the
    /// rest workers.
    pub(crate) fn simple(image: Option<String>, nodes: Option<u32>) -> Self {
        let n = nodes.unwrap_or(1).clamp(1, MAX_NODES);
        Self {
            image: image.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            workers: n - 1,
            ..Self::default()
        }
    }

    pub(crate) fn node_count(&self) -> u32 {
        self.control_planes + self.workers
    }
}

fn valid_cidr(s: &str) -> bool {
    let Some((ip, len)) = s.split_once('/') else {
        return false;
    };
    let Ok(len) = len.parse::<u8>() else {
        return false;
    };
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(_)) => len <= 32,
        Ok(std::net::IpAddr::V6(_)) => len <= 128,
        Err(_) => false,
    }
}

fn valid_feature_gate(s: &str) -> bool {
    let Some((name, val)) = s.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric())
        && name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && matches!(val, "true" | "false")
}

fn valid_image(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '\\')
}

/// Problems with a spec; empty when it can be created. `path_exists` checks
/// mount sources (injected so tests need no real directories).
pub(crate) fn validate_spec(spec: &ClusterSpec, path_exists: impl Fn(&str) -> bool) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(img) = &spec.image {
        if !valid_image(img) {
            errors.push("Node image is not a valid image reference".into());
        }
    }
    if spec.control_planes == 0 || spec.control_planes.is_multiple_of(2) || spec.control_planes > 5 {
        errors.push("Control planes must be 1, 3 or 5 (etcd needs an odd quorum)".into());
    }
    if spec.node_count() > MAX_NODES {
        errors.push(format!("At most {MAX_NODES} nodes per cluster"));
    }
    let mut seen: HashSet<(u16, String)> = HashSet::new();
    let mut check_host_port = |port: u16, proto: &str, what: &str, errors: &mut Vec<String>| {
        if port == 0 {
            errors.push(format!("{what}: host port is required"));
        } else if !seen.insert((port, proto.to_ascii_uppercase())) {
            errors.push(format!("{what}: host port {port}/{proto} is used twice"));
        } else if (16443..=16999).contains(&port) {
            errors.push(format!("{what}: host port {port} is reserved for cluster API servers"));
        } else if crate::commands::kind_access::port_is_ephemeral(port) {
            errors.push(format!(
                "{what}: host port {port} is in the ephemeral range, which the engine network does not forward to the host"
            ));
        }
    };
    if spec.ingress {
        check_host_port(spec.ingress_http_port, "TCP", "Ingress HTTP", &mut errors);
        check_host_port(spec.ingress_https_port, "TCP", "Ingress HTTPS", &mut errors);
    }
    for (i, m) in spec.port_mappings.iter().enumerate() {
        let what = format!("Port mapping {}", i + 1);
        if m.container_port == 0 {
            errors.push(format!("{what}: container port is required"));
        }
        if !matches!(m.protocol.to_ascii_uppercase().as_str(), "TCP" | "UDP" | "SCTP") {
            errors.push(format!("{what}: protocol must be TCP, UDP or SCTP"));
        }
        check_host_port(m.host_port, &m.protocol, &what, &mut errors);
    }
    let mut targets = HashSet::new();
    for (i, m) in spec.mounts.iter().enumerate() {
        let what = format!("Mount {}", i + 1);
        let host = Path::new(&m.host_path);
        let target = Path::new(&m.container_path);
        if !host.is_absolute() || m.host_path.contains("..") {
            errors.push(format!("{what}: host path must be an absolute path"));
        } else if !path_exists(&m.host_path) {
            errors.push(format!("{what}: {} does not exist", m.host_path));
        }
        if !target.is_absolute() || m.container_path.contains("..") || m.container_path == "/" {
            errors.push(format!("{what}: container path must be an absolute path"));
        } else if target.starts_with(CONTAINERD_PATH) || target.starts_with("/etc/kubernetes") {
            errors.push(format!("{what}: {} is managed by the app", m.container_path));
        } else if !targets.insert(m.container_path.trim_end_matches('/').to_string()) {
            errors.push(format!("{what}: container path {} is used twice", m.container_path));
        }
    }
    for (label, v) in [("Pod subnet", &spec.pod_subnet), ("Service subnet", &spec.service_subnet)] {
        if let Some(s) = v.as_deref().filter(|s| !s.is_empty()) {
            if !valid_cidr(s) {
                errors.push(format!("{label} must be a CIDR such as 10.244.0.0/16"));
            }
        }
    }
    for g in &spec.feature_gates {
        if !valid_feature_gate(g) {
            errors.push(format!("Feature gate '{g}' must look like Name=true"));
        }
    }
    errors
}

fn s(v: &str) -> Yaml {
    Yaml::String(v.to_string())
}

fn key(h: &Hash, k: &str) -> Option<Yaml> {
    h.get(&s(k)).cloned()
}

/// The user-facing kind config for a spec — before [`finalize_config`] adds
/// the per-node state mounts, API port and patches.
pub(crate) fn spec_to_yaml(spec: &ClusterSpec) -> String {
    let mut root = Hash::new();
    root.insert(s("kind"), s("Cluster"));
    root.insert(s("apiVersion"), s("kind.x-k8s.io/v1alpha4"));
    let mut networking = Hash::new();
    if let Some(p) = spec.pod_subnet.as_deref().filter(|v| !v.is_empty()) {
        networking.insert(s("podSubnet"), s(p));
    }
    if let Some(p) = spec.service_subnet.as_deref().filter(|v| !v.is_empty()) {
        networking.insert(s("serviceSubnet"), s(p));
    }
    if !networking.is_empty() {
        root.insert(s("networking"), Yaml::Hash(networking));
    }
    if !spec.feature_gates.is_empty() {
        let mut gates = Hash::new();
        for g in &spec.feature_gates {
            if let Some((n, v)) = g.split_once('=') {
                gates.insert(s(n), Yaml::Boolean(v == "true"));
            }
        }
        root.insert(s("featureGates"), Yaml::Hash(gates));
    }
    let mounts: Vec<Yaml> = spec
        .mounts
        .iter()
        .map(|m| {
            let mut h = Hash::new();
            h.insert(s("hostPath"), s(&m.host_path));
            h.insert(s("containerPath"), s(&m.container_path));
            if m.read_only {
                h.insert(s("readOnly"), Yaml::Boolean(true));
            }
            Yaml::Hash(h)
        })
        .collect();
    let mut nodes = Vec::new();
    for i in 0..spec.node_count() {
        let mut node = Hash::new();
        let cp = i < spec.control_planes;
        node.insert(s("role"), s(if cp { "control-plane" } else { "worker" }));
        if let Some(img) = &spec.image {
            node.insert(s("image"), s(img));
        }
        if i == 0 {
            let mut maps = Vec::new();
            if spec.ingress {
                node.insert(
                    s("kubeadmConfigPatches"),
                    Yaml::Array(vec![s(
                        "kind: InitConfiguration\nnodeRegistration:\n  kubeletExtraArgs:\n    node-labels: \"ingress-ready=true\"\n",
                    )]),
                );
                for (c, h) in [(80, spec.ingress_http_port), (443, spec.ingress_https_port)] {
                    let mut m = Hash::new();
                    m.insert(s("containerPort"), Yaml::Integer(c));
                    m.insert(s("hostPort"), Yaml::Integer(h as i64));
                    m.insert(s("protocol"), s("TCP"));
                    maps.push(Yaml::Hash(m));
                }
            }
            for pm in &spec.port_mappings {
                let mut m = Hash::new();
                m.insert(s("containerPort"), Yaml::Integer(pm.container_port as i64));
                m.insert(s("hostPort"), Yaml::Integer(pm.host_port as i64));
                m.insert(s("protocol"), s(&pm.protocol.to_ascii_uppercase()));
                maps.push(Yaml::Hash(m));
            }
            if !maps.is_empty() {
                node.insert(s("extraPortMappings"), Yaml::Array(maps));
            }
        }
        if !mounts.is_empty() {
            node.insert(s("extraMounts"), Yaml::Array(mounts.clone()));
        }
        nodes.push(Yaml::Hash(node));
    }
    root.insert(s("nodes"), Yaml::Array(nodes));
    emit(&Yaml::Hash(root))
}

fn emit(doc: &Yaml) -> String {
    let mut out = String::new();
    let mut em = YamlEmitter::new(&mut out);
    em.multiline_strings(true);
    let _ = em.dump(doc);
    let out = out.strip_prefix("---\n").unwrap_or(&out).to_string();
    out + "\n"
}

/// What [`finalize_config`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FinalConfig {
    pub yaml: String,
    pub nodes: usize,
    /// Host ports the nodes publish (besides the API port).
    pub host_ports: Vec<u16>,
}

/// containerd reads per-registry `hosts.toml` from here once this is set.
pub(crate) const REGISTRY_CONTAINERD_PATCH: &str =
    "[plugins.\"io.containerd.grpc.v1.cri\".registry]\n  config_path = \"/etc/containerd/certs.d\"\n";

/// Parses a kind config (generated or edited) and forces in what the app
/// relies on:
///  - every node mounts `state_dir(i)` at /var/lib/containerd (any user mount
///    there is replaced);
///  - `networking.apiServerAddress/apiServerPort` when `api_port` is given;
///  - kubeadm's longer init timeouts for v1beta4 images;
///  - the registry `config_path` patch when `registry` is set.
pub(crate) fn finalize_config(
    text: &str,
    state_dir: impl Fn(usize) -> String,
    api_port: Option<u16>,
    registry: bool,
) -> Result<FinalConfig, String> {
    let docs = YamlLoader::load_from_str(text).map_err(|e| format!("Config is not valid YAML: {e}"))?;
    if docs.len() != 1 {
        return Err("Config must be a single YAML document".into());
    }
    let Yaml::Hash(mut root) = docs.into_iter().next().unwrap_or(Yaml::Null) else {
        return Err("Config must be a mapping (kind: Cluster …)".into());
    };
    if key(&root, "kind").and_then(|v| v.as_str().map(str::to_string)).as_deref() != Some("Cluster") {
        return Err("Config must have kind: Cluster".into());
    }
    if key(&root, "apiVersion").and_then(|v| v.as_str().map(str::to_string)).as_deref()
        != Some("kind.x-k8s.io/v1alpha4")
    {
        return Err("Config must have apiVersion: kind.x-k8s.io/v1alpha4".into());
    }
    let mut nodes = match root.remove(&s("nodes")) {
        Some(Yaml::Array(n)) if !n.is_empty() => n,
        None => vec![Yaml::Hash({
            let mut h = Hash::new();
            h.insert(s("role"), s("control-plane"));
            h
        })],
        _ => return Err("nodes must be a non-empty list".into()),
    };
    if nodes.len() > MAX_NODES as usize {
        return Err(format!("At most {MAX_NODES} nodes per cluster"));
    }
    let mut host_ports = Vec::new();
    let mut control_planes: u32 = 0;
    let mut first_image: Option<String> = None;
    for (i, node) in nodes.iter_mut().enumerate() {
        let Yaml::Hash(h) = node else {
            return Err(format!("Node {} must be a mapping", i + 1));
        };
        match key(h, "role").as_ref().and_then(Yaml::as_str) {
            Some("control-plane") => control_planes += 1,
            Some("worker") => {}
            _ => return Err(format!("Node {}: role must be control-plane or worker", i + 1)),
        }
        if first_image.is_none() {
            first_image = key(h, "image").and_then(|v| v.as_str().map(str::to_string));
        }
        if let Some(Yaml::Array(maps)) = h.get(&s("extraPortMappings")) {
            for m in maps {
                if let Some(p) = m["hostPort"].as_i64().and_then(|p| u16::try_from(p).ok()) {
                    host_ports.push(p);
                }
            }
        }
        let mut mounts: Vec<Yaml> = match h.remove(&s("extraMounts")) {
            Some(Yaml::Array(m)) => m,
            None => Vec::new(),
            _ => return Err(format!("Node {}: extraMounts must be a list", i + 1)),
        };
        mounts.retain(|m| {
            m["containerPath"]
                .as_str()
                .map(|p| !Path::new(p).starts_with(CONTAINERD_PATH))
                .unwrap_or(true)
        });
        let mut ours = Hash::new();
        ours.insert(s("hostPath"), s(&state_dir(i)));
        ours.insert(s("containerPath"), s(CONTAINERD_PATH));
        ours.insert(s("readOnly"), Yaml::Boolean(false));
        mounts.push(Yaml::Hash(ours));
        h.insert(s("extraMounts"), Yaml::Array(mounts));
    }
    if control_planes == 0 {
        return Err("At least one node must be a control-plane".into());
    }
    if control_planes.is_multiple_of(2) {
        return Err("Control planes must be an odd number (etcd quorum)".into());
    }

    if let Some(port) = api_port {
        let mut net = match root.remove(&s("networking")) {
            Some(Yaml::Hash(n)) => n,
            None => Hash::new(),
            _ => return Err("networking must be a mapping".into()),
        };
        net.insert(s("apiServerAddress"), s("127.0.0.1"));
        net.insert(s("apiServerPort"), Yaml::Integer(port as i64));
        root.insert(s("networking"), Yaml::Hash(net));
    }

    let timeouts = super::kind_access::kubeadm_timeouts_patch(first_image.as_deref());
    if !timeouts.is_empty() {
        let patch = "kind: InitConfiguration\ntimeouts:\n  controlPlaneComponentHealthCheck: 8m0s\n  kubernetesAPICall: 5m0s\n";
        let mut patches = match root.remove(&s("kubeadmConfigPatches")) {
            Some(Yaml::Array(p)) => p,
            None => Vec::new(),
            _ => return Err("kubeadmConfigPatches must be a list".into()),
        };
        if !patches.iter().any(|p| p.as_str().is_some_and(|t| t.contains("timeouts:"))) {
            patches.push(s(patch));
        }
        root.insert(s("kubeadmConfigPatches"), Yaml::Array(patches));
    }

    if registry {
        let mut patches = match root.remove(&s("containerdConfigPatches")) {
            Some(Yaml::Array(p)) => p,
            None => Vec::new(),
            _ => return Err("containerdConfigPatches must be a list".into()),
        };
        if !patches.iter().any(|p| p.as_str().is_some_and(|t| t.contains("config_path"))) {
            patches.push(s(REGISTRY_CONTAINERD_PATCH));
        }
        root.insert(s("containerdConfigPatches"), Yaml::Array(patches));
    }

    let count = nodes.len();
    root.insert(s("nodes"), Yaml::Array(nodes));
    Ok(FinalConfig { yaml: emit(&Yaml::Hash(root)), nodes: count, host_ports })
}

/// Whether the config text wires the registry `config_path` (so the app must
/// write `hosts.toml` into the nodes after create).
pub(crate) fn config_uses_registry(text: &str) -> bool {
    text.contains("config_path") && text.contains(CERTS_D)
}


// ------------------------------------------------------------------ commands

/// What the wizard shows next to the YAML editor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPreview {
    /// The editable config (the spec rendered, or the user's text echoed).
    pub yaml: String,
    /// What kind will actually get: state mounts, API port, patches added.
    pub final_yaml: Option<String>,
    pub nodes: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn preview_cluster_config(
    state: tauri::State<'_, std::sync::Arc<crate::state::AppState>>,
    spec: ClusterSpec,
    config: Option<String>,
) -> Result<ConfigPreview, String> {
    Ok(preview_cluster_config_core(&state, &spec, config).await)
}

pub(crate) async fn preview_cluster_config_core(
    state: &crate::state::AppState,
    spec: &ClusterSpec,
    config: Option<String>,
) -> ConfigPreview {
    let config = config.filter(|c| !c.trim().is_empty());
    let (yaml, mut errors) = match config {
        Some(c) => (c, Vec::new()),
        None => (spec_to_yaml(spec), validate_spec(spec, |p| Path::new(p).exists())),
    };
    let registry = spec.local_registry || config_uses_registry(&yaml);
    let mut warnings = Vec::new();
    let (final_yaml, nodes) = match finalize_config(
        &yaml,
        |i| format!("<app data>/kind-nodes/<name>/node{i}"),
        Some(super::kind_access::API_PORT_FIRST),
        registry,
    ) {
        Ok(f) => {
            let published = super::kind_access::kind_published_ports(state).await;
            for p in &f.host_ports {
                if published.contains(p) {
                    errors.push(format!("Host port {p} is already published by another cluster"));
                } else if !super::kind_access::host_port_is_free(*p) {
                    errors.push(format!("Host port {p} is already in use on this machine"));
                }
            }
            (Some(f.yaml), f.nodes)
        }
        Err(e) => {
            errors.push(e);
            (None, 0)
        }
    };
    if nodes > 4 {
        warnings.push(format!("{nodes} nodes need several GB of RAM and take minutes to start"));
    }
    if final_yaml.is_some() {
        warnings.push(
            "The API port shown is a placeholder; the real one is picked at create time".into(),
        );
    }
    ConfigPreview { yaml, final_yaml, nodes, errors, warnings }
}

/// One choice for the node-image picker.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NodeImageOption {
    pub image: String,
    /// `v1.31.0`
    pub version: String,
    /// Already pulled into the engine.
    pub local: bool,
    /// kind's built-in default.
    pub default: bool,
}

/// `kindest/node:vX.Y.Z@sha256:…` baked into the kind binary.
pub(crate) fn default_image_in_binary(bytes: &[u8]) -> Option<String> {
    let needle = b"kindest/node:v";
    let mut from = 0;
    while let Some(pos) = bytes[from..].windows(needle.len()).position(|w| w == needle) {
        let start = from + pos;
        let end = bytes[start..]
            .iter()
            .position(|b| !(b.is_ascii_alphanumeric() || b"/:.@-_".contains(b)))
            .map(|n| start + n)
            .unwrap_or(bytes.len());
        let candidate = String::from_utf8_lossy(&bytes[start..end]).to_string();
        if let Some((_, digest)) = candidate.split_once("@sha256:") {
            if digest.len() >= 64 {
                return Some(candidate[..candidate.find("@sha256:").unwrap() + 8 + 64].to_string());
            }
        }
        from = start + needle.len();
    }
    None
}

pub(crate) fn image_version(image: &str) -> String {
    let no_digest = image.split('@').next().unwrap_or(image);
    no_digest.rsplit_once(':').map(|(_, t)| t.to_string()).unwrap_or_default()
}

fn version_key(v: &str) -> (u32, u32, u32) {
    let mut it = v.trim_start_matches('v').split(['.', '-']).map(|p| p.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Merges the default, local and remote tags into one list, newest first.
pub(crate) fn merge_node_images(
    default: Option<String>,
    local: Vec<String>,
    remote: Vec<String>,
) -> Vec<NodeImageOption> {
    let mut by_version: std::collections::BTreeMap<String, NodeImageOption> = Default::default();
    for tag in remote {
        // Docker Hub lists bare tags (`v1.31.0`).
        let v = if tag.contains(':') { image_version(&tag) } else { tag.clone() };
        if v.starts_with('v') && !v.contains("alpha") && !v.contains("beta") && !v.contains("rc") {
            by_version.entry(v.clone()).or_insert(NodeImageOption {
                image: format!("kindest/node:{v}"),
                version: v,
                local: false,
                default: false,
            });
        }
    }
    for img in local {
        let v = image_version(&img);
        if v.is_empty() {
            continue;
        }
        let e = by_version.entry(v.clone()).or_insert(NodeImageOption {
            image: img.clone(),
            version: v,
            local: true,
            default: false,
        });
        e.local = true;
    }
    if let Some(d) = default {
        let v = image_version(&d);
        let e = by_version.entry(v.clone()).or_insert(NodeImageOption {
            image: d.clone(),
            version: v,
            local: false,
            default: true,
        });
        e.default = true;
        e.image = d;
    }
    let mut out: Vec<_> = by_version.into_values().collect();
    out.sort_by_key(|o| std::cmp::Reverse(version_key(&o.version)));
    out
}

static REMOTE_TAGS: tokio::sync::Mutex<Option<(std::time::Instant, Vec<String>)>> =
    tokio::sync::Mutex::const_new(None);

async fn remote_node_tags() -> Vec<String> {
    let mut cache = REMOTE_TAGS.lock().await;
    if let Some((at, tags)) = cache.as_ref() {
        if at.elapsed() < std::time::Duration::from_secs(3600) {
            return tags.clone();
        }
    }
    let url = "https://hub.docker.com/v2/repositories/kindest/node/tags?page_size=60&ordering=last_updated";
    let fetched = async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .ok()?;
        let body = client.get(url).send().await.ok()?.bytes().await.ok()?;
        let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
        Some(
            v["results"]
                .as_array()?
                .iter()
                .filter_map(|r| r["name"].as_str().map(str::to_string))
                .collect::<Vec<_>>(),
        )
    }
    .await;
    match fetched {
        Some(tags) => {
            *cache = Some((std::time::Instant::now(), tags.clone()));
            tags
        }
        None => cache.as_ref().map(|(_, t)| t.clone()).unwrap_or_default(),
    }
}

#[tauri::command]
pub async fn list_kind_node_images(
    state: tauri::State<'_, std::sync::Arc<crate::state::AppState>>,
) -> Result<Vec<NodeImageOption>, String> {
    Ok(list_kind_node_images_core(&state).await)
}

pub(crate) async fn list_kind_node_images_core(state: &crate::state::AppState) -> Vec<NodeImageOption> {
    let binary = state.kind.binary();
    let default = tokio::task::spawn_blocking(move || {
        std::fs::read(binary).ok().and_then(|b| default_image_in_binary(&b))
    })
    .await
    .ok()
    .flatten();
    let mut local = Vec::new();
    if let Ok(client) = super::kind::local_client(state).await {
        let mut filters = std::collections::HashMap::new();
        filters.insert("reference".to_string(), vec!["kindest/node".to_string()]);
        let opts = bollard::query_parameters::ListImagesOptionsBuilder::default()
            .filters(&filters)
            .build();
        if let Ok(list) = client.docker().list_images(Some(opts)).await {
            local = list.into_iter().flat_map(|i| i.repo_tags).collect();
        }
    }
    let remote = remote_node_tags().await;
    merge_node_images(default, local, remote)
}


// ------------------------------------------------------------------ presets

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClusterPreset {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Shipped with the app (cannot be deleted).
    #[serde(default)]
    pub builtin: bool,
    pub spec: ClusterSpec,
}

pub(crate) fn builtin_presets() -> Vec<ClusterPreset> {
    let p = |name: &str, description: &str, spec: ClusterSpec| ClusterPreset {
        name: name.into(),
        description: description.into(),
        builtin: true,
        spec,
    };
    vec![
        p("Minimal", "One control-plane node, nothing else", ClusterSpec::default()),
        p(
            "Ingress-ready",
            "One node labelled ingress-ready with HTTP/HTTPS on host ports 18080/18443",
            ClusterSpec { ingress: true, ..ClusterSpec::default() },
        ),
        p(
            "Dev",
            "Control plane + 2 workers, ingress ports and the local registry",
            ClusterSpec { workers: 2, ingress: true, local_registry: true, ..ClusterSpec::default() },
        ),
        p(
            "HA",
            "3 control planes behind kind's load balancer + 2 workers",
            ClusterSpec { control_planes: 3, workers: 2, ..ClusterSpec::default() },
        ),
    ]
}

fn presets_path() -> Option<std::path::PathBuf> {
    crate::config::AppConfig::config_dir().map(|d| d.join("cluster-presets.json"))
}

fn load_user_presets() -> Vec<ClusterPreset> {
    presets_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Vec<ClusterPreset>>(&b).ok())
        .unwrap_or_default()
}

fn store_user_presets(list: &[ClusterPreset]) -> Result<(), String> {
    let path = presets_path().ok_or("config directory is unavailable")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

static PRESETS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tauri::command]
pub fn list_cluster_presets() -> Vec<ClusterPreset> {
    let mut out = builtin_presets();
    out.extend(load_user_presets().into_iter().map(|mut p| {
        p.builtin = false;
        p
    }));
    out
}

#[tauri::command]
pub fn save_cluster_preset(preset: ClusterPreset) -> Result<Vec<ClusterPreset>, String> {
    let name = preset.name.trim().to_string();
    if name.is_empty() || name.len() > 60 {
        return Err("Preset name must be 1–60 characters".into());
    }
    if builtin_presets().iter().any(|b| b.name.eq_ignore_ascii_case(&name)) {
        return Err(format!("'{name}' is a built-in preset name"));
    }
    let _g = PRESETS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load_user_presets();
    list.retain(|p| !p.name.eq_ignore_ascii_case(&name));
    list.push(ClusterPreset { name, builtin: false, ..preset });
    store_user_presets(&list)?;
    drop(_g);
    Ok(list_cluster_presets())
}

#[tauri::command]
pub fn delete_cluster_preset(name: String) -> Result<Vec<ClusterPreset>, String> {
    let _g = PRESETS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut list = load_user_presets();
    list.retain(|p| !p.name.eq_ignore_ascii_case(name.trim()));
    store_user_presets(&list)?;
    drop(_g);
    Ok(list_cluster_presets())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_exist(_: &str) -> bool {
        true
    }

    #[test]
    fn default_spec_is_valid_and_renders_one_control_plane() {
        let spec = ClusterSpec::default();
        assert!(validate_spec(&spec, all_exist).is_empty());
        let y = spec_to_yaml(&spec);
        assert!(y.contains("kind: Cluster"));
        assert_eq!(y.matches("role: control-plane").count(), 1);
        assert!(!y.contains("worker"));
    }

    #[test]
    fn simple_spec_maps_node_count() {
        let s = ClusterSpec::simple(Some(" kindest/node:v1.31.0 ".into()), Some(3));
        assert_eq!((s.control_planes, s.workers), (1, 2));
        assert_eq!(s.image.as_deref(), Some("kindest/node:v1.31.0"));
        assert_eq!(ClusterSpec::simple(None, Some(99)).node_count(), MAX_NODES);
    }

    #[test]
    fn validation_catches_bad_topology_ports_mounts_and_cidrs() {
        let spec = ClusterSpec {
            control_planes: 2,
            workers: 9,
            ingress: true,
            ingress_http_port: 18080,
            ingress_https_port: 18080,
            port_mappings: vec![PortMapping { container_port: 0, host_port: 16500, protocol: "ICMP".into() }],
            mounts: vec![
                HostMount { host_path: "rel/path".into(), container_path: "/data".into(), read_only: false },
                HostMount { host_path: "/x".into(), container_path: "/var/lib/containerd/x".into(), read_only: true },
            ],
            pod_subnet: Some("10.0.0.0".into()),
            feature_gates: vec!["bad".into()],
            ..ClusterSpec::default()
        };
        let errs = validate_spec(&spec, all_exist);
        let joined = errs.join("\n");
        for needle in [
            "odd quorum",
            "At most 10",
            "used twice",
            "container port is required",
            "protocol must be",
            "reserved for cluster API",
            "absolute path",
            "managed by the app",
            "Pod subnet",
            "Feature gate",
        ] {
            assert!(joined.contains(needle), "missing {needle}: {joined}");
        }
        let missing = ClusterSpec {
            mounts: vec![HostMount { host_path: "/nope".into(), container_path: "/d".into(), read_only: false }],
            ..ClusterSpec::default()
        };
        assert!(validate_spec(&missing, |_| false)[0].contains("does not exist"));
    }

    #[test]
    fn ingress_and_ha_render_as_kind_expects() {
        let spec = ClusterSpec {
            control_planes: 3,
            workers: 2,
            ingress: true,
            port_mappings: vec![PortMapping { container_port: 30080, host_port: 18000, protocol: "tcp".into() }],
            mounts: vec![HostMount { host_path: "/srv/data".into(), container_path: "/data".into(), read_only: true }],
            pod_subnet: Some("10.250.0.0/16".into()),
            feature_gates: vec!["SomeGate=true".into()],
            ..ClusterSpec::default()
        };
        let y = spec_to_yaml(&spec);
        let doc = &YamlLoader::load_from_str(&y).unwrap()[0];
        let nodes = doc["nodes"].as_vec().unwrap();
        assert_eq!(nodes.len(), 5);
        assert_eq!(nodes.iter().filter(|n| n["role"].as_str() == Some("control-plane")).count(), 3);
        let maps = nodes[0]["extraPortMappings"].as_vec().unwrap();
        assert_eq!(maps.len(), 3);
        assert_eq!(maps[0]["containerPort"].as_i64(), Some(80));
        assert_eq!(maps[0]["hostPort"].as_i64(), Some(18080));
        assert_eq!(maps[2]["protocol"].as_str(), Some("TCP"));
        assert!(nodes[0]["kubeadmConfigPatches"][0].as_str().unwrap().contains("ingress-ready=true"));
        assert!(nodes[1]["extraPortMappings"].is_badvalue());
        assert_eq!(nodes[4]["extraMounts"][0]["readOnly"].as_bool(), Some(true));
        assert_eq!(doc["networking"]["podSubnet"].as_str(), Some("10.250.0.0/16"));
        assert_eq!(doc["featureGates"]["SomeGate"].as_bool(), Some(true));
    }

    #[test]
    fn finalize_forces_state_mounts_api_port_and_patches() {
        let edited = "kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\nnetworking:\n  podSubnet: 10.9.0.0/16\nnodes:\n- role: control-plane\n  extraMounts:\n  - hostPath: /evil\n    containerPath: /var/lib/containerd\n  - hostPath: /srv\n    containerPath: /srv\n  extraPortMappings:\n  - containerPort: 80\n    hostPort: 18080\n- role: worker\n";
        let f = finalize_config(edited, |i| format!("/state/node{i}"), Some(16450), true).unwrap();
        assert_eq!(f.nodes, 2);
        assert_eq!(f.host_ports, vec![18080]);
        let doc = &YamlLoader::load_from_str(&f.yaml).unwrap()[0];
        let m0 = doc["nodes"][0]["extraMounts"].as_vec().unwrap();
        assert_eq!(m0.len(), 2);
        assert_eq!(m0[1]["hostPath"].as_str(), Some("/state/node0"));
        assert!(!f.yaml.contains("/evil"));
        assert_eq!(doc["nodes"][1]["extraMounts"][0]["hostPath"].as_str(), Some("/state/node1"));
        assert_eq!(doc["networking"]["apiServerPort"].as_i64(), Some(16450));
        assert_eq!(doc["networking"]["podSubnet"].as_str(), Some("10.9.0.0/16"));
        assert!(doc["kubeadmConfigPatches"][0].as_str().unwrap().contains("kubernetesAPICall"));
        assert!(config_uses_registry(&f.yaml));
        // Idempotent: finalizing the output again changes nothing.
        let again = finalize_config(&f.yaml, |i| format!("/state/node{i}"), Some(16450), true).unwrap();
        assert_eq!(again.yaml, f.yaml);
    }

    #[test]
    fn finalize_rejects_broken_configs() {
        let bad = [
            ("nodes: [", "valid YAML"),
            ("kind: Pod\napiVersion: kind.x-k8s.io/v1alpha4\n", "kind: Cluster"),
            ("kind: Cluster\napiVersion: v1\n", "apiVersion"),
            ("kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\nnodes:\n- role: worker\n", "control-plane"),
            ("kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\nnodes:\n- role: control-plane\n- role: control-plane\n", "odd"),
            ("kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\nnodes:\n- role: boss\n", "role must be"),
        ];
        for (text, needle) in bad {
            let err = finalize_config(text, |i| format!("/s/{i}"), None, false).unwrap_err();
            assert!(err.contains(needle), "{text:?} → {err}");
        }
        // No nodes at all = kind's default single control plane.
        let f = finalize_config("kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\n", |i| format!("/s/{i}"), None, false).unwrap();
        assert_eq!(f.nodes, 1);
    }

    #[test]
    fn older_images_get_no_timeouts_patch() {
        let y = "kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\nnodes:\n- role: control-plane\n  image: kindest/node:v1.29.2\n";
        let f = finalize_config(y, |i| format!("/s/{i}"), None, false).unwrap();
        assert!(!f.yaml.contains("timeouts"));
    }

    #[test]
    fn builtin_presets_are_all_valid() {
        for p in builtin_presets() {
            assert!(validate_spec(&p.spec, all_exist).is_empty(), "{}", p.name);
        }
    }

    #[test]
    fn default_image_is_found_in_binary_bytes() {
        let digest = "a".repeat(64);
        let blob = format!("\0\0junk kindest/node:v1.37.0@sha256:{digest}\0more kindest/node:v1.30.0");
        assert_eq!(
            default_image_in_binary(blob.as_bytes()).unwrap(),
            format!("kindest/node:v1.37.0@sha256:{digest}")
        );
        assert!(default_image_in_binary(b"nothing here").is_none());
    }

    #[test]
    fn node_images_merge_newest_first_with_flags() {
        let out = merge_node_images(
            Some("kindest/node:v1.37.0@sha256:x".into()),
            vec!["kindest/node:v1.31.0".into()],
            vec!["v1.36.1".into(), "v1.31.0".into(), "v1.37.0-rc.1".into(), "latest".into()],
        );
        let versions: Vec<_> = out.iter().map(|o| o.version.as_str()).collect();
        assert_eq!(versions, ["v1.37.0", "v1.36.1", "v1.31.0"]);
        assert!(out[0].default && out[0].image.contains("@sha256"));
        assert!(out[2].local && !out[1].local);
    }
}
