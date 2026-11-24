use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::config::AppConfig;
use crate::docker::cli_bridge::{reconcile, xdg_symlink_state};
use crate::docker::models::{DaemonState, DfReport, EngineInfo};
use crate::state::AppState;

#[tauri::command]
pub async fn get_engine_status(state: State<'_, Arc<AppState>>) -> Result<DaemonState, String> {
    Ok(state.daemon_state())
}

#[tauri::command]
pub async fn toggle_engine(state: State<'_, Arc<AppState>>) -> Result<DaemonState, String> {
    // Same gate as the tray, MCP and doctor paths: with a remote endpoint
    // active this would start/stop the local engine (and drop the shared
    // client the remote is using).
    state.engine_control_guard().await?;
    let _lifecycle = crate::runtime::claim_engine_lifecycle()?;
    let current = state.daemon_state();
    match current {
        DaemonState::Running => {
            let engine = state.engine.clone();
            crate::runtime::engine_op(move || engine.stop())
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?;
            state.invalidate_client();
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
            // Engine caps (Settings → Docker Engine) apply as soon as the
            // daemon is up; a failure only logs — the engine keeps running.
            if let Err(e) = apply_engine_limits_now(&state).await {
                tracing::warn!("could not apply engine resource limits: {e}");
            }
            Ok(state.daemon_state())
        }
        other => Ok(other),
    }
}

#[tauri::command]
pub async fn get_engine_info(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<EngineInfo>, String> {
    let client = match state.get_client().await {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    Ok(client.engine_info().await.ok())
}

#[tauri::command]
pub async fn get_disk_usage(state: State<'_, Arc<AppState>>) -> Result<DfReport, String> {
    let client = state.get_client().await?;
    client.system_df().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prune_system(state: State<'_, Arc<AppState>>) -> Result<u64, String> {
    let client = state.get_client().await?;
    client.system_prune().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_app_config(state: State<'_, Arc<AppState>>) -> Result<AppConfig, String> {
    Ok(state.config.read().await.clone())
}

/// Regenerates the CLI shim so it exports the `DOCKER_HOST` of the endpoint
/// that is active in `config` — the local engine socket when none is.
pub fn refresh_shim(state: &AppState, config: &crate::config::AppConfig) {
    if !crate::docker::engine::shim_installed() {
        return;
    }
    let host = config
        .active_endpoint
        .as_deref()
        .filter(|id| *id != "local")
        .and_then(|id| config.endpoints.iter().find(|e| e.id == id))
        .and_then(|e| crate::docker::models::endpoint_docker_host(&e.kind, &e.uri));
    let paths = state.engine.paths();
    if let Err(e) =
        crate::docker::engine::ensure_shim_for(&paths, &config.cli_mode, host.as_deref())
    {
        tracing::warn!("could not refresh the docker shim: {e:#}");
    }
}

#[tauri::command]
pub async fn save_app_config(
    state: State<'_, Arc<AppState>>,
    config: AppConfig,
) -> Result<(), String> {
    save_app_config_core(&state, config).await
}

/// App-free core of [`save_app_config`], shared with the MCP server.
pub(crate) async fn save_app_config_core(
    state: &AppState,
    mut config: AppConfig,
) -> Result<(), String> {
    // Turning off "Auto startup on boot" also clears its dependent switch, so
    // the persisted config never carries a setting the UI cannot show.
    config.normalize();
    // The login entry is part of the setting's contract: if it cannot be
    // written, fail the save rather than pretend the toggle took effect.
    crate::autostart::apply(config.auto_start_on_boot, config.start_minimized)
        .map_err(|e| format!("Could not update the autostart entry: {e}"))?;
    // Settings, Images, Volumes and Kubernetes each save the whole config
    // from a copy they fetched earlier. Anything another command owns and may
    // have changed since (groups, routes, endpoints, registries, ...) is taken
    // from the live config under the write lock, so a stale copy cannot undo
    // it. The merge, save and swap happen as one step.
    let mut live = state.config.write().await;
    preserve_owned_fields(&mut config, &live);
    config.save().await.map_err(|e| e.to_string())?;
    // Sync data_root immediately to engine paths so subsequent daemon restarts take effect without restarting the app
    state.engine.set_data_root(Some(&config.data_root));
    // Keep the process-wide CLI preference in step so the next docker
    // invocation resolves against the mode the user just picked.
    crate::docker::engine::set_cli_mode(&config.cli_mode);
    // A shim pins the binary it was generated for, so switching mode has to
    // rewrite it — otherwise `docker -v` in a terminal keeps reporting the old
    // CLI no matter what Settings says.
    // The shim exports the active endpoint's DOCKER_HOST, so a remote switch
    // moves the terminal with the UI.
    refresh_shim(state, &config);
    // The close handler reads a lock-free mirror; update it before the config
    // is moved so the very next window close honours the new setting.
    state.set_close_to_tray(config.close_to_tray);
    *live = config;
    drop(live);
    // Caps changed on a running engine apply immediately; never reject the
    // save over it (the next engine start re-applies them anyway).
    if let Err(e) = apply_engine_limits_now(state).await {
        tracing::warn!("could not apply engine resource limits after save: {e}");
    }
    Ok(())
}

/// Copies the fields that dedicated commands own from `live` into `incoming`.
fn preserve_owned_fields(incoming: &mut AppConfig, live: &AppConfig) {
    incoming.schema_version = live.schema_version;
    incoming.data_root = live.data_root.clone(); // migrate_data_root
    incoming.endpoints = live.endpoints.clone(); // endpoint commands
    incoming.active_endpoint = live.active_endpoint.clone();
    incoming.registries = live.registries.clone(); // registry commands
    incoming.default_registry = live.default_registry.clone();
    incoming.container_groups = live.container_groups.clone(); // group saves
    incoming.kind_cluster_prefs = live.kind_cluster_prefs.clone(); // per-cluster prefs
    incoming.gateway_routes = live.gateway_routes.clone(); // gateway commands
    incoming.gateway_enabled = live.gateway_enabled;
    incoming.gateway_http_port = live.gateway_http_port;
    incoming.gateway_https_port = live.gateway_https_port;
}

/// Reads the configured engine caps and, when our engine is running, pushes
/// them into its cgroup. Returns the engine's report, or `Ok("")` when there
/// is nothing to apply (engine stopped, or limits disabled).
pub async fn apply_engine_limits_now(state: &AppState) -> Result<String, String> {
    let (memory_mb, cpu_cores) = {
        let cfg = state.config.read().await;
        (cfg.engine_memory_limit_mb, cfg.engine_cpu_limit()?)
    };
    if memory_mb == 0 && cpu_cores <= 0.0 {
        return Ok(String::new());
    }
    if state.daemon_state() != DaemonState::Running {
        return Ok(String::new());
    }
    state.engine.apply_resource_limits(memory_mb, cpu_cores)
}

#[tauri::command]
pub async fn apply_engine_limits(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    apply_engine_limits_now(&state).await
}

#[derive(Clone, serde::Serialize)]
pub struct CliBridgeStatus {
    pub enabled: bool,
    pub link_state: Option<bool>,
}

#[tauri::command]
pub async fn get_cli_bridge_status(
    state: State<'_, Arc<AppState>>,
) -> Result<CliBridgeStatus, String> {
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
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<CliBridgeStatus, String> {
    state.engine_control_guard().await?;
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
    state: State<'_, Arc<AppState>>,
    path: String,
) -> Result<InspectPathResult, String> {
    check_data_root_path_core(&state, path).await
}

/// App-free core of [`check_data_root_path`], shared with the MCP server.
pub(crate) async fn check_data_root_path_core(
    state: &AppState,
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
    state: State<'_, Arc<AppState>>,
    target_path: String,
    move_data: bool,
    clear_old: bool,
) -> Result<(), String> {
    migrate_data_root_core(&state, target_path, move_data, clear_old).await
}

/// App-free core of [`migrate_data_root`], shared with the MCP server. Keeps
/// the engine-control guard: migrating rewrites the bundled engine's data root.
pub(crate) async fn migrate_data_root_core(
    state: &AppState,
    target_path: String,
    move_data: bool,
    clear_old: bool,
) -> Result<(), String> {
    state.engine_control_guard().await?;
    let _lifecycle = crate::runtime::claim_engine_lifecycle()?;
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
        if let Some(msg) = crate::docker::engine::migrate_dtype_conflict(&old_data_root, &target) {
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
        state.invalidate_client();
    }

    // Every step from here can fail. The engine used to stay stopped when one
    // did, which looked like the migration had broken Docker. On failure it is
    // brought back up on the old data root.
    let moved = move_data_root(state, &old_data_root, &target, move_data).await;
    if let Err(e) = moved {
        if was_running {
            state.engine.set_data_root(Some(&old_data_root.to_string_lossy()));
            let engine = state.engine.clone();
            if let Ok(Err(err)) = crate::runtime::engine_op(move || engine.start()).await {
                tracing::warn!("could not restart the engine on the old data root: {err}");
            }
            let _ = state.get_client().await;
        }
        return Err(e);
    }

    // Restart engine if it was running before. The config lock must not be
    // held here: `get_client` reads the config too.
    if was_running {
        let engine = state.engine.clone();
        crate::runtime::engine_op(move || engine.start())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("Data root moved, but the engine did not start on it: {e}"))?;
        let _ = state.get_client().await;
    }

    // Only now, with the new root saved (and running, when it was), is the old
    // copy expendable. Clearing it first lost the data whenever a later step failed.
    if clear_old && old_data_root.exists() && old_data_root != target {
        tracing::info!("Clearing old data root directory: {:?}", old_data_root);
        if let Err(e) = tokio::fs::remove_dir_all(&old_data_root).await {
            tracing::warn!("could not clear the old data root: {e}");
        }
    }

    Ok(())
}

/// Copy (when asked), make snapshotter dirs d_type-compatible, then point the
/// engine and the saved config at `target`.
async fn move_data_root(
    state: &AppState,
    old_data_root: &std::path::Path,
    target: &std::path::Path,
    move_data: bool,
) -> Result<(), String> {
    tokio::fs::create_dir_all(target)
        .await
        .map_err(|e| format!("Failed to create target directory: {e}"))?;

    if move_data && old_data_root.exists() && old_data_root != target {
        tracing::info!("Copying data from {:?} to {:?}", old_data_root, target);
        // `cp -a` keeps the rootless layer's ownership, modes and symlinks.
        // Async so a multi-GB copy does not pin a runtime worker.
        let status = tokio::process::Command::new("cp")
            .arg("-a")
            .arg(format!("{}/.", old_data_root.display()))
            .arg(format!("{}/", target.display()))
            .status()
            .await
            .map_err(|e| format!("Failed to copy data root content: {e}"))?;
        if !status.success() {
            return Err("Failed to copy existing data to new path".to_string());
        }
    }

    // After the copy, make containerd's snapshotter dirs d_type-compatible if
    // the new root's filesystem needs it. Idempotent.
    let compat_target = target.to_path_buf();
    tokio::task::spawn_blocking(move || {
        crate::docker::engine::ensure_snapshotter_compat(&compat_target)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let target_str = target.to_string_lossy().to_string();
    {
        let mut config = state.config.write().await;
        let mut next = config.clone();
        next.data_root = target_str.clone();
        next.save().await.map_err(|e| e.to_string())?;
        *config = next;
    }
    state.engine.set_data_root(Some(&target_str));
    Ok(())
}

/// Check Docker's official static channel against the installed engine.
///
/// Errors are returned rather than swallowed: the UI has to be able to say
/// "couldn't check" instead of reporting a false "up to date".
#[tauri::command]
pub async fn check_engine_update(
    state: State<'_, Arc<AppState>>,
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
    state: State<'_, Arc<AppState>>,
    version: String,
) -> Result<String, String> {
    state.engine_control_guard().await?;
    let version = version.trim().trim_start_matches('v').to_string();
    if version.is_empty() {
        return Err("No engine version to install".to_string());
    }
    // A second click (or Settings and the first-run wizard both firing) ran two
    // installs over the same staging dir and stopped/started the daemon twice.
    let _lifecycle = crate::runtime::claim_engine_lifecycle()?;

    let was_running = state.daemon_state() == DaemonState::Running;
    // A first-ever install leaves the engine stopped unless we start it here:
    // nothing else on this path triggers a boot.
    let should_start = was_running || state.engine.installed_version().is_none();
    let plan = crate::docker::engine::step_plan(crate::docker::engine::ENGINE_UPDATE_STEPS);
    let emit = |progress: &crate::docker::engine::InstallProgress| {
        let _ = app.emit("engine-install-progress", progress);
    };

    // Step 0 — stop the daemon that holds the binaries being replaced.
    if was_running {
        emit(&crate::docker::engine::InstallProgress::stage(
            "engine",
            &version,
            &plan,
            0,
            Some(0.02),
        ));
        let engine = state.engine.clone();
        crate::runtime::engine_op(move || engine.stop())
            .await
            .map_err(|e| e.to_string())
            .and_then(|inner| inner.map_err(|e| e.to_string()))?;
        let _ = app.emit("engine-state-changed", state.daemon_state());
        emit(&crate::docker::engine::InstallProgress::note(
            "engine",
            &version,
            &plan,
            0,
            Some(0.04),
            "Docker Engine stopped",
        ));
    } else {
        emit(&crate::docker::engine::InstallProgress::note(
            "engine",
            &version,
            &plan,
            0,
            Some(0.04),
            "Engine already stopped — no daemon to stop",
        ));
    }

    // Steps 1–4 — download, verify, replace, verify version.
    let paths = state.engine.paths();
    let step = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(1));
    let progress_app = app.clone();
    let seen = step.clone();
    let install = crate::docker::engine::install_engine_update(&paths, &version, move |progress| {
        if let Some(index) = progress.step {
            seen.store(index, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = progress_app.emit("engine-install-progress", &progress);
    })
    .await;

    if let Err(e) = install {
        let msg = format!("{e:#}");
        emit(&crate::docker::engine::InstallProgress::failed(
            "engine",
            &version,
            &plan,
            step.load(std::sync::atomic::Ordering::Relaxed),
            msg.clone(),
        ));
        // A failed update leaves the previous engine intact; if we stopped it,
        // bring it back so the app is not left without a daemon.
        if was_running {
            let engine = state.engine.clone();
            let _ = crate::runtime::engine_op(move || engine.start()).await;
            let _ = state.get_client().await;
            let _ = app.emit("engine-state-changed", state.daemon_state());
        }
        return Err(msg);
    }

    // Step 5 — start the new engine.
    if should_start {
        emit(&crate::docker::engine::InstallProgress::stage(
            "engine",
            &version,
            &plan,
            5,
            Some(0.97),
        ));
        let engine = state.engine.clone();
        if let Err(e) = crate::runtime::engine_op(move || engine.start())
            .await
            .map_err(|e| e.to_string())
            .and_then(|inner| inner.map_err(|e| e.to_string()))
        {
            let msg = format!("Docker Engine {version} is installed but failed to start: {e}");
            emit(&crate::docker::engine::InstallProgress::failed(
                "engine",
                &version,
                &plan,
                5,
                msg.clone(),
            ));
            let _ = app.emit("engine-state-changed", state.daemon_state());
            return Err(msg);
        }
        let _ = state.get_client().await;
        // A restarted engine gets a new socket, so the terminal bridge has to
        // re-attach before the next reconcile tick would.
        let _ = apply_cli_bridge(&state).await;
        emit(&crate::docker::engine::InstallProgress::note(
            "engine",
            &version,
            &plan,
            5,
            Some(0.99),
            format!("Docker Engine {version} is running"),
        ));
    } else {
        emit(&crate::docker::engine::InstallProgress::note(
            "engine",
            &version,
            &plan,
            5,
            Some(0.99),
            "Engine left stopped (it was not running before the update)",
        ));
    }

    let _ = app.emit("engine-state-changed", state.daemon_state());
    emit(&crate::docker::engine::InstallProgress::finished(
        "engine", &version, &plan,
    ));
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
pub async fn check_docker_cli(state: State<'_, Arc<AppState>>) -> Result<CliStatus, String> {
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
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<CliShimStatus, String> {
    state.engine_control_guard().await?;
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

/// Inspect the engine's install, data root, snapshotter links and containerd
/// plugin load states. Read-only — safe to call while the engine runs.
///
/// Only the *bundled* engine can be diagnosed: `EnginePaths` describe its data
/// root, and the checks read that filesystem. While a remote endpoint is active
/// the card is shown as unavailable rather than reporting the wrong machine.
#[tauri::command]
pub async fn diagnose_engine(
    state: State<'_, Arc<AppState>>,
) -> Result<crate::docker::doctor::EngineDoctorReport, String> {
    state.engine_control_guard().await?;
    let paths = state.engine.paths();
    Ok(crate::docker::doctor::diagnose(&paths))
}

/// Apply repairs for the requested checks (or every auto-repairable one when
/// `check_ids` is absent) and return what changed.
///
/// Repairs rewrite engine files under the data root, so they take the same
/// control lock as a data-root migration: only one engine-mutating operation at
/// a time. If the engine is running it keeps running — its registrations are
/// re-read on the next start, which is why the UI restarts the engine after a
/// repair.
#[tauri::command]
pub async fn repair_engine(
    state: State<'_, Arc<AppState>>,
    check_ids: Option<Vec<String>>,
) -> Result<crate::docker::doctor::RepairReport, String> {
    // Refused while a remote endpoint is active: repairing would rewrite the
    // *bundled* engine's files, which that endpoint does not use — the user
    // would see "repaired" and nothing would change.
    state.engine_control_guard().await?;
    let paths = state.engine.paths();
    let auto = crate::docker::doctor::auto_repairable();

    let ids: Vec<&str> = match check_ids.as_deref() {
        Some(requested) => {
            if requested.is_empty() {
                auto
            } else {
                requested.iter().map(String::as_str).collect()
            }
        }
        None => auto,
    };

    let report = crate::docker::doctor::repair(&paths, &ids);
    if !report.failed.is_empty() {
        for (id, reason) in &report.failed {
            tracing::warn!("engine repair failed for {id}: {reason}");
        }
    }
    Ok(report)
}

/// Check the docker-compose plugin against its own GitHub release channel.
#[tauri::command]
pub async fn check_compose_update(
    state: State<'_, Arc<AppState>>,
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
    state: State<'_, Arc<AppState>>,
    version: String,
) -> Result<String, String> {
    let version = version.trim().trim_start_matches('v').to_string();
    let plan = crate::docker::engine::step_plan(crate::docker::engine::COMPOSE_UPDATE_STEPS);
    let step = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let paths = state.engine.paths();
    let progress_app = app.clone();
    let seen = step.clone();
    let result = crate::docker::engine::install_compose_update(&paths, &version, move |progress| {
        if let Some(index) = progress.step {
            seen.store(index, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = progress_app.emit("engine-install-progress", &progress);
    })
    .await;
    match result {
        Ok(()) => Ok(version),
        Err(e) => {
            let msg = format!("{e:#}");
            let _ = app.emit(
                "engine-install-progress",
                crate::docker::engine::InstallProgress::failed(
                    "compose",
                    &version,
                    &plan,
                    step.load(std::sync::atomic::Ordering::Relaxed),
                    msg.clone(),
                ),
            );
            Err(msg)
        }
    }
}

/// Check the bundled `docker` CLI against Docker's static channel.
#[tauri::command]
pub async fn check_cli_update(
    state: State<'_, Arc<AppState>>,
) -> Result<crate::docker::engine::CliUpdateInfo, String> {
    let paths = state.engine.paths();
    crate::docker::engine::check_cli_update(&paths)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Replace only the bundled `docker` binary; the daemon keeps running.
#[tauri::command]
pub async fn install_cli_update(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    version: String,
) -> Result<String, String> {
    let version = version.trim().trim_start_matches('v').to_string();
    let plan = crate::docker::engine::step_plan(crate::docker::engine::CLI_UPDATE_STEPS);
    let step = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let paths = state.engine.paths();
    let progress_app = app.clone();
    let seen = step.clone();
    let result = crate::docker::engine::install_cli_update(&paths, &version, move |progress| {
        if let Some(index) = progress.step {
            seen.store(index, std::sync::atomic::Ordering::Relaxed);
        }
        let _ = progress_app.emit("engine-install-progress", &progress);
    })
    .await;
    match result {
        Ok(()) => {
            let _ = apply_cli_bridge(&state).await;
            Ok(version)
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let _ = app.emit(
                "engine-install-progress",
                crate::docker::engine::InstallProgress::failed(
                    "cli",
                    &version,
                    &plan,
                    step.load(std::sync::atomic::Ordering::Relaxed),
                    msg.clone(),
                ),
            );
            Err(msg)
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct EmbeddedBinaryInfo {
    pub name: String,
    pub category: String,
    pub version: Option<String>,
    pub path: Option<String>,
    pub is_active: bool,
    pub description: String,
}

/// Lists all bundled and embedded tool binaries (dockerd, docker, containerd, runc, rootlesskit, docker-compose, kind)
/// along with their active state, version, path, and purpose.
#[tauri::command]
pub async fn get_embedded_binaries(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<EmbeddedBinaryInfo>, String> {
    // Every row probes a binary (`--version`); run them off the async
    // runtime so the Settings panel cannot stall other commands.
    let caddy_running = state.caddy.is_running().await;
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || embedded_binaries_blocking(&state, caddy_running))
        .await
        .map_err(|e| e.to_string())?
}

fn embedded_binaries_blocking(
    state: &AppState,
    caddy_running: bool,
) -> Result<Vec<EmbeddedBinaryInfo>, String> {
    let engine_paths = state.engine.paths();
    let bin_dir = engine_paths.bin_dir();
    let mut list = Vec::new();

    // 1. Docker (dockerd + docker CLI)
    let dockerd_path = bin_dir.join("dockerd");
    let docker_path = bin_dir.join("docker");
    let docker_ver = if dockerd_path.is_file() {
        crate::docker::engine::installed_engine_version(&engine_paths)
            .or_else(|| run_cmd_version(&dockerd_path, &["--version"]))
    } else if docker_path.is_file() {
        crate::docker::engine::bundled_cli_version(&engine_paths)
            .or_else(|| run_cmd_version(&docker_path, &["--version"]))
    } else {
        None
    };
    list.push(EmbeddedBinaryInfo {
        name: "docker / dockerd".into(),
        category: "Engine & CLI".into(),
        version: docker_ver,
        path: if dockerd_path.is_file() {
            Some(dockerd_path.display().to_string())
        } else if docker_path.is_file() {
            Some(docker_path.display().to_string())
        } else {
            None
        },
        is_active: state.daemon_state() == DaemonState::Running,
        description: "Official Docker daemon & CLI command-line tool suite".into(),
    });

    // 3. containerd
    let containerd_path = bin_dir.join("containerd");
    let containerd_ver = if containerd_path.is_file() {
        run_cmd_version(&containerd_path, &["--version"])
            .map(|s| clean_containerd_version(&s))
    } else {
        None
    };
    list.push(EmbeddedBinaryInfo {
        name: "containerd".into(),
        category: "Container Runtime".into(),
        version: containerd_ver,
        path: containerd_path.is_file().then(|| containerd_path.display().to_string()),
        is_active: state.daemon_state() == DaemonState::Running,
        description: "Industry-standard core container runtime supervisor".into(),
    });

    // 4. runc
    let runc_path = bin_dir.join("runc");
    let runc_ver = if runc_path.is_file() {
        run_cmd_version(&runc_path, &["--version"])
            .map(|s| clean_runc_version(&s))
    } else {
        None
    };
    list.push(EmbeddedBinaryInfo {
        name: "runc".into(),
        category: "OCI Runtime".into(),
        version: runc_ver,
        path: runc_path.is_file().then(|| runc_path.display().to_string()),
        is_active: runc_path.is_file(),
        description: "Low-level CLI tool for spawning containers according to OCI specification".into(),
    });

    // 5. rootlesskit
    let rootless_path = bin_dir.join("rootlesskit");
    let rootless_ver = if rootless_path.is_file() {
        run_cmd_version(&rootless_path, &["--version"])
            .map(|s| clean_rootless_version(&s))
    } else {
        None
    };
    list.push(EmbeddedBinaryInfo {
        name: "rootlesskit".into(),
        category: "Rootless Engine".into(),
        version: rootless_ver,
        path: rootless_path.is_file().then(|| rootless_path.display().to_string()),
        is_active: state.daemon_state() == DaemonState::Running,
        description: "Linux user-namespace supervisor providing unprivileged rootless isolation".into(),
    });

    // 6. docker-compose
    let compose_plugin = engine_paths.compose_plugin();
    let compose_ver = if compose_plugin.is_file() {
        crate::docker::engine::compose_plugin_version(&compose_plugin)
    } else {
        None
    };
    list.push(EmbeddedBinaryInfo {
        name: "docker-compose".into(),
        category: "Compose Plugin".into(),
        version: compose_ver,
        path: compose_plugin.is_file().then(|| compose_plugin.display().to_string()),
        is_active: compose_plugin.is_file(),
        description: "Multi-container Docker orchestration plugin (CLI v2)".into(),
    });

    // 7. kind (Kubernetes IN Docker)
    let kind_paths = &state.kind;
    let kind_status = crate::docker::kind::kind_status(kind_paths);
    list.push(EmbeddedBinaryInfo {
        name: "kind".into(),
        category: "Kubernetes Tool".into(),
        version: kind_status.version,
        path: kind_status.path,
        is_active: kind_status.installed,
        description: "Kubernetes IN Docker runner and cluster management tool".into(),
    });

    // 8. caddy (HTTPS Reverse Proxy Gateway)
    let caddy_paths = &state.caddy.paths;
    let caddy_installed = caddy_paths.is_installed();
    let caddy_ver = if caddy_installed {
        run_cmd_version(&caddy_paths.binary(), &["version"])
            .map(|s| s.split_whitespace().next().unwrap_or("").to_string())
    } else {
        None
    };
    list.push(EmbeddedBinaryInfo {
        name: "caddy".into(),
        category: "Gateway & Proxy".into(),
        version: caddy_ver,
        path: caddy_installed.then(|| caddy_paths.binary().display().to_string()),
        is_active: caddy_running,
        description: "Zero-config HTTPS reverse proxy gateway with automatic local CA certificate".into(),
    });

    Ok(list)
}

fn run_cmd_version(bin: &std::path::Path, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args);
    // Bounded: a hung binary froze the "Embedded binaries" settings panel.
    let out = crate::procutil::output_with_timeout(cmd, crate::procutil::PROBE, &bin.display().to_string()).ok()?;
    let text = if !out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stdout).to_string()
    } else {
        String::from_utf8_lossy(&out.stderr).to_string()
    };
    text.lines().next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn clean_containerd_version(s: &str) -> String {
    // e.g. "containerd github.com/containerd/containerd/v2 v2.4.2 b7a252..." -> "v2.4.2"
    for part in s.split_whitespace() {
        if part.starts_with('v') && part.chars().nth(1).map_or(false, |c| c.is_ascii_digit()) {
            return part.to_string();
        }
    }
    s.to_string()
}

fn clean_runc_version(s: &str) -> String {
    // e.g. "runc version 1.5.2" -> "1.5.2"
    s.strip_prefix("runc version ")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| s.to_string())
}

fn clean_rootless_version(s: &str) -> String {
    // e.g. "rootlesskit version 3.3.0" -> "3.3.0"
    s.strip_prefix("rootlesskit version ")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| s.to_string())
}
