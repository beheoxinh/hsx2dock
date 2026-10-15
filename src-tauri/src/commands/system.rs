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
            let engine = state.engine.clone();
            crate::runtime::engine_op(move || engine.stop())
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            *state.client.write().await = None;
            Ok(state.daemon_state())
        }
        DaemonState::Stopped | DaemonState::NotInstalled => {
            let engine = state.engine.clone();
            crate::runtime::engine_op(move || engine.start())
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            // Wait briefly for socket to become live or let get_client connect
            let _ = state.get_client().await;
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
    // Sync data_root immediately to engine paths so subsequent daemon restarts take effect without restarting the app
    state.engine.set_data_root(Some(&config.data_root));
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
    let engine_paths = state.engine.paths();
    let engine_socket = &engine_paths.socket;
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

#[derive(serde::Serialize)]
pub struct InspectPathResult {
    pub exists: bool,
    pub is_dir: bool,
    pub is_empty: bool,
    pub current_data_root: String,
    /// `null` when the path is fine, otherwise the reason it is unusable.
    pub fs_error: Option<String>,
}

#[tauri::command]
pub async fn check_data_root_path(
    state: State<'_, AppState>,
    path: String,
) -> Result<InspectPathResult, String> {
    let p = std::path::PathBuf::from(path.trim());
    let current_data_root = state.engine.paths().data_root.to_string_lossy().to_string();

    let exists = p.exists();
    let is_dir = p.is_dir();
    let mut is_empty = true;

    if exists && is_dir {
        if let Ok(mut entries) = std::fs::read_dir(&p) {
            if entries.next().is_some() {
                is_empty = false;
            }
        }
    }

    // Surface a d_type-incompatible filesystem here so the wizard can stop the
    // user before a daemon restart + copy that could never have worked.
    let fs_error = crate::docker::engine::check_data_root_filesystem(&p)
        .err()
        .map(|e| e.to_string());

    Ok(InspectPathResult {
        exists,
        is_dir,
        is_empty,
        current_data_root,
        fs_error,
    })
}

#[tauri::command]
pub async fn migrate_data_root(
    state: State<'_, AppState>,
    target_path: String,
    move_data: bool,
    clear_old: bool,
) -> Result<(), String> {
    let target = std::path::PathBuf::from(target_path.trim());
    if target.as_os_str().is_empty() {
        return Err("Target data root path cannot be empty".to_string());
    }

    // Re-check server-side: the wizard check is advisory, and a bad filesystem
    // would otherwise leave the engine unable to pull any image.
    crate::docker::engine::check_data_root_filesystem(&target).map_err(|e| e.to_string())?;

    let old_data_root = state.engine.paths().data_root.clone();

    // 1. If engine is currently running, stop it first before moving files
    let was_running = state.daemon_state() == DaemonState::Running;
    if was_running {
        let engine = state.engine.clone();
        crate::runtime::engine_op(move || engine.stop())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        *state.client.write().await = None;
    }

    // 2. Ensure target directory exists
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| format!("Failed to create target directory: {e}"))?;

    // 3. Move/Copy data if requested and old directory exists
    if move_data && old_data_root.exists() && old_data_root != target {
        tracing::info!("Copying data from {:?} to {:?}", old_data_root, target);
        // Use cp -a via std::process::Command for rootless layer preserving permissions and symlinks
        let status = std::process::Command::new("cp")
            .arg("-a")
            .arg(format!("{}/.", old_data_root.display()))
            .arg(format!("{}/", target.display()))
            .status()
            .map_err(|e| format!("Failed to copy data root content: {e}"))?;

        if !status.success() {
            return Err("Failed to copy existing data to new path".to_string());
        }
    }

    // 4. Clear old data if requested
    if clear_old && old_data_root.exists() && old_data_root != target {
        tracing::info!("Clearing old data root directory: {:?}", old_data_root);
        let _ = tokio::fs::remove_dir_all(&old_data_root).await;
    }

    // 5. Update engine paths immediately
    state.engine.set_data_root(Some(&target.to_string_lossy()));

    // 6. Save updated path into AppConfig
    let mut config = state.config.write().await;
    config.data_root = target.to_string_lossy().to_string();
    config.save().await.map_err(|e| e.to_string())?;

    // 7. Restart engine if it was running before
    if was_running {
        let engine = state.engine.clone();
        crate::runtime::engine_op(move || engine.start())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let _ = state.get_client().await;
    }

    Ok(())
}
