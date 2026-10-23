use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

use crate::config::AppConfig;
use crate::docker::client::DockerClient;
use crate::docker::engine::{EngineManager, EnginePaths};
use crate::docker::kind::KindPaths;
use crate::docker::models::DaemonState;

/// One live interactive exec session. `stdin_tx` feeds the container-side
/// writer (dropping it closes stdin); `task` is the output-relay task.
pub struct ExecSession {
    /// The exec id on the engine, kept so resize calls don't need the container.
    pub exec_id: String,
    pub stdin_tx: mpsc::UnboundedSender<Vec<u8>>,
    pub task: tokio::task::JoinHandle<()>,
}

pub struct AppState {
    pub client: Arc<RwLock<Option<DockerClient>>>,
    pub engine: Arc<EngineManager>,
    pub config: Arc<RwLock<AppConfig>>,
    /// One build at a time; the guard also drives the UI's Cancel affordance.
    pub build_running: Arc<std::sync::atomic::AtomicBool>,
    /// Set by `cancel_image_build`; the build stream polls it and aborts.
    pub build_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Live interactive exec sessions (backend→frontend events; stdin flows in
    /// through Tauri commands).
    pub exec_sessions: Arc<RwLock<HashMap<String, ExecSession>>>,
    /// Isolated kind (Kubernetes IN Docker) binary + kubeconfig layout.
    pub kind: KindPaths,
    /// Mirror of `AppConfig::close_to_tray`. The window-close handler runs on
    /// the GTK thread and must decide synchronously, so it reads this instead
    /// of awaiting the config lock.
    pub close_to_tray: Arc<std::sync::atomic::AtomicBool>,
}

impl AppState {
    pub async fn new() -> Self {
        let config = AppConfig::load().await.unwrap_or_default();
        let paths = EnginePaths::with_custom_data_root(Some(&config.data_root));
        let engine = Arc::new(EngineManager::new(paths));
        let socket = engine.socket();
        let client = match DockerClient::connect(Some(&socket)) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("Failed initial connection to Docker: {e}");
                None
            }
        };

        let close_to_tray = config.close_to_tray;

        Self {
            client: Arc::new(RwLock::new(client)),
            engine,
            config: Arc::new(RwLock::new(config)),
            build_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            build_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            exec_sessions: Arc::new(RwLock::new(HashMap::new())),
            kind: KindPaths::for_app(),
            close_to_tray: Arc::new(std::sync::atomic::AtomicBool::new(close_to_tray)),
        }
    }

    pub fn daemon_state(&self) -> DaemonState {
        if !self.engine.is_installed() {
            return DaemonState::Stopped;
        }
        if self.engine.is_running() {
            return DaemonState::Running;
        }
        if self.engine.socket_live() {
            return DaemonState::Running;
        }
        DaemonState::Stopped
    }

    pub async fn get_client(&self) -> Result<DockerClient, String> {
        let read_guard = self.client.read().await;
        if let Some(ref c) = *read_guard {
            if c.is_healthy().await {
                return Ok(c.clone());
            }
        }
        drop(read_guard);

        let active = { self.config.read().await.active_endpoint.clone() };
        let remote = match active.as_deref() {
            None | Some("local") => None,
            Some(id) => {
                let cfg = self.config.read().await;
                cfg.endpoints.iter().find(|e| e.id == id).cloned()
            }
        };

        let mut write_guard = self.client.write().await;
        let client = match remote {
            Some(ep) => DockerClient::connect_for_endpoint(
                &ep.kind,
                &ep.uri,
                Some(&self.engine.socket()),
            )
            .map_err(|e| format!("Endpoint '{}' unreachable: {e}", ep.name))?,
            None => {
                let socket = self.engine.socket();
                DockerClient::connect(Some(&socket))
                    .map_err(|e| format!("Docker daemon not reachable: {e}"))?
            }
        };
        let client = match client.ping().await {
            Ok(_) => client,
            Err(e) => return Err(format!("Docker daemon not reachable: {e}")),
        };
        *write_guard = Some(client.clone());
        Ok(client)
    }

    /// Engine lifecycle commands talk to the daemon this app owns, so they
    /// must not act on a remote endpoint by accident. Returns the error to
    /// surface when one is active.
    pub async fn engine_control_guard(&self) -> Result<(), String> {
        let active = { self.config.read().await.active_endpoint.clone() };
        let id = match active.as_deref() {
            None | Some("local") => return Ok(()),
            Some(id) => id.to_string(),
        };
        let name = {
            let cfg = self.config.read().await;
            cfg.endpoints
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.name.clone())
                .unwrap_or_else(|| id.clone())
        };
        Err(format!(
            "Engine controls are unavailable while the remote endpoint '{name}' is active"
        ))
    }

    /// Drops the cached client so the next `get_client` reconnects to whatever
    /// the active endpoint is now.
    pub fn invalidate_client(&self) {
        if let Ok(mut guard) = self.client.try_write() {
            *guard = None;
        }
    }

    /// Persists the in-memory config to disk.
    pub async fn save_config(&self) -> Result<(), String> {
        let config = self.config.read().await.clone();
        let result = config.save().await.map_err(|e| e.to_string());
        self.set_close_to_tray(config.close_to_tray);
        result
    }

    /// Mirrors the persisted preference into the flag the window-close handler
    /// reads synchronously. Every writer of `config.close_to_tray` routes
    /// through here so the two never disagree.
    pub fn set_close_to_tray(&self, enabled: bool) {
        self.close_to_tray
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_close_to_tray(&self) -> bool {
        self.close_to_tray
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}
