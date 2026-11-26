//! Live side of Compose: the Docker event stream for compose containers
//! (pushes `compose-event` to the UI instead of it polling, and raises desktop
//! alerts for unhealthy / crashed / restart-looping services), and the
//! `docker compose watch` sessions the app starts and stops.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::compose::{err, file_target, progress_sink};
use crate::state::AppState;

const PROJECT_LABEL: &str = "com.docker.compose.project";
const SERVICE_LABEL: &str = "com.docker.compose.service";

/// One container event of a compose project, as the UI receives it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ComposeEvent {
    pub project: String,
    pub service: String,
    pub container: String,
    /// `start`, `die`, `health_status: unhealthy`, `destroy`, …
    pub action: String,
    pub exit_code: Option<i64>,
    pub time: i64,
}

/// A problem worth a desktop notification.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ComposeAlert {
    pub project: String,
    pub service: String,
    pub container: String,
    /// `unhealthy`, `crashed` or `restart-loop`.
    pub kind: String,
    pub message: String,
    pub time: i64,
}

/// Crashes inside this window count toward a restart loop.
const LOOP_WINDOW: Duration = Duration::from_secs(5 * 60);
const LOOP_CRASHES: usize = 3;
/// The same alert for the same container is not repeated within this.
const ALERT_COOLDOWN: Duration = Duration::from_secs(10 * 60);
/// A `die` this soon after `kill`/`stop` was asked for, not a crash.
const STOP_GRACE: Duration = Duration::from_secs(30);

/// Turns the event stream into alerts. Pure (time is passed in) so the
/// rules are testable without a daemon.
#[derive(Default)]
pub struct AlertRules {
    stopping: HashMap<String, Instant>,
    crashes: HashMap<String, Vec<Instant>>,
    last_alert: HashMap<(String, String), Instant>,
}

impl AlertRules {
    /// Drops per-container state that can no longer change an answer. Without
    /// this, a container that was stopped (but never destroyed) kept its
    /// `stopping` entry for the life of the app, and every crashed container
    /// kept an empty-ish crash list forever.
    fn prune(&mut self, now: Instant) {
        self.stopping.retain(|_, t| now.duration_since(*t) < STOP_GRACE);
        self.crashes.retain(|_, list| {
            list.retain(|t| now.duration_since(*t) < LOOP_WINDOW);
            !list.is_empty()
        });
    }

    pub fn observe(&mut self, ev: &ComposeEvent, now: Instant) -> Option<ComposeAlert> {
        self.prune(now);
        let id = ev.container.clone();
        let alert = match ev.action.as_str() {
            "kill" | "stop" => {
                self.stopping.insert(id.clone(), now);
                None
            }
            "destroy" => {
                self.stopping.remove(&id);
                self.crashes.remove(&id);
                None
            }
            "health_status: unhealthy" => Some(("unhealthy", format!("{} is unhealthy", ev.service))),
            "die" => {
                let code = ev.exit_code.unwrap_or(0);
                let asked = self
                    .stopping
                    .get(&id)
                    .is_some_and(|t| now.duration_since(*t) < STOP_GRACE);
                if code == 0 || asked {
                    None
                } else {
                    let list = self.crashes.entry(id.clone()).or_default();
                    list.retain(|t| now.duration_since(*t) < LOOP_WINDOW);
                    list.push(now);
                    if list.len() >= LOOP_CRASHES {
                        Some((
                            "restart-loop",
                            format!("{} crashed {} times in 5 minutes (exit {code})", ev.service, list.len()),
                        ))
                    } else {
                        Some(("crashed", format!("{} exited with code {code}", ev.service)))
                    }
                }
            }
            _ => None,
        }?;
        let key = (id, alert.0.to_string());
        if self.last_alert.get(&key).is_some_and(|t| now.duration_since(*t) < ALERT_COOLDOWN) {
            return None;
        }
        self.last_alert.insert(key, now);
        self.last_alert.retain(|_, t| now.duration_since(*t) < ALERT_COOLDOWN * 3);
        Some(ComposeAlert {
            project: ev.project.clone(),
            service: ev.service.clone(),
            container: ev.container.clone(),
            kind: alert.0.into(),
            message: alert.1,
            time: ev.time,
        })
    }
}

/// Recent alerts, newest last, for the UI's alert list.
static RECENT_ALERTS: LazyLock<Mutex<Vec<ComposeAlert>>> = LazyLock::new(Default::default);
const MAX_RECENT_ALERTS: usize = 100;

#[tauri::command]
pub fn compose_recent_alerts() -> Vec<ComposeAlert> {
    RECENT_ALERTS.lock().map(|a| a.clone()).unwrap_or_default()
}

#[tauri::command]
pub fn compose_clear_alerts(project: Option<String>) {
    if let Ok(mut a) = RECENT_ALERTS.lock() {
        match project {
            Some(p) => a.retain(|x| x.project != p),
            None => a.clear(),
        }
    }
}

fn to_event(m: &bollard::models::EventMessage) -> Option<ComposeEvent> {
    let actor = m.actor.as_ref()?;
    let attrs = actor.attributes.as_ref()?;
    let project = attrs.get(PROJECT_LABEL)?.clone();
    Some(ComposeEvent {
        project,
        service: attrs.get(SERVICE_LABEL).cloned().unwrap_or_default(),
        container: actor.id.clone().unwrap_or_default(),
        action: m.action.clone().unwrap_or_default(),
        exit_code: attrs.get("exitCode").and_then(|c| c.parse().ok()),
        time: m.time.unwrap_or(0),
    })
}

/// Actions the UI refreshes on; exec_* and the like are noise.
fn interesting(action: &str) -> bool {
    matches!(
        action,
        "create" | "start" | "restart" | "stop" | "kill" | "die" | "destroy" | "pause" | "unpause" | "oom"
    ) || action.starts_with("health_status")
}

async fn alerts_enabled(app: &AppHandle, project: &str) -> bool {
    match app.try_state::<Arc<AppState>>() {
        Some(s) => {
            let cfg = s.config.read().await;
            let p = &cfg.notifications.compose_alerts;
            p.enabled && !p.muted_projects.iter().any(|m| m == project)
        }
        None => true,
    }
}

fn notify(app: &AppHandle, a: &ComposeAlert) {
    use tauri_plugin_notification::NotificationExt;
    let title = match a.kind.as_str() {
        "unhealthy" => format!("Compose {}: service unhealthy", a.project),
        "restart-loop" => format!("Compose {}: restart loop", a.project),
        _ => format!("Compose {}: service crashed", a.project),
    };
    if let Err(e) = app.notification().builder().title(title).body(a.message.clone()).show() {
        tracing::warn!("desktop notification failed: {e}");
    }
}

/// Follows the daemon's container events for compose projects for as long
/// as the app runs, reconnecting when the engine restarts.
pub fn spawn_compose_event_watch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut rules = AlertRules::default();
        loop {
            let Some(state) = app.try_state::<Arc<AppState>>().map(|s| s.inner().clone()) else {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            };
            let Ok(client) = state.get_client().await else {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            };
            let mut filters = HashMap::new();
            filters.insert("type".to_string(), vec!["container".to_string()]);
            filters.insert("label".to_string(), vec![PROJECT_LABEL.to_string()]);
            let opts = bollard::query_parameters::EventsOptionsBuilder::default().filters(&filters).build();
            let mut stream = client.docker().events(Some(opts));
            while let Some(item) = stream.next().await {
                let Ok(msg) = item else { break };
                let Some(ev) = to_event(&msg) else { continue };
                if !interesting(&ev.action) {
                    continue;
                }
                let _ = app.emit("compose-event", &ev);
                if let Some(alert) = rules.observe(&ev, Instant::now()) {
                    if let Ok(mut list) = RECENT_ALERTS.lock() {
                        list.push(alert.clone());
                        let extra = list.len().saturating_sub(MAX_RECENT_ALERTS);
                        list.drain(..extra);
                    }
                    let _ = app.emit("compose-alert", &alert);
                    if alerts_enabled(&app, &alert.project).await {
                        notify(&app, &alert);
                    }
                }
            }
            // The stream ended: engine stopped or restarted. Try again soon.
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

// ---------------------------------------------------------------------------
// `docker compose watch` sessions
// ---------------------------------------------------------------------------

struct WatchSession {
    child: Child,
    op_id: String,
}

static WATCHES: LazyLock<Mutex<HashMap<PathBuf, WatchSession>>> = LazyLock::new(Default::default);

fn key(file: &Path) -> PathBuf {
    file.canonicalize().unwrap_or_else(|_| file.to_path_buf())
}

/// A watch session for this compose file is running.
pub fn is_watching(file: &Path) -> bool {
    let mut w = WATCHES.lock().unwrap_or_else(|e| e.into_inner());
    // An exited session is left in place for its reaper thread. Removing it
    // here made the reaper find nothing and return without emitting
    // `compose-watch-ended`, so the UI kept showing a dead watch.
    w.get_mut(&key(file))
        .is_some_and(|s| matches!(s.child.try_wait(), Ok(None)))
}

#[derive(Clone, Serialize)]
struct WatchEnded {
    file: String,
    op_id: String,
    code: Option<i32>,
}

/// Starts `docker compose watch` for the project (builds/starts it first, then
/// syncs or rebuilds on file changes). Its output streams as
/// `compose-progress` under `op_id`; `compose-watch-ended` fires when it stops.
#[tauri::command]
pub async fn compose_watch_start(app: AppHandle, file_path: String, op_id: String) -> Result<(), String> {
    let target = file_target(&file_path, None, Vec::new())?;
    let file = target.file.clone().ok_or("no compose file")?;
    if is_watching(&file) {
        return Err("watch is already running for this project".into());
    }
    let mut args = target.global_args().map_err(err)?;
    args.push("watch".into());
    let paths = crate::docker::engine::EnginePaths::new();
    let mut cmd = crate::docker::engine::docker_command_resolved(&paths);
    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("cannot start compose watch: {e}"))?;
    // `LineSink` is Send but not Sync: the two pipe readers share it behind a lock.
    let sink = Arc::new(Mutex::new(progress_sink(&app, Some(op_id.clone()))));
    for pipe in [
        child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let sink = sink.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if let Some(s) = sink.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                    s(&line);
                }
            }
        });
    }
    let k = key(&file);
    WATCHES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(k.clone(), WatchSession { child, op_id: op_id.clone() });
    // Reaper: reports the end however it came (stop, crash, `develop` missing).
    let file_s = file.display().to_string();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        let mut w = WATCHES.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = w.get_mut(&k) else { return };
        if s.op_id != op_id {
            return;
        }
        match s.child.try_wait() {
            Ok(None) => continue,
            res => {
                let code = res.ok().flatten().and_then(|st| st.code());
                w.remove(&k);
                drop(w);
                let _ = app.emit("compose-watch-ended", WatchEnded { file: file_s.clone(), op_id: op_id.clone(), code });
                return;
            }
        }
    });
    Ok(())
}

/// Stops the project's watch session (the project itself keeps running).
#[tauri::command]
pub async fn compose_watch_stop(file_path: String) -> Result<(), String> {
    let file = crate::docker::compose::resolve_compose_file(&file_path).map_err(err)?;
    let mut w = WATCHES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = w.get_mut(&key(&file)) {
        let _ = s.child.kill();
        // The reaper notices the exit and emits `compose-watch-ended`.
    }
    Ok(())
}

/// Ends every watch session (app shutdown).
pub fn stop_all_watches() {
    let mut w = WATCHES.lock().unwrap_or_else(|e| e.into_inner());
    for (_, mut s) in w.drain() {
        let _ = s.child.kill();
        let _ = s.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(action: &str, code: Option<i64>) -> ComposeEvent {
        ComposeEvent {
            project: "shop".into(),
            service: "api".into(),
            container: "c1".into(),
            action: action.into(),
            exit_code: code,
            time: 0,
        }
    }

    #[test]
    fn a_requested_stop_is_not_a_crash() {
        let mut r = AlertRules::default();
        let t = Instant::now();
        assert!(r.observe(&ev("kill", None), t).is_none());
        assert!(r.observe(&ev("die", Some(137)), t + Duration::from_secs(2)).is_none());
        // Well after the stop, a non-zero exit is a crash again.
        let a = r.observe(&ev("die", Some(1)), t + Duration::from_secs(60)).unwrap();
        assert_eq!(a.kind, "crashed");
    }

    #[test]
    fn clean_exit_is_quiet_and_unhealthy_alerts() {
        let mut r = AlertRules::default();
        let t = Instant::now();
        assert!(r.observe(&ev("die", Some(0)), t).is_none());
        assert_eq!(r.observe(&ev("health_status: unhealthy", None), t).unwrap().kind, "unhealthy");
        // Cooldown: the same alert does not repeat at once.
        assert!(r.observe(&ev("health_status: unhealthy", None), t + Duration::from_secs(5)).is_none());
        assert!(r.observe(&ev("health_status: unhealthy", None), t + ALERT_COOLDOWN + Duration::from_secs(1)).is_some());
    }

    #[test]
    fn three_crashes_make_a_restart_loop() {
        let mut r = AlertRules::default();
        let t = Instant::now();
        assert_eq!(r.observe(&ev("die", Some(2)), t).unwrap().kind, "crashed");
        // Second crash: "crashed" again but inside the cooldown, so quiet.
        assert!(r.observe(&ev("die", Some(2)), t + Duration::from_secs(10)).is_none());
        let a = r.observe(&ev("die", Some(2)), t + Duration::from_secs(20)).unwrap();
        assert_eq!(a.kind, "restart-loop");
        assert!(a.message.contains("3 times"));
    }

    #[test]
    fn state_for_gone_containers_is_pruned() {
        let mut r = AlertRules::default();
        let t = Instant::now();
        // A stop with no destroy, and one crash, for two different containers.
        r.observe(&ev("kill", None), t);
        let mut other = ev("die", Some(1));
        other.container = "c2".into();
        r.observe(&other, t);
        assert_eq!(r.stopping.len(), 1);
        assert_eq!(r.crashes.len(), 1);
        // A much later event from an unrelated container prunes both.
        let mut later = ev("start", None);
        later.container = "c3".into();
        let late = t + LOOP_WINDOW + Duration::from_secs(1);
        r.observe(&later, late);
        assert!(r.stopping.is_empty());
        assert!(r.crashes.is_empty());
    }

    #[test]
    fn noise_actions_are_ignored() {
        assert!(interesting("die"));
        assert!(interesting("health_status: healthy"));
        assert!(!interesting("exec_start: sh"));
        assert!(!interesting("attach"));
    }
}
