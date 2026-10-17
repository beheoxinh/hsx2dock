use tauri::{AppHandle, Emitter, State};

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
    // Keep the process-wide CLI preference in step so the next docker
    // invocation resolves against the mode the user just picked.
    crate::docker::engine::set_cli_mode(&config.cli_mode);
    // A shim pins the binary it was generated for, so switching mode has to
    // rewrite it — otherwise `docker -v` in a terminal keeps reporting the old
    // CLI no matter what Settings says.
    if crate::docker::engine::shim_installed() {
        let paths = state.engine.paths();
        let mode = config.cli_mode.clone();
        if let Err(e) = crate::docker::engine::ensure_shim(&paths, &mode) {
            tracing::warn!("could not refresh the docker shim after a mode change: {e:#}");
        }
    }
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
    {
        let mut config = state.config.write().await;
        config.cli_bridge_enabled = enabled;
        config.save().await.map_err(|e| e.to_string())?;
    }
    let link_state = apply_cli_bridge(&state).await;

    let config = state.config.read().await;
    Ok(CliBridgeStatus {
        enabled: config.cli_bridge_enabled,
        link_state,
    })
}

/// Bring the terminal bridge in line with `cli_bridge_enabled`.
///
/// Runs reconcile using the current daemon state so enabling the bridge when
/// the engine comes up auto-links without needing the user to toggle it again.
pub async fn apply_cli_bridge(state: &AppState) -> Option<bool> {
    let engine_socket = state.engine.paths().socket.clone();
    let is_live = state.daemon_state() == DaemonState::Running;
    {
        let config = state.config.read().await;
        let _ = reconcile(
            &engine_socket,
            config.cli_bridge_enabled,
            config.cli_bridge_set_default,
            is_live,
            &config.cli_bridge_prev_context,
        );
    }
    xdg_symlink_state(&engine_socket)
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
    // user before a daemon restart + copy that could never have worked. A bad
    // filesystem is still workable when we can retarget containerd's
    // snapshotter dirs onto a d_type-capable one, so only report it when that
    // fallback is unavailable.
    let fs_error = if crate::docker::engine::dir_supports_d_type(&p) == Some(false)
        && crate::docker::engine::dtype_compat_root().is_some()
    {
        None
    } else {
        crate::docker::engine::check_data_root_filesystem(&p)
            .err()
            .map(|e| e.to_string())
    };

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

    let old_data_root = state.engine.paths().data_root.clone();

    // Migrating onto the active data root is a no-op that would otherwise offer
    // to wipe it; refuse before stopping the engine or touching any file.
    let canon = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if canon(&target) == canon(&old_data_root) {
        return Err("This path is already the active data root — nothing to migrate.".to_string());
    }

    // Fail fast, before stopping the engine or copying anything, if the target
    // filesystem cannot report d_type and no compatible fallback exists. Also,
    // if source contains non-empty snapshotter dirs that must become symlinks on
    // a d_type-less target, refuse before moving 42M of data.
    if crate::docker::engine::dir_supports_d_type(&target) == Some(false) {
        if let Some(msg) = crate::docker::engine::migrate_dtype_conflict(&old_data_root, &target)
        {
            return Err(msg);
        }
        if crate::docker::engine::dtype_compat_root().is_none() {
            return crate::docker::engine::check_data_root_filesystem(&target)
                .map(|_| ())
                .map_err(|e| e.to_string());
        }
    }

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

    // 3b. After the copy, make containerd's snapshotter dirs d_type-compatible
    // if the new root's filesystem needs it. Idempotent, and no-op when the
    // filesystem already reports real entry types.
    crate::docker::engine::ensure_snapshotter_compat(&target).map_err(|e| e.to_string())?;

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

/// Check Docker's official static channel against the installed engine.
///
/// Errors are returned rather than swallowed: the UI has to be able to say
/// "couldn't check" instead of reporting a false "up to date".
#[tauri::command]
pub async fn check_engine_update(
    state: State<'_, AppState>,
) -> Result<crate::docker::engine::EngineUpdateInfo, String> {
    let paths = state.engine.paths();
    crate::docker::engine::check_engine_update(&paths)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Download a newer engine and swap it in, streaming `engine-install-progress`
/// events while it runs.
///
/// The daemon is stopped first because it holds the binaries being replaced,
/// and restarted afterwards only if it was running — a failed install leaves
/// the previous engine intact, so there is always something to come back to.
#[tauri::command]
pub async fn install_engine_update(
    app: AppHandle,
    state: State<'_, AppState>,
    version: String,
) -> Result<String, String> {
    let version = version.trim().trim_start_matches('v').to_string();
    if version.is_empty() {
        return Err("No engine version to install".to_string());
    }

    let was_running = state.daemon_state() == DaemonState::Running;
    // A first-ever install leaves the engine stopped unless we start it here:
    // nothing else on this path triggers a boot.
    let should_start = was_running || state.engine.installed_version().is_none();
    if was_running {
        let engine = state.engine.clone();
        crate::runtime::engine_op(move || engine.stop())
            .await
            .map_err(|e| e.to_string())
            .and_then(|inner| inner.map_err(|e| e.to_string()))?;
        let _ = app.emit("engine-state-changed", state.daemon_state());
    }

    let paths = state.engine.paths();
    let progress_app = app.clone();
    let install = crate::docker::engine::install_engine_update(&paths, &version, move |progress| {
        let _ = progress_app.emit("engine-install-progress", &progress);
    })
    .await;

    let restart = if should_start {
        let engine = state.engine.clone();
        crate::runtime::engine_op(move || engine.start())
            .await
            .map_err(|e| e.to_string())
            .and_then(|inner| inner.map_err(|e| e.to_string()))
    } else {
        Ok(())
    };

    install.map_err(|e| format!("{e:#}"))?;
    restart?;
    if should_start {
        let _ = state.get_client().await;
        // A restarted engine gets a new socket, so the terminal bridge has to
        // re-attach before the next reconcile tick would.
        let _ = apply_cli_bridge(&state).await;
    }

    let _ = app.emit("engine-state-changed", state.daemon_state());
    Ok(version)
}

#[derive(Clone, serde::Serialize)]
pub struct CliShimStatus {
    pub installed: bool,
    pub path: Option<String>,
    /// Whether the shim would actually win in a terminal (`active`),
    /// is unreachable (`not_on_path`), or is beaten by another docker
    /// (`shadowed`). `None` when no shim is installed.
    pub effect: Option<crate::docker::engine::ShimEffect>,
    /// A `docker` already on PATH that the shim would override, so the UI can
    /// warn before the user hides their own install.
    pub shadows: Option<String>,
    /// The binary the shim execs. If it differs from what the resolver currently
    /// picks, the shim is stale and should be regenerated.
    pub target: Option<String>,
    pub stale: bool,
}

/// Everything the Docker CLI settings card needs, in one round trip: which
/// binary the app resolved, the versions it compared, and whether the
/// terminal shim is in place.
#[derive(Clone, serde::Serialize)]
pub struct CliStatus {
    pub resolution: crate::docker::engine::CliResolution,
    pub shim: CliShimStatus,
}

/// Which docker CLI the app is using, and why.
///
/// Also normalises the compose plugin layout first: the answer is only
/// truthful if `docker compose` will actually resolve to the bundled plugin.
#[tauri::command]
pub async fn check_docker_cli(state: State<'_, AppState>) -> Result<CliStatus, String> {
    let paths = state.engine.paths();
    let _ = crate::docker::engine::ensure_compose_layout(&paths);
    let config = state.config.read().await;
    let resolution = crate::docker::engine::resolve_cli(&paths, &config.cli_mode, &[])?;
    // Report what the shim would override, so the card can warn before the
    // user hides a docker they already had.
    let shadows = crate::docker::engine::find_system_docker(
        &paths,
        &crate::docker::engine::shim_path()
            .into_iter()
            .collect::<Vec<_>>(),
    )
    .map(|p| p.display().to_string());
    // Cloned before `resolution` is moved into the return value; the shim is
    // stale when the binary it pins is no longer the one the resolver picked.
    let resolved_path = resolution.path.clone();
    Ok(CliStatus {
        resolution,
        shim: CliShimStatus {
            shadows,
            ..current_shim_status(Some(&resolved_path))
        },
    })
}

/// Read-only shim state, shared by the builders above.
///
/// `expected` is the binary the resolver would pick right now; passing it lets
/// the card tell "installed and current" apart from "installed but pinned to a
/// CLI the user has since switched away from".
fn current_shim_status(expected: Option<&std::path::Path>) -> CliShimStatus {
    let target = crate::docker::engine::shim_target();
    let stale = match (&target, expected) {
        (Some(t), Some(e)) => t.as_path() != e,
        _ => false,
    };
    CliShimStatus {
        installed: crate::docker::engine::shim_installed(),
        path: crate::docker::engine::shim_path().map(|p| p.display().to_string()),
        effect: crate::docker::engine::shim_effect(),
        shadows: None,
        target: target.map(|t| t.display().to_string()),
        stale,
    }
}

/// Install or remove the `~/.local/bin/docker` shim.
#[tauri::command]
pub async fn toggle_cli_shim(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<CliShimStatus, String> {
    let paths = state.engine.paths();
    let mode = state.config.read().await.cli_mode.clone();

    if enabled {
        crate::docker::engine::ensure_shim(&paths, &mode).map_err(|e| format!("{e:#}"))?;
    } else {
        crate::docker::engine::remove_shim().map_err(|e| format!("{e:#}"))?;
    }

    let mut config = state.config.write().await;
    config.cli_shim_enabled = enabled;
    config.save().await.map_err(|e| e.to_string())?;

    let expected = crate::docker::engine::resolve_cli(&paths, &mode, &[])
        .ok()
        .map(|r| r.path);
    Ok(current_shim_status(expected.as_deref()))
}

/// Check the docker-compose plugin against its own GitHub release channel.
#[tauri::command]
pub async fn check_compose_update(
    state: State<'_, AppState>,
) -> Result<crate::docker::engine::ComposeUpdateInfo, String> {
    let paths = state.engine.paths();
    crate::docker::engine::check_compose_update(&paths)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Download and install a newer docker-compose plugin, streaming progress on
/// the same channel the engine update uses.
#[tauri::command]
pub async fn install_compose_update(
    app: AppHandle,
    state: State<'_, AppState>,
    version: String,
) -> Result<String, String> {
    let paths = state.engine.paths();
    let progress_app = app.clone();
    crate::docker::engine::install_compose_update(&paths, &version, move |progress| {
        let _ = progress_app.emit("engine-install-progress", &progress);
    })
    .await
    .map_err(|e| format!("{e:#}"))?;
    Ok(version)
}
