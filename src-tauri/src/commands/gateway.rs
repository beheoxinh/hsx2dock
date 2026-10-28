use std::sync::Arc;
use tauri::State;

use crate::docker::caddy::{
    clear_app_hosts_entries, inspect_hosts_sync, parse_hosts_file, save_raw_hosts_content,
    update_hosts_file, CaddyInstallStatus, GatewayRoute, HostsFileModel, HostsSyncStatus,
    CADDY_VERSION,
};
use crate::state::AppState;

#[tauri::command]
pub async fn get_gateway_status(
    state: State<'_, Arc<AppState>>,
) -> Result<CaddyInstallStatus, String> {
    let supervisor = &state.caddy;
    let paths = &supervisor.paths;
    let installed = paths.is_installed();
    let running = supervisor.is_running().await;

    let config = state.config.read().await;
    let http_port = config.gateway_http_port;
    let https_port = config.gateway_https_port;
    let routes = &config.gateway_routes;

    let hosts_status = inspect_hosts_sync(routes);

    let version = if installed {
        std::fs::read_to_string(paths.version_dir(&paths.active_version()).join("VERSION"))
            .ok()
            .map(|s| s.trim().to_string())
            .or(Some(paths.active_version()))
    } else {
        None
    };

    let auto_start = config.gateway_auto_start;

    Ok(CaddyInstallStatus {
        installed,
        version,
        latest: CADDY_VERSION.to_string(),
        path: installed.then(|| paths.binary().display().to_string()),
        running,
        admin_url: "http://127.0.0.1:2019".to_string(),
        http_port,
        https_port,
        trusted: true,
        hosts_synced: hosts_status.synced,
        auto_start,
    })
}

#[tauri::command]
pub async fn list_gateway_routes(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<GatewayRoute>, String> {
    let config = state.config.read().await;
    Ok(config.gateway_routes.clone())
}

#[tauri::command]
pub async fn save_gateway_routes(
    state: State<'_, Arc<AppState>>,
    routes: Vec<GatewayRoute>,
) -> Result<(), String> {
    {
        let mut config = state.config.write().await;
        config.gateway_routes = routes.clone();
        config.save().await.map_err(|e| e.to_string())?;
    }

    let config = state.config.read().await;
    if config.gateway_enabled && state.caddy.is_running().await {
        state
            .caddy
            .apply_routes(config.gateway_http_port, config.gateway_https_port, &routes)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
pub async fn toggle_gateway(
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<bool, String> {
    {
        let mut config = state.config.write().await;
        config.gateway_enabled = enabled;
        config.save().await.map_err(|e| e.to_string())?;
    }

    let config = state.config.read().await;
    if enabled {
        state
            .caddy
            .start(
                config.gateway_http_port,
                config.gateway_https_port,
                &config.gateway_routes,
            )
            .await
            .map_err(|e| e.to_string())?;
    } else {
        state.caddy.stop().await.map_err(|e| e.to_string())?;
    }

    Ok(state.caddy.is_running().await)
}

#[tauri::command]
pub async fn trust_gateway_ca(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.caddy.trust_root_ca().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_hosts_sync(state: State<'_, Arc<AppState>>) -> Result<HostsSyncStatus, String> {
    let config = state.config.read().await;
    Ok(inspect_hosts_sync(&config.gateway_routes))
}

#[tauri::command]
pub async fn sync_hosts_file(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let config = state.config.read().await;
    update_hosts_file(&config.gateway_routes).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_gateway_auto_start(
    state: State<'_, Arc<AppState>>,
    auto_start: bool,
) -> Result<(), String> {
    let mut config = state.config.write().await;
    config.gateway_auto_start = auto_start;
    config.save().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_hosts_file_model(
    state: State<'_, Arc<AppState>>,
) -> Result<HostsFileModel, String> {
    let config = state.config.read().await;
    Ok(parse_hosts_file(&config.gateway_routes))
}

#[tauri::command]
pub async fn save_raw_hosts(new_content: String) -> Result<(), String> {
    save_raw_hosts_content(&new_content).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_app_hosts(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let _ = state;
    clear_app_hosts_entries().map_err(|e| e.to_string())
}
