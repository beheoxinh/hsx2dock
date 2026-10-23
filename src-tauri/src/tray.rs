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
const ID_RESTART: &str = "engine-restart";
const ID_NAV_DASHBOARD: &str = "nav-dashboard";
const ID_NAV_CONTAINERS: &str = "nav-containers";
const ID_NAV_IMAGES: &str = "nav-images";
const ID_NAV_VOLUMES: &str = "nav-volumes";
const ID_NAV_CLUSTERS: &str = "nav-kubernetes";
const ID_NAV_SETTINGS: &str = "nav-settings";
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
    /// App name + version, mirroring Docker Desktop's title row.
    title_row: MenuItem<Wry>,
    /// Engine state plus live Docker counts.
    stats_row: MenuItem<Wry>,
    /// Exactly one row per cluster currently known (capped), so the menu has
    /// no blank placeholders; it is rebuilt when the cluster set changes.
    cluster_rows: Vec<MenuItem<Wry>>,
    toggle: MenuItem<Wry>,
    restart: MenuItem<Wry>,
    /// Names of the clusters this menu was laid out for.
    cluster_sig: String,
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

/// The freshly built menu plus the handles `refresh` mutates in place.
struct MenuParts {
    menu: Menu<Wry>,
    title_row: MenuItem<Wry>,
    stats_row: MenuItem<Wry>,
    cluster_rows: Vec<MenuItem<Wry>>,
    toggle: MenuItem<Wry>,
    restart: MenuItem<Wry>,
    /// Cluster names this layout was built for; a change triggers a rebuild.
    cluster_sig: String,
}

fn cluster_signature(clusters: &[ClusterStat]) -> String {
    clusters
        .iter()
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

/// Status line under the title — the same shape Docker Desktop uses: engine
/// state first, live Docker counts only while the daemon is up.
fn format_status_row(daemon: &DaemonState, stats: Option<&TrayStats>) -> String {
    match stats {
        Some(s) => format!(
            "Engine {} · {} containers · {} running · {} images",
            state_label(daemon),
            s.containers,
            s.running,
            s.images
        ),
        None => format!("Engine {}", state_label(daemon)),
    }
}

fn format_cluster_row(c: &ClusterStat) -> String {
    let phase = if c.running == 0 {
        "stopped"
    } else if c.running == c.nodes {
        "running"
    } else {
        "degraded"
    };
    format!("{} · {}/{} nodes · {}", c.name, c.running, c.nodes, phase)
}

/// Lays the menu out for exactly the clusters that exist right now. Rebuilding
/// instead of keeping placeholder rows is what keeps the tray compact — there
/// are no empty rows when a cluster disappears or the engine stops.
fn build_menu(
    app: &AppHandle,
    daemon: &DaemonState,
    stats: Option<&TrayStats>,
) -> tauri::Result<MenuParts> {
    let title_row = MenuItem::with_id(
        app,
        "app-title",
        format!("Hsx2Dock {}", env!("CARGO_PKG_VERSION")),
        false,
        None::<&str>,
    )?;
    let stats_row = MenuItem::with_id(
        app,
        "tray-status",
        format_status_row(daemon, stats),
        false,
        None::<&str>,
    )?;

    let clusters = stats.map(|s| s.clusters.as_slice()).unwrap_or(&[]);
    let shown = clusters.len().min(MAX_CLUSTER_ROWS);
    // Enabled: clicking a row opens that cluster in the Kubernetes page.
    let cluster_rows = clusters[..shown]
        .iter()
        .map(|c| {
            MenuItem::with_id(
                app,
                format!("{CLUSTER_ROW_PREFIX}{}", c.name),
                format_cluster_row(c),
                true,
                None::<&str>,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let transitioning = matches!(
        daemon,
        DaemonState::Starting | DaemonState::Stopping | DaemonState::Installing
    );
    let toggle = MenuItem::with_id(
        app,
        ID_TOGGLE,
        if *daemon == DaemonState::Running {
            "Stop Engine"
        } else {
            "Start Engine"
        },
        !transitioning,
        None::<&str>,
    )?;
    let restart = MenuItem::with_id(
        app,
        ID_RESTART,
        "Restart Engine",
        *daemon == DaemonState::Running,
        None::<&str>,
    )?;
    let nav_dashboard = MenuItem::with_id(app, ID_NAV_DASHBOARD, "Dashboard", true, None::<&str>)?;
    let nav_containers =
        MenuItem::with_id(app, ID_NAV_CONTAINERS, "Containers", true, None::<&str>)?;
    let nav_images = MenuItem::with_id(app, ID_NAV_IMAGES, "Images", true, None::<&str>)?;
    let nav_volumes = MenuItem::with_id(app, ID_NAV_VOLUMES, "Volumes", true, None::<&str>)?;
    let nav_clusters =
        MenuItem::with_id(app, ID_NAV_CLUSTERS, "Kubernetes", true, None::<&str>)?;
    let nav_settings = MenuItem::with_id(app, ID_NAV_SETTINGS, "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::new(app)?;
    menu.append(&title_row)?;
    menu.append(&stats_row)?;
    // The cluster block (and its separator) exists only when there is a
    // cluster to show — otherwise the menu stays short.
    if !cluster_rows.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&MenuItem::with_id(
            app,
            "clusters-title",
            "Clusters",
            false,
            None::<&str>,
        )?)?;
        for row in &cluster_rows {
            menu.append(row)?;
        }
        if clusters.len() > MAX_CLUSTER_ROWS {
            menu.append(&MenuItem::with_id(
                app,
                "clusters-more",
                format!("…and {} more", clusters.len() - MAX_CLUSTER_ROWS),
                false,
                None::<&str>,
            )?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&nav_dashboard)?;
    menu.append(&nav_containers)?;
    menu.append(&nav_images)?;
    menu.append(&nav_volumes)?;
    menu.append(&nav_clusters)?;
    menu.append(&nav_settings)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&restart)?;
    menu.append(&toggle)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;

    Ok(MenuParts {
        menu,
        title_row,
        stats_row,
        cluster_rows,
        toggle,
        restart,
        cluster_sig: cluster_signature(clusters),
    })
}

/// Creates the tray + menu, manages [`TrayState`], wires event listeners, and
/// spawns the monitor task. Call from app setup, after `AppState` is managed.
pub fn init_tray(app: &AppHandle) -> tauri::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Signal>();

    let daemon = app
        .try_state::<AppState>()
        .map(|s| s.daemon_state())
        .unwrap_or_default();
    // Only the clusters that exist at build time get rows; `refresh` rebuilds
    // the menu when that set changes, so the tray never grows blank rows.
    let built = build_menu(app, &daemon, None)?;

    let tx_menu = tx.clone();
    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(state_icon(state_color(&daemon)))
        .icon_as_template(false)
        .tooltip(&format_tooltip(&state_label(&daemon), None))
        .menu(&built.menu)
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
            title_row: built.title_row,
            stats_row: built.stats_row,
            cluster_rows: built.cluster_rows,
            toggle: built.toggle,
            restart: built.restart,
            cluster_sig: built.cluster_sig,
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
        ID_RESTART => restart_engine_from_tray(app).await,
        // Page shortcuts mirror the sidebar one-for-one (no duplicate actions).
        ID_NAV_DASHBOARD
        | ID_NAV_CONTAINERS
        | ID_NAV_IMAGES
        | ID_NAV_VOLUMES
        | ID_NAV_CLUSTERS
        | ID_NAV_SETTINGS => {
            show_main_window(app);
            let target = match id {
                ID_NAV_DASHBOARD => "dashboard",
                ID_NAV_CONTAINERS => "containers",
                ID_NAV_IMAGES => "images",
                ID_NAV_VOLUMES => "volumes",
                ID_NAV_CLUSTERS => "kubernetes",
                _ => "settings",
            };
            let _ = app.emit("tray-navigate", target);
        }
        ID_QUIT => app.exit(0),
        // Clicking a cluster row opens that cluster. Navigation state lives in
        // the React shell, so the tray only reveals the window and emits the
        // intent.
        id if id.starts_with(CLUSTER_ROW_PREFIX) => {
            show_main_window(app);
            let cluster = &id[CLUSTER_ROW_PREFIX.len()..];
            // One event carries both the page and the cluster so the shell can
            // mount the page and open the monitor in the same render pass.
            let _ = app.emit("tray-navigate", format!("kubernetes:{cluster}"));
        }
        _ => {}
    }
}

/// Label "main" comes from tauri.conf.json; show+unminimize+focus covers every
/// way the window can be hidden.
fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
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

/// Tray Restart: a stop followed by a start through the same gated path.
/// Refuses to run against a remote endpoint, exactly like the toggle.
async fn restart_engine_from_tray(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    if let Err(e) = state.engine_control_guard().await {
        record_notice(app, "Engine controls unavailable", &e);
        return;
    }
    if state.daemon_state() != DaemonState::Running {
        return;
    }
    let engine = state.engine.clone();
    match crate::runtime::engine_op(move || engine.stop()).await {
        Ok(Ok(())) => {
            *state.client.write().await = None;
        }
        Ok(Err(e)) => {
            record_notice(app, "Engine restart failed", &e.to_string());
            return;
        }
        Err(e) => {
            record_notice(app, "Engine restart failed", &e);
            return;
        }
    }
    let engine = state.engine.clone();
    match crate::runtime::engine_op(move || engine.start()).await {
        Ok(Ok(())) => {
            let _ = state.get_client().await;
        }
        Ok(Err(e)) => record_notice(app, "Engine restart failed", &e.to_string()),
        Err(e) => record_notice(app, "Engine restart failed", &e),
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
#[derive(Clone)]
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

    let sig = stats
        .as_ref()
        .map(|s| cluster_signature(&s.clusters))
        .unwrap_or_default();
    // GTK menu items cannot be hidden once added, so a changed cluster set
    // rebuilds the menu instead of leaving blank placeholder rows behind.
    if parts.cluster_sig != sig {
        if let Some(ts) = app.try_state::<TrayState>() {
            if let Ok(mut guard) = ts.parts.lock() {
                if let Ok(built) = build_menu(app, &daemon, stats.as_ref()) {
                    if let Some(tray) = &guard.tray {
                        let _ = tray.set_menu(Some(built.menu));
                    }
                    guard.title_row = built.title_row;
                    guard.stats_row = built.stats_row;
                    guard.cluster_rows = built.cluster_rows;
                    guard.toggle = built.toggle;
                    guard.restart = built.restart;
                    guard.cluster_sig = built.cluster_sig;
                    let refreshed = guard.clone();
                    drop(guard);
                    update_rows(&refreshed, &daemon, &label, stats.as_ref());
                    return;
                }
            }
        }
    }

    update_rows(&parts, &daemon, &label, stats.as_ref());
}

/// Writes the live values into the current menu handles. Row count and names
/// are owned by `build_menu`; this only updates text and enabled state.
fn update_rows(
    parts: &TrayParts,
    daemon: &DaemonState,
    _label: &str,
    stats: Option<&TrayStats>,
) {
    let _ = parts.stats_row.set_text(format_status_row(daemon, stats));
    let running = *daemon == DaemonState::Running;
    let transitioning = matches!(
        daemon,
        DaemonState::Starting | DaemonState::Stopping | DaemonState::Installing
    );
    let _ = parts.toggle.set_text(if running {
        "Stop Engine"
    } else {
        "Start Engine"
    });
    let _ = parts.toggle.set_enabled(!transitioning);
    // Restart only makes sense against a live engine, and must not race a
    // start/stop that is already in flight.
    let _ = parts.restart.set_enabled(running);
    let clusters = stats.map(|s| s.clusters.as_slice()).unwrap_or(&[]);
    for (row, c) in parts.cluster_rows.iter().zip(clusters) {
        let _ = row.set_text(format_cluster_row(c));
    }
}

async fn fetch_stats(state: &AppState, daemon: &DaemonState) -> Option<TrayStats> {
    if *daemon != DaemonState::Running {
        return None;
    }
    let client = state.get_client().await.ok()?;
    let containers = client.list_containers(true).await.ok()?;
    let total = containers.len();
    let running = containers.iter().filter(|c| c.running).count();
    let configured = state.config.read().await.kind_default_image.clone();
    let images = client
        .list_images_visible(Some(configured.as_str()))
        .await
        .ok()?
        .len();
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
    // Alphabetical: the tray rows keep their position across refreshes, so the
    // menu is only rebuilt when the *set* of clusters changes.
    clusters.sort_by(|a, b| a.name.cmp(&b.name));
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

/// The container glyph the app is branded with. It ships white, so tinting is
/// a flat RGB swap: the alpha channel is the shape (anti-aliased edges
/// included) and keeps the menu-bar silhouette.
const TRAY_GLYPH_PNG: &[u8] = include_bytes!("../icons/tray-glyph.png");

fn state_icon(rgb: [u8; 3]) -> Image<'static> {
    tinted_glyph(rgb)
        .unwrap_or_else(|| Image::new_owned(vec![0u8; 32 * 32 * 4], 32, 32))
}

/// RGBA buffer of the glyph painted in `rgb`; `None` only if the bundled PNG
/// fails to decode (a corrupt asset, i.e. a build error rather than a runtime
/// condition) — the caller falls back to a blank icon instead of panicking.
fn tinted_glyph(rgb: [u8; 3]) -> Option<Image<'static>> {
    let glyph = Image::from_bytes(TRAY_GLYPH_PNG).ok()?;
    let (width, height) = (glyph.width(), glyph.height());
    let mut rgba = glyph.rgba().to_vec();
    for px in rgba.chunks_exact_mut(4) {
        px[0] = rgb[0];
        px[1] = rgb[1];
        px[2] = rgb[2];
    }
    Some(Image::new_owned(rgba, width, height))
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
    fn tray_glyph_keeps_its_silhouette_and_takes_the_state_colour() {
        let img = tinted_glyph([36, 150, 237]).expect("bundled glyph decodes");
        let (w, h) = (img.width() as usize, img.height() as usize);
        let rgba = img.rgba();
        let px = |x: usize, y: usize| &rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];

        assert_eq!(px(0, 0)[3], 0, "corner stays transparent");

        let solid = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .find(|&(x, y)| px(x, y)[3] == 255)
            .expect("glyph has opaque pixels");
        assert_eq!(&px(solid.0, solid.1)[..3], &[36, 150, 237], "tinted");
    }

    #[test]
    fn status_row_shows_counts_only_with_live_stats() {
        let stats = TrayStats {
            containers: 4,
            running: 2,
            images: 9,
            kinds: 0,
            clusters: vec![],
        };
        assert_eq!(
            format_status_row(&DaemonState::Running, Some(&stats)),
            "Engine Running · 4 containers · 2 running · 9 images"
        );
        assert_eq!(
            format_status_row(&DaemonState::Stopped, None),
            "Engine Stopped"
        );
        assert_eq!(
            format_status_row(&DaemonState::Starting, None),
            "Engine Starting…"
        );
    }

    #[test]
    fn cluster_row_text_names_the_node_phase() {
        let full = ClusterStat {
            name: "kind-local".into(),
            nodes: 3,
            running: 3,
        };
        assert_eq!(format_cluster_row(&full), "kind-local · 3/3 nodes · running");
        let degraded = ClusterStat {
            name: "kind-mixed".into(),
            nodes: 3,
            running: 1,
        };
        assert_eq!(
            format_cluster_row(&degraded),
            "kind-mixed · 1/3 nodes · degraded"
        );
        let stopped = ClusterStat {
            name: "kind-off".into(),
            nodes: 2,
            running: 0,
        };
        assert_eq!(format_cluster_row(&stopped), "kind-off · 0/2 nodes · stopped");
    }

    #[test]
    fn signature_only_changes_when_the_cluster_set_changes() {
        let a = ClusterStat {
            name: "a".into(),
            nodes: 1,
            running: 1,
        };
        let b = ClusterStat {
            name: "b".into(),
            nodes: 1,
            running: 0,
        };
        // Node counts are rendered into the row text, not the layout, so they
        // must not force a rebuild.
        let a_running = ClusterStat {
            name: "a".into(),
            nodes: 4,
            running: 4,
        };
        assert_eq!(cluster_signature(&[a.clone()]), cluster_signature(&[a_running]));
        assert_ne!(cluster_signature(&[a.clone()]), cluster_signature(&[a, b]));
        assert_eq!(cluster_signature(&[]), "");
    }
}
