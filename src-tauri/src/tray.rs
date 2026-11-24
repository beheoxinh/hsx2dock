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
    /// A spawned menu action finished; only triggers a refresh.
    Refresh,
}

/// Everything needed to mutate the tray in place. Menu items are `Clone`
/// (Arc-backed), so callers snapshot out of the mutex and never hold the
/// guard across an `.await`.
#[derive(Clone)]
struct TrayParts {
    tray: Option<TrayIcon<Wry>>,
    menu: Menu<Wry>,
    title_row: MenuItem<Wry>,
    stats_row: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    restart: MenuItem<Wry>,
    group_items: Vec<(String, Submenu<Wry>)>,
    cluster_items: Vec<(String, Submenu<Wry>)>,
    menu_sig: String,
    last_color: Option<[u8; 3]>,
    last_tooltip: Option<String>,
    last_status_text: Option<String>,
    last_toggle_state: Option<(bool, bool)>,
    last_restart_enabled: Option<bool>,
    /// Last text written to each group/cluster submenu, keyed `g:<id>` /
    /// `c:<name>`. Every tick rewrote all of them, and each write bumps the
    /// DBusMenu revision, so the panel re-read the whole menu every few seconds.
    item_texts: HashMap<String, String>,
    tx: tokio::sync::mpsc::UnboundedSender<Signal>,
}

pub struct TrayState {
    parts: Mutex<TrayParts>,
}

impl TrayState {
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
}

/// The freshly built menu plus the handles `refresh` mutates in place.
struct MenuParts {
    menu: Menu<Wry>,
    title_row: MenuItem<Wry>,
    stats_row: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    restart: MenuItem<Wry>,
    group_items: Vec<(String, Submenu<Wry>)>,
    cluster_items: Vec<(String, Submenu<Wry>)>,
    /// Signature of dynamic sections (groups, clusters, gateway routes); changes trigger rebuild.
    menu_sig: String,
}

fn menu_signature(daemon: &DaemonState, stats: Option<&TrayStats>) -> String {
    let mut parts = vec![format!("daemon:{:?}", daemon)];
    let Some(s) = stats else {
        return parts.join("|");
    };
    // Only structural changes (membership, running/total counts, gateway routes)
    // trigger a full GTK/DBusMenu rebuild; CPU/RAM figures update in-place via
    // `Submenu::set_text` so an open tray menu never flickers or closes.
    for g in &s.groups {
        parts.push(format!("g:{}:{}:{}", g.id, g.running, g.total));
    }
    for c in &s.clusters {
        parts.push(format!("c:{}:{}:{}", c.name, c.running, c.nodes));
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
    let menu = Menu::new(app)?;
    populate_menu(&menu, app, daemon, stats)
}

/// Clears `menu` in-place and populates it with fresh items.
///
/// Mutating the single `Menu` instance instead of calling `tray.set_menu(...)`
/// keeps the underlying `GtkMenu` / `DbusmenuServer` alive on Linux
/// (`libayatana-appindicator`), preventing GNOME Shell's AppIndicator extension
/// from losing the menu layout and rendering an empty popup.
fn populate_menu(
    menu: &Menu<Wry>,
    app: &AppHandle,
    daemon: &DaemonState,
    stats: Option<&TrayStats>,
) -> tauri::Result<MenuParts> {
    if let Ok(existing) = menu.items() {
        for item in existing {
            let _ = match item {
                tauri::menu::MenuItemKind::MenuItem(i) => menu.remove(&i),
                tauri::menu::MenuItemKind::Submenu(s) => menu.remove(&s),
                tauri::menu::MenuItemKind::Predefined(p) => menu.remove(&p),
                tauri::menu::MenuItemKind::Check(c) => menu.remove(&c),
                tauri::menu::MenuItemKind::Icon(i) => menu.remove(&i),
            };
        }
    }

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

    menu.append(&title_row)?;
    menu.append(&stats_row)?;

    let mut group_items: Vec<(String, Submenu<Wry>)> = Vec::new();
    let mut cluster_items: Vec<(String, Submenu<Wry>)> = Vec::new();

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
                let group_title = format_group_title(g);
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
                group_items.push((g.id.clone(), group_sub));
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
                let cluster_title = format_cluster_title(c);
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
                cluster_items.push((c.name.clone(), cluster_sub));
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
        menu: menu.clone(),
        title_row,
        stats_row,
        toggle,
        restart,
        group_items,
        cluster_items,
        menu_sig: menu_signature(daemon, stats),
    })
}

fn format_group_title(g: &GroupStat) -> String {
    let res_str = if g.running > 0 && (g.cpu_percent > 0.0 || g.mem_usage_mb > 0.0) {
        format!(" · CPU: {:.1}% | RAM: {:.0} MB", g.cpu_percent, g.mem_usage_mb)
    } else {
        String::new()
    };
    format!("{} ({}/{} running){}", g.name, g.running, g.total, res_str)
}

fn format_cluster_title(c: &ClusterStat) -> String {
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
    format!("{} ({}/{} nodes, {}){}", c.name, c.running, c.nodes, phase, res_str)
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

    let initial_color = state_color(&daemon);
    let initial_tooltip = format_tooltip(&state_label(&daemon), None);
    let tx_menu = tx.clone();
    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(state_icon(initial_color))
        .icon_as_template(false)
        .tooltip(&initial_tooltip)
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
            menu: built.menu,
            title_row: built.title_row,
            stats_row: built.stats_row,
            toggle: built.toggle,
            restart: built.restart,
            group_items: built.group_items,
            cluster_items: built.cluster_items,
            // Leave empty so the first `refresh` on the running GTK main loop
            // always populates the menu and emits `LayoutUpdated` for GNOME
            // AppIndicator, even when the Docker engine is stopped.
            menu_sig: String::new(),
            last_color: Some(initial_color),
            last_tooltip: Some(initial_tooltip),
            last_status_text: None,
            last_toggle_state: None,
            last_restart_enabled: None,
            item_texts: HashMap::new(),
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
        // Progress fires per layer chunk (hundreds/s); only the final event
        // matters here, so skip the full parse for the rest.
        if !payload.contains("\"done\":true") {
            return None;
        }
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
        // Progress fires per layer chunk (hundreds/s); only the final event
        // matters here, so skip the full parse for the rest.
        if !payload.contains("\"done\":true") {
            return None;
        }
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
        // Progress fires per layer chunk (hundreds/s); only the final event
        // matters here, so skip the full parse for the rest.
        if !payload.contains("\"done\":true") {
            return None;
        }
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
    let tx_loop = tx.clone();
    crate::runtime::spawn(async move {
        let mut ctx = MonitorCtx {
            notified: HashMap::new(),
            // Deliberately None: the first `engine-state-changed` we hear is a
            // real transition (autostart, toggle, or death), so toast it.
            last_engine: None,
        };
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        refresh(&handle, &mut ctx).await;
        loop {
            tokio::select! {
                sig = rx.recv() => {
                    let Some(sig) = sig else { break };
                    if let Signal::Menu(id) = sig {
                        // Group/cluster power and engine toggles take minutes;
                        // awaited here they froze the tray's refresh and
                        // queued every other click (Quit included) behind
                        // them. They run on their own and ask for a refresh.
                        let app = handle.clone();
                        let tx = tx_loop.clone();
                        crate::runtime::spawn(async move {
                            handle_menu(&app, &id).await;
                            let _ = tx.send(Signal::Refresh);
                        });
                        continue;
                    }
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
        Signal::Refresh => {}
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
            // Reaped on a helper thread so each click does not leave a zombie.
            if let Ok(mut child) = std::process::Command::new("xdg-open").arg(url).spawn() {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
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
    let _lifecycle = match crate::runtime::claim_engine_lifecycle() {
        Ok(g) => g,
        Err(e) => return record_notice(app, "Engine busy", &e),
    };
    match state.daemon_state() {
        DaemonState::Running => {
            let engine = state.engine.clone();
            match crate::runtime::engine_op(move || engine.stop()).await {
                Ok(Ok(())) => {
                    state.invalidate_client();
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
    let _lifecycle = match crate::runtime::claim_engine_lifecycle() {
        Ok(g) => g,
        Err(e) => return record_notice(app, "Engine busy", &e),
    };
    if state.daemon_state() != DaemonState::Running {
        return;
    }
    let engine = state.engine.clone();
    match crate::runtime::engine_op(move || engine.stop()).await {
        Ok(Ok(())) => {
            state.invalidate_client();
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
    // `try_read` fell back to the defaults whenever a writer held the lock,
    // so a notification the user turned off could still fire.
    match app.try_state::<std::sync::Arc<AppState>>() {
        Some(s) => s.config.read().await.notifications.clone(),
        None => Default::default(),
    }
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

async fn refresh(app: &AppHandle, _ctx: &mut MonitorCtx) {
    let Some(state) = app.try_state::<std::sync::Arc<AppState>>() else {
        return;
    };
    let daemon = state.daemon_state();
    let stats = fetch_stats(&state, &daemon).await;

    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        apply_tray_update(&handle, &daemon, stats.as_ref());
    });
}

/// Runs on the GTK/UI main thread so `Menu`, `Submenu`, `set_menu`, `set_icon`,
/// and `set_tooltip` never touch GTK3/libayatana-appindicator from a Tokio worker.
fn apply_tray_update(app: &AppHandle, daemon: &DaemonState, stats: Option<&TrayStats>) {
    let Some(ts) = app.try_state::<TrayState>() else {
        return;
    };
    let Ok(mut guard) = ts.parts.lock() else {
        return;
    };

    let label = state_label(daemon);
    let color = state_color(daemon);
    let tooltip = format_tooltip(
        &label,
        stats.map(|s| (s.containers, s.running, s.images, s.kinds)),
    );

    if guard.last_color != Some(color) {
        if let Some(tray) = &guard.tray {
            let _ = tray.set_icon(Some(state_icon(color)));
        }
        guard.last_color = Some(color);
    }
    if guard.last_tooltip.as_deref() != Some(tooltip.as_str()) {
        if let Some(tray) = &guard.tray {
            let _ = tray.set_tooltip(Some(&tooltip));
        }
        guard.last_tooltip = Some(tooltip);
    }

    let sig = menu_signature(daemon, stats);
    if guard.menu_sig != sig {
        let menu = guard.menu.clone();
        if let Ok(built) = populate_menu(&menu, app, daemon, stats) {
            guard.title_row = built.title_row;
            guard.stats_row = built.stats_row;
            guard.toggle = built.toggle;
            guard.restart = built.restart;
            guard.group_items = built.group_items;
            guard.cluster_items = built.cluster_items;
            guard.menu_sig = built.menu_sig;
            guard.last_status_text = Some(format_status_row(daemon, stats));
            let running = *daemon == DaemonState::Running;
            let transitioning = matches!(
                daemon,
                DaemonState::Starting | DaemonState::Stopping | DaemonState::Installing
            );
            guard.last_toggle_state = Some((running, !transitioning));
            guard.last_restart_enabled = Some(running);
            guard.item_texts.clear();
            return;
        }
    }

    update_rows(&mut guard, daemon, &label, stats);
}

/// Writes the live values into the current menu handles. Row count and names
/// are owned by `populate_menu`; this only updates text and enabled state when
/// values actually change so DBusMenu revision does not churn every tick.
fn update_rows(parts: &mut TrayParts, daemon: &DaemonState, _label: &str, stats: Option<&TrayStats>) {
    let status_text = format_status_row(daemon, stats);
    if parts.last_status_text.as_deref() != Some(status_text.as_str()) {
        let _ = parts.stats_row.set_text(&status_text);
        parts.last_status_text = Some(status_text);
    }

    let running = *daemon == DaemonState::Running;
    let transitioning = matches!(
        daemon,
        DaemonState::Starting | DaemonState::Stopping | DaemonState::Installing
    );
    let toggle_state = (running, !transitioning);
    if parts.last_toggle_state != Some(toggle_state) {
        let _ = parts.toggle.set_text(if running {
            "Stop Engine"
        } else {
            "Start Engine"
        });
        let _ = parts.toggle.set_enabled(!transitioning);
        parts.last_toggle_state = Some(toggle_state);
    }

    if parts.last_restart_enabled != Some(running) {
        let _ = parts.restart.set_enabled(running);
        parts.last_restart_enabled = Some(running);
    }

    if let Some(s) = stats {
        for (id, sub) in &parts.group_items {
            if let Some(g) = s.groups.iter().find(|g| &g.id == id) {
                let text = format_group_title(g);
                let key = format!("g:{id}");
                if parts.item_texts.get(&key) != Some(&text) {
                    let _ = sub.set_text(&text);
                    parts.item_texts.insert(key, text);
                }
            }
        }
        for (name, sub) in &parts.cluster_items {
            if let Some(c) = s.clusters.iter().find(|c| &c.name == name) {
                let text = format_cluster_title(c);
                let key = format!("c:{name}");
                if parts.item_texts.get(&key) != Some(&text) {
                    let _ = sub.set_text(&text);
                    parts.item_texts.insert(key, text);
                }
            }
        }
    }
}

/// Image count for the status row. Listing images walks every layer on the
/// daemon side; the count rarely changes, so a minute-old figure is fine and
/// saves three of every four list calls.
async fn cached_image_count(
    client: &crate::docker::client::DockerClient,
    configured: &str,
) -> Option<usize> {
    static CACHE: Mutex<Option<(std::time::Instant, usize)>> = Mutex::new(None);
    const TTL: Duration = Duration::from_secs(60);
    if let Ok(guard) = CACHE.lock() {
        if let Some((at, n)) = *guard {
            if at.elapsed() < TTL {
                return Some(n);
            }
        }
    }
    let n = client.list_images_visible(Some(configured)).await.ok()?.len();
    if let Ok(mut guard) = CACHE.lock() {
        *guard = Some((std::time::Instant::now(), n));
    }
    Some(n)
}

async fn fetch_stats(state: &AppState, daemon: &DaemonState) -> Option<TrayStats> {
    if *daemon != DaemonState::Running {
        return None;
    }
    let client = state.get_client().await.ok()?;
    let containers = client.list_containers(true).await.ok()?;
    let total = containers.len();
    let running = containers.iter().filter(|c| c.running).count();
    // Only the pieces of config the tray reads, in one lock: cloning the whole
    // config (endpoints, registries, kind prefs…) every tick was wasted work.
    let (configured, container_groups, gateway_routes, https_port) = {
        let cfg = state.config.read().await;
        (
            cfg.kind_default_image.clone(),
            cfg.container_groups.clone(),
            cfg.gateway_routes.clone(),
            cfg.gateway_https_port,
        )
    };
    let images = cached_image_count(&client, &configured).await?;

    // 1. Group stats: read from cached_stats updated by background monitor
    // so tray refresh is instant and does not bombard the Docker socket with sequential HTTP calls
    let stats_by_id: HashMap<String, (f64, f64)> = {
        let guard = state.cached_stats.read().await;
        guard
            .iter()
            .map(|(id, s)| {
                let mem_mb = s.mem_usage as f64 / (1024.0 * 1024.0);
                (id.clone(), (s.cpu_percent, mem_mb))
            })
            .collect()
    };

    // Build container groups (compose projects + custom groups + isolated)
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
    for cg in &container_groups {
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
    let gateway_running = state.caddy.is_running().await;
    let mut active_routes: Vec<ActiveRouteStat> = Vec::new();
    if gateway_running {
        for r in &gateway_routes {
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

/// Docker-engine blue while Running, cool slate when Stopped, amber for
/// transitions, and rose red for errors.
fn state_color(state: &DaemonState) -> [u8; 3] {
    match state {
        DaemonState::Running => [36, 150, 237], // #2496ED
        DaemonState::Stopped | DaemonState::NotInstalled => [100, 116, 139], // #64748B
        DaemonState::Starting | DaemonState::Stopping | DaemonState::Installing => {
            [245, 158, 11] // #F59E0B
        }
        DaemonState::Error(_) => [239, 68, 68], // #EF4444
    }
}

fn state_icon(rgb: [u8; 3]) -> Image<'static> {
    static ICON_CACHE: std::sync::OnceLock<Mutex<HashMap<[u8; 3], Image<'static>>>> =
        std::sync::OnceLock::new();
    let cache = ICON_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(img) = guard.get(&rgb) {
            return img.clone();
        }
    }
    let img = badge_icon(rgb).unwrap_or_else(|| Image::new_owned(vec![0u8; 64 * 64 * 4], 64, 64));
    if let Ok(mut guard) = cache.lock() {
        guard.insert(rgb, img.clone());
    }
    img
}

/// Signed distance to an axis-aligned rounded rectangle centred at the origin.
#[inline]
fn sd_rounded_box(px: f32, py: f32, half: f32, radius: f32) -> f32 {
    let bx = half - radius;
    let qx = px.abs() - bx;
    let qy = py.abs() - bx;
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - radius
}

/// Point-in-convex-quad test (works regardless of winding order).
#[inline]
fn point_in_quad(px: f32, py: f32, q: &[(f32, f32); 4]) -> bool {
    let mut pos = 0u8;
    let mut neg = 0u8;
    for i in 0..4 {
        let (x1, y1) = q[i];
        let (x2, y2) = q[(i + 1) % 4];
        let cross = (x2 - x1) * (py - y1) - (y2 - y1) * (px - x1);
        if cross >= 0.0 {
            pos += 1;
        }
        if cross <= 0.0 {
            neg += 1;
        }
    }
    pos == 4 || neg == 4
}

/// A crisp 64x64 HiDPI squircle badge in the state colour with a subtle
/// gradient, inner rim, and 4x4 supersampled isometric container cube.
///
/// Designed to stay sharp and legible on both light and dark GNOME panels at
/// any scale factor (16px–64px).
fn badge_icon(rgb: [u8; 3]) -> Option<Image<'static>> {
    const SIZE: usize = 64;
    const CENTER: f32 = (SIZE - 1) as f32 / 2.0;
    const HALF: f32 = 27.0;
    const CORNER_R: f32 = 13.0;
    const CUBE_SCALE: f32 = 15.5;
    const SS: usize = 4;
    const INV_SS2: f32 = 1.0 / (SS * SS) as f32;

    const TOP_FACE: [(f32, f32); 4] = [(0.0, -0.92), (0.84, -0.46), (0.0, 0.0), (-0.84, -0.46)];
    const LEFT_FACE: [(f32, f32); 4] = [(-0.88, -0.34), (-0.06, 0.12), (-0.06, 0.94), (-0.88, 0.48)];
    const RIGHT_FACE: [(f32, f32); 4] = [(0.06, 0.12), (0.88, -0.34), (0.88, 0.48), (0.06, 0.94)];

    let mut out = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut acc_r = 0.0f32;
            let mut acc_g = 0.0f32;
            let mut acc_b = 0.0f32;
            let mut acc_a = 0.0f32;

            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = x as f32 + (sx as f32 + 0.5) / SS as f32 - 0.5;
                    let fy = y as f32 + (sy as f32 + 0.5) / SS as f32 - 0.5;
                    let px = fx - CENTER;
                    let py = fy - CENTER;

                    let dist = sd_rounded_box(px, py, HALF, CORNER_R);
                    if dist > 0.0 {
                        continue;
                    }

                    // Vertical gradient + subtle inner rim for panel contrast.
                    let t = ((py + HALF) / (2.0 * HALF)).clamp(0.0, 1.0);
                    let grad = (0.5 - t) * 26.0;
                    let rim = if dist > -1.35 {
                        (0.5 - t) * 36.0 + 8.0
                    } else {
                        0.0
                    };
                    let mut r = (rgb[0] as f32 + grad + rim).clamp(0.0, 255.0);
                    let mut g = (rgb[1] as f32 + grad + rim).clamp(0.0, 255.0);
                    let mut b = (rgb[2] as f32 + grad + rim).clamp(0.0, 255.0);

                    // Isometric 3D container cube in the centre.
                    let cx = px / CUBE_SCALE;
                    let cy = py / CUBE_SCALE;
                    if point_in_quad(cx, cy, &TOP_FACE) {
                        r = 255.0;
                        g = 255.0;
                        b = 255.0;
                    } else if point_in_quad(cx, cy, &LEFT_FACE) {
                        r = 236.0;
                        g = 243.0;
                        b = 252.0;
                    } else if point_in_quad(cx, cy, &RIGHT_FACE) {
                        r = 210.0;
                        g = 224.0;
                        b = 240.0;
                    }

                    acc_r += r;
                    acc_g += g;
                    acc_b += b;
                    acc_a += 1.0;
                }
            }

            if acc_a > 0.0 {
                let i = (y * SIZE + x) * 4;
                out[i] = (acc_r / acc_a).round().clamp(0.0, 255.0) as u8;
                out[i + 1] = (acc_g / acc_a).round().clamp(0.0, 255.0) as u8;
                out[i + 2] = (acc_b / acc_a).round().clamp(0.0, 255.0) as u8;
                out[i + 3] = (acc_a * INV_SS2 * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Some(Image::new_owned(out, SIZE as u32, SIZE as u32))
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
    fn badge_icon_is_visible_on_any_panel() {
        let img = badge_icon([100, 116, 139]).expect("badge icon must render");
        let rgba = img.rgba();
        assert_eq!(rgba.len(), 64 * 64 * 4);
        let mut opaque = 0usize;
        let mut badge_colored = 0usize;
        let mut glyph_white = 0usize;
        for px in rgba.chunks_exact(4) {
            if px[3] > 200 {
                opaque += 1;
                if px[0] >= 205 && px[1] >= 220 && px[2] >= 235 {
                    glyph_white += 1;
                } else if px[2] > px[0] {
                    badge_colored += 1;
                }
            }
        }
        // A solid 64x64 squircle badge with crisp 3D cube inside.
        assert!(opaque > 2200, "badge must be a filled squircle, got {opaque}");
        assert!(badge_colored > 1200, "badge must carry the state colour, got {badge_colored}");
        assert!(glyph_white > 400, "cube glyph must be rendered in white, got {glyph_white}");
    }

    #[test]
    fn state_colors_distinguish_running_stopped_transition() {
        assert_ne!(
            state_color(&DaemonState::Running),
            state_color(&DaemonState::Stopped)
        );
        assert_ne!(
            state_color(&DaemonState::Starting),
            state_color(&DaemonState::Error("x".into()))
        );
        assert_ne!(
            state_color(&DaemonState::Running),
            state_color(&DaemonState::Starting)
        );
    }

    #[test]
    fn tray_badge_keeps_the_glyph_silhouette_inside_the_disc() {
        let img = badge_icon([36, 150, 237]).expect("badge icon renders");
        let rgba = img.rgba();
        let px = |x: usize, y: usize| &rgba[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];

        assert_eq!(px(0, 0)[3], 0, "corner stays transparent");
        assert_eq!(px(63, 63)[3], 0, "bottom-right corner stays transparent");
        // The glyph silhouette survives: white pixels inside the blue squircle.
        let white = rgba
            .chunks_exact(4)
            .filter(|p| p[3] == 255 && p[0] >= 205 && p[1] >= 220 && p[2] >= 235)
            .count();
        assert!(white > 400, "glyph must be visible in white, got {white}");
        let blue = rgba
            .chunks_exact(4)
            .filter(|p| p[3] == 255 && p[2] > p[0].saturating_add(100))
            .count();
        assert!(blue > 1200, "squircle must be filled with the state colour, got {blue}");
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
        let sig1 = menu_signature(&DaemonState::Running, Some(&stats));
        assert!(sig1.contains("Web Stack") || sig1.contains("grp1"));
        assert!(sig1.contains("kind-local"));
        assert!(sig1.contains("app.localhost"));

        // Changing container count or resource updates the signature
        stats.groups[0].running = 1;
        let sig2 = menu_signature(&DaemonState::Running, Some(&stats));
        assert_ne!(sig1, sig2);
    }
}
