use tauri::State;

use crate::config::AppConfig;
use crate::docker::cli_bridge::{reconcile, xdg_symlink_state};
use crate::docker::models::{DaemonState, DfReport, EngineInfo};
use crate::state::AppState;

#[tauri::command]
pub async fn get_engine_status(state: State<'_, AppState>) -> Result<DaemonState, String> {
    Ok(state.daemon_state())
}

#[tauri::command]
pub async fn toggle_engine(state: State<'_, AppState>) -> Result<DaemonState, String> {
    let current = state.daemon_state();
    match current {
        DaemonState::Running => {
            state.engine.stop().map_err(|e| e.to_string())?;
            Ok(state.daemon_state())
        }
        DaemonState::Stopped | DaemonState::NotInstalled => {
            state.engine.start().map_err(|e| e.to_string())?;
            Ok(state.daemon_state())
        }
        other => Ok(other),
    }
}

#[tauri::command]
pub async fn get_engine_info(state: State<'_, AppState>) -> Result<Option<EngineInfo>, String> {
    let client = match state.get_client().await {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    Ok(client.engine_info().await.ok())
}

#[tauri::command]
pub async fn get_disk_usage(state: State<'_, AppState>) -> Result<DfReport, String> {
    let client = state.get_client().await?;
    client.system_df().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prune_system(state: State<'_, AppState>) -> Result<u64, String> {
    let client = state.get_client().await?;
    client.system_prune().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_app_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
pub async fn save_app_config(
    state: State<'_, AppState>,
    config: AppConfig,
) -> Result<(), String> {
    config.save().await.map_err(|e| e.to_string())?;
    *state.config.write().await = config;
    Ok(())
}

#[derive(Clone, serde::Serialize)]
pub struct CliBridgeStatus {
    pub enabled: bool,
    pub link_state: Option<bool>,
}

#[tauri::command]
pub async fn get_cli_bridge_status(state: State<'_, AppState>) -> Result<CliBridgeStatus, String> {
    let config = state.config.read().await;
    let engine_socket = &state.engine.paths().socket;
    let link_state = xdg_symlink_state(engine_socket);

    Ok(CliBridgeStatus {
        enabled: config.cli_bridge_enabled,
        link_state,
    })
}

#[tauri::command]
pub async fn toggle_cli_bridge(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<CliBridgeStatus, String> {
    let mut config = state.config.write().await;
    config.cli_bridge_enabled = enabled;
    config.save().await.map_err(|e| e.to_string())?;

    let engine_socket = state.engine.paths().socket.clone();
    let is_live = state.daemon_state() == DaemonState::Running;
    let _ = reconcile(
        &engine_socket,
        config.cli_bridge_enabled,
        config.cli_bridge_set_default,
        is_live,
        &config.cli_bridge_prev_context,
    );

    Ok(CliBridgeStatus {
        enabled: config.cli_bridge_enabled,
        link_state: xdg_symlink_state(&engine_socket),
    })
}
