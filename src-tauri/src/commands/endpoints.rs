use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

use crate::config::EndpointConfig;
use crate::docker::client::DockerClient;
use crate::docker::models::{
    endpoint_host_port, endpoint_id_from_name, validate_endpoint, EndpointInput, EndpointStatus,
};
use crate::state::AppState;

#[tauri::command]
pub async fn list_endpoints(state: State<'_, AppState>) -> Result<Vec<EndpointStatus>, String> {
    let (active, endpoints) = {
        let cfg = state.config.read().await;
        (cfg.active_endpoint.clone(), cfg.endpoints.clone())
    };
    let engine = state.engine.clone();
    let socket = engine.paths().socket.clone();
    let socket_str = socket.display().to_string();

    let mut items: Vec<EndpointStatus> = Vec::new();

    // The local bundled engine is always the first (and immovable) row.
    let mut local = EndpointStatus::local(&socket_str);
    let active_id = active.as_deref().unwrap_or("local");
    local.is_active = active_id == "local";
    items.push(local);

    for ep in &endpoints {
        let mut status = EndpointStatus {
            id: ep.id.clone(),
            name: ep.name.clone(),
            kind: ep.kind.clone(),
            uri: ep.uri.clone(),
            is_active: active_id == ep.id,
            reachable: false,
            version: None,
            latency_ms: None,
            error: None,
        };
        if status.is_active {
            if let Ok(client) = state.get_client().await {
                let t0 = Instant::now();
                match client.version_string().await {
                    Ok(v) => {
                        status.reachable = true;
                        status.version = Some(v);
                        status.latency_ms = Some(t0.elapsed().as_millis() as u64);
                        status.error = None;
                    }
                    Err(e) => {
                        status.reachable = false;
                        status.error = Some(e.to_string());
                    }
                }
            } else {
                status.reachable = false;
                status.error = Some("Engine not reachable".to_string());
            }
        }
        items.push(status);
    }
    Ok(items)
}

#[tauri::command]
pub async fn test_endpoint(ep: EndpointInput) -> Result<EndpointStatus, String> {
    validate_endpoint(&ep.kind, &ep.uri)?;
    let socket = None;
    let t0 = Instant::now();
    let client = DockerClient::connect_for_endpoint(&ep.kind, &ep.uri, socket)
        .map_err(|e| e.to_string())?;
    let version = client.version_string().await.map_err(|e| e.to_string())?;
    let latency = Some(t0.elapsed().as_millis() as u64);
    Ok(EndpointStatus {
        id: ep.id.clone().unwrap_or_else(|| endpoint_id_from_name(&ep.name)),
        name: ep.name.clone(),
        kind: ep.kind.clone(),
        uri: ep.uri.clone(),
        is_active: false,
        reachable: true,
        version: Some(version),
        latency_ms: latency,
        error: None,
    })
}

#[tauri::command]
pub async fn save_endpoint(state: State<'_, AppState>, ep: EndpointInput) -> Result<(), String> {
    validate_endpoint(&ep.kind, &ep.uri)?;
    // Verify reachability before saving (the UI will have tested it too; this
    // guards against TOCTOU while still keeping the form simple).
    let socket = None;
    let probe = DockerClient::connect_for_endpoint(&ep.kind, &ep.uri, socket)
        .map_err(|e| format!("{e:#}"))?;
    probe.ping().await.map_err(|e| format!("{e:#}"))?;
    let id = ep.id.clone().unwrap_or_else(|| endpoint_id_from_name(&ep.name));
    let cfg = EndpointConfig {
        id: id.clone(),
        name: ep.name.clone(),
        kind: ep.kind.clone(),
        uri: ep.uri.clone(),
    };
    {
        let mut config = state.config.write().await;
        let mut next: Vec<EndpointConfig> = config.endpoints.clone();
        next.retain(|x| x.id != id);
        next.push(cfg);
        config.endpoints = next;
    }
    state.save_config().await?;
    Ok(())
}

#[tauri::command]
pub async fn remove_endpoint(state: State<'_, AppState>, id: String) -> Result<(), String> {
    if id == "local" {
        return Err("Cannot remove the bundled engine".to_string());
    }
    {
        let mut config = state.config.write().await;
        let before = config.endpoints.len();
        config.endpoints.retain(|x| x.id != id);
        if config.endpoints.len() == before {
            // Nothing removed: return a non-destructive error to the UI.
            return Err("Unknown endpoint".to_string());
        }
        if config.active_endpoint.as_deref() == Some(id.as_str()) {
            config.active_endpoint = None;
        }
    }
    state.save_config().await?;
    Ok(())
}

#[tauri::command]
pub async fn set_active_endpoint(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    // Switching requires a live daemon for the new target. For the local
    // engine the supervisor owns the socket; for remotes we test on-connect.
    if id == "local" {
        {
            let mut config = state.config.write().await;
            config.active_endpoint = None;
        }
        state.save_config().await?;
        state.invalidate_client();
        crate::commands::system::refresh_shim(&state, &*state.config.read().await);
        let _ = crate::commands::system::apply_cli_bridge(&state).await;
        let _ = app.emit("endpoint-changed", id);
        return Ok(());
    }
    let target = {
        let cfg = state.config.read().await;
        cfg.endpoints.iter().find(|x| x.id == id).cloned()
    };
    let Some(target) = target else {
        return Err("Unknown endpoint".to_string());
    };
    validate_endpoint(&target.kind, &target.uri)?;
    // Prove reachability before committing the switch.
    let socket = None;
    let probe = DockerClient::connect_for_endpoint(&target.kind, &target.uri, socket)
        .map_err(|e| format!("{e:#}"))?;
    probe.ping().await.map_err(|e| format!("{e:#}"))?;
    {
        let mut config = state.config.write().await;
        config.active_endpoint = Some(id.clone());
    }
    state.save_config().await?;
    state.invalidate_client();
    crate::commands::system::refresh_shim(&state, &*state.config.read().await);
    let _ = app.emit("endpoint-changed", id);
    Ok(())
}

#[tauri::command]
pub async fn get_active_endpoint(state: State<'_, AppState>) -> Result<Option<EndpointStatus>, String> {
    let (active, endpoints) = {
        let cfg = state.config.read().await;
        (cfg.active_endpoint.clone(), cfg.endpoints.clone())
    };
    let engine = state.engine.clone();
    let socket = engine.paths().socket.clone();
    let socket_str = socket.display().to_string();
    let active_id = active.as_deref().unwrap_or("local");
    if active_id == "local" {
        let mut local = EndpointStatus::local(&socket_str);
        if let Ok(client) = state.get_client().await {
            let t0 = Instant::now();
            if let Ok(v) = client.version_string().await {
                local.reachable = true;
                local.version = Some(v);
                local.latency_ms = Some(t0.elapsed().as_millis() as u64);
            }
        }
        return Ok(Some(local));
    }
    if let Some(ep) = endpoints.iter().find(|x| x.id == active_id) {
        let mut status = EndpointStatus {
            id: ep.id.clone(),
            name: ep.name.clone(),
            kind: ep.kind.clone(),
            uri: ep.uri.clone(),
            is_active: true,
            reachable: false,
            version: None,
            latency_ms: None,
            error: None,
        };
        if let Ok(client) = state.get_client().await {
            let t0 = Instant::now();
            match client.version_string().await {
                Ok(v) => {
                    status.reachable = true;
                    status.version = Some(v);
                    status.latency_ms = Some(t0.elapsed().as_millis() as u64);
                }
                Err(e) => {
                    status.error = Some(e.to_string());
                }
            }
        }
        return Ok(Some(status));
    }
    Ok(None)
}

/// Helper for the UI: a compact host:port string for the currently active
/// endpoint. For the local socket this is still the engine socket string.
#[tauri::command]
pub async fn active_endpoint_host(state: State<'_, AppState>) -> Result<String, String> {
    let (active, endpoints) = {
        let cfg = state.config.read().await;
        (cfg.active_endpoint.clone(), cfg.endpoints.clone())
    };
    let active_id = active.as_deref().unwrap_or("local");
    if active_id == "local" {
        let engine = state.engine.clone();
        return Ok(engine.paths().socket.display().to_string());
    }
    if let Some(ep) = endpoints.iter().find(|x| x.id == active_id) {
        return Ok(endpoint_host_port(&ep.kind, &ep.uri));
    }
    let engine = state.engine.clone();
    Ok(engine.paths().socket.display().to_string())
}
