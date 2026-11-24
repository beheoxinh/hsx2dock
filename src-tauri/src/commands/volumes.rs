use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tauri::State;

use crate::docker::models::VolumeItem;
use crate::fsutil::dir_size;
use crate::state::AppState;

/// Lists volumes and, for each one whose mountpoint is a readable local
/// directory, attaches the on-disk size. Drivers that expose a non-local path
/// (or an unreadable one) get `size: None` rather than failing the whole list.
#[tauri::command]
pub async fn list_volumes(state: State<'_, Arc<AppState>>) -> Result<Vec<VolumeItem>, String> {
    list_volumes_core(&state).await
}

/// App-free core of [`list_volumes`], shared with the MCP server.
pub(crate) async fn list_volumes_core(state: &AppState) -> Result<Vec<VolumeItem>, String> {
    let client = state.get_client().await?;
    let mut volumes = client.list_volumes().await.map_err(|e| e.to_string())?;

    // A remote daemon's mountpoints are paths on *its* host. Walking them here
    // read whatever happened to sit at the same path locally (often the
    // system Docker's own volumes) and reported that as the remote size.
    if state.engine_control_guard().await.is_err() {
        for v in volumes.iter_mut() {
            v.size = None;
            v.restricted_dirs = 0;
        }
        return Ok(volumes);
    }

    // Size calculation walks the filesystem; keep it off the async runtime.
    let sized = tokio::task::spawn_blocking(move || {
        for v in volumes.iter_mut() {
            let path = Path::new(&v.mountpoint).to_path_buf();
            match dir_size(&path) {
                Ok(ds) => {
                    v.size = Some(ds.bytes);
                    v.restricted_dirs = ds.restricted_dirs;
                }
                // Unreadable / non-local mountpoint: leave size unknown.
                Err(_) => {
                    v.size = None;
                    v.restricted_dirs = 0;
                }
            }
        }
        volumes
    })
    .await
    .map_err(|e| format!("size calculation task failed: {e}"))?;

    Ok(sized)
}

#[tauri::command]
pub async fn create_volume(
    state: State<'_, Arc<AppState>>,
    name: String,
    driver: String,
    driver_opts: HashMap<String, String>,
    labels: HashMap<String, String>,
) -> Result<(), String> {
    let client = state.get_client().await?;
    client
        .create_volume(&name, &driver, driver_opts, labels)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_volume(
    state: State<'_, Arc<AppState>>,
    name: String,
    force: bool,
) -> Result<(), String> {
    remove_volume_core(&state, &name, force).await
}

/// App-free core of [`remove_volume`].
pub(crate) async fn remove_volume_core(
    state: &AppState,
    name: &str,
    force: bool,
) -> Result<(), String> {
    let client = state.get_client().await?;
    let volumes = client.list_volumes().await.map_err(|e| e.to_string())?;
    if volumes.iter().any(|v| v.protected && v.name == name) {
        return Err(
            "This volume is managed by kind. Remove the cluster from the \
             Kubernetes page instead."
                .to_string(),
        );
    }
    client
        .remove_volume(&name, force)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn prune_volumes(state: State<'_, Arc<AppState>>) -> Result<u64, String> {
    prune_volumes_core(&state).await
}

/// App-free core of [`prune_volumes`].
pub(crate) async fn prune_volumes_core(state: &AppState) -> Result<u64, String> {
    let client = state.get_client().await?;
    client.prune_volumes().await.map_err(|e| e.to_string())
}
