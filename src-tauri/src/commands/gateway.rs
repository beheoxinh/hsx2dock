use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::docker::caddy::{
    check_caddy_update as check_caddy_update_core, clear_app_hosts_entries,
    inspect_hosts_sync, install_caddy_version, parse_hosts_file, save_raw_hosts_content,
    update_hosts_file, CaddyInstallStatus, CaddyUpdateInfo, GatewayRoute, HostsFileModel,
    HostsSyncStatus, CADDY_UPDATE_STEPS, CADDY_VERSION,
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

    // Copy out and drop the lock before touching /etc/hosts: the file read
    // ran on an async worker with the config read lock held.
    let (http_port, https_port, routes, auto_start) = {
        let config = state.config.read().await;
        (
            config.gateway_http_port,
            config.gateway_https_port,
            config.gateway_routes.clone(),
            config.gateway_auto_start,
        )
    };
    let hosts_status = tokio::task::spawn_blocking(move || inspect_hosts_sync(&routes))
        .await
        .map_err(|e| e.to_string())?;

    let version = if installed {
        std::fs::read_to_string(paths.version_dir(&paths.active_version()).join("VERSION"))
            .ok()
            .map(|s| s.trim().to_string())
            .or(Some(paths.active_version()))
    } else {
        None
    };

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
    save_gateway_routes_core(&state, routes).await
}

/// App-free core of [`save_gateway_routes`] (kind ingress exposure uses it).
pub(crate) async fn save_gateway_routes_core(
    state: &AppState,
    routes: Vec<GatewayRoute>,
) -> Result<(), String> {
    crate::docker::caddy::validate_routes(&routes).map_err(|e| e.to_string())?;
    let (http_port, https_port) = {
        // Save a copy, then swap: a failed write left memory holding routes
        // the disk never got, which the next unrelated save then persisted.
        let mut config = state.config.write().await;
        let mut next = config.clone();
        next.gateway_routes = routes.clone();
        next.save().await.map_err(|e| e.to_string())?;
        *config = next;
        (config.gateway_http_port, config.gateway_https_port)
    };

    // The hosts sync goes first and off the async workers: it may wait on the
    // pkexec password dialog, which used to run with the config lock held and
    // stall every other command, and a reload failure skipped it entirely.
    // It only prompts when the managed block actually changes.
    let hosts_routes = routes.clone();
    match tokio::task::spawn_blocking(move || update_hosts_file(&hosts_routes)).await {
        Ok(Err(e)) => tracing::warn!("Auto-syncing /etc/hosts failed: {e}"),
        Err(e) => tracing::warn!("Auto-syncing /etc/hosts panicked: {e}"),
        Ok(Ok(())) => {}
    }

    if state.caddy.is_running().await {
        state
            .caddy
            .apply_routes(http_port, https_port, &routes)
            .await
            .map_err(|e| format!("Routes saved, but reloading the gateway failed: {e}"))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn toggle_gateway(
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<bool, String> {
    // Copy what Caddy needs and release the lock: start polls for up to 3s,
    // and a held read lock stalls every config writer (and, behind it, every
    // reader) for that long.
    let (http_port, https_port, routes) = {
        let config = state.config.read().await;
        (
            config.gateway_http_port,
            config.gateway_https_port,
            config.gateway_routes.clone(),
        )
    };
    if enabled {
        state
            .caddy
            .start(http_port, https_port, &routes)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        state.caddy.stop().await.map_err(|e| e.to_string())?;
    }

    // Persist only what actually happened: saving `enabled` before a start
    // that then failed left the switch on for a gateway that never ran.
    let running = state.caddy.is_running().await;
    {
        // Save a copy first: a failed write must not leave memory and disk
        // disagreeing about whether the gateway is on.
        let mut config = state.config.write().await;
        let mut next = config.clone();
        next.gateway_enabled = running;
        next.save().await.map_err(|e| e.to_string())?;
        *config = next;
    }
    Ok(running)
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
    // `update_hosts_file` runs pkexec synchronously; the password dialog can
    // stay open for minutes. Run it off the async workers and without the
    // config lock held.
    let routes = state.config.read().await.gateway_routes.clone();
    tokio::task::spawn_blocking(move || update_hosts_file(&routes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_gateway_auto_start(
    state: State<'_, Arc<AppState>>,
    auto_start: bool,
) -> Result<(), String> {
    let mut config = state.config.write().await;
    let mut next = config.clone();
    next.gateway_auto_start = auto_start;
    next.save().await.map_err(|e| e.to_string())?;
    *config = next;
    Ok(())
}

/// Which enabled routes' upstreams accept a TCP connection right now.
#[tauri::command]
pub async fn probe_gateway_upstreams(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<crate::docker::caddy::UpstreamProbe>, String> {
    let routes = state.config.read().await.gateway_routes.clone();
    Ok(crate::docker::caddy::probe_upstreams(&routes).await)
}

#[tauri::command]
pub async fn get_hosts_file_model(
    state: State<'_, Arc<AppState>>,
) -> Result<HostsFileModel, String> {
    let config = state.config.read().await;
    Ok(parse_hosts_file(&config.gateway_routes))
}

#[tauri::command]
pub async fn check_caddy_update(
    state: State<'_, Arc<AppState>>,
) -> Result<CaddyUpdateInfo, String> {
    check_caddy_update_core(&state.caddy.paths)
        .await
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn install_caddy_update(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    version: String,
) -> Result<(), String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let plan = crate::docker::engine::step_plan(CADDY_UPDATE_STEPS);
    let step = Arc::new(AtomicUsize::new(0));
    let handle = app.clone();
    let seen = step.clone();
    let paths = &state.caddy.paths;
    let result = install_caddy_version(paths, &version, move |progress| {
        if let Some(index) = progress.step {
            seen.store(index, Ordering::Relaxed);
        }
        let _ = handle.emit("engine-install-progress", &progress);
    })
    .await;

    match result {
        Ok(_) => {
            // A running gateway keeps executing the old binary until it is
            // restarted, while the UI already reports the new version as
            // active. Restart it on the new one with the saved routes.
            if state.caddy.is_running().await {
                let (http_port, https_port, routes) = {
                    let config = state.config.read().await;
                    (
                        config.gateway_http_port,
                        config.gateway_https_port,
                        config.gateway_routes.clone(),
                    )
                };
                state.caddy.stop().await.map_err(|e| {
                    format!("Caddy {version} installed, but stopping the old gateway failed: {e}")
                })?;
                state
                    .caddy
                    .start(http_port, https_port, &routes)
                    .await
                    .map_err(|e| {
                        format!("Caddy {version} installed, but restarting the gateway failed: {e}")
                    })?;
            }
            Ok(())
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let failed = crate::docker::engine::InstallProgress::failed(
                "caddy",
                &version,
                &plan,
                step.load(Ordering::Relaxed),
                msg.clone(),
            );
            let _ = app.emit("engine-install-progress", &failed);
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn set_gateway_ports(
    state: State<'_, Arc<AppState>>,
    http_port: u16,
    https_port: u16,
) -> Result<(), String> {
    if http_port == 0 || https_port == 0 {
        return Err("Port numbers must be greater than 0".into());
    }
    if http_port == https_port {
        return Err("HTTP and HTTPS ports must be distinct".into());
    }
    let routes = {
        let mut config = state.config.write().await;
        let mut next = config.clone();
        next.gateway_http_port = http_port;
        next.gateway_https_port = https_port;
        next.save().await.map_err(|e| e.to_string())?;
        *config = next;
        config.gateway_routes.clone()
    };
    if state.caddy.is_running().await {
        state
            .caddy
            .apply_routes(http_port, https_port, &routes)
            .await
            .map_err(|e| format!("Ports saved, but reloading the gateway failed: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn save_raw_hosts(new_content: String) -> Result<(), String> {
    // pkexec blocks on the password dialog — keep it off the async workers.
    tokio::task::spawn_blocking(move || save_raw_hosts_content(&new_content))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_app_hosts(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let _ = state;
    tokio::task::spawn_blocking(clear_app_hosts_entries)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}
