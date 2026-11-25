use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tauri::State;

use crate::docker::models::VolumeItem;
use crate::fsutil::{dir_size, DirSize};
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

    // Single flight: the Dashboard and the Volumes page list together, and
    // each used to walk every volume on disk on its own.
    let _sizing = VOLUME_SIZING.lock().await;
    let mountpoints: Vec<String> = volumes.iter().map(|v| v.mountpoint.clone()).collect();
    let sizes = tokio::task::spawn_blocking(move || sized_mountpoints(&mountpoints))
        .await
        .map_err(|e| format!("size calculation task failed: {e}"))?;
    for v in volumes.iter_mut() {
        match sizes.get(&v.mountpoint).copied().flatten() {
            Some(ds) => {
                v.size = Some(ds.bytes);
                v.restricted_dirs = ds.restricted_dirs;
            }
            // Unreadable / non-local mountpoint: leave size unknown.
            None => {
                v.size = None;
                v.restricted_dirs = 0;
            }
        }
    }
    Ok(volumes)
}

/// How long a walked size is reused. Every list used to walk every volume in
/// series — hundreds of volumes (or one big database) made each refresh of
/// the Volumes page and the Dashboard take seconds of disk I/O.
const VOLUME_SIZE_TTL: std::time::Duration = std::time::Duration::from_secs(120);
const VOLUME_WALKERS: usize = 6;

type SizeEntry = (std::time::Instant, Option<DirSize>);
static VOLUME_SIZES: std::sync::Mutex<Option<HashMap<String, SizeEntry>>> =
    std::sync::Mutex::new(None);
static VOLUME_SIZING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Sizes for `mountpoints`: fresh cache entries as they are, stale or new
/// ones walked in parallel. Entries for volumes that no longer exist are
/// dropped. `None` = unreadable mountpoint.
fn sized_mountpoints(mountpoints: &[String]) -> HashMap<String, Option<DirSize>> {
    let mut cache = VOLUME_SIZES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .unwrap_or_default();
    let wanted: std::collections::HashSet<&String> = mountpoints.iter().collect();
    cache.retain(|k, _| wanted.contains(k));
    let stale: Vec<String> = mountpoints
        .iter()
        .filter(|m| {
            cache
                .get(*m)
                .is_none_or(|(ts, _)| ts.elapsed() >= VOLUME_SIZE_TTL)
        })
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    for (m, size) in walk_parallel(&stale) {
        cache.insert(m, (std::time::Instant::now(), size));
    }
    let out = cache.iter().map(|(k, (_, s))| (k.clone(), *s)).collect();
    *VOLUME_SIZES.lock().unwrap_or_else(|e| e.into_inner()) = Some(cache);
    out
}

fn walk_parallel(paths: &[String]) -> Vec<(String, Option<DirSize>)> {
    if paths.is_empty() {
        return Vec::new();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = VOLUME_WALKERS.min(paths.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(p) = paths.get(i) else { break };
                        done.push((p.clone(), dir_size(Path::new(p)).ok()));
                    }
                    done
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
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

#[cfg(test)]
mod size_cache_tests {
    use super::*;

    #[test]
    fn parallel_walk_sizes_every_path_and_marks_missing_ones() {
        let base = std::env::temp_dir().join(format!("hsx-volwalk-{}", std::process::id()));
        let paths: Vec<String> = (0..9)
            .map(|i| {
                let d = base.join(format!("v{i}"));
                std::fs::create_dir_all(&d).unwrap();
                std::fs::write(d.join("f"), vec![0u8; i * 10]).unwrap();
                d.display().to_string()
            })
            .chain(std::iter::once(base.join("missing").display().to_string()))
            .collect();
        let got: HashMap<String, Option<DirSize>> = walk_parallel(&paths).into_iter().collect();
        assert_eq!(got.len(), 10);
        for i in 0..9 {
            assert_eq!(got[&paths[i]].unwrap().bytes, (i * 10) as u64);
        }
        assert!(got[&paths[9]].is_none());
        let _ = std::fs::remove_dir_all(&base);
    }
}
