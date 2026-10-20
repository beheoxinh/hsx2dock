use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub mod commands;
pub mod config;
pub mod docker;
pub mod fsutil;
pub mod runtime;
pub mod state;
pub mod tray;

use commands::*;
use state::AppState;

fn start_background_monitoring(app_handle: AppHandle) {
    let handle = app_handle.clone();
    crate::runtime::spawn(async move {
        let mut last_state = None;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if let Some(state) = handle.try_state::<AppState>() {
                let status = state.daemon_state();
                if last_state.as_ref() != Some(&status) {
                    last_state = Some(status.clone());
                    // The bridge can only attach once the daemon's socket
                    // exists, so enabling it while the engine was stopped left
                    // the config on with nothing linked. Re-apply on every
                    // transition; reconcile is a no-op when the bridge is off.
                    let is_live = status == crate::docker::models::DaemonState::Running;
                    if is_live {
                        let _ = crate::commands::system::apply_cli_bridge(&state).await;
                        // Every path that brings the engine up (app open with
                        // auto-start, manual start, restart after crash) gets
                        // the configured caps + group auto-start. Off the
                        // monitor's critical path so a slow cgroup write or a
                        // misbehaving container never stalls the tick.
                        let handle2 = handle.clone();
                        crate::runtime::spawn(async move {
                            let Some(st) = handle2.try_state::<AppState>() else {
                                return;
                            };
                            if let Err(e) =
                                crate::commands::system::apply_engine_limits_now(&st).await
                            {
                                tracing::warn!("could not apply engine resource limits: {e}");
                            }
                            // Groups are this engine's lifecycle — never
                            // auto-start them into a remote endpoint.
                            if st.config.read().await.active_endpoint.is_none() {
                                crate::commands::groups::auto_start_groups(&st).await;
                            }
                        });
                        // Kind clusters flagged auto-start come up after the
                        // engine, staggered and capped (see auto_start_kind_clusters).
                        {
                            let app = handle.clone();
                            tauri::async_runtime::spawn(async move {
                                let Some(st2) = app.try_state::<AppState>() else {
                                    return;
                                };
                                if let Err(e) =
                                    crate::commands::kind::auto_start_kind_clusters(app.clone(), &st2)
                                        .await
                                {
                                    tracing::warn!("kind auto-start failed: {e}");
                                }
                            });
                        }
                    }
                    let _ = handle.emit("engine-state-changed", status);
                }

                // If running, poll running containers stats and broadcast list
                if let Ok(client) = state.get_client().await {
                    if let Ok(all_containers) = client.list_containers(true).await {
                        let _ = handle.emit("containers-tick", all_containers.clone());

                        let running_ids: Vec<String> = all_containers
                            .into_iter()
                            .filter(|c| c.state == "running")
                            .map(|c| c.id)
                            .collect();

                        if !running_ids.is_empty() {
                            let mut stats_map = HashMap::new();
                            for id in running_ids {
                                if let Ok(stats) = client.container_stats(&id).await {
                                    stats_map.insert(id, stats);
                                }
                            }
                            let _ = handle.emit("container-stats-tick", stats_map);
                        }
                    }
                }
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_state = crate::runtime::runtime().block_on(AppState::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .setup(|app| {
            let handle = app.handle().clone();
            
            // Check auto-start engine daemon
            if let Some(state) = handle.try_state::<AppState>() {
                // Seed the process-wide CLI preference from the saved config so
                // the first compose/docker invocation resolves the same binary
                // the Settings card will report.
                {
                    let mode = crate::runtime::runtime()
                        .block_on(async { state.config.read().await.cli_mode.clone() });
                    docker::engine::set_cli_mode(&mode);
                }
                let should_autostart = crate::runtime::runtime().block_on(async {
                    state.config.read().await.auto_start_daemon
                });
                if should_autostart && state.daemon_state() != docker::models::DaemonState::Running {
                    let engine = state.engine.clone();
                    let h = handle.clone();
                    crate::runtime::spawn(async move {
                        tracing::info!("Auto-starting engine daemon...");
                        let _ = crate::runtime::engine_op(move || engine.start()).await;
                        // Trigger client reconnection
                        if let Some(s) = h.try_state::<AppState>() {
                            let _ = s.get_client().await;
                            let _ = h.emit("engine-state-changed", s.daemon_state());
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
                    if let Ok(Some(monitor)) = win.primary_monitor().or_else(|_| win.current_monitor()) {
                        let scale_factor = monitor.scale_factor();
                        let screen_size = monitor.size();
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

            // Tray + desktop notifications. AppState is managed on the builder
            // so the tray can read daemon state immediately; a missing display
            // bus must not take the whole app down, hence the warn-and-continue.
            if let Err(e) = crate::tray::init_tray(&handle) {
                tracing::warn!("system tray unavailable: {e}");
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
            // Interactive exec / debug
            exec::start_interactive_exec,
            exec::debug_container,
            exec::write_exec_stdin,
            exec::resize_exec_session,
            exec::stop_exec_session,
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
            kind::get_cluster_metrics,
            kind::get_cluster_summary,
            kind::install_metrics_server,

            // Registries
            registries::list_registries,
            registries::test_registry_auth,
            registries::save_registry_auth,
            registries::remove_registry_auth,
            // Networks
            networks::list_networks,
            networks::create_network,
            networks::remove_network,
            networks::prune_networks,
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
            // System
            system::get_engine_status,
            system::apply_engine_limits,
            system::toggle_engine,
            system::get_engine_info,
            system::get_disk_usage,
            system::prune_system,
            system::get_app_config,
            system::save_app_config,
            system::get_cli_bridge_status,
            system::toggle_cli_bridge,
            system::check_data_root_path,
            system::migrate_data_root,
            system::check_engine_update,
            system::install_engine_update,
            system::check_docker_cli,
            system::toggle_cli_shim,
            system::check_compose_update,
            system::install_compose_update,
            endpoints::list_endpoints,
            endpoints::test_endpoint,
            endpoints::save_endpoint,
            endpoints::remove_endpoint,
            endpoints::set_active_endpoint,
            endpoints::get_active_endpoint,
            endpoints::active_endpoint_host,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
