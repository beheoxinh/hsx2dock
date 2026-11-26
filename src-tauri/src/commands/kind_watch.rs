//! Background watch over running kind clusters:
//! - incident alerts (CrashLoopBackOff, image pull errors, OOM kills, restart
//!   spikes, NotReady nodes, unreachable API server, Pending PVCs) with
//!   desktop notifications on state change only, plus a cooldown;
//! - resource history: a minute ring (24 h) and a 15-minute ring (7 days)
//!   per cluster, kept on disk.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use super::kind::kubectl_json;
use crate::state::AppState;

const TICK: Duration = Duration::from_secs(30);
const NOTIFY_COOLDOWN: Duration = Duration::from_secs(15 * 60);
const SPIKE_WINDOW: Duration = Duration::from_secs(10 * 60);
const SPIKE_RESTARTS: i64 = 3;
const MINUTE_CAP: usize = 24 * 60;
const QUARTER_CAP: usize = 7 * 24 * 4;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClusterAlert {
    pub key: String,
    pub cluster: String,
    /// crash-loop | image-pull | oom | restart-spike | node-not-ready | api-down | pvc-pending
    pub kind: String,
    pub severity: String,
    pub namespace: Option<String>,
    pub object: String,
    pub message: String,
    /// Unix seconds when first seen.
    pub since: i64,
}

pub(crate) fn alert_enabled(p: &crate::config::K8sAlertPrefs, kind: &str) -> bool {
    p.enabled
        && match kind {
            "crash-loop" => p.crash_loop,
            "image-pull" => p.image_pull,
            "oom" => p.oom,
            "restart-spike" => p.restart_spike,
            "node-not-ready" => p.node_not_ready,
            "api-down" => p.api_down,
            "pvc-pending" => p.pvc_pending,
            _ => true,
        }
}

#[derive(Default)]
struct WatchState {
    /// Current alerts per cluster.
    alerts: HashMap<String, Vec<ClusterAlert>>,
    /// Restart counts per container over the spike window.
    restarts: HashMap<String, VecDeque<(Instant, i64)>>,
    /// Consecutive API failures per cluster.
    api_failures: HashMap<String, u32>,
    /// Last notification per alert key.
    notified: HashMap<String, Instant>,
    /// Cluster of the latest notification, for click-to-open.
    last_notified: Option<(String, Instant)>,
}

fn watch() -> &'static Mutex<WatchState> {
    static W: OnceLock<Mutex<WatchState>> = OnceLock::new();
    W.get_or_init(Default::default)
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn parse_ts(s: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s?).ok().map(|d| d.timestamp())
}

/// Pod-level alerts from `kubectl get pods -A -o json`. Restart counts are
/// returned for spike tracking.
/// (namespace, pod, container, restart count)
pub(crate) type RestartCount = (String, String, String, i64);

pub(crate) fn pod_alerts(cluster: &str, pods: &serde_json::Value, now: i64) -> (Vec<ClusterAlert>, Vec<RestartCount>) {
    let mut out = Vec::new();
    let mut counts = Vec::new();
    for p in pods["items"].as_array().into_iter().flatten() {
        let ns = p["metadata"]["namespace"].as_str().unwrap_or("").to_string();
        let name = p["metadata"]["name"].as_str().unwrap_or("").to_string();
        let statuses = p["status"]["containerStatuses"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(p["status"]["initContainerStatuses"].as_array().into_iter().flatten());
        for cs in statuses {
            let cname = cs["name"].as_str().unwrap_or("");
            let obj = format!("{name}/{cname}");
            let mk = |kind: &str, sev: &str, msg: String| ClusterAlert {
                key: format!("{cluster}|{kind}|{ns}/{obj}"),
                cluster: cluster.to_string(),
                kind: kind.into(),
                severity: sev.into(),
                namespace: Some(ns.clone()),
                object: obj.clone(),
                message: msg,
                since: now,
            };
            let waiting = cs["state"]["waiting"]["reason"].as_str().unwrap_or("");
            let detail = cs["state"]["waiting"]["message"].as_str().unwrap_or("");
            // Between back-offs the container sits in `terminated` (reason
            // Error) most of the time, so a non-zero exit after several
            // restarts counts as a crash loop too.
            let crashed_again = waiting.is_empty()
                && cs["state"]["terminated"]["exitCode"].as_i64().is_some_and(|c| c != 0)
                && cs["restartCount"].as_i64().unwrap_or(0) >= 3
                && p["spec"]["restartPolicy"].as_str() != Some("Never");
            let waiting = if crashed_again { "CrashLoopBackOff" } else { waiting };
            match waiting {
                "CrashLoopBackOff" => out.push(mk(
                    "crash-loop",
                    "error",
                    format!("{obj} is crash-looping ({} restarts)", cs["restartCount"].as_i64().unwrap_or(0)),
                )),
                "ImagePullBackOff" | "ErrImagePull" | "InvalidImageName" => out.push(mk(
                    "image-pull",
                    "error",
                    format!("{obj} cannot pull {}: {}", cs["image"].as_str().unwrap_or("its image"), first_line(detail)),
                )),
                _ => {}
            }
            let term = &cs["lastState"]["terminated"];
            if term["reason"].as_str() == Some("OOMKilled") {
                let at = parse_ts(term["finishedAt"].as_str()).unwrap_or(0);
                if now - at < 600 {
                    out.push(mk("oom", "warning", format!("{obj} was OOM-killed")));
                }
            }
            counts.push((ns.clone(), name.clone(), cname.to_string(), cs["restartCount"].as_i64().unwrap_or(0)));
        }
    }
    (out, counts)
}

fn first_line(s: &str) -> &str {
    let l = s.lines().next().unwrap_or("");
    if l.len() > 160 { &l[..l.char_indices().nth(160).map(|(i, _)| i).unwrap_or(l.len())] } else { l }
}

pub(crate) fn node_alerts(cluster: &str, nodes: &serde_json::Value, now: i64) -> Vec<ClusterAlert> {
    nodes["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|n| {
            let name = n["metadata"]["name"].as_str()?.to_string();
            let ready = n["status"]["conditions"]
                .as_array()?
                .iter()
                .find(|c| c["type"] == "Ready")?;
            if ready["status"] == "True" {
                return None;
            }
            // Fresh nodes are NotReady for a while; give them two minutes.
            let changed = parse_ts(ready["lastTransitionTime"].as_str()).unwrap_or(0);
            if now - changed < 120 {
                return None;
            }
            Some(ClusterAlert {
                key: format!("{cluster}|node-not-ready|{name}"),
                cluster: cluster.into(),
                kind: "node-not-ready".into(),
                severity: "error".into(),
                namespace: None,
                message: format!("Node {name} is NotReady: {}", ready["message"].as_str().unwrap_or("no heartbeat")),
                object: name,
                since: changed,
            })
        })
        .collect()
}

pub(crate) fn pvc_alerts(cluster: &str, pvcs: &serde_json::Value, now: i64) -> Vec<ClusterAlert> {
    pvcs["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            if p["status"]["phase"].as_str() != Some("Pending") {
                return None;
            }
            let created = parse_ts(p["metadata"]["creationTimestamp"].as_str()).unwrap_or(now);
            if now - created < 120 {
                return None;
            }
            let ns = p["metadata"]["namespace"].as_str().unwrap_or("").to_string();
            let name = p["metadata"]["name"].as_str().unwrap_or("").to_string();
            Some(ClusterAlert {
                key: format!("{cluster}|pvc-pending|{ns}/{name}"),
                cluster: cluster.into(),
                kind: "pvc-pending".into(),
                severity: "warning".into(),
                namespace: Some(ns),
                message: format!("PVC {name} has been Pending for {} min", (now - created) / 60),
                object: name,
                since: created,
            })
        })
        .collect()
}

/// Containers whose restart count rose by `SPIKE_RESTARTS`+ within the window.
fn spike_alerts(
    cluster: &str,
    counts: &[RestartCount],
    hist: &mut HashMap<String, VecDeque<(Instant, i64)>>,
    now_i: Instant,
    now: i64,
) -> Vec<ClusterAlert> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (ns, pod, c, n) in counts {
        let key = format!("{cluster}|{ns}/{pod}/{c}");
        seen.insert(key.clone());
        let q = hist.entry(key).or_default();
        q.push_back((now_i, *n));
        while q.front().is_some_and(|(t, _)| now_i.duration_since(*t) > SPIKE_WINDOW) {
            q.pop_front();
        }
        let first = q.front().map(|x| x.1).unwrap_or(*n);
        if n - first >= SPIKE_RESTARTS {
            out.push(ClusterAlert {
                key: format!("{cluster}|restart-spike|{ns}/{pod}/{c}"),
                cluster: cluster.into(),
                kind: "restart-spike".into(),
                severity: "warning".into(),
                namespace: Some(ns.clone()),
                object: format!("{pod}/{c}"),
                message: format!("{pod}/{c} restarted {} times in 10 min", n - first),
                since: now,
            });
        }
    }
    let prefix = format!("{cluster}|");
    hist.retain(|k, _| !k.starts_with(&prefix) || seen.contains(k));
    out
}

/// Collects the current alerts of one running cluster.
async fn collect(state: &AppState, cluster: &str) -> Vec<ClusterAlert> {
    let now = now_unix();
    let nodes = kubectl_json(state, cluster, &["get", "nodes", "-o", "json", "--request-timeout=10s"]).await;
    let nodes = match nodes {
        Ok(v) => {
            watch().lock().unwrap_or_else(|e| e.into_inner()).api_failures.remove(cluster);
            v
        }
        Err(e) => {
            let mut w = watch().lock().unwrap_or_else(|e| e.into_inner());
            let n = w.api_failures.entry(cluster.to_string()).or_default();
            *n += 1;
            if *n >= 2 {
                return vec![ClusterAlert {
                    key: format!("{cluster}|api-down|apiserver"),
                    cluster: cluster.into(),
                    kind: "api-down".into(),
                    severity: "error".into(),
                    namespace: None,
                    object: "kube-apiserver".into(),
                    message: format!("The API server is not answering: {}", first_line(&e)),
                    since: now,
                }];
            }
            return Vec::new();
        }
    };
    let mut out = node_alerts(cluster, &nodes, now);
    if let Ok(pods) = kubectl_json(state, cluster, &["get", "pods", "-A", "-o", "json", "--request-timeout=15s"]).await {
        let (a, counts) = pod_alerts(cluster, &pods, now);
        out.extend(a);
        let mut w = watch().lock().unwrap_or_else(|e| e.into_inner());
        let spikes = spike_alerts(cluster, &counts, &mut w.restarts, Instant::now(), now);
        drop(w);
        // A crash-looping container also spikes; keep only the crash loop.
        for s in spikes {
            let base = s.key.replace("|restart-spike|", "|crash-loop|");
            if !out.iter().any(|a| a.key == base) {
                out.push(s);
            }
        }
    }
    if let Ok(pvcs) = kubectl_json(state, cluster, &["get", "pvc", "-A", "-o", "json", "--request-timeout=10s"]).await {
        out.extend(pvc_alerts(cluster, &pvcs, now));
    }
    out
}

/// Starts the watcher; one tick every 30 s while the engine runs.
pub fn spawn_watcher(app: AppHandle) {
    WATCHER_RUNNING.store(true, std::sync::atomic::Ordering::Relaxed);
    tauri::async_runtime::spawn(async move {
        let mut ticks: u64 = 0;
        loop {
            tokio::time::sleep(TICK).await;
            ticks += 1;
            let Some(state) = app.try_state::<Arc<AppState>>().map(|s| s.inner().clone()) else { continue };
            if super::kind::local_client(&state).await.is_err() {
                continue;
            }
            let Ok(clusters) = super::kind::list_kind_clusters_core(&state).await else { continue };
            // History every other tick (once a minute).
            if ticks.is_multiple_of(2) {
                record_history(&clusters).await;
            }
            let prefs = state.config.read().await.notifications.k8s_alerts.clone();
            let running: HashSet<String> = clusters.iter().filter(|c| c.running).map(|c| c.name.clone()).collect();
            {
                let mut w = watch().lock().unwrap_or_else(|e| e.into_inner());
                w.alerts.retain(|c, _| running.contains(c));
            }
            for c in clusters.iter().filter(|c| c.running) {
                // Nodes still booting: their API is expected to be down.
                if c.uptime_seconds.is_some_and(|u| u < 180) {
                    continue;
                }
                let current = collect(&state, &c.name).await;
                let fresh = {
                    let mut w = watch().lock().unwrap_or_else(|e| e.into_inner());
                    let prev = w.alerts.get(&c.name).cloned().unwrap_or_default();
                    let prev_keys: HashSet<&str> = prev.iter().map(|a| a.key.as_str()).collect();
                    // Keep the original first-seen time of ongoing alerts.
                    let merged: Vec<ClusterAlert> = current
                        .into_iter()
                        .map(|mut a| {
                            if let Some(p) = prev.iter().find(|p| p.key == a.key) {
                                a.since = p.since;
                            }
                            a
                        })
                        .collect();
                    let fresh: Vec<ClusterAlert> =
                        merged.iter().filter(|a| !prev_keys.contains(a.key.as_str())).cloned().collect();
                    w.alerts.insert(c.name.clone(), merged);
                    let now_i = Instant::now();
                    let fresh: Vec<ClusterAlert> = fresh
                        .into_iter()
                        .filter(|a| alert_enabled(&prefs, &a.kind) && !prefs.muted_clusters.contains(&a.cluster))
                        .filter(|a| {
                            let ok = w.notified.get(&a.key).is_none_or(|t| now_i.duration_since(*t) >= NOTIFY_COOLDOWN);
                            if ok {
                                w.notified.insert(a.key.clone(), now_i);
                            }
                            ok
                        })
                        .collect();
                    if !fresh.is_empty() {
                        w.last_notified = Some((c.name.clone(), now_i));
                    }
                    w.notified.retain(|_, t| now_i.duration_since(*t) < NOTIFY_COOLDOWN * 4);
                    fresh
                };
                if !fresh.is_empty() {
                    notify(&app, &c.name, &fresh);
                }
                let list = watch().lock().unwrap_or_else(|e| e.into_inner()).alerts.get(&c.name).cloned().unwrap_or_default();
                let _ = app.emit("cluster-alerts", serde_json::json!({ "cluster": c.name, "alerts": list, "fresh": fresh }));
            }
        }
    });
}

fn notify(app: &AppHandle, cluster: &str, fresh: &[ClusterAlert]) {
    use tauri_plugin_notification::NotificationExt;
    let (title, body) = if fresh.len() == 1 {
        (format!("Cluster {cluster}: {}", kind_label(&fresh[0].kind)), fresh[0].message.clone())
    } else {
        (
            format!("Cluster {cluster}: {} new problems", fresh.len()),
            fresh.iter().take(3).map(|a| a.message.as_str()).collect::<Vec<_>>().join("\n"),
        )
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("desktop notification failed: {e}");
    }
}

fn kind_label(k: &str) -> &'static str {
    match k {
        "crash-loop" => "CrashLoopBackOff",
        "image-pull" => "image pull failure",
        "oom" => "container OOM-killed",
        "restart-spike" => "restart spike",
        "node-not-ready" => "node NotReady",
        "api-down" => "API server down",
        "pvc-pending" => "PVC Pending",
        _ => "problem",
    }
}

#[tauri::command]
pub fn list_cluster_alerts(cluster: Option<String>) -> Vec<ClusterAlert> {
    list_cluster_alerts_core(cluster.as_deref())
}

pub(crate) fn list_cluster_alerts_core(cluster: Option<&str>) -> Vec<ClusterAlert> {
    let w = watch().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<ClusterAlert> = w
        .alerts
        .iter()
        .filter(|(c, _)| cluster.is_none_or(|x| x == c.as_str()))
        .flat_map(|(_, v)| v.clone())
        .collect();
    out.sort_by(|a, b| a.cluster.cmp(&b.cluster).then(b.since.cmp(&a.since)));
    out
}

/// Alerts for MCP clients. The watcher only runs inside the desktop app; the
/// standalone MCP server has no cache, so it scans running clusters live.
/// Two consecutive scans are needed for "api-down", like the watcher.
pub(crate) async fn scan_cluster_alerts(state: &AppState, cluster: Option<&str>) -> Vec<ClusterAlert> {
    if WATCHER_RUNNING.load(std::sync::atomic::Ordering::Relaxed) {
        return list_cluster_alerts_core(cluster);
    }
    let Ok(clusters) = super::kind::list_kind_clusters_core(state).await else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for c in clusters.iter().filter(|c| c.running && cluster.is_none_or(|x| x == c.name)) {
        out.extend(collect(state, &c.name).await);
    }
    out.sort_by(|a, b| a.cluster.cmp(&b.cluster).then(b.since.cmp(&a.since)));
    out
}

static WATCHER_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The cluster of a notification shown in the last two minutes, consumed
/// once: the window calls this when it gains focus (clicking a desktop
/// notification focuses the app), and opens that cluster.
#[tauri::command]
pub fn take_alert_focus() -> Option<String> {
    let mut w = watch().lock().unwrap_or_else(|e| e.into_inner());
    match w.last_notified.take() {
        Some((c, t)) if t.elapsed() < Duration::from_secs(120) => Some(c),
        _ => None,
    }
}

// ---------------------------------------------------------------- history

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistorySample {
    /// Unix seconds.
    pub t: i64,
    pub cpu_percent: f64,
    pub mem_mb: f64,
    pub pods: f64,
    pub running_pods: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct HistoryFile {
    pub minute: VecDeque<HistorySample>,
    pub quarter: VecDeque<HistorySample>,
}

fn history_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(crate::migrate::APP_DIR)
        .join("kind-history")
}

fn histories() -> &'static Mutex<HashMap<String, HistoryFile>> {
    static H: OnceLock<Mutex<HashMap<String, HistoryFile>>> = OnceLock::new();
    H.get_or_init(Default::default)
}

fn load_history(cluster: &str) -> HistoryFile {
    std::fs::read(history_dir().join(format!("{cluster}.json")))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Appends a minute sample; every 15 minutes folds the last quarter-hour
/// into the 7-day ring. Returns whether the file should be flushed.
pub(crate) fn push_sample(h: &mut HistoryFile, s: HistorySample) -> bool {
    let bucket = s.t / 900;
    let prev_bucket = h.minute.back().map(|p| p.t / 900);
    if let Some(pb) = prev_bucket.filter(|pb| *pb != bucket) {
        let items: Vec<&HistorySample> = h.minute.iter().filter(|m| m.t / 900 == pb).collect();
        if !items.is_empty() {
            let n = items.len() as f64;
            h.quarter.push_back(HistorySample {
                t: pb * 900,
                cpu_percent: items.iter().map(|m| m.cpu_percent).sum::<f64>() / n,
                mem_mb: items.iter().map(|m| m.mem_mb).sum::<f64>() / n,
                pods: items.iter().map(|m| m.pods).sum::<f64>() / n,
                running_pods: items.iter().map(|m| m.running_pods).sum::<f64>() / n,
            });
            while h.quarter.len() > QUARTER_CAP {
                h.quarter.pop_front();
            }
        }
    }
    h.minute.push_back(s);
    while h.minute.len() > MINUTE_CAP {
        h.minute.pop_front();
    }
    prev_bucket.is_some_and(|pb| pb != bucket) || h.minute.len().is_multiple_of(5)
}

/// Appends this tick's samples and writes the files that reached a flush
/// point. Disk reads and writes run on the blocking pool and never while the
/// `histories` lock is held, so a slow disk cannot stall other history callers.
async fn record_history(clusters: &[super::kind::KindCluster]) {
    let t = now_unix();
    let running: Vec<&super::kind::KindCluster> = clusters.iter().filter(|c| c.running).collect();
    let missing: Vec<String> = {
        let all = histories().lock().unwrap_or_else(|e| e.into_inner());
        running.iter().filter(|c| !all.contains_key(&c.name)).map(|c| c.name.clone()).collect()
    };
    let loaded: Vec<(String, HistoryFile)> = if missing.is_empty() {
        Vec::new()
    } else {
        tokio::task::spawn_blocking(move || {
            missing
                .into_iter()
                .map(|name| {
                    let h = load_history(&name);
                    (name, h)
                })
                .collect::<Vec<(String, HistoryFile)>>()
        })
        .await
        .unwrap_or_default()
    };
    let writes: Vec<(String, Vec<u8>)> = {
        let mut all = histories().lock().unwrap_or_else(|e| e.into_inner());
        for (name, h) in loaded {
            all.entry(name).or_insert(h);
        }
        let mut writes = Vec::new();
        for c in running {
            let Some(cpu) = c.cpu_percent else { continue };
            // Not loaded (the read task failed): skip rather than overwrite the file.
            let Some(h) = all.get_mut(&c.name) else { continue };
            let flush = push_sample(
                h,
                HistorySample {
                    t,
                    cpu_percent: cpu,
                    mem_mb: c.mem_used_mb.unwrap_or(0.0),
                    pods: c.pod_count.unwrap_or(0) as f64,
                    running_pods: c.running_pod_count.unwrap_or(0) as f64,
                },
            );
            if flush {
                if let Ok(b) = serde_json::to_vec(&*h) {
                    writes.push((c.name.clone(), b));
                }
            }
        }
        writes
    };
    if writes.is_empty() {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || {
        let dir = history_dir();
        let _ = std::fs::create_dir_all(&dir);
        for (name, body) in writes {
            let tmp = dir.join(format!("{name}.json.tmp"));
            if std::fs::write(&tmp, body).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join(format!("{name}.json")));
            }
        }
    })
    .await;
}

/// Samples for `range` (1h, 6h, 24h → minute ring; 7d → 15-minute ring).
pub(crate) fn history_range(h: &HistoryFile, range: &str, now: i64) -> Result<Vec<HistorySample>, String> {
    let (secs, src) = match range {
        "1h" => (3600, &h.minute),
        "6h" => (6 * 3600, &h.minute),
        "24h" => (24 * 3600, &h.minute),
        "7d" => (7 * 24 * 3600, &h.quarter),
        _ => return Err(format!("unknown range '{range}' (1h, 6h, 24h, 7d)")),
    };
    let mut out: Vec<HistorySample> = src.iter().filter(|s| now - s.t <= secs).cloned().collect();
    // The 7-day view also shows the quarter-hour not folded yet.
    if range == "7d" {
        if let Some(last) = h.minute.back() {
            if out.last().is_none_or(|q| last.t / 900 > q.t / 900) {
                out.push(last.clone());
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn cluster_history(cluster: String, range: String) -> Result<Vec<HistorySample>, String> {
    cluster_history_core(&cluster, &range)
}

pub(crate) fn cluster_history_core(cluster: &str, range: &str) -> Result<Vec<HistorySample>, String> {
    super::kind::check_cluster_name(cluster)?;
    // Read the file before locking, as the watcher does: the lock must not
    // wait on disk I/O.
    let cached = histories().lock().unwrap_or_else(|e| e.into_inner()).contains_key(cluster);
    let loaded = (!cached).then(|| load_history(cluster));
    let mut all = histories().lock().unwrap_or_else(|e| e.into_inner());
    let h = all.entry(cluster.to_string()).or_insert_with(|| loaded.unwrap_or_default());
    history_range(h, range, now_unix())
}

/// Drops a deleted cluster's history.
pub(crate) fn forget_cluster(cluster: &str) {
    if super::kind::check_cluster_name(cluster).is_err() {
        return;
    }
    histories().lock().unwrap_or_else(|e| e.into_inner()).remove(cluster);
    let _ = std::fs::remove_file(history_dir().join(format!("{cluster}.json")));
    let mut w = watch().lock().unwrap_or_else(|e| e.into_inner());
    w.alerts.remove(cluster);
    // A deleted cluster must not keep its API-failure streak (the next cluster
    // with this name would start one failure from the alert threshold).
    w.api_failures.remove(cluster);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_alerts_detect_crash_pull_oom() {
        let now = 1_800_000_000;
        let fin = chrono::DateTime::from_timestamp(now - 60, 0).unwrap().to_rfc3339();
        let pods = serde_json::json!({"items": [{
            "metadata": {"namespace": "app", "name": "web-1"},
            "status": {"containerStatuses": [
                {"name": "web", "restartCount": 7, "state": {"waiting": {"reason": "CrashLoopBackOff"}},
                 "lastState": {"terminated": {"reason": "OOMKilled", "finishedAt": fin}}},
                {"name": "side", "image": "nope:1", "restartCount": 0,
                 "state": {"waiting": {"reason": "ImagePullBackOff", "message": "not found\nmore"}}}
            ]}
        }]});
        let (a, counts) = pod_alerts("dev", &pods, now);
        let kinds: Vec<&str> = a.iter().map(|x| x.kind.as_str()).collect();
        assert_eq!(kinds, ["crash-loop", "oom", "image-pull"]);
        assert!(a[2].message.contains("nope:1: not found"));
        assert_eq!(counts.len(), 2);
    }
    #[test]
    fn pod_alerts_count_a_terminated_restarting_container_as_crash_loop() {
        let pods = serde_json::json!({"items": [
            {"metadata": {"namespace": "default", "name": "c"}, "spec": {"restartPolicy": "Always"},
             "status": {"containerStatuses": [{"name": "c", "restartCount": 3,
                 "state": {"terminated": {"exitCode": 1, "reason": "Error"}}}]}},
            {"metadata": {"namespace": "default", "name": "fresh"}, "spec": {"restartPolicy": "Always"},
             "status": {"containerStatuses": [{"name": "c", "restartCount": 1,
                 "state": {"terminated": {"exitCode": 1, "reason": "Error"}}}]}},
            {"metadata": {"namespace": "default", "name": "job"}, "spec": {"restartPolicy": "Never"},
             "status": {"containerStatuses": [{"name": "c", "restartCount": 5,
                 "state": {"terminated": {"exitCode": 1, "reason": "Error"}}}]}}
        ]});
        let (a, _) = pod_alerts("dev", &pods, 0);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].key, "dev|crash-loop|default/c/c");
    }


    #[test]
    fn spikes_need_three_restarts_in_window() {
        let mut hist = HashMap::new();
        let t0 = Instant::now();
        let c = |n| vec![("ns".to_string(), "p".to_string(), "c".to_string(), n)];
        assert!(spike_alerts("dev", &c(1), &mut hist, t0, 0).is_empty());
        assert!(spike_alerts("dev", &c(3), &mut hist, t0 + Duration::from_secs(60), 0).is_empty());
        assert_eq!(spike_alerts("dev", &c(4), &mut hist, t0 + Duration::from_secs(120), 0).len(), 1);
        // Gone pods are forgotten.
        spike_alerts("dev", &[], &mut hist, t0, 0);
        assert!(hist.is_empty());
    }

    #[test]
    fn nodes_and_pvcs_respect_grace() {
        let now = 1_800_000_000;
        let ts = |d: i64| chrono::DateTime::from_timestamp(now - d, 0).unwrap().to_rfc3339();
        let nodes = serde_json::json!({"items": [
            {"metadata": {"name": "a"}, "status": {"conditions": [{"type": "Ready", "status": "False", "lastTransitionTime": ts(600)}]}},
            {"metadata": {"name": "b"}, "status": {"conditions": [{"type": "Ready", "status": "Unknown", "lastTransitionTime": ts(30)}]}},
            {"metadata": {"name": "c"}, "status": {"conditions": [{"type": "Ready", "status": "True", "lastTransitionTime": ts(600)}]}}
        ]});
        let n = node_alerts("dev", &nodes, now);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].object, "a");
        let pvcs = serde_json::json!({"items": [
            {"metadata": {"namespace": "x", "name": "data", "creationTimestamp": ts(300)}, "status": {"phase": "Pending"}},
            {"metadata": {"namespace": "x", "name": "new", "creationTimestamp": ts(10)}, "status": {"phase": "Pending"}},
            {"metadata": {"namespace": "x", "name": "ok", "creationTimestamp": ts(900)}, "status": {"phase": "Bound"}}
        ]});
        assert_eq!(pvc_alerts("dev", &pvcs, now).len(), 1);
    }

    #[test]
    fn history_rings_fold_quarters() {
        let mut h = HistoryFile::default();
        let base = 1_800_000_000 / 900 * 900;
        for i in 0..20 {
            push_sample(&mut h, HistorySample { t: base + i * 60, cpu_percent: i as f64, ..Default::default() });
        }
        // 15 samples in the first bucket folded into one quarter at t=base.
        assert_eq!(h.quarter.len(), 1);
        assert_eq!(h.quarter[0].t, base);
        assert!((h.quarter[0].cpu_percent - 7.0).abs() < 1e-9);
        let now = base + 19 * 60;
        assert_eq!(history_range(&h, "1h", now).unwrap().len(), 20);
        assert_eq!(history_range(&h, "7d", now).unwrap().len(), 2);
        assert!(history_range(&h, "2d", now).is_err());
        for i in 0..(MINUTE_CAP as i64 + 10) {
            push_sample(&mut h, HistorySample { t: base + 10_000 + i * 60, ..Default::default() });
        }
        assert_eq!(h.minute.len(), MINUTE_CAP);
    }

    #[test]
    fn prefs_gate_kinds() {
        let mut p = crate::config::K8sAlertPrefs::default();
        assert!(alert_enabled(&p, "oom"));
        p.oom = false;
        assert!(!alert_enabled(&p, "oom"));
        p.enabled = false;
        assert!(!alert_enabled(&p, "crash-loop"));
    }
}
