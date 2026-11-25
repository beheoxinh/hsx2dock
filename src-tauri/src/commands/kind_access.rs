//! Reaching kind clusters from outside the app: the API server port the host
//! can connect to, the user's own kubeconfig, a host terminal, manifest
//! diff/apply, and logs of every pod behind a label selector.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::State;
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

use super::kind::{
    apply_yaml_core, control_plane_node, docker_host_for, k8s_name_check, kind_command,
    kubectl_json, kubectl_raw, local_client,
};
use crate::docker::kind::KindPaths;
use crate::state::AppState;

// ==================== API server port ====================
//
// The engine runs behind rootlesskit + pasta, and pasta forwards ports bound
// inside the engine's network namespace to the host — except the ephemeral
// range (`net.ipv4.ip_local_port_range`, 32768–60999). kind picks a random
// ephemeral API port by default, so the host could never reach the cluster.
// New clusters get a fixed port below that range instead.

/// Ports handed to new clusters' API servers.
pub(crate) const API_PORT_FIRST: u16 = 16443;
pub(crate) const API_PORT_LAST: u16 = 16999;

/// Serialises "pick a port, write the config" so two creates in flight never
/// pick the same one.
static PORT_PICK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The lowest port in the API range that no cluster config claims and that
/// `free` reports as unused on the host.
pub(crate) fn pick_api_server_port(taken: &HashSet<u16>, free: impl Fn(u16) -> bool) -> Option<u16> {
    (API_PORT_FIRST..=API_PORT_LAST).find(|p| !taken.contains(p) && free(*p))
}

/// `apiServerPort:` values of every generated cluster config next to the app
/// kubeconfig (stopped clusters hold no socket but still own their port).
fn claimed_api_ports(paths: &KindPaths) -> HashSet<u16> {
    let Some(dir) = paths.kubeconfig.parent() else {
        return HashSet::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with("cluster-config-") && n.ends_with(".yaml")
        })
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|s| config_api_port(&s))
        .collect()
}

/// The `apiServerPort` of a generated kind config.
pub(crate) fn config_api_port(config: &str) -> Option<u16> {
    config.lines().find_map(|l| {
        l.trim()
            .strip_prefix("apiServerPort:")
            .and_then(|v| v.trim().trim_matches('"').parse().ok())
    })
}

fn host_port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// `networking:` block for a new cluster's config, or empty when every port in
/// the range is taken (kind then falls back to a random port).
pub(crate) fn networking_yaml(port: Option<u16>) -> String {
    match port {
        Some(p) => format!("networking:\n  apiServerAddress: \"127.0.0.1\"\n  apiServerPort: {p}\n"),
        None => String::new(),
    }
}

/// kubeadm's 1-minute default for API calls is too short for a node's first
/// boot on a fresh containerd dir (it imports every bundled image before the
/// static pods start): `kind create` failed with "could not bootstrap the
/// admin user" while etcd was still starting. The `timeouts` block only
/// exists in kubeadm v1beta4 (Kubernetes 1.31+), so older node images get
/// no patch.
pub(crate) fn kubeadm_timeouts_patch(image: Option<&str>) -> &'static str {
    const PATCH: &str = "kubeadmConfigPatches:\n  - |\n    kind: InitConfiguration\n    \
                         timeouts:\n      controlPlaneComponentHealthCheck: 8m0s\n      \
                         kubernetesAPICall: 5m0s\n";
    let minor = image
        .and_then(|i| i.rsplit_once(":v1."))
        .and_then(|(_, rest)| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|m| m.parse::<u32>().ok());
    match (image, minor) {
        (None, _) => PATCH,
        (Some(_), Some(m)) if m >= 31 => PATCH,
        _ => "",
    }
}

/// Picks a port and writes the config while holding the pick lock, so the
/// next create sees this one's claim.
pub(crate) fn write_config_with_api_port(
    paths: &KindPaths,
    config_path: &Path,
    nodes_yaml: &str,
    image: Option<&str>,
    bound: &HashSet<u16>,
) -> Result<Option<u16>, String> {
    let _pick = PORT_PICK.lock().unwrap_or_else(|e| e.into_inner());
    let mut taken = claimed_api_ports(paths);
    taken.extend(bound);
    let port = pick_api_server_port(&taken, host_port_free);
    if port.is_none() {
        tracing::warn!(
            "no free API server port in {API_PORT_FIRST}-{API_PORT_LAST}; kind picks one \
             (it may not be reachable from the host)"
        );
    }
    let config_yaml = format!(
        "kind: Cluster\napiVersion: kind.x-k8s.io/v1alpha4\n{}{}nodes:\n{nodes_yaml}",
        networking_yaml(port),
        kubeadm_timeouts_patch(image),
    );
    std::fs::write(config_path, config_yaml)
        .map_err(|e| format!("failed to write kind cluster config: {e}"))?;
    Ok(port)
}

/// Whether pasta skips forwarding `port` (the host can't reach it).
pub(crate) fn port_is_ephemeral(port: u16) -> bool {
    let (lo, hi) = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
        .ok()
        .and_then(|s| {
            let mut it = s.split_whitespace().filter_map(|v| v.parse::<u16>().ok());
            Some((it.next()?, it.next()?))
        })
        .unwrap_or((32768, 60999));
    (lo..=hi).contains(&port)
}

/// API ports published by every existing control-plane node, running or not.
/// A stopped cluster holds no socket, and one created outside the app has no
/// config file here, but either would clash with a new cluster on its port.
pub(crate) async fn bound_api_ports(state: &AppState) -> HashSet<u16> {
    let Ok(client) = local_client(state).await else {
        return HashSet::new();
    };
    let mut filters = std::collections::HashMap::new();
    filters.insert(
        "label".to_string(),
        vec!["io.x-k8s.kind.role=control-plane".to_string()],
    );
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .all(true)
        .filters(&filters)
        .build();
    let Ok(list) = client.docker().list_containers(Some(opts)).await else {
        return HashSet::new();
    };
    let mut ports = HashSet::new();
    for id in list.into_iter().filter_map(|c| c.id) {
        if let Ok(info) = client.docker().inspect_container(&id, None).await {
            ports.extend(
                info.host_config
                    .and_then(|h| h.port_bindings)
                    .and_then(|m| m.get("6443/tcp").cloned().flatten())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|b| b.host_port.as_deref().and_then(|p| p.parse::<u16>().ok())),
            );
        }
    }
    ports
}

/// Host port the control-plane node publishes the API server (6443) on.
async fn api_server_host_port(state: &AppState, cluster: &str) -> Result<Option<u16>, String> {
    let client = local_client(state).await?;
    let info = client
        .docker()
        .inspect_container(&control_plane_node(cluster), None)
        .await
        .map_err(|e| format!("cluster '{cluster}' not found: {e}"))?;
    let from_host_config = info
        .host_config
        .as_ref()
        .and_then(|h| h.port_bindings.as_ref())
        .and_then(|m| m.get("6443/tcp").cloned().flatten());
    let from_network = info
        .network_settings
        .as_ref()
        .and_then(|n| n.ports.as_ref())
        .and_then(|m| m.get("6443/tcp").cloned().flatten());
    Ok(from_host_config
        .or(from_network)
        .unwrap_or_default()
        .iter()
        .find_map(|b| b.host_port.as_deref().and_then(|p| p.parse().ok())))
}

async fn api_reachable(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            std::time::Duration::from_millis(1500),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await,
        Ok(Ok(_))
    )
}

// ==================== Host kubeconfig ====================

/// The kubeconfig kubectl on the host reads first: the first `$KUBECONFIG`
/// entry, else `~/.kube/config`.
pub(crate) fn host_kubeconfig_path() -> Option<PathBuf> {
    if let Some(first) = std::env::var_os("KUBECONFIG")
        .map(|v| v.to_string_lossy().into_owned())
        .and_then(|v| v.split(':').find(|s| !s.is_empty()).map(PathBuf::from))
    {
        return Some(first);
    }
    dirs::home_dir().map(|h| h.join(".kube").join("config"))
}

/// One edit of the user's kubeconfig at a time (export and remove both
/// rewrite the whole file).
static HOST_KUBECONFIG: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Context names in a kubeconfig document.
pub(crate) fn kubeconfig_contexts(text: &str) -> Vec<String> {
    let Ok(docs) = YamlLoader::load_from_str(text) else {
        return Vec::new();
    };
    let Some(doc) = docs.first() else {
        return Vec::new();
    };
    doc["contexts"]
        .as_vec()
        .map(|v| {
            v.iter()
                .filter_map(|c| c["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Drops the cluster, user and context named `name` (kind names all three
/// `kind-<cluster>`). When `current-context` pointed there it goes back to
/// `previous` if that context still exists, else it is cleared.
/// Returns the new document and whether anything was removed.
pub(crate) fn remove_kubeconfig_entries(
    text: &str,
    name: &str,
    previous: Option<&str>,
) -> Result<(String, bool), String> {
    let mut docs =
        YamlLoader::load_from_str(text).map_err(|e| format!("kubeconfig is not valid YAML: {e}"))?;
    let Some(Yaml::Hash(root)) = docs.first_mut() else {
        return Ok((text.to_string(), false));
    };
    let mut removed = false;
    for section in ["clusters", "contexts", "users"] {
        if let Some(Yaml::Array(items)) = root.get_mut(&Yaml::String(section.into())) {
            let before = items.len();
            items.retain(|i| i["name"].as_str() != Some(name));
            removed |= items.len() != before;
        }
    }
    let current = Yaml::String("current-context".into());
    if root.get(&current).and_then(Yaml::as_str) == Some(name) {
        let still_there = |ctx: &str| {
            root.get(&Yaml::String("contexts".into()))
                .and_then(Yaml::as_vec)
                .is_some_and(|v| v.iter().any(|c| c["name"].as_str() == Some(ctx)))
        };
        let next = previous.filter(|p| *p != name && still_there(p)).unwrap_or("");
        root.insert(current, Yaml::String(next.to_string()));
        removed = true;
    }
    if !removed {
        return Ok((text.to_string(), false));
    }
    let mut out = String::new();
    YamlEmitter::new(&mut out)
        .dump(&docs[0])
        .map_err(|e| format!("could not write kubeconfig: {e}"))?;
    // The emitter starts with a document marker kubectl doesn't need.
    let out = out.strip_prefix("---\n").unwrap_or(&out).to_string();
    Ok((out + "\n", true))
}

/// Where the context that was current before an export is remembered, per
/// exported context, so removing the cluster can switch back to it.
fn previous_contexts_path(paths: &KindPaths) -> Option<PathBuf> {
    paths
        .kubeconfig
        .parent()
        .map(|d| d.join("host-kubeconfig-previous.json"))
}

fn load_previous_contexts(paths: &KindPaths) -> serde_json::Map<String, serde_json::Value> {
    previous_contexts_path(paths)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn save_previous_contexts(paths: &KindPaths, map: &serde_json::Map<String, serde_json::Value>) {
    if let Some(p) = previous_contexts_path(paths) {
        let _ = std::fs::write(p, serde_json::Value::Object(map.clone()).to_string());
    }
}

/// `current-context` of a kubeconfig document ("" when unset).
pub(crate) fn kubeconfig_current(text: &str) -> String {
    YamlLoader::load_from_str(text)
        .ok()
        .and_then(|d| d.first().and_then(|d| d["current-context"].as_str().map(str::to_string)))
        .unwrap_or_default()
}

/// Copies the kubeconfig aside before the app rewrites it.
fn backup_kubeconfig(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = path.with_file_name(format!(
        "{}.hsx2dock.bak",
        path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
    ));
    std::fs::copy(path, &backup)
        .map_err(|e| format!("could not back up {}: {e}", path.display()))?;
    Ok(Some(backup))
}

/// Writes through a temp file + rename so a crash never leaves half a config,
/// and keeps it private (it holds client keys).
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("hsx2dock.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterAccess {
    /// Context (and cluster/user) name kind uses: `kind-<cluster>`.
    pub context: String,
    /// Host port of the API server, when the node is known.
    pub api_port: Option<u16>,
    /// A TCP connect from the host to the API port succeeded.
    pub api_reachable: bool,
    /// The port is in the range the engine's network never forwards — the
    /// cluster must be recreated to be reachable from the host.
    pub ephemeral_port: bool,
    /// The user's kubeconfig (`$KUBECONFIG` or `~/.kube/config`).
    pub host_kubeconfig: String,
    pub in_host_kubeconfig: bool,
    /// Whether that kubeconfig's current context is this cluster.
    pub host_current: bool,
    /// kubectl found on the host `PATH`.
    pub host_kubectl: bool,
}

#[tauri::command]
pub async fn get_cluster_access(
    state: State<'_, Arc<AppState>>,
    cluster: String,
) -> Result<ClusterAccess, String> {
    get_cluster_access_core(&state, &cluster).await
}

pub(crate) async fn get_cluster_access_core(
    state: &AppState,
    cluster: &str,
) -> Result<ClusterAccess, String> {
    let cluster = cluster.trim();
    super::kind::check_cluster_name(cluster)?;
    let context = format!("kind-{cluster}");
    let api_port = api_server_host_port(state, cluster).await?;
    let reachable = match api_port {
        Some(p) => api_reachable(p).await,
        None => false,
    };
    let host = host_kubeconfig_path();
    let text = host
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    let in_host = kubeconfig_contexts(&text).iter().any(|c| c == &context);
    let host_current = kubeconfig_current(&text) == context;
    Ok(ClusterAccess {
        api_port,
        api_reachable: reachable,
        ephemeral_port: api_port.is_some_and(port_is_ephemeral),
        host_kubeconfig: host.map(|p| p.display().to_string()).unwrap_or_default(),
        in_host_kubeconfig: in_host,
        host_current,
        host_kubectl: which("kubectl").is_some(),
        context,
    })
}

fn unreachable_reason(cluster: &str, access: &ClusterAccess) -> String {
    match access.api_port {
        Some(p) if access.ephemeral_port => format!(
            "cluster '{cluster}' publishes its API server on port {p}, inside the range the \
             engine's network never forwards to the host; recreate the cluster to get a \
             reachable port"
        ),
        Some(p) => format!("the API server of '{cluster}' is not answering on 127.0.0.1:{p} — is the cluster running?"),
        None => format!("cluster '{cluster}' has no published API server port"),
    }
}

/// Merges the cluster's context into the user's kubeconfig (and makes it the
/// current context — that is what `kind export kubeconfig` does). The file
/// is backed up to `<file>.hsx2dock.bak` first.
#[tauri::command]
pub async fn export_kubeconfig_to_host(
    state: State<'_, Arc<AppState>>,
    cluster: String,
) -> Result<String, String> {
    export_kubeconfig_to_host_core(&state, &cluster).await
}

pub(crate) async fn export_kubeconfig_to_host_core(
    state: &AppState,
    cluster: &str,
) -> Result<String, String> {
    let cluster = cluster.trim();
    let access = get_cluster_access_core(state, cluster).await?;
    if !access.api_reachable {
        return Err(unreachable_reason(cluster, &access));
    }
    let path = host_kubeconfig_path().ok_or("no home directory for ~/.kube/config")?;
    let _edit = HOST_KUBECONFIG.lock().await;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let before = kubeconfig_current(&std::fs::read_to_string(&path).unwrap_or_default());
    let backup = backup_kubeconfig(&path)?;
    kind_export(state, cluster, &path).await?;
    // kind makes the cluster the current context; remember what it replaced.
    if !before.is_empty() && before != access.context {
        let mut prev = load_previous_contexts(&state.kind);
        prev.insert(access.context.clone(), serde_json::Value::String(before));
        save_previous_contexts(&state.kind, &prev);
    }
    Ok(match backup {
        Some(b) => format!("Context {} added to {} (backup: {})", access.context, path.display(), b.display()),
        None => format!("Context {} written to {}", access.context, path.display()),
    })
}

/// `kind export kubeconfig --name <cluster> --kubeconfig <path>`.
async fn kind_export(state: &AppState, cluster: &str, path: &Path) -> Result<(), String> {
    let paths = &state.kind;
    if !paths.is_installed() {
        return Err("kind is not installed yet".into());
    }
    let mut cmd = kind_command(paths, &docker_host_for(state));
    cmd.args(["export", "kubeconfig", "--name", cluster, "--kubeconfig"]);
    cmd.arg(path);
    let out = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| "kind export kubeconfig timed out".to_string())?
        .map_err(|e| format!("run kind export kubeconfig: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "kind export kubeconfig failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Removes the cluster's context, cluster and user from the user's kubeconfig
/// (backed up first). Other entries are kept; comments are not.
#[tauri::command]
pub async fn remove_kubeconfig_from_host(
    state: State<'_, Arc<AppState>>,
    cluster: String,
) -> Result<String, String> {
    remove_kubeconfig_from_host_core(&state, &cluster).await
}

pub(crate) async fn remove_kubeconfig_from_host_core(
    state: &AppState,
    cluster: &str,
) -> Result<String, String> {
    let cluster = cluster.trim();
    super::kind::check_cluster_name(cluster)?;
    let context = format!("kind-{cluster}");
    let path = host_kubeconfig_path().ok_or("no home directory for ~/.kube/config")?;
    let _edit = HOST_KUBECONFIG.lock().await;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!("{} does not exist", path.display()))
        }
        Err(e) => return Err(format!("read {}: {e}", path.display())),
    };
    let mut prev = load_previous_contexts(&state.kind);
    let previous = prev.get(&context).and_then(|v| v.as_str()).map(str::to_string);
    let (next, removed) = remove_kubeconfig_entries(&text, &context, previous.as_deref())?;
    if !removed {
        return Ok(format!("{context} is not in {}", path.display()));
    }
    let backup = backup_kubeconfig(&path)?;
    write_private(&path, &next)?;
    if prev.remove(&context).is_some() {
        save_previous_contexts(&state.kind, &prev);
    }
    let now = kubeconfig_current(&next);
    Ok(format!(
        "Removed {context} from {}{}{}",
        path.display(),
        if now.is_empty() { String::new() } else { format!("; current context is {now} again") },
        backup.map(|b| format!(" (backup: {})", b.display())).unwrap_or_default()
    ))
}

// ==================== Host terminal ====================

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join(bin))
            .find(|f| f.is_file())
    })
}

/// Terminal emulators tried in order, with the flag that runs a command.
const TERMINALS: &[(&str, &[&str])] = &[
    ("ptyxis", &["--new-window", "--"]),
    ("kgx", &["--"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("tilix", &["-e"]),
    ("alacritty", &["-e"]),
    ("kitty", &[]),
    ("wezterm", &["start", "--"]),
    ("foot", &[]),
    ("xterm", &["-e"]),
];

/// Per-cluster kubeconfig the host terminal points at, so it neither depends
/// on nor changes the user's `~/.kube/config`.
fn terminal_kubeconfig_path(paths: &KindPaths, cluster: &str) -> Option<PathBuf> {
    paths
        .kubeconfig
        .parent()
        .map(|d| d.join("kubeconfigs").join(format!("{cluster}.yaml")))
}

/// Shell that greets, then hands over to the user's login shell with
/// `KUBECONFIG` pointing at the cluster.
pub(crate) fn terminal_script(kubeconfig: &str, context: &str) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    format!(
        "export KUBECONFIG={kc}; \
         printf '\\033[1mkubectl → %s\\033[0m  (KUBECONFIG=%s)\\n' {ctx} {kc}; \
         command -v kubectl >/dev/null 2>&1 || echo 'kubectl is not installed on this machine.'; \
         exec \"${{SHELL:-/bin/sh}}\" -l",
        kc = q(kubeconfig),
        ctx = q(context),
    )
}

/// Opens a terminal window on the host with `KUBECONFIG` set to this cluster.
#[tauri::command]
pub async fn open_cluster_terminal(
    state: State<'_, Arc<AppState>>,
    cluster: String,
) -> Result<String, String> {
    let cluster = cluster.trim().to_string();
    let access = get_cluster_access_core(&state, &cluster).await?;
    if !access.api_reachable {
        return Err(unreachable_reason(&cluster, &access));
    }
    let kc = terminal_kubeconfig_path(&state.kind, &cluster)
        .ok_or("kind data directory is unavailable")?;
    if let Some(dir) = kc.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    // Rewritten every time: a recreated cluster has new certificates.
    let _ = std::fs::remove_file(&kc);
    kind_export(&state, &cluster, &kc).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&kc, std::fs::Permissions::from_mode(0o600));
    }

    let (term, flags) = TERMINALS
        .iter()
        .find(|(t, _)| which(t).is_some())
        .ok_or("no terminal emulator found (tried ptyxis, gnome-terminal, konsole, xterm, …)")?;
    let script = terminal_script(&kc.display().to_string(), &access.context);
    let mut argv: Vec<String> = vec![term.to_string()];
    argv.extend(flags.iter().map(|s| s.to_string()));
    argv.extend(["sh".into(), "-c".into(), script]);

    // Outside the app's own unit when systemd is there: stopping the app
    // must not close the user's terminal.
    let mut cmd = if which("systemd-run").is_some() {
        let mut c = std::process::Command::new("systemd-run");
        c.args(["--user", "--collect", "--quiet", "--scope", "--"]);
        c.args(&argv);
        c
    } else {
        let mut c = std::process::Command::new(&argv[0]);
        c.args(&argv[1..]);
        c
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("could not start {term}: {e}"))?;
    // Reap it in the background; a scope wrapper exits when the terminal does.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(format!("Opened {term} with context {}", access.context))
}

// ==================== Manifests: diff and apply ====================

/// One argv string is capped at 128 KiB by the kernel and the manifest rides
/// base64 (4/3 larger) inside one, so the raw YAML must stay below this.
pub(crate) const MANIFEST_MAX_BYTES: usize = 90 * 1024;

fn check_manifest(yaml: &str) -> Result<(), String> {
    if yaml.trim().is_empty() {
        return Err("Manifest yaml is required".into());
    }
    if yaml.len() > MANIFEST_MAX_BYTES {
        return Err(format!(
            "manifest is {} KiB; the limit is {} KiB — split it into several applies",
            yaml.len() / 1024,
            MANIFEST_MAX_BYTES / 1024
        ));
    }
    Ok(())
}

/// `kubectl diff` exits 1 when there are differences, so the script maps 0
/// and 1 to success and keeps real failures (>1) as errors.
pub(crate) fn diff_script(encoded: &str) -> String {
    format!(
        "printf '%s' '{encoded}' | base64 -d | kubectl --kubeconfig=/etc/kubernetes/admin.conf \
         --request-timeout=25s diff -f -; rc=$?; [ \"$rc\" -le 1 ] || exit \"$rc\""
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestDiff {
    /// Unified diff of live vs. the server-side dry-run of the manifest.
    pub diff: String,
    pub changed: bool,
}

#[tauri::command]
pub async fn diff_cluster_yaml(
    state: State<'_, Arc<AppState>>,
    cluster: String,
    yaml: String,
) -> Result<ManifestDiff, String> {
    diff_cluster_yaml_core(&state, &cluster, &yaml).await
}

pub(crate) async fn diff_cluster_yaml_core(
    state: &AppState,
    cluster: &str,
    yaml: &str,
) -> Result<ManifestDiff, String> {
    use base64::Engine as _;
    check_manifest(yaml)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(yaml.as_bytes());
    let client = local_client(state).await?;
    let out = client
        .exec_capture(
            &control_plane_node(cluster),
            vec!["sh".into(), "-c".into(), diff_script(&encoded)],
        )
        .await
        .map_err(|e| format!("kubectl diff: {e}"))?;
    let diff = out.trim_end().to_string();
    Ok(ManifestDiff {
        changed: diff.lines().any(|l| l.starts_with("+++") || l.starts_with("---")),
        diff,
    })
}

#[tauri::command]
pub async fn apply_cluster_yaml(
    state: State<'_, Arc<AppState>>,
    cluster: String,
    yaml: String,
) -> Result<String, String> {
    apply_cluster_yaml_core(&state, &cluster, &yaml).await
}

pub(crate) async fn apply_cluster_yaml_core(
    state: &AppState,
    cluster: &str,
    yaml: &str,
) -> Result<String, String> {
    check_manifest(yaml)?;
    apply_yaml_core(state, cluster, yaml).await
}

// ==================== Logs of every pod behind a selector ====================

/// Label selectors: keys, values and the set-based operators only — anything
/// that could reach kubectl as a flag or a shell word is refused.
pub(crate) fn check_selector(sel: &str) -> Result<(), String> {
    let ok = !sel.is_empty()
        && sel.len() <= 512
        && !sel.starts_with('-')
        && sel
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=!,() ".contains(c));
    if ok {
        Ok(())
    } else {
        Err(format!("invalid label selector '{sel}'"))
    }
}

/// `matchLabels` of a workload as a selector string (`a=b,c=d`, sorted).
pub(crate) fn selector_from_match_labels(v: &serde_json::Value) -> Option<String> {
    let labels = v.pointer("/spec/selector/matchLabels")?.as_object()?;
    if labels.is_empty() {
        return None;
    }
    let mut pairs: Vec<String> = labels
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|v| format!("{k}={v}")))
        .collect();
    pairs.sort();
    Some(pairs.join(","))
}

/// `kubectl logs --prefix` lines are `[pod/<pod>/<container>] <ts> <msg>`;
/// moving the timestamp first lets the log views sort and merge them like
/// container logs.
pub(crate) fn timestamp_first(line: &str) -> String {
    if let Some(rest) = line.strip_prefix('[') {
        if let Some((prefix, tail)) = rest.split_once("] ") {
            if let Some((ts, msg)) = tail.split_once(' ') {
                if ts.len() >= 20 && ts.as_bytes()[4] == b'-' {
                    return format!("{ts} [{prefix}] {msg}");
                }
            }
        }
    }
    line.to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectorLogs {
    /// The selector that was used (resolved from the workload when given).
    pub selector: String,
    /// Lines `<RFC3339 ts> [pod/<pod>/<container>] <msg>`, oldest first.
    pub lines: String,
}

/// Logs of every pod matching `selector` — or the selector of the workload
/// `workload_kind/workload_name` — across all their containers.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn get_selector_logs(
    state: State<'_, Arc<AppState>>,
    cluster: String,
    namespace: String,
    selector: Option<String>,
    workload_kind: Option<String>,
    workload_name: Option<String>,
    tail: Option<u32>,
    since_time: Option<String>,
) -> Result<SelectorLogs, String> {
    get_selector_logs_core(
        &state,
        &cluster,
        &namespace,
        selector,
        workload_kind.zip(workload_name),
        tail,
        since_time,
    )
    .await
}

pub(crate) async fn get_selector_logs_core(
    state: &AppState,
    cluster: &str,
    namespace: &str,
    selector: Option<String>,
    workload: Option<(String, String)>,
    tail: Option<u32>,
    since_time: Option<String>,
) -> Result<SelectorLogs, String> {
    k8s_name_check(namespace)?;
    let selector = match (selector.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()), workload) {
        (Some(s), _) => s,
        (None, Some((kind, name))) => {
            k8s_name_check(&name)?;
            let kind = kind.trim().to_lowercase();
            if !["deployment", "statefulset", "daemonset", "replicaset", "job"].contains(&kind.as_str()) {
                return Err(format!("{kind} has no pod selector"));
            }
            let target = format!("{kind}/{name}");
            let v = kubectl_json(state, cluster, &["get", &target, "-n", namespace, "-o", "json"]).await?;
            selector_from_match_labels(&v)
                .ok_or_else(|| format!("{target} has no matchLabels selector"))?
        }
        (None, None) => return Err("a selector or a workload is required".into()),
    };
    check_selector(&selector)?;
    let tail = tail.unwrap_or(200).clamp(10, 5000);
    let mut args: Vec<String> = vec![
        "logs".into(),
        "-l".into(),
        selector.clone(),
        "-n".into(),
        namespace.into(),
        "--all-containers=true".into(),
        "--prefix=true".into(),
        "--timestamps=true".into(),
        "--ignore-errors=true".into(),
        "--max-log-requests=20".into(),
        format!("--tail={tail}"),
    ];
    if let Some(since) = since_time.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        chrono::DateTime::parse_from_rfc3339(since)
            .map_err(|_| format!("since_time '{since}' is not an RFC 3339 timestamp"))?;
        args.push(format!("--since-time={since}"));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = kubectl_raw(state, cluster, &refs).await?;
    let mut lines: Vec<String> = out.lines().filter(|l| !l.trim().is_empty()).map(timestamp_first).collect();
    // Pods are read one after another; interleave them by time.
    lines.sort_by(|a, b| a.split(' ').next().cmp(&b.split(' ').next()));
    Ok(SelectorLogs {
        selector,
        lines: lines.join("\n"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_port_skips_claimed_and_busy_ports() {
        let taken: HashSet<u16> = [16443, 16444].into_iter().collect();
        assert_eq!(pick_api_server_port(&taken, |p| p != 16445), Some(16446));
        assert_eq!(pick_api_server_port(&HashSet::new(), |_| false), None);
        assert!(!port_is_ephemeral(API_PORT_LAST) || API_PORT_LAST >= 32768);
    }

    #[test]
    fn kubeadm_timeouts_only_for_v1beta4_images() {
        assert!(kubeadm_timeouts_patch(None).contains("kubernetesAPICall: 5m0s"));
        assert!(!kubeadm_timeouts_patch(Some("kindest/node:v1.37.0")).is_empty());
        assert!(!kubeadm_timeouts_patch(Some("kindest/node:v1.31.2@sha256:ab")).is_empty());
        assert_eq!(kubeadm_timeouts_patch(Some("kindest/node:v1.29.4")), "");
        assert_eq!(kubeadm_timeouts_patch(Some("my/node:latest")), "");
        // Valid YAML that kind reads as one patch document.
        let doc = &YamlLoader::load_from_str(kubeadm_timeouts_patch(None)).unwrap()[0];
        let patch = doc["kubeadmConfigPatches"][0].as_str().unwrap();
        assert!(patch.starts_with("kind: InitConfiguration\ntimeouts:\n"));
    }

    #[test]
    fn generated_config_carries_its_api_port() {
        let yaml = format!("kind: Cluster\n{}nodes:\n", networking_yaml(Some(16450)));
        assert!(yaml.contains("apiServerAddress: \"127.0.0.1\""));
        assert_eq!(config_api_port(&yaml), Some(16450));
        assert_eq!(networking_yaml(None), "");
        assert_eq!(config_api_port("kind: Cluster\nnodes:\n"), None);
    }

    const KUBECONFIG: &str = "apiVersion: v1
clusters:
- cluster:
    server: https://127.0.0.1:16443
  name: kind-a
- cluster:
    server: https://prod.example:6443
  name: prod
contexts:
- context:
    cluster: kind-a
    user: kind-a
  name: kind-a
- context:
    cluster: prod
    user: admin
  name: prod
current-context: kind-a
kind: Config
users:
- name: kind-a
  user:
    token: x
- name: admin
  user:
    token: y
";

    #[test]
    fn removing_a_context_keeps_every_other_entry() {
        assert_eq!(kubeconfig_contexts(KUBECONFIG), vec!["kind-a", "prod"]);
        let (out, removed) = remove_kubeconfig_entries(KUBECONFIG, "kind-a", None).unwrap();
        assert!(removed);
        assert_eq!(kubeconfig_contexts(&out), vec!["prod"]);
        let doc = &YamlLoader::load_from_str(&out).unwrap()[0];
        assert_eq!(doc["clusters"].as_vec().unwrap().len(), 1);
        assert_eq!(doc["users"][0]["name"].as_str(), Some("admin"));
        assert_eq!(doc["current-context"].as_str(), Some(""));
        assert_eq!(doc["clusters"][0]["cluster"]["server"].as_str(), Some("https://prod.example:6443"));
    }

    #[test]
    fn removing_an_absent_context_leaves_the_text_untouched() {
        let (out, removed) = remove_kubeconfig_entries(KUBECONFIG, "kind-zzz", None).unwrap();
        assert!(!removed);
        assert_eq!(out, KUBECONFIG);
        assert!(remove_kubeconfig_entries("a: [", "x", None).is_err());
    }

    #[test]
    fn removing_the_current_context_switches_back_to_the_previous_one() {
        let (out, _) = remove_kubeconfig_entries(KUBECONFIG, "kind-a", Some("prod")).unwrap();
        assert_eq!(kubeconfig_current(&out), "prod");
        // A previous context that is gone since is not restored.
        let (out, _) = remove_kubeconfig_entries(KUBECONFIG, "kind-a", Some("gone")).unwrap();
        assert_eq!(kubeconfig_current(&out), "");
        assert_eq!(kubeconfig_current(KUBECONFIG), "kind-a");
    }

    #[test]
    fn diff_script_tolerates_exit_one_only() {
        let s = diff_script("QUJD");
        assert!(s.contains("diff -f -"));
        assert!(s.contains("[ \"$rc\" -le 1 ] || exit \"$rc\""));
    }

    #[test]
    fn manifest_size_is_bounded() {
        assert!(check_manifest("  ").is_err());
        assert!(check_manifest("kind: Namespace").is_ok());
        assert!(check_manifest(&"a".repeat(MANIFEST_MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn selectors_reject_flags_and_shell_words() {
        assert!(check_selector("app=web,tier!=db").is_ok());
        assert!(check_selector("env in (prod, dev)").is_ok());
        assert!(check_selector("app.kubernetes.io/name=x").is_ok());
        assert!(check_selector("--all").is_err());
        assert!(check_selector("app=$(id)").is_err());
        assert!(check_selector("a;b").is_err());
        assert!(check_selector("").is_err());
    }

    #[test]
    fn match_labels_become_a_sorted_selector() {
        let v = serde_json::json!({"spec": {"selector": {"matchLabels": {"tier": "web", "app": "shop"}}}});
        assert_eq!(selector_from_match_labels(&v).as_deref(), Some("app=shop,tier=web"));
        assert_eq!(selector_from_match_labels(&serde_json::json!({"spec": {}})), None);
    }

    #[test]
    fn prefixed_log_lines_lead_with_the_timestamp() {
        assert_eq!(
            timestamp_first("[pod/web-1/nginx] 2026-11-25T05:07:56.592092330Z GET /"),
            "2026-11-25T05:07:56.592092330Z [pod/web-1/nginx] GET /"
        );
        assert_eq!(timestamp_first("plain line"), "plain line");
        assert_eq!(timestamp_first("[pod/x/y] short msg"), "[pod/x/y] short msg");
    }

    #[test]
    fn terminal_script_quotes_paths() {
        let s = terminal_script("/home/o'neil/k.yaml", "kind-a");
        assert!(s.contains(r"export KUBECONFIG='/home/o'\''neil/k.yaml'"));
        assert!(s.ends_with("-l"));
    }
}
