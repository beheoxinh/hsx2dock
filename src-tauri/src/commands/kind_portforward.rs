//! Port-forward manager: background `kubectl port-forward` processes on the
//! host, restarted when they drop (pod restart, API blip), stopped with their
//! cluster or the app. Saved forwards can come back when the cluster starts.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::k8s_tools::{tool_command, Tool};
use crate::state::AppState;

/// Host ports handed out automatically: below the ephemeral range pasta does
/// not forward, above the API server range.
const AUTO_PORT_FIRST: u16 = 20000;
const AUTO_PORT_LAST: u16 = 32767;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ForwardSpec {
    pub cluster: String,
    pub namespace: String,
    /// `svc` or `pod`.
    pub kind: String,
    pub name: String,
    pub remote_port: u16,
    /// 0 = pick a free one.
    #[serde(default)]
    pub local_port: u16,
    /// Start again whenever the cluster starts.
    #[serde(default)]
    pub auto_start: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardInfo {
    pub id: String,
    #[serde(flatten)]
    pub spec: ForwardSpec,
    /// starting | active | reconnecting | stopped | error
    pub status: String,
    pub message: Option<String>,
    pub restarts: u32,
    pub url: String,
}

struct Entry {
    info: ForwardInfo,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

fn registry() -> &'static std::sync::Mutex<HashMap<String, Entry>> {
    static R: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Entry>>> = std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

pub(crate) fn forward_id(s: &ForwardSpec) -> String {
    format!("{}/{}/{}/{}:{}", s.cluster, s.namespace, s.kind, s.name, s.remote_port)
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

pub(crate) fn check_spec(s: &ForwardSpec) -> Result<(), String> {
    super::kind::check_cluster_name(&s.cluster)?;
    if !valid_name(&s.namespace) || !valid_name(&s.name) {
        return Err("namespace and name must be valid Kubernetes names".into());
    }
    if !matches!(s.kind.as_str(), "svc" | "pod" | "deployment") {
        return Err("kind must be svc, pod or deployment".into());
    }
    if s.remote_port == 0 {
        return Err("remote port is required".into());
    }
    if s.local_port != 0 && super::kind_access::port_is_ephemeral(s.local_port) {
        return Err(format!(
            "local port {} is in the ephemeral range; pick one below 32768",
            s.local_port
        ));
    }
    Ok(())
}

fn port_free(p: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", p)).is_ok()
}

/// First free port at or after a stable start derived from the target, so a
/// forward tends to get the same port each time.
pub(crate) fn pick_local_port(spec: &ForwardSpec, used: &[u16], free: impl Fn(u16) -> bool) -> Option<u16> {
    let span = (AUTO_PORT_LAST - AUTO_PORT_FIRST) as u32 + 1;
    let hash = forward_id(spec).bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let start = hash % span;
    (0..span)
        .map(|i| AUTO_PORT_FIRST + ((start + i) % span) as u16)
        .find(|p| !used.contains(p) && free(*p))
}

fn set_status(id: &str, status: &str, message: Option<String>, restart: bool) {
    let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(e) = r.get_mut(id) {
        e.info.status = status.into();
        e.info.message = message;
        if restart {
            e.info.restarts += 1;
        }
    }
}

#[tauri::command]
pub async fn start_port_forward(
    state: State<'_, Arc<AppState>>,
    spec: ForwardSpec,
) -> Result<ForwardInfo, String> {
    start_port_forward_core(state.inner().clone(), spec).await
}

pub(crate) async fn start_port_forward_core(state: Arc<AppState>, mut spec: ForwardSpec) -> Result<ForwardInfo, String> {
    check_spec(&spec)?;
    let id = forward_id(&spec);
    {
        let r = registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(e) = r.get(&id) {
            if e.stop.is_some() {
                return Ok(e.info.clone());
            }
        }
    }
    let used: Vec<u16> = {
        let r = registry().lock().unwrap_or_else(|e| e.into_inner());
        r.values().filter(|e| e.stop.is_some()).map(|e| e.info.spec.local_port).collect()
    };
    if spec.local_port == 0 {
        spec.local_port = pick_local_port(&spec, &used, port_free).ok_or("no free local port")?;
    } else if used.contains(&spec.local_port) || !port_free(spec.local_port) {
        return Err(format!("local port {} is already in use", spec.local_port));
    }
    // Fail fast (tool download, kubeconfig) before reporting it started.
    let _probe = tool_command(&state, Tool::Kubectl, &spec.cluster).await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let info = ForwardInfo {
        id: id.clone(),
        url: format!("http://127.0.0.1:{}", spec.local_port),
        spec: spec.clone(),
        status: "starting".into(),
        message: None,
        restarts: 0,
    };
    registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.clone(), Entry { info: info.clone(), stop: Some(tx) });
    if spec.auto_start {
        let mut saved = load_saved();
        saved.retain(|s| forward_id(s) != id);
        saved.push(spec.clone());
        store_saved(&saved);
    }
    tauri::async_runtime::spawn(run_forward(state, spec, id, rx));
    Ok(info)
}

async fn run_forward(state: Arc<AppState>, spec: ForwardSpec, id: String, mut stop: tokio::sync::oneshot::Receiver<()>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let mut cmd = match tool_command(&state, Tool::Kubectl, &spec.cluster).await {
            Ok(c) => c,
            Err(e) => {
                set_status(&id, "error", Some(e), false);
                return;
            }
        };
        cmd.args([
            "port-forward",
            "--address",
            "127.0.0.1",
            "-n",
            &spec.namespace,
            &format!("{}/{}", spec.kind, spec.name),
            &format!("{}:{}", spec.local_port, spec.remote_port),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                set_status(&id, "error", Some(e.to_string()), false);
                return;
            }
        };
        let started = std::time::Instant::now();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let id_out = id.clone();
        if let Some(out) = stdout {
            tauri::async_runtime::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    if l.starts_with("Forwarding from") {
                        set_status(&id_out, "active", None, false);
                    }
                }
            });
        }
        let last_err = Arc::new(std::sync::Mutex::new(String::new()));
        if let Some(err) = stderr {
            let sink = last_err.clone();
            tauri::async_runtime::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    *sink.lock().unwrap_or_else(|e| e.into_inner()) = l;
                }
            });
        }
        tokio::select! {
            _ = &mut stop => {
                let _ = child.kill().await;
                set_status(&id, "stopped", None, false);
                return;
            }
            _ = child.wait() => {}
        }
        let msg = last_err.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // A forward that ran a while earned a fast retry.
        if started.elapsed() > Duration::from_secs(30) {
            backoff = Duration::from_secs(1);
        }
        set_status(&id, "reconnecting", (!msg.is_empty()).then_some(msg), true);
        tokio::select! {
            _ = &mut stop => {
                set_status(&id, "stopped", None, false);
                return;
            }
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

#[tauri::command]
pub fn stop_port_forward(id: String, forget: Option<bool>) -> Result<(), String> {
    stop_port_forward_core(&id, forget.unwrap_or(true))
}

pub(crate) fn stop_port_forward_core(id: &str, forget: bool) -> Result<(), String> {
    let entry = registry().lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    if let Some(Entry { stop: Some(tx), .. }) = entry {
        let _ = tx.send(());
    }
    if forget {
        let mut saved = load_saved();
        let before = saved.len();
        saved.retain(|s| forward_id(s) != id);
        if saved.len() != before {
            store_saved(&saved);
        }
    }
    Ok(())
}

/// Stops every forward of a cluster (cluster stopped/deleted); saved ones stay saved.
pub(crate) fn stop_cluster_forwards(cluster: &str) {
    let ids: Vec<String> = registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .filter(|e| e.info.spec.cluster == cluster)
        .map(|e| e.info.id.clone())
        .collect();
    for id in ids {
        let _ = stop_port_forward_core(&id, false);
    }
}

/// App exit: stop everything.
pub(crate) fn stop_all_forwards() {
    let ids: Vec<String> = registry().lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect();
    for id in ids {
        let _ = stop_port_forward_core(&id, false);
    }
}

#[tauri::command]
pub fn list_port_forwards(cluster: Option<String>) -> Vec<ForwardInfo> {
    list_port_forwards_core(cluster.as_deref())
}

pub(crate) fn list_port_forwards_core(cluster: Option<&str>) -> Vec<ForwardInfo> {
    let r = registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<ForwardInfo> = r
        .values()
        .map(|e| e.info.clone())
        .filter(|i| cluster.is_none_or(|c| i.spec.cluster == c))
        .collect();
    // Saved-but-not-running ones show as stopped so the UI can start them.
    for s in load_saved() {
        let id = forward_id(&s);
        if cluster.is_some_and(|c| s.cluster != c) || r.contains_key(&id) {
            continue;
        }
        out.push(ForwardInfo {
            id,
            url: format!("http://127.0.0.1:{}", s.local_port),
            spec: s,
            status: "stopped".into(),
            message: None,
            restarts: 0,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

fn saved_path() -> Option<std::path::PathBuf> {
    crate::config::AppConfig::config_dir().map(|d| d.join("port-forwards.json"))
}

fn load_saved() -> Vec<ForwardSpec> {
    saved_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn store_saved(list: &[ForwardSpec]) {
    let Some(path) = saved_path() else { return };
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(body) = serde_json::to_vec_pretty(list) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, body).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

/// Cluster just started: bring back its saved auto-start forwards once the
/// API answers.
pub(crate) fn restore_cluster_forwards(state: Arc<AppState>, cluster: String) {
    let saved: Vec<ForwardSpec> = load_saved().into_iter().filter(|s| s.cluster == cluster && s.auto_start).collect();
    if saved.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        for _ in 0..60 {
            if super::kind::kubectl_raw(&state, &cluster, &["get", "--raw", "/readyz"]).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        for s in saved {
            if let Err(e) = start_port_forward_core(state.clone(), s).await {
                tracing::warn!("could not restore port-forward on {cluster}: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ForwardSpec {
        ForwardSpec {
            cluster: "dev".into(),
            namespace: "default".into(),
            kind: "svc".into(),
            name: "web".into(),
            remote_port: 80,
            local_port: 0,
            auto_start: false,
        }
    }

    #[test]
    fn specs_are_validated() {
        assert!(check_spec(&spec()).is_ok());
        assert!(check_spec(&ForwardSpec { kind: "job".into(), ..spec() }).is_err());
        assert!(check_spec(&ForwardSpec { name: "Web;rm".into(), ..spec() }).is_err());
        assert!(check_spec(&ForwardSpec { local_port: 40000, ..spec() }).is_err());
        assert!(check_spec(&ForwardSpec { remote_port: 0, ..spec() }).is_err());
    }

    #[test]
    fn local_ports_are_stable_and_skip_used_ones() {
        let a = pick_local_port(&spec(), &[], |_| true).unwrap();
        assert!((AUTO_PORT_FIRST..=AUTO_PORT_LAST).contains(&a));
        assert_eq!(pick_local_port(&spec(), &[], |_| true), Some(a));
        let b = pick_local_port(&spec(), &[a], |_| true).unwrap();
        assert_ne!(a, b);
        assert!(pick_local_port(&spec(), &[], |_| false).is_none());
    }
}
