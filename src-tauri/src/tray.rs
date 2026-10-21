//! System tray: live engine-state icon, Docker/cluster telemetry, and quick
//! actions.
//!
//! All async work funnels through one monitor task fed by a signal channel —
//! menu handlers and event listeners are sync closures, but toggling the engine
//! needs `AppState`'s async locks, so they only enqueue signals.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::image::Image;
use tauri::{AppHandle, Emitter, Listener, Manager, Wry};
use tauri_plugin_notification::NotificationExt;

use crate::docker::models::DaemonState;
use crate::state::AppState;

/// A terminal stream event may re-emit `done` (chunked sends, retries); the
/// same logical outcome must toast once inside this window.
const NOTIFY_COOLDOWN: Duration = Duration::from_secs(15);
/// Menu + tooltip refresh cadence when nothing else is happening.
const REFRESH_INTERVAL: Duration = Duration::from_secs(15);
/// Running clusters shown in the tray; the rest collapse into a "more" row.
const MAX_CLUSTER_ROWS: usize = 5;

// Menu ids — matched in the tray's `on_menu_event` handler.
const ID_TOGGLE: &str = "engine-toggle";
const ID_OPEN: &str = "open-dashboard";
const ID_NAV_CONTAINERS: &str = "nav-containers";
const ID_NAV_IMAGES: &str = "nav-images";
const ID_NAV_CLUSTERS: &str = "nav-kubernetes";
const ID_QUIT: &str = "quit";
const CLUSTER_ROW_PREFIX: &str = "cluster-row-";

enum Signal {
    Engine(DaemonState),
    Build {
        tag: String,
        error: Option<String>,
        cancelled: bool,
    },
    Transfer {
        pushed: bool,
        image: String,
        error: Option<String>,
    },
    Kind {
        created: bool,
        name: String,
    },
    Menu(String),
}

/// Everything needed to mutate the tray in place. Menu items are `Clone`
/// (Arc-backed), so callers snapshot out of the mutex and never hold the
/// guard across an `.await`.
#[derive(Clone)]
struct TrayParts {
    tray: Option<TrayIcon<Wry>>,
    engine_header: MenuItem<Wry>,
    containers_row: MenuItem<Wry>,
    images_row: MenuItem<Wry>,
    /// One row per running cluster, plus a trailing "and N more" row.
    cluster_rows: Vec<MenuItem<Wry>>,
    clusters_more: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    tx: tokio::sync::mpsc::UnboundedSender<Signal>,
}

pub struct TrayState {
    parts: Mutex<TrayParts>,
}

impl TrayState {
    fn parts(&self) -> Option<TrayParts> {
        self.parts.lock().ok().map(|guard| guard.clone())
    }

    fn send(&self, signal: Signal) {
        if let Ok(parts) = self.parts.lock() {
            let _ = parts.tx.send(signal);
        }
    }
}

/// Hook for `commands/kind.rs` completion points: queues the success toast
/// through the monitor so prefs/dedup/history all run in one place.
pub fn notify_kind_event(app: &AppHandle, created: bool, name: &str) {
    if let Some(state) = app.try_state::<TrayState>() {
        state.send(Signal::Kind {
            created,
            name: name.to_string(),
        });
    }
}

struct MonitorCtx {
    notified: HashMap<String, Instant>,
    last_engine: Option<DaemonState>,
    /// Last colour pushed to the tray — StatusNotifier hosts reload the
    /// pixmap on every `set_icon`, which flickers if we rewrite it each tick.
    last_color: Option<[u8; 3]>,
}

/// Creates the tray + menu, manages [`TrayState`], wires event listeners, and
/// spawns the monitor task. Call from app setup, after `AppState` is managed.
pub fn init_tray(app: &AppHandle) -> tauri::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Signal>();

    let header = MenuItem::with_id(app, "engine-status", "Engine: …", false, None::<&str>)?;
    let containers_row =
        MenuItem::with_id(app, "stats-containers", "Containers: —", false, None::<&str>)?;
    let images_row = MenuItem::with_id(app, "stats-images", "Images: —", false, None::<&str>)?;
    // One disabled row per cluster. `refresh` rewrites them from the same
    // container list the Docker stats come from, so repainting the tray never
    // triggers a kubectl round trip.
    let cluster_rows: Vec<MenuItem<Wry>> = (0..MAX_CLUSTER_ROWS)
        .map(|i| {
            MenuItem::with_id(
                app,
                format!("{CLUSTER_ROW_PREFIX}{i}"),
                "·",
                false,
                None::<&str>,
            )
        })
        .collect::<Result<_, _>>()?;
    let clusters_more =
        MenuItem::with_id(app, "clusters-more", "No kind clusters", false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, ID_TOGGLE, "Start Engine", true, None::<&str>)?;
    let open = MenuItem::with_id(app, ID_OPEN, "Open Dashboard", true, None::<&str>)?;
    let nav_containers =
        MenuItem::with_id(app, ID_NAV_CONTAINERS, "Containers", true, None::<&str>)?;
    let nav_images = MenuItem::with_id(app, ID_NAV_IMAGES, "Images", true, None::<&str>)?;
    let nav_clusters =
        MenuItem::with_id(app, ID_NAV_CLUSTERS, "Manage Kubernetes", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::new(app)?;
    menu.append(&header)?;
    menu.append(&containers_row)?;
    menu.append(&images_row)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        "clusters-title",
        "Kubernetes clusters",
        false,
        None::<&str>,
    )?)?;
    for row in &cluster_rows {
        menu.append(row)?;
    }
    menu.append(&clusters_more)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&toggle)?;
    menu.append(&open)?;
    menu.append(&nav_containers)?;
    menu.append(&nav_images)?;
    menu.append(&nav_clusters)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;

    let daemon = app
        .try_state::<AppState>()
        .map(|s| s.daemon_state())
        .unwrap_or_default();
    let tx_menu = tx.clone();
    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(state_icon(state_color(&daemon)))
        .icon_as_template(false)
        .tooltip(&format_tooltip(&state_label(&daemon), None))
        .menu(&menu)
        .show_menu_on_left_click(true)
        // Menu clicks only enqueue: engine ops and config writes are async and
        // must not block the GTK main loop.
        .on_menu_event(move |_app, event| {
            let _ = tx_menu.send(Signal::Menu(event.id().as_ref().to_string()));
        })
        .build(app)?;

    app.manage(TrayState {
        parts: Mutex::new(TrayParts {
            tray: Some(tray),
            engine_header: header,
            containers_row,
            images_row,
            cluster_rows,
            clusters_more,
            toggle,
            tx: tx.clone(),
        }),
    });

    // Progress events are emitted from the image/kind commands; listeners
    // translate them into signals so dedup + prefs run once, in the task.
    listen_signal(app, "engine-state-changed", tx.clone(), |payload| {
        serde_json::from_str::<DaemonState>(payload)
            .ok()
            .map(Signal::Engine)
    });
    listen_signal(app, "build-progress", tx.clone(), |payload| {
        serde_json::from_str::<BuildEvt>(payload)
            .ok()
            .filter(|e| e.done)
            .map(|e| Signal::Build {
                tag: e.tag,
                error: e.error,
                cancelled: e.cancelled,
            })
    });
    listen_signal(app, "pull-progress", tx.clone(), |payload| {
        serde_json::from_str::<TransferEvt>(payload)
            .ok()
            .filter(|e| e.done)
            .map(|e| Signal::Transfer {
                pushed: false,
                image: e.image,
                error: e.error,
            })
    });
    listen_signal(app, "push-progress", tx.clone(), |payload| {
        serde_json::from_str::<TransferEvt>(payload)
            .ok()
            .filter(|e| e.done)
            .map(|e| Signal::Transfer {
                pushed: true,
                image: e.image,
                error: e.error,
            })
    });

    let handle = app.clone();
    crate::runtime::spawn(async move {
        let mut ctx = MonitorCtx {
            notified: HashMap::new(),
            // Deliberately None: the first `engine-state-changed` we hear is a
            // real transition (autostart, toggle, or death), so toast it.
            last_engine: None,
            last_color: None,
        };
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        refresh(&handle, &mut ctx).await;
        loop {
            tokio::select! {
                sig = rx.recv() => {
                    let Some(sig) = sig else { break };
                    handle_signal(&handle, &mut ctx, sig).await;
                    refresh(&handle, &mut ctx).await;
                }
                _ = ticker.tick() => refresh(&handle, &mut ctx).await,
            }
        }
    });

    Ok(())
}

/// Subscribes `parse` over an emitted event and forwards the signal; payloads
/// that fail to parse are dropped (never crash the tray on a shape change).
fn listen_signal<F>(
    app: &AppHandle,
    event: &str,
    tx: tokio::sync::mpsc::UnboundedSender<Signal>,
    parse: F,
) where
    F: Fn(&str) -> Option<Signal> + Send + 'static,
{
    app.listen(event, move |e| {
        if let Some(sig) = parse(e.payload()) {
            let _ = tx.send(sig);
        }
    });
}

#[derive(Deserialize)]
struct BuildEvt {
    tag: String,
    #[allow(dead_code)]
    done: bool,
    #[serde(default)]
    cancelled: bool,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct TransferEvt {
    image: String,
    #[allow(dead_code)]
    done: bool,
    #[serde(default)]
    error: Option<String>,
}

async fn handle_signal(app: &AppHandle, ctx: &mut MonitorCtx, signal: Signal) {
    match signal {
        Signal::Engine(state) => {
            let prefs = notification_prefs(app).await;
            if prefs.engine_state && ctx.last_engine.as_ref() != Some(&state) {
                match &state {
                    DaemonState::Running => {
                        record_notice(app, "Engine started", "Docker engine is running")
                    }
                    DaemonState::Stopped => {
                        record_notice(app, "Engine stopped", "Docker engine has stopped")
                    }
                    DaemonState::Error(e) => record_notice(app, "Engine error", e),
                    // Transient / install states are not worth a toast; the
                    // follow-up settled state covers the outcome.
                    _ => {}
                }
            }
            ctx.last_engine = Some(state);
        }
        Signal::Build {
            tag,
            error,
            cancelled,
        } => {
            if cancelled {
                return;
            }
            let prefs = notification_prefs(app).await;
            if !prefs.build_done {
                return;
            }
            if !should_notify(&mut ctx.notified, &format!("build:{tag}"), NOTIFY_COOLDOWN) {
                return;
            }
            match error {
                Some(err) => {
                    let last = err.lines().last().unwrap_or(err.as_str());
                    record_notice(app, &format!("Build failed — {tag}"), last);
                }
                None => record_notice(app, "Build succeeded", &tag),
            }
        }
        Signal::Transfer {
            pushed,
            image,
            error,
        } => {
            let prefs = notification_prefs(app).await;
            if !prefs.transfer_done {
                return;
            }
            let verb = if pushed { "push" } else { "pull" };
            if !should_notify(&mut ctx.notified, &format!("{verb}:{image}"), NOTIFY_COOLDOWN) {
                return;
            }
            match error {
                Some(err) => {
                    let last = err.lines().last().unwrap_or(err.as_str());
                    let title = if pushed {
                        "Image push failed"
                    } else {
                        "Image pull failed"
                    };
                    record_notice(app, title, &format!("{image}: {last}"));
                }
                None => {
                    let title = if pushed { "Image pushed" } else { "Image pulled" };
                    record_notice(app, title, &image);
                }
            }
        }
        Signal::Kind { created, name } => {
            let prefs = notification_prefs(app).await;
            if !prefs.kind_events {
                return;
            }
            let verb = if created { "create" } else { "delete" };
            if !should_notify(&mut ctx.notified, &format!("kind:{verb}:{name}"), NOTIFY_COOLDOWN)
            {
                return;
            }
            if created {
                record_notice(app, &format!("Kind cluster '{name}' is ready"), "");
            } else {
                record_notice(app, &format!("Kind cluster '{name}' deleted"), "");
            }
        }
        Signal::Menu(id) => handle_menu(app, &id).await,
    }
}

async fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        ID_TOGGLE => toggle_engine_from_tray(app).await,
        ID_OPEN => {
            // Label "main" comes from tauri.conf.json; show+unminimize+focus
            // covers every way the window can be hidden.
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }
        ID_QUIT => app.exit(0),
        // Quick navigation: reveal the window, then let the React shell pick
        // the page. The tray stays a thin shell — navigation state lives in the
        // frontend, so it owns the actual switch.
        ID_NAV_CONTAINERS | ID_NAV_IMAGES | ID_NAV_CLUSTERS => {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
            let target = match id {
                ID_NAV_CONTAINERS => "containers",
                ID_NAV_IMAGES => "images",
                _ => "kubernetes",
            };
            let _ = app.emit("tray-navigate", target);
        }
        _ => {}
    }
}

/// Tray Start/Stop: same EngineManager path as `toggle_engine`, but hard-gated
/// on `engine_control_guard` so a remote endpoint can never be bypassed.
async fn toggle_engine_from_tray(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    if let Err(e) = state.engine_control_guard().await {
        // Immediate feedback to a click — not a preference-gated broadcast.
        record_notice(app, "Engine controls unavailable", &e);
        return;
    }
    match state.daemon_state() {
        DaemonState::Running => {
            let engine = state.engine.clone();
            match crate::runtime::engine_op(move || engine.stop()).await {
                Ok(Ok(())) => {
                    *state.client.write().await = None;
                }
                Ok(Err(e)) => record_notice(app, "Engine stop failed", &e.to_string()),
                Err(e) => record_notice(app, "Engine stop failed", &e),
            }
        }
        DaemonState::Stopped | DaemonState::NotInstalled => {
            let engine = state.engine.clone();
            match crate::runtime::engine_op(move || engine.start()).await {
                Ok(Ok(())) => {
                    let _ = state.get_client().await;
                }
                Ok(Err(e)) => record_notice(app, "Engine start failed", &e.to_string()),
                Err(e) => record_notice(app, "Engine start failed", &e),
            }
        }
        // Mid-transition clicks are ignored; the running op owns the state.
        _ => {}
    }
}

async fn notification_prefs(app: &AppHandle) -> crate::config::NotificationPrefs {
    app.try_state::<AppState>()
        .and_then(|s| s.config.try_read().ok().map(|c| c.notifications.clone()))
        .unwrap_or_default()
}

/// Shows a desktop notification. Callers check the Settings prefs first;
/// interactive failures (guard errors, start/stop failures) bypass them.
fn record_notice(app: &AppHandle, title: &str, body: &str) {
    show_toast(app, title, body);
}

fn show_toast(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
    {
        tracing::warn!("desktop notification failed: {e}");
    }
}

/// One-shot dedupe for terminal stream events: streams can re-emit `done`,
/// so the same logical outcome must toast once per cooldown window.
pub fn should_notify(seen: &mut HashMap<String, Instant>, key: &str, cooldown: Duration) -> bool {
    let now = Instant::now();
    if let Some(prev) = seen.get(key) {
        if now.duration_since(*prev) < cooldown {
            return false;
        }
    }
    seen.insert(key.to_string(), now);
    if seen.len() > 256 {
        seen.retain(|_, t| now.duration_since(*t) < cooldown);
    }
    true
}

struct TrayStats {
    containers: usize,
    running: usize,
    images: usize,
    kinds: usize,
    clusters: Vec<ClusterStat>,
}

/// One kind cluster as the tray menu reports it.
struct ClusterStat {
    name: String,
    nodes: usize,
    running: usize,
}

async fn refresh(app: &AppHandle, ctx: &mut MonitorCtx) {
    let Some(parts) = app.try_state::<TrayState>().and_then(|t| t.parts()) else { return };
    let Some(state) = app.try_state::<AppState>() else { return };
    let daemon = state.daemon_state();

    let stats = fetch_stats(&state, &daemon).await;
    let label = state_label(&daemon);

    if let Some(tray) = &parts.tray {
        // Raw RGBA swapped in place; on Linux StatusNotifier hosts that cache
        // the pixmap the swap can lag — tooltip and menu stay authoritative.
        // Only rewrite the pixmap when the colour actually changed: every
        // set_icon forces a host-side reload, which flickers on some DEs.
        let color = state_color(&daemon);
        if ctx.last_color != Some(color) {
            let _ = tray.set_icon(Some(state_icon(color)));
            ctx.last_color = Some(color);
        }
        let _ = tray.set_tooltip(Some(&format_tooltip(
            &label,
            stats
                .as_ref()
                .map(|s| (s.containers, s.running, s.images, s.kinds)),
        )));
    }

    let _ = parts.engine_header.set_text(format!("Engine: {label}"));
    match &stats {
        Some(s) => {
            let _ = parts.containers_row.set_text(format!(
                "Containers: {} total · {} running",
                s.containers, s.running
            ));
            let _ = parts.images_row.set_text(format!("Images: {}", s.images));
        }
        None => {
            let _ = parts.containers_row.set_text("Containers: —");
            let _ = parts.images_row.set_text("Images: —");
        }
    }

    let toggle_label = if daemon == DaemonState::Running {
        "Stop Engine"
    } else {
        "Start Engine"
    };
    let _ = parts.toggle.set_text(toggle_label);

    // Cluster rows: a cluster is running while at least one node container is.
    let clusters: &[ClusterStat] = stats
        .as_ref()
        .map(|s| s.clusters.as_slice())
        .unwrap_or(&[]);
    for (i, row) in parts.cluster_rows.iter().enumerate() {
        let text = match clusters.get(i) {
            Some(c) => {
                let phase = if c.running == 0 {
                    "stopped"
                } else if c.running == c.nodes {
                    "running"
                } else {
                    "degraded"
                };
                format!("{} · {}/{} nodes · {}", c.name, c.running, c.nodes, phase)
            }
            None => String::new(),
        };
        let _ = row.set_text(text);
        let _ = row.set_enabled(i < clusters.len());
    }
    let more = if clusters.is_empty() {
        "No kind clusters".to_string()
    } else if clusters.len() > MAX_CLUSTER_ROWS {
        format!("…and {} more", clusters.len() - MAX_CLUSTER_ROWS)
    } else {
        format!("{} cluster(s)", clusters.len())
    };
    let _ = parts.clusters_more.set_text(more);
}

async fn fetch_stats(state: &AppState, daemon: &DaemonState) -> Option<TrayStats> {
    if *daemon != DaemonState::Running {
        return None;
    }
    let client = state.get_client().await.ok()?;
    let containers = client.list_containers(true).await.ok()?;
    let total = containers.len();
    let running = containers.iter().filter(|c| c.running).count();
    let images = client.list_images().await.ok()?.len();
    // Group kind node containers into a compact per-cluster line. The container
    // list already carries the cluster label, so the tray never shells out to
    // kubectl just to redraw its menu.
    let mut grouped: HashMap<String, (usize, usize)> = HashMap::new();
    for c in &containers {
        if let Some(name) = &c.kind_cluster {
            let entry = grouped.entry(name.clone()).or_insert((0, 0));
            entry.0 += 1;
            if c.running {
                entry.1 += 1;
            }
        }
    }
    let mut clusters: Vec<ClusterStat> = grouped
        .into_iter()
        .map(|(name, (nodes, running))| ClusterStat {
            name,
            nodes,
            running,
        })
        .collect();
    // Running cluster first, then alphabetical: the menu is a monitor, not a
    // directory.
    clusters.sort_by(|a, b| b.running.cmp(&a.running).then_with(|| a.name.cmp(&b.name)));
    let kinds = clusters.len();
    Some(TrayStats {
        containers: total,
        running,
        images,
        kinds,
        clusters,
    })
}

pub fn state_label(state: &DaemonState) -> String {
    let s = match state {
        DaemonState::Running => "Running".to_string(),
        DaemonState::Stopped => "Stopped".to_string(),
        DaemonState::Starting => "Starting…".to_string(),
        DaemonState::Stopping => "Stopping…".to_string(),
        DaemonState::NotInstalled => "Not installed".to_string(),
        DaemonState::Installing => "Installing…".to_string(),
        DaemonState::Error(e) => format!("Error — {e}"),
    };
    // Keep the menu bar readable when the daemon reports a long error.
    if s.chars().count() > 80 {
        let cut: String = s.chars().take(77).collect();
        format!("{cut}…")
    } else {
        s
    }
}

/// Docker-engine blue while Running, zinc when Stopped, amber for transitions
/// and errors — the same vocabulary the dashboard uses.
fn state_color(state: &DaemonState) -> [u8; 3] {
    match state {
        DaemonState::Running => [36, 150, 237], // #2496ED
        DaemonState::Stopped | DaemonState::NotInstalled => [113, 113, 122], // #71717A
        DaemonState::Starting | DaemonState::Stopping | DaemonState::Installing => {
            [245, 158, 11] // #F59E0B
        }
        DaemonState::Error(_) => [245, 158, 11],
    }
}

fn state_icon(rgb: [u8; 3]) -> Image<'static> {
    const SIZE: u32 = 32;
    Image::new_owned(rounded_square_rgba(rgb), SIZE, SIZE)
}

/// Rounded square with 3x3 supersampling so menu-bar edges stay soft.
fn rounded_square_rgba(rgb: [u8; 3]) -> Vec<u8> {
    const W: usize = 32;
    const PAD: f64 = 3.0;
    const RADIUS: f64 = 7.0;
    const SS: u32 = 3;
    let mut out = vec![0u8; W * W * 4];
    for y in 0..W {
        for x in 0..W {
            let mut hits = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f64 + (sx as f64 + 0.5) / SS as f64;
                    let py = y as f64 + (sy as f64 + 0.5) / SS as f64;
                    if inside_rounded_rect(px, py, PAD, W as f64 - PAD, RADIUS) {
                        hits += 1;
                    }
                }
            }
            let o = (y * W + x) * 4;
            out[o] = rgb[0];
            out[o + 1] = rgb[1];
            out[o + 2] = rgb[2];
            out[o + 3] = (hits * 255 / (SS * SS)) as u8;
        }
    }
    out
}

fn inside_rounded_rect(x: f64, y: f64, min: f64, max: f64, r: f64) -> bool {
    if x < min || x > max || y < min || y > max {
        return false;
    }
    let cx = x.clamp(min + r, max - r);
    let cy = y.clamp(min + r, max - r);
    (x - cx).powi(2) + (y - cy).powi(2) <= r * r
}

/// Tooltip shown on hover (and, on Linux, whatever the host honours — many
/// StatusNotifier implementations ignore it; the menu carries the same data).
pub fn format_tooltip(label: &str, stats: Option<(usize, usize, usize, usize)>) -> String {
    match stats {
        Some((total, running, images, kinds)) => format!(
            "Hsx2Dock — Engine {label} · {total} containers ({running} running) · \
             {images} images · {kinds} kind {}",
            if kinds == 1 { "cluster" } else { "clusters" }
        ),
        None => format!("Hsx2Dock — Engine {label}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_notify_coalesces_repeats_but_not_other_keys() {
        let mut seen = HashMap::new();
        let cd = Duration::from_secs(15);
        assert!(should_notify(&mut seen, "build:img:1", cd));
        assert!(
            !should_notify(&mut seen, "build:img:1", cd),
            "a repeated done for the same key must be swallowed"
        );
        assert!(
            should_notify(&mut seen, "pull:img:1", cd),
            "a different event key still notifies"
        );
    }

    #[test]
    fn tooltip_carries_stats_and_pluralizes() {
        assert_eq!(
            format_tooltip("Running", None),
            "Hsx2Dock — Engine Running"
        );
        assert_eq!(
            format_tooltip("Running", Some((12, 7, 3, 1))),
            "Hsx2Dock — Engine Running · 12 containers (7 running) · 3 images · 1 kind cluster"
        );
        let two = format_tooltip("Running", Some((1, 1, 0, 2)));
        assert!(two.ends_with("2 kind clusters"), "got: {two}");
    }

    #[test]
    fn state_colors_distinguish_running_stopped_transition() {
        assert_ne!(
            state_color(&DaemonState::Running),
            state_color(&DaemonState::Stopped)
        );
        assert_eq!(
            state_color(&DaemonState::Starting),
            state_color(&DaemonState::Error("x".into()))
        );
        assert_ne!(
            state_color(&DaemonState::Running),
            state_color(&DaemonState::Starting)
        );
    }

    #[test]
    fn rounded_icon_has_transparent_corners_and_opaque_center() {
        let rgba = rounded_square_rgba([36, 150, 237]);
        let w = 32usize;
        let alpha = |x: usize, y: usize| rgba[(y * w + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "corner must be fully transparent");
        assert_eq!(alpha(w / 2, w / 2), 255, "center must be fully opaque");
    }
}
