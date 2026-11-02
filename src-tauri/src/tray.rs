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
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Listener, Manager, Wry};
use tauri_plugin_notification::NotificationExt;

use crate::docker::models::DaemonState;
use crate::state::AppState;

/// A terminal stream event may re-emit `done` (chunked sends, retries); the
/// same logical outcome must toast once inside this window.
const NOTIFY_COOLDOWN: Duration = Duration::from_secs(15);
/// Menu + tooltip refresh cadence when nothing else is happening.
const REFRESH_INTERVAL: Duration = Duration::from_secs(15);

// Menu ids — matched in the tray's `on_menu_event` handler.
const ID_TOGGLE: &str = "engine-toggle";
const ID_RESTART: &str = "engine-restart";
const ID_OPEN_DASHBOARD: &str = "open-dashboard";
const ID_QUIT: &str = "quit";

// Dynamic action prefixes
const GROUP_START_PREFIX: &str = "grp-start-";
const GROUP_STOP_PREFIX: &str = "grp-stop-";
const CLUSTER_NAV_PREFIX: &str = "cls-nav-";
const CLUSTER_START_PREFIX: &str = "cls-start-";
const CLUSTER_STOP_PREFIX: &str = "cls-stop-";
const GATEWAY_ROUTE_PREFIX: &str = "gw-route-";

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
    title_row: MenuItem<Wry>,
    stats_row: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    restart: MenuItem<Wry>,
    menu_sig: String,
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

/// Shown when "Close to tray" hides the window, so the close does not read as
/// a hang. Gated by Settings → Notifications → "Hidden to tray" like every
/// other category; the tray Quit path is what stops the app.
pub fn notify_hidden_to_tray(app: &AppHandle) {
    let enabled = app
        .try_state::<std::sync::Arc<AppState>>()
        .and_then(|state| {
            state
                .config
                .try_read()
                .ok()
                .map(|cfg| cfg.notifications.tray_hidden)
        })
        .unwrap_or(true);
    if !enabled {
        return;
    }
    record_notice(
        app,
        "Hsx2Dock is still running",
        "The window is in the tray and the engine keeps running. Click the tray icon to reopen, or Quit there to stop everything.",
    );
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
    toggle: MenuItem<Wry>,
    restart: MenuItem<Wry>,
    /// Signature of dynamic sections (groups, clusters, gateway routes); changes trigger rebuild.
    menu_sig: String,
}

fn menu_signature(stats: Option<&TrayStats>) -> String {
    let Some(s) = stats else {
        return String::new();
    };
    let mut parts = Vec::new();
    for g in &s.groups {
        parts.push(format!("g:{}:{}:{}:{:.1}:{:.1}", g.id, g.running, g.total, g.cpu_percent, g.mem_usage_mb));
    }
    for c in &s.clusters {
        parts.push(format!("c:{}:{}:{}:{:.1}:{:.1}", c.name, c.running, c.nodes, c.cpu_percent, c.mem_usage_mb));
    }
    parts.push(format!("gw:{}:{}", s.gateway_running, s.active_routes.len()));
    for r in &s.active_routes {
        parts.push(format!("r:{}:{}", r.domain, r.target));
    }
    parts.join("|")
}

/// Status line under the title — engine state and summary.
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

/// Lays the menu out cleanly: Container Groups, Kubernetes Clusters, Gateway,
/// Open Dashboard, Engine controls, and Quit.
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
    let open_dashboard = MenuItem::with_id(
        app,
        ID_OPEN_DASHBOARD,
        "Open Hsx2Dock Dashboard",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?;

    let menu = Menu::new(app)?;
    menu.append(&title_row)?;
    menu.append(&stats_row)?;

    if let Some(s) = stats {
        // ================= Container Groups Section =================
        if !s.groups.is_empty() {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
            let groups_header = MenuItem::with_id(
                app,
                "header-groups",
                format!("Container Groups ({})", s.groups.len()),
                false,
                None::<&str>,
            )?;
            menu.append(&groups_header)?;

            for g in &s.groups {
                let res_str = if g.running > 0 && (g.cpu_percent > 0.0 || g.mem_usage_mb > 0.0) {
                    format!(" · CPU: {:.1}% | RAM: {:.0} MB", g.cpu_percent, g.mem_usage_mb)
                } else {
                    String::new()
                };
                let group_title = format!("{} ({}/{} running){}", g.name, g.running, g.total, res_str);
                let group_sub = Submenu::new(app, &group_title, true)?;

                let start_item = MenuItem::with_id(
                    app,
                    format!("{GROUP_START_PREFIX}{}", g.id),
                    "Start Group",
                    g.running < g.total,
                    None::<&str>,
                )?;
                let stop_item = MenuItem::with_id(
                    app,
                    format!("{GROUP_STOP_PREFIX}{}", g.id),
                    "Stop Group",
                    g.running > 0,
                    None::<&str>,
                )?;

                group_sub.append(&start_item)?;
                group_sub.append(&stop_item)?;
                menu.append(&group_sub)?;
            }
        }

        // ================= Kubernetes Clusters Section =================
        if !s.clusters.is_empty() {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
            let active_cls = s.clusters.iter().filter(|c| c.running > 0).count();
            let cls_header = MenuItem::with_id(
                app,
                "header-clusters",
                format!("Kubernetes Clusters ({}/{} active)", active_cls, s.clusters.len()),
                false,
                None::<&str>,
            )?;
            menu.append(&cls_header)?;

            for c in &s.clusters {
                let phase = if c.running == 0 {
                    "stopped"
                } else if c.running == c.nodes {
                    "running"
                } else {
                    "degraded"
                };
                let res_str = if c.running > 0 && (c.cpu_percent > 0.0 || c.mem_usage_mb > 0.0) {
                    format!(" · CPU: {:.1}% | RAM: {:.0} MB", c.cpu_percent, c.mem_usage_mb)
                } else {
                    String::new()
                };
                let cluster_title = format!("{} ({}/{} nodes, {}){}", c.name, c.running, c.nodes, phase, res_str);
                let cluster_sub = Submenu::new(app, &cluster_title, true)?;

                let nav_item = MenuItem::with_id(
                    app,
                    format!("{CLUSTER_NAV_PREFIX}{}", c.name),
                    "Open in App",
                    true,
                    None::<&str>,
                )?;
                let start_item = MenuItem::with_id(
                    app,
                    format!("{CLUSTER_START_PREFIX}{}", c.name),
                    "Start Cluster",
                    c.running < c.nodes,
                    None::<&str>,
                )?;
                let stop_item = MenuItem::with_id(
                    app,
                    format!("{CLUSTER_STOP_PREFIX}{}", c.name),
                    "Stop Cluster",
                    c.running > 0,
                    None::<&str>,
                )?;

                cluster_sub.append(&nav_item)?;
                cluster_sub.append(&start_item)?;
                cluster_sub.append(&stop_item)?;
                menu.append(&cluster_sub)?;
            }
        }

        // ================= Gateway Section =================
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        let gw_status_label = if s.gateway_running {
            format!("Gateway: Online ({} bound routes)", s.active_routes.len())
        } else {
            "Gateway: Offline".to_string()
        };
        let gw_header = MenuItem::with_id(
            app,
            "header-gateway",
            gw_status_label,
            false,
            None::<&str>,
        )?;
        menu.append(&gw_header)?;

        if s.gateway_running && !s.active_routes.is_empty() {
            let routes_sub = Submenu::new(
                app,
                format!("Public URLs ({})", s.active_routes.len()),
                true,
            )?;
            for r in &s.active_routes {
                let container_info = r
                    .container_name
                    .as_deref()
                    .map(|c| format!(" [{}]", c))
                    .unwrap_or_default();
                let route_item = MenuItem::with_id(
                    app,
                    format!("{GATEWAY_ROUTE_PREFIX}{}", r.public_url),
                    format!("{} → {}{}", r.public_url, r.target, container_info),
                    true,
                    None::<&str>,
                )?;
                routes_sub.append(&route_item)?;
            }
            menu.append(&routes_sub)?;
        }
    }

    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&open_dashboard)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&restart)?;
    menu.append(&toggle)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;

    Ok(MenuParts {
        menu,
        title_row,
        stats_row,
        toggle,
        restart,
        menu_sig: menu_signature(stats),
    })
}

/// Creates the tray + menu, manages [`TrayState`], wires event listeners, and
/// spawns the monitor task. Call from app setup, after `AppState` is managed.
pub fn init_tray(app: &AppHandle) -> tauri::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Signal>();

    let daemon = app
        .try_state::<std::sync::Arc<AppState>>()
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
            toggle: built.toggle,
            restart: built.restart,
            menu_sig: built.menu_sig,
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
            if !should_notify(
                &mut ctx.notified,
                &format!("{verb}:{image}"),
                NOTIFY_COOLDOWN,
            ) {
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
                    let title = if pushed {
                        "Image pushed"
                    } else {
                        "Image pulled"
                    };
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
            if !should_notify(
                &mut ctx.notified,
                &format!("kind:{verb}:{name}"),
                NOTIFY_COOLDOWN,
            ) {
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
        ID_OPEN_DASHBOARD => {
            show_main_window(app);
            let _ = app.emit("tray-navigate", "dashboard");
        }
        ID_QUIT => app.exit(0),

        // Container Group start/stop actions
        id if id.starts_with(GROUP_START_PREFIX) => {
            let group_id = &id[GROUP_START_PREFIX.len()..];
            if let Some(state) = app.try_state::<std::sync::Arc<AppState>>() {
                match crate::commands::groups::start_container_group_core(&state, group_id).await {
                    Ok(summary) => record_notice(app, "Container Group Started", &summary),
                    Err(e) => record_notice(app, "Start Group Failed", &e),
                }
            }
        }
        id if id.starts_with(GROUP_STOP_PREFIX) => {
            let group_id = &id[GROUP_STOP_PREFIX.len()..];
            if let Some(state) = app.try_state::<std::sync::Arc<AppState>>() {
                match crate::commands::groups::stop_container_group_core(&state, group_id).await {
                    Ok(summary) => record_notice(app, "Container Group Stopped", &summary),
                    Err(e) => record_notice(app, "Stop Group Failed", &e),
                }
            }
        }

        // Kubernetes Cluster actions
        id if id.starts_with(CLUSTER_NAV_PREFIX) => {
            show_main_window(app);
            let cluster = &id[CLUSTER_NAV_PREFIX.len()..];
            let _ = app.emit("tray-navigate", format!("kubernetes:{cluster}"));
        }
        id if id.starts_with(CLUSTER_START_PREFIX) => {
            let cluster = &id[CLUSTER_START_PREFIX.len()..];
            if let Some(state) = app.try_state::<std::sync::Arc<AppState>>() {
                match crate::commands::kind::set_cluster_power(Some(app), &state, cluster, true).await {
                    Ok(()) => record_notice(app, "Cluster Started", &format!("Kind cluster '{cluster}' started")),
                    Err(e) => record_notice(app, "Start Cluster Failed", &e),
                }
            }
        }
        id if id.starts_with(CLUSTER_STOP_PREFIX) => {
            let cluster = &id[CLUSTER_STOP_PREFIX.len()..];
            if let Some(state) = app.try_state::<std::sync::Arc<AppState>>() {
                match crate::commands::kind::set_cluster_power(Some(app), &state, cluster, false).await {
                    Ok(()) => record_notice(app, "Cluster Stopped", &format!("Kind cluster '{cluster}' stopped")),
                    Err(e) => record_notice(app, "Stop Cluster Failed", &e),
                }
            }
        }

        // Public Gateway Route item click -> open in default browser
        id if id.starts_with(GATEWAY_ROUTE_PREFIX) => {
            let url = &id[GATEWAY_ROUTE_PREFIX.len()..];
            let _ = std::process::Command::new("xdg-open")
                .arg(url)
                .spawn();
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
    let Some(state) = app.try_state::<std::sync::Arc<AppState>>() else {
        return;
    };
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
    let Some(state) = app.try_state::<std::sync::Arc<AppState>>() else {
        return;
    };
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
    app.try_state::<std::sync::Arc<AppState>>()
        .and_then(|s| s.config.try_read().ok().map(|c| c.notifications.clone()))
        .unwrap_or_default()
}

/// Shows a desktop notification. Callers check the Settings prefs first;
/// interactive failures (guard errors, start/stop failures) bypass them.
fn record_notice(app: &AppHandle, title: &str, body: &str) {
    show_toast(app, title, body);
}

fn show_toast(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
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
    groups: Vec<GroupStat>,
    clusters: Vec<ClusterStat>,
    gateway_running: bool,
    active_routes: Vec<ActiveRouteStat>,
}

#[derive(Clone, Debug, PartialEq)]
struct GroupStat {
    id: String,
    name: String,
    total: usize,
    running: usize,
    cpu_percent: f64,
    mem_usage_mb: f64,
}

/// One kind cluster as the tray menu reports it.
#[derive(Clone, Debug, PartialEq)]
struct ClusterStat {
    name: String,
    nodes: usize,
    running: usize,
    cpu_percent: f64,
    mem_usage_mb: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct ActiveRouteStat {
    domain: String,
    public_url: String,
    target: String,
    container_name: Option<String>,
}

async fn refresh(app: &AppHandle, ctx: &mut MonitorCtx) {
    let Some(parts) = app.try_state::<TrayState>().and_then(|t| t.parts()) else {
        return;
    };
    let Some(state) = app.try_state::<std::sync::Arc<AppState>>() else {
        return;
    };
    let daemon = state.daemon_state();

    let stats = fetch_stats(&state, &daemon).await;
    let label = state_label(&daemon);

    if let Some(tray) = &parts.tray {
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

    let sig = menu_signature(stats.as_ref());
    // Menu rebuilds whenever dynamic items change (counts, resources, routes, or engine state).
    if parts.menu_sig != sig {
        if let Some(ts) = app.try_state::<TrayState>() {
            if let Ok(mut guard) = ts.parts.lock() {
                if let Ok(built) = build_menu(app, &daemon, stats.as_ref()) {
                    if let Some(tray) = &guard.tray {
                        let _ = tray.set_menu(Some(built.menu));
                    }
                    guard.title_row = built.title_row;
                    guard.stats_row = built.stats_row;
                    guard.toggle = built.toggle;
                    guard.restart = built.restart;
                    guard.menu_sig = built.menu_sig;
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
fn update_rows(parts: &TrayParts, daemon: &DaemonState, _label: &str, stats: Option<&TrayStats>) {
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
    let _ = parts.restart.set_enabled(running);
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

    // 1. Group stats: measure CPU % and RAM MB for running containers
    let mut stats_by_id: HashMap<String, (f64, f64)> = HashMap::new();
    for c in &containers {
        if c.running {
            if let Ok(s) = client.container_stats(&c.id).await {
                let mem_mb = s.mem_usage as f64 / (1024.0 * 1024.0);
                stats_by_id.insert(c.id.clone(), (s.cpu_percent, mem_mb));
            }
        }
    }

    // Build container groups (compose projects + custom groups + isolated)
    let config = state.config.read().await.clone();
    let mut groups: Vec<GroupStat> = Vec::new();

    // Compose projects
    let mut compose_map: HashMap<String, Vec<&crate::docker::models::ContainerItem>> = HashMap::new();
    for c in &containers {
        if let Some(ref proj) = c.compose_project {
            compose_map.entry(proj.clone()).or_default().push(c);
        }
    }
    for (proj, members) in compose_map {
        let mut g_running = 0;
        let mut g_cpu = 0.0;
        let mut g_mem = 0.0;
        for m in &members {
            if m.running {
                g_running += 1;
                if let Some(&(cpu, mem)) = stats_by_id.get(&m.id) {
                    g_cpu += cpu;
                    g_mem += mem;
                }
            }
        }
        groups.push(GroupStat {
            id: format!("compose:{proj}"),
            name: format!("compose: {proj}"),
            total: members.len(),
            running: g_running,
            cpu_percent: (g_cpu * 10.0).round() / 10.0,
            mem_usage_mb: (g_mem * 10.0).round() / 10.0,
        });
    }

    // Custom user groups
    let mut custom_claimed_tokens = std::collections::HashSet::new();
    for cg in &config.container_groups {
        if cg.id.starts_with("compose:") || cg.id == "__isolated__" {
            continue;
        }
        for token in &cg.container_ids {
            custom_claimed_tokens.insert(token.as_str());
        }

        let mut members = Vec::new();
        for token in &cg.container_ids {
            if let Some(c) = containers.iter().find(|c| {
                c.id == *token || c.short_id == *token || c.name == *token || c.name.trim_start_matches('/') == *token
            }) {
                members.push(c);
            }
        }

        let mut g_running = 0;
        let mut g_cpu = 0.0;
        let mut g_mem = 0.0;
        for m in &members {
            if m.running {
                g_running += 1;
                if let Some(&(cpu, mem)) = stats_by_id.get(&m.id) {
                    g_cpu += cpu;
                    g_mem += mem;
                }
            }
        }
        groups.push(GroupStat {
            id: cg.id.clone(),
            name: cg.name.clone(),
            total: members.len(),
            running: g_running,
            cpu_percent: (g_cpu * 10.0).round() / 10.0,
            mem_usage_mb: (g_mem * 10.0).round() / 10.0,
        });
    }

    // Isolated (standalone) containers not in compose or custom groups, and not kind node containers
    let isolated: Vec<&crate::docker::models::ContainerItem> = containers
        .iter()
        .filter(|c| {
            c.compose_project.is_none()
                && c.kind_cluster.is_none()
                && !custom_claimed_tokens.contains(c.id.as_str())
                && !custom_claimed_tokens.contains(c.short_id.as_str())
                && !custom_claimed_tokens.contains(c.name.as_str())
                && !custom_claimed_tokens.contains(c.name.trim_start_matches('/'))
        })
        .collect();

    if !isolated.is_empty() {
        let mut g_running = 0;
        let mut g_cpu = 0.0;
        let mut g_mem = 0.0;
        for m in &isolated {
            if m.running {
                g_running += 1;
                if let Some(&(cpu, mem)) = stats_by_id.get(&m.id) {
                    g_cpu += cpu;
                    g_mem += mem;
                }
            }
        }
        groups.push(GroupStat {
            id: "__isolated__".to_string(),
            name: "Standalone Containers".to_string(),
            total: isolated.len(),
            running: g_running,
            cpu_percent: (g_cpu * 10.0).round() / 10.0,
            mem_usage_mb: (g_mem * 10.0).round() / 10.0,
        });
    }
    groups.sort_by(|a, b| a.name.cmp(&b.name));

    // 2. Kind Kubernetes Clusters
    let mut cluster_map: HashMap<String, Vec<&crate::docker::models::ContainerItem>> = HashMap::new();
    for c in &containers {
        if let Some(ref name) = c.kind_cluster {
            cluster_map.entry(name.clone()).or_default().push(c);
        }
    }
    let mut clusters: Vec<ClusterStat> = Vec::new();
    for (name, nodes) in cluster_map {
        let mut c_running = 0;
        let mut c_cpu = 0.0;
        let mut c_mem = 0.0;
        for n in &nodes {
            if n.running {
                c_running += 1;
                if let Some(&(cpu, mem)) = stats_by_id.get(&n.id) {
                    c_cpu += cpu;
                    c_mem += mem;
                }
            }
        }
        clusters.push(ClusterStat {
            name,
            nodes: nodes.len(),
            running: c_running,
            cpu_percent: (c_cpu * 10.0).round() / 10.0,
            mem_usage_mb: (c_mem * 10.0).round() / 10.0,
        });
    }
    clusters.sort_by(|a, b| a.name.cmp(&b.name));
    let kinds = clusters.len();

    // 3. Gateway status & active bound routes
    let gateway_running = config.gateway_enabled && state.caddy.is_running().await;
    let mut active_routes: Vec<ActiveRouteStat> = Vec::new();
    if gateway_running {
        let https_port = config.gateway_https_port;
        for r in &config.gateway_routes {
            if !r.enabled {
                continue;
            }
            // Check if bound container is actively running
            if let Some(ref cname) = r.container_name {
                let is_running = containers.iter().any(|c| {
                    (c.name == *cname || c.name.trim_start_matches('/') == *cname || c.id == *cname || c.short_id == *cname)
                        && c.running
                });
                if !is_running {
                    continue;
                }
            }
            let host_port_str = if https_port == 443 {
                r.domain.clone()
            } else {
                format!("{}:{}", r.domain, https_port)
            };
            let path_str = r.path.as_deref().unwrap_or("");
            let public_url = format!("https://{}{}", host_port_str, path_str);
            active_routes.push(ActiveRouteStat {
                domain: r.domain.clone(),
                public_url,
                target: r.target.clone(),
                container_name: r.container_name.clone(),
            });
        }
    }

    Some(TrayStats {
        containers: total,
        running,
        images,
        kinds,
        groups,
        clusters,
        gateway_running,
        active_routes,
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
    tinted_glyph(rgb).unwrap_or_else(|| Image::new_owned(vec![0u8; 32 * 32 * 4], 32, 32))
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
        assert_eq!(format_tooltip("Running", None), "Hsx2Dock — Engine Running");
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
            groups: vec![],
            clusters: vec![],
            gateway_running: false,
            active_routes: vec![],
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
    fn menu_signature_tracks_dynamic_changes() {
        let mut stats = TrayStats {
            containers: 4,
            running: 2,
            images: 9,
            kinds: 1,
            groups: vec![GroupStat {
                id: "grp1".into(),
                name: "Web Stack".into(),
                total: 2,
                running: 2,
                cpu_percent: 1.5,
                mem_usage_mb: 120.0,
            }],
            clusters: vec![ClusterStat {
                name: "kind-local".into(),
                nodes: 3,
                running: 3,
                cpu_percent: 4.2,
                mem_usage_mb: 850.0,
            }],
            gateway_running: true,
            active_routes: vec![ActiveRouteStat {
                domain: "app.localhost".into(),
                public_url: "https://app.localhost".into(),
                target: "127.0.0.1:8080".into(),
                container_name: Some("web".into()),
            }],
        };
        let sig1 = menu_signature(Some(&stats));
        assert!(sig1.contains("Web Stack") || sig1.contains("grp1"));
        assert!(sig1.contains("kind-local"));
        assert!(sig1.contains("app.localhost"));

        // Changing container count or resource updates the signature
        stats.groups[0].running = 1;
        let sig2 = menu_signature(Some(&stats));
        assert_ne!(sig1, sig2);
    }
}
