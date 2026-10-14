use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub mod commands;
pub mod config;
pub mod docker;
pub mod runtime;
pub mod state;

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
                    let _ = handle.emit("engine-state-changed", status);
                }

                // If running, poll running containers stats
                if let Ok(client) = state.get_client().await {
                    if let Ok(containers) = client.list_containers(false).await {
                        let running_ids: Vec<String> = containers
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
        .manage(app_state)
        .setup(|app| {
            let handle = app.handle().clone();
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
            containers::run_container,
            // Images
            images::list_images,
            images::remove_image,
            images::tag_image,
            images::prune_images,
            images::get_image_history,
            images::search_hub,
            images::pull_image,
            // Volumes
            volumes::list_volumes,
            volumes::create_volume,
            volumes::remove_volume,
            volumes::prune_volumes,
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
            // System
            system::get_engine_status,
            system::toggle_engine,
            system::get_engine_info,
            system::get_disk_usage,
            system::prune_system,
            system::get_app_config,
            system::save_app_config,
            system::get_cli_bridge_status,
            system::toggle_cli_bridge,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
