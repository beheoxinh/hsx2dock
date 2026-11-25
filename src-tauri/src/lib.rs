use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub mod autostart;
pub mod commands;
pub mod config;
pub mod docker;
pub mod fsutil;
pub mod mcp;
pub mod migrate;
pub mod ops;
pub mod procutil;
pub mod runtime;
pub mod state;
pub mod tray;

use commands::*;
use state::AppState;

/// Upper bound for one `list_containers` call in the monitor. A wedged daemon
/// must not freeze the loop that also detects engine state transitions.
const MONITOR_LIST_TIMEOUT: Duration = Duration::from_secs(8);
/// Per-container stats deadline; one stuck container (mid-stop, paused cgroup)
/// must not hold the whole tick back.
const MONITOR_STATS_TIMEOUT: Duration = Duration::from_secs(4);
/// How many stats requests the monitor keeps in flight. With 200 running
/// containers the old `join_all` fired 200 requests at dockerd every 2s.
const MONITOR_STATS_CONCURRENCY: usize = 12;
/// Consecutive identical readings required before an engine state change is
/// believed. A busy daemon can refuse one socket connect, and a fake
/// Running→Stopped→Running flap used to re-run auto-start (which stops every
/// running container whose group has auto-start off).
const STATE_CONFIRM_TICKS: u8 = 2;

/// Samples stats for `ids` with bounded concurrency and per-call timeouts,
/// turning one-shot samples into a real CPU % against the previous sample.
/// Containers sampled per 2s tick before the stats cadence slows down.
const STATS_PER_TICK_BUDGET: usize = 60;

async fn sample_stats(
    client: &crate::docker::client::DockerClient,
    ids: Vec<String>,
    prev: &HashMap<String, crate::docker::models::ContainerStats>,
) -> HashMap<String, crate::docker::models::ContainerStats> {
    use futures_util::stream::{self, StreamExt};
    let futs: Vec<_> = ids
        .into_iter()
        .map(|id| {
            let c = client.clone();
            async move {
                let s = tokio::time::timeout(MONITOR_STATS_TIMEOUT, c.container_stats_one_shot(&id))
                    .await
                    .ok()?
                    .ok()?;
                Some((id, s))
            }
        })
        .collect();
    let results: Vec<Option<(String, crate::docker::models::ContainerStats)>> = stream::iter(futs)
        .buffer_unordered(MONITOR_STATS_CONCURRENCY)
        .collect()
        .await;
    results
        .into_iter()
        .flatten()
        .map(|(id, mut s)| {
            s.cpu_percent = prev.get(&id).map(|p| s.cpu_percent_since(p)).unwrap_or(0.0);
            (id, s)
        })
        .collect()
}

fn start_background_monitoring(app_handle: AppHandle) {
    let handle = app_handle.clone();
    // Containers Docker restores during an engine start have a StartedAt at or
    // after this moment; anything older was deliberate and is left alone.
    let app_started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    crate::runtime::spawn(async move {
        let mut last_state = None;
        // Candidate state and how many ticks in a row it was observed.
        let mut pending_state: Option<(crate::docker::models::DaemonState, u8)> = None;
        let mut last_containers: Option<Vec<crate::docker::models::ContainerItem>> = None;
        let mut hidden_ticks: u32 = 0;
        let mut was_hidden = false;
        let mut stats_tick: u32 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Some(state) = handle.try_state::<std::sync::Arc<AppState>>() else {
                continue;
            };
            // A saved remote endpoint is live when it answers; the bundled
            // engine's state says nothing about it. Bounded, so a dead remote
            // cannot stall the tick.
            let remote_active = state.remote_endpoint_active().await;
            let observed = if remote_active {
                match tokio::time::timeout(MONITOR_LIST_TIMEOUT, state.get_client()).await {
                    Ok(Ok(_)) => crate::docker::models::DaemonState::Running,
                    _ => crate::docker::models::DaemonState::Stopped,
                }
            } else {
                state.daemon_state()
            };
            // The very first reading is accepted as-is; later changes must be
            // seen STATE_CONFIRM_TICKS times in a row.
            let status = if last_state.is_none() || last_state.as_ref() == Some(&observed) {
                pending_state = None;
                observed
            } else {
                let count = match &pending_state {
                    Some((s, n)) if *s == observed => n + 1,
                    _ => 1,
                };
                pending_state = Some((observed.clone(), count));
                if count >= STATE_CONFIRM_TICKS {
                    pending_state = None;
                    observed
                } else {
                    last_state.clone().unwrap_or(observed)
                }
            };
            let is_live = status == crate::docker::models::DaemonState::Running;
            if !is_live {
                last_containers = None;
            }
            if last_state.as_ref() != Some(&status) {
                // The first tick after launch proves nothing about what the
                // engine did while the app was closed; only a real
                // stopped→running transition may drop restored clusters.
                let had_previous_state = last_state.is_some();
                last_state = Some(status.clone());
                // The bridge can only attach once the daemon's socket
                // exists, so enabling it while the engine was stopped left
                // the config on with nothing linked. Re-apply on every
                // transition; reconcile is a no-op when the bridge is off.
                if is_live {
                    // Bounded: this runs on the monitor loop, and a hung
                    // bridge apply used to stall every tick behind it.
                    if tokio::time::timeout(
                        Duration::from_secs(10),
                        crate::commands::system::apply_cli_bridge(&state),
                    )
                    .await
                    .is_err()
                    {
                        tracing::warn!("applying the CLI bridge timed out");
                    }
                    // Every path that brings the engine up (app open with
                    // auto-start, manual start, restart after crash) gets
                    // the configured caps + group auto-start. Off the
                    // monitor's critical path so a slow cgroup write or a
                    // misbehaving container never stalls the tick.
                    let handle2 = handle.clone();
                    crate::runtime::spawn(async move {
                        let Some(st) = handle2.try_state::<std::sync::Arc<AppState>>() else {
                            return;
                        };
                        if let Err(e) =
                            crate::commands::system::apply_engine_limits_now(&st).await
                        {
                            tracing::warn!("could not apply engine resource limits: {e}");
                        }
                        // Groups are this engine's lifecycle — never
                        // auto-start them into a remote endpoint.
                        // An engine this app does not own (socket from an
                        // earlier session or another owner) was not just
                        // booted: its containers are where the user left them.
                        let owns_engine = st.engine.is_running();
                        if st.config.read().await.active_endpoint.is_none()
                            && (had_previous_state || owns_engine)
                        {
                            let restored_since = if had_previous_state { 0 } else { app_started };
                            crate::commands::groups::auto_start_groups(&st, restored_since).await;
                        }
                    });
                    // Kind clusters: pin node restart policies (kind's
                    // on-failure:1 lets Docker resurrect clusters behind
                    // the app's back), drop clusters restored against the
                    // auto-start switches, then start the flagged ones
                    // staggered and capped.
                    // Bundled-engine lifecycle only: its restart policies and
                    // clusters must never be rewritten on a remote endpoint.
                    if !remote_active {
                        let app = handle.clone();
                        let restored_since = if had_previous_state { 0 } else { app_started };
                        crate::runtime::spawn(async move {
                            let Some(st2) = app.try_state::<std::sync::Arc<AppState>>() else {
                                return;
                            };
                            if let Err(e) = crate::commands::kind::reconcile_kind_auto_start(
                                app.clone(),
                                &st2,
                                restored_since,
                            )
                            .await
                            {
                                tracing::warn!("kind auto-start reconcile failed: {e}");
                            }
                        });
                    }
                }
                let _ = handle.emit("engine-state-changed", status.clone());
            }

            // Engine down: nothing to poll. Drop stale figures so the tray
            // and a re-opened window never show yesterday's CPU/RAM.
            if !is_live {
                if !state.cached_stats.read().await.is_empty() {
                    state.cached_stats.write().await.clear();
                }
                continue;
            }

            let window_visible = handle
                .get_webview_window("main")
                .and_then(|w| w.is_visible().ok())
                .unwrap_or(true);

            if !window_visible {
                was_hidden = true;
                hidden_ticks = hidden_ticks.wrapping_add(1);
                // When hidden to tray, only refresh cached_stats every ~14s
                // (matching tray REFRESH_INTERVAL) and skip WebView IPC emits.
                if hidden_ticks % 7 != 0 {
                    continue;
                }
            } else if was_hidden {
                was_hidden = false;
                hidden_ticks = 0;
                // Force a fresh containers-tick emit when the window returns.
                last_containers = None;
            }

            let Ok(client) = state.get_client().await else {
                continue;
            };
            let Ok(Ok(all_containers)) =
                tokio::time::timeout(MONITOR_LIST_TIMEOUT, client.list_containers(true)).await
            else {
                continue;
            };
            if window_visible && last_containers.as_ref() != Some(&all_containers) {
                let _ = handle.emit("containers-tick", &all_containers);
                last_containers = Some(all_containers.clone());
            }

            let running_ids: Vec<String> = all_containers
                .into_iter()
                .filter(|c| c.state == "running")
                .map(|c| c.id)
                .collect();

            // Many running containers: sample stats less often. Every tick
            // fired one stats request per container, so 200 containers kept
            // the daemon answering ~100 requests a second.
            stats_tick = stats_tick.wrapping_add(1);
            let stride = (running_ids.len() / STATS_PER_TICK_BUDGET).max(1) as u32;
            if !running_ids.is_empty() && !stats_tick.is_multiple_of(stride) {
                continue;
            }
            if !running_ids.is_empty() {
                let prev = state.cached_stats.read().await.clone();
                let stats_map = sample_stats(&client, running_ids, &prev).await;
                // Keep app_state's cached_stats current for tray & background
                // readers. Replaced wholesale, so removed containers drop out.
                *state.cached_stats.write().await = stats_map.clone();
                if window_visible {
                    let _ = handle.emit("container-stats-tick", stats_map);
                }
            } else {
                let was_not_empty = !state.cached_stats.read().await.is_empty();
                if was_not_empty {
                    state.cached_stats.write().await.clear();
                    if window_visible {
                        let _ = handle.emit(
                            "container-stats-tick",
                            HashMap::<String, crate::docker::models::ContainerStats>::new(),
                        );
                    }
                }
            }
        }
    });
}

/// Where the GUI's own log lives. The engine has `log/engine.log`; the app
/// process itself — tray, gateway auto-start, self-repair, MCP — logs here.
fn app_log_file() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("hsx2dock").join("log").join("app.log"))
}

/// Install a tracing subscriber for the GUI process.
///
/// Without this every `tracing::info!/warn!` in the app (gateway auto-start
/// failures, self-repair, MCP) goes nowhere — stdout is discarded when the app
/// is launched from a launcher, which made production failures undebuggable.
/// Logs append to `~/.local/share/hsx2dock/log/app.log`, rotated at 10 MiB to
/// `app.log.1`. Falls back silently to stderr when the log dir is unwritable.
fn init_app_logging() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let file = app_log_file().and_then(|path| {
        let dir = path.parent()?;
        std::fs::create_dir_all(dir).ok()?;
        // Rotate: a runaway warn-loop must not grow the file without bound.
        const MAX_BYTES: u64 = 10 * 1024 * 1024;
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.len() > MAX_BYTES {
                let _ = std::fs::rename(&path, dir.join("app.log.1"));
            }
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    });

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false);

    match file {
        Some(f) => {
            let _ = subscriber
                .with_writer(std::sync::Mutex::new(f))
                .try_init();
        }
        None => {
            let _ = subscriber.with_writer(std::io::stderr).try_init();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_app_logging();
    // Rename any pre-rebrand (`docker-manager`) tree before anything reads a
    // path, so `AppState`/`EnginePaths` only ever see the new identity.
    crate::migrate::migrate_legacy_identity();
    // Shared rather than owned: the MCP HTTP server keeps its own handle so it
    // can serve requests without going through Tauri's borrow of the state.
    let app_state = std::sync::Arc::new(crate::runtime::runtime().block_on(AppState::new()));

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        // Close to tray: the window is hidden, not destroyed, so the engine
        // supervisor and every background task keep running. `AppHandle::exit`
        // (tray → Quit) does not raise this event, so Quit still shuts down.
        .on_window_event(|window, event| {
            let tauri::WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            let app = window.app_handle();
            let Some(state) = app.try_state::<std::sync::Arc<AppState>>() else {
                return;
            };
            if !state.is_close_to_tray() {
                return;
            }
            api.prevent_close();
            let _ = window.hide();
            crate::tray::notify_hidden_to_tray(app);
        })
        .setup(|app| {
            let handle = app.handle().clone();

            // Lifecycle progress (group starts, auto-start) reaches the UI as
            // `container-op` / `group-op` events once the registry has a handle.
            if let Some(state) = handle.try_state::<std::sync::Arc<AppState>>() {
                state.ops.attach(handle.clone());
            }

            // Check auto-start engine daemon
            if let Some(state) = handle.try_state::<std::sync::Arc<AppState>>() {
                // Seed the process-wide CLI preference from the saved config so
                // the first compose/docker invocation resolves the same binary
                // the Settings card will report.
                {
                    let mode = crate::runtime::runtime()
                        .block_on(async { state.config.read().await.cli_mode.clone() });
                    docker::engine::set_cli_mode(&mode);
                }
                // The shim hardcodes the engine socket and DOCKER_CONFIG paths.
                // Rewrite it on every launch so it follows the current layout
                // (including a tree just migrated from the old names) instead
                // of pointing at a socket that no longer exists.
                {
                    let config = crate::runtime::runtime()
                        .block_on(async { state.config.read().await.clone() });
                    crate::commands::system::refresh_shim(&state, &config);
                    // Keep the login entry in step with the saved settings; a
                    // config dir restored from backup (or cleaned by hand) must
                    // not silently drop autostart.
                    if let Err(e) = crate::autostart::sync(&config) {
                        tracing::warn!("could not sync the autostart entry: {e}");
                    }
                }
                let should_autostart = crate::runtime::runtime()
                    .block_on(async { state.config.read().await.auto_start_daemon });
                if should_autostart && state.daemon_state() != docker::models::DaemonState::Running
                {
                    // Claimed here, like every other start path, so a manual
                    // start or data-root move that wins the race makes this one
                    // back off instead of starting the engine twice.
                    match crate::runtime::claim_engine_lifecycle() {
                        Err(e) => tracing::warn!("skipping engine auto-start: {e}"),
                        Ok(lifecycle) => {
                            let engine = state.engine.clone();
                            let h = handle.clone();
                            crate::runtime::spawn(async move {
                                // Held until the start has finished.
                                let _lifecycle = lifecycle;
                                tracing::info!("Auto-starting engine daemon...");
                                let _ = crate::runtime::engine_op(move || engine.start()).await;
                                // Trigger client reconnection
                                if let Some(s) = h.try_state::<std::sync::Arc<AppState>>() {
                                    let _ = s.get_client().await;
                                    let _ = h.emit("engine-state-changed", s.daemon_state());
                                }
                            });
                        }
                    }
                }
            }

            // The MCP HTTP transport is opt-in from Settings. When it is on,
            // it comes back up with the app so a configured agent keeps
            // working across restarts. A failure here is logged, never fatal:
            // an occupied port must not stop the GUI from opening.
            if let Some(state) = handle.try_state::<std::sync::Arc<AppState>>() {
                let enabled = crate::runtime::runtime()
                    .block_on(async { state.config.read().await.mcp_enabled });
                if enabled {
                    let state = state.inner().clone();
                    crate::runtime::spawn(async move {
                        if let Err(e) = crate::commands::mcp::apply_mcp_enabled(&state, true).await
                        {
                            tracing::warn!("could not start the MCP HTTP server: {e}");
                        }
                    });
                }
            }

            // Gateway Caddy auto-start when app launches
            if let Some(state) = handle.try_state::<std::sync::Arc<AppState>>() {
                let (auto_start, http_port, https_port, routes) = crate::runtime::runtime().block_on(async {
                    let cfg = state.config.read().await;
                    (
                        cfg.gateway_auto_start,
                        cfg.gateway_http_port,
                        cfg.gateway_https_port,
                        cfg.gateway_routes.clone(),
                    )
                });

                if auto_start {
                    let supervisor = state.caddy.clone();
                    crate::runtime::spawn(async move {
                        tracing::info!("Auto-starting Caddy HTTPS Gateway...");
                        if let Err(e) = supervisor.start(http_port, https_port, &routes).await {
                            tracing::warn!("Could not auto-start Caddy Gateway: {e}");
                        }
                    });
                }
            }

            // Resize window to 90% width and 95% height of primary monitor
            if let Some(window) = app.get_webview_window("main") {
                let win = window.clone();
                // Spawn a short delayed task or perform on event loop so WebKitGTK / Wayland has realized the window
                crate::runtime::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                    if let Ok(Some(monitor)) =
                        win.primary_monitor().or_else(|_| win.current_monitor())
                    {
                        let scale_factor = monitor.scale_factor();
                        // The work area excludes panels/docks, so 95% of the
                        // height never slides under a bottom bar. This is the
                        // only startup resize: the frontend used to apply a
                        // second one, so the window visibly jumped twice.
                        let screen_size = monitor.work_area().size;
                        let target_width = (screen_size.width as f64 * 0.90) as u32;
                        let target_height = (screen_size.height as f64 * 0.95) as u32;
                        tracing::info!(
                            "Setting window size: {}x{} (monitor: {}x{}, scale: {})",
                            target_width,
                            target_height,
                            screen_size.width,
                            screen_size.height,
                            scale_factor
                        );
                        let _ = win.set_size(tauri::Size::Physical(tauri::PhysicalSize {
                            width: target_width,
                            height: target_height,
                        }));
                        let _ = win.center();
                    }
                });
            }

            // `--start-minimized` is only passed by our own autostart entry, and
            // the config is re-checked so a stale entry can never hide a manual
            // launch.
            let start_hidden = std::env::args()
                .any(|arg| arg == crate::autostart::START_MINIMIZED_ARG)
                && handle
                    .try_state::<std::sync::Arc<AppState>>()
                    .map(|state| {
                        crate::runtime::runtime().block_on(async {
                            let cfg = state.config.read().await;
                            cfg.auto_start_on_boot && cfg.start_minimized
                        })
                    })
                    .unwrap_or(false);

            // Tray + desktop notifications. AppState is managed on the builder
            // so the tray can read daemon state immediately; a missing display
            // bus must not take the whole app down, hence the warn-and-continue.
            let tray_ready = match crate::tray::init_tray(&handle) {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!("system tray unavailable: {e}");
                    false
                }
            };

            if let Some(window) = app.get_webview_window("main") {
                if start_hidden && tray_ready {
                    let _ = window.hide();
                } else {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            start_background_monitoring(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Containers
            containers::list_containers,
            containers::start_container,
            containers::stop_container,
            containers::restart_container,
            containers::pause_container,
            containers::unpause_container,
            containers::remove_container,
            containers::rename_container,
            containers::inspect_container,
            containers::get_container_logs,
            containers::get_container_stats,
            containers::get_container_sizes,
            containers::run_container,
            containers::update_container_resources,
            containers::recreate_container,
            // Interactive exec / debug
            exec::start_interactive_exec,
            exec::debug_container,
            exec::write_exec_stdin,
            exec::resize_exec_session,
            exec::stop_exec_session,
            exec::close_all_exec_sessions,
            // Images
            images::list_images,
            images::remove_image,
            images::tag_image,
            images::prune_images,
            images::get_image_history,
            images::search_hub,
            images::build_image,
            images::cancel_image_build,
            images::validate_build_context,
            images::preview_build_command,
            images::pull_image,
            images::push_image,
            // Volumes
            volumes::list_volumes,
            volumes::create_volume,
            volumes::remove_volume,
            volumes::prune_volumes,
            // Kind
            kind::kind_status,
            kind::install_kind,
            kind::install_kind_update,
            kind::check_kind_update,
            kind::list_kind_clusters,
            kind::create_kind_cluster,
            kind::delete_kind_cluster,
            kind::start_kind_cluster,
            kind::stop_kind_cluster,
            kind::get_kind_kubeconfig,
            kind::load_image_to_cluster,
            kind::get_cluster_nodes,
            kind::get_cluster_pods,
            kind::get_cluster_events,
            kind::get_cluster_workloads,
            kind::get_cluster_prefs,
            kind::set_cluster_prefs,
            kind::get_pod_logs,
            kind::get_resource_yaml,
            kind::get_resource_events,
            kind::get_workload_logs,
            kind::delete_pod,
            kind::delete_k8s_resource,
            kind::scale_workload,
            kind::restart_workload,
            kind::create_namespace,
            kind::get_cluster_services,
            kind::get_cluster_config_resources,
            kind::get_cluster_pvcs,
            kind::get_cluster_namespaces,
            kind::get_cluster_ingresses,
            kind::get_cluster_pvs,
            kind::set_node_schedulable,
            kind::drain_node,
            kind::get_cluster_metrics,
            kind::get_cluster_summary,
            kind::install_metrics_server,
            kind::metrics_server_status,
            kind::detect_pod_shell,
            // Registries
            registries::list_registries,
            registries::test_registry_auth,
            registries::save_registry_auth,
            registries::set_default_registry,
            registries::remove_registry_auth,
            // Networks
            networks::list_networks,
            networks::create_network,
            networks::remove_network,
            networks::prune_networks,
            networks::repair_container_network,
            // Compose
            compose::list_compose_projects,
            compose::compose_up,
            compose::compose_down,
            compose::compose_logs,
            compose::compose_project_action,
            // Container groups
            groups::list_container_groups,
            groups::save_container_groups,
            groups::start_container_group,
            groups::stop_container_group,
            groups::list_container_ops,
            // System
            system::get_engine_status,
            system::apply_engine_limits,
            system::toggle_engine,
            system::get_engine_info,
            system::get_disk_usage,
            system::prune_system,
            system::get_app_config,
            system::save_app_config,
            // MCP front-end (Settings → MCP)
            commands::mcp::mcp_status,
            commands::mcp::mcp_set_enabled,
            commands::mcp::mcp_rotate_token,
            commands::mcp::mcp_token,
            commands::mcp::mcp_catalogue,
            commands::mcp::mcp_resources,
            commands::mcp::mcp_prompts,
            system::get_cli_bridge_status,
            system::toggle_cli_bridge,
            system::check_data_root_path,
            system::migrate_data_root,
            system::check_engine_update,
            system::install_engine_update,
            system::check_docker_cli,
            system::check_cli_update,
            system::install_cli_update,
            system::toggle_cli_shim,
            system::check_compose_update,
            system::install_compose_update,
            system::diagnose_engine,
            system::repair_engine,
            system::get_embedded_binaries,
            // Gateway (Caddy reverse proxy & hosts manager)
            commands::gateway::get_gateway_status,
            commands::gateway::list_gateway_routes,
            commands::gateway::save_gateway_routes,
            commands::gateway::toggle_gateway,
            commands::gateway::trust_gateway_ca,
            commands::gateway::check_hosts_sync,
            commands::gateway::sync_hosts_file,
            commands::gateway::set_gateway_auto_start,
            commands::gateway::probe_gateway_upstreams,
            commands::gateway::get_hosts_file_model,
            commands::gateway::save_raw_hosts,
            commands::gateway::clear_app_hosts,
            commands::gateway::check_caddy_update,
            commands::gateway::install_caddy_update,
            commands::gateway::set_gateway_ports,
            endpoints::list_endpoints,
            endpoints::test_endpoint,
            endpoints::save_endpoint,
            endpoints::remove_endpoint,
            endpoints::set_active_endpoint,
            endpoints::get_active_endpoint,
            endpoints::active_endpoint_host,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Graceful cleanup: stop Caddy gateway when app exits
                if let Some(state) = app.try_state::<std::sync::Arc<AppState>>() {
                    let supervisor = state.caddy.clone();
                    let _ = crate::runtime::runtime().block_on(async move {
                        let _ = supervisor.stop().await;
                    });
                }
            }
        });
}
