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
    /// The loopback MCP HTTP server, when Settings has it enabled. Holding the
    /// value is what keeps it alive; setting it to `None` stops it.
    pub mcp_server: Arc<RwLock<Option<crate::mcp::http::RunningServer>>>,
    /// Caddy Gateway supervisor for routing HTTP/HTTPS requests to local containers
    pub caddy: Arc<crate::docker::caddy::CaddySupervisor>,
    /// Cached live container stats shared across background monitoring and tray
    pub cached_stats: Arc<RwLock<HashMap<String, crate::docker::models::ContainerStats>>>,
    /// In-flight container/group lifecycle operations (pending spinners).
    pub ops: Arc<crate::ops::OpsRegistry>,
    /// When the cached client last answered a ping. Every command used to
    /// ping before doing its real work; a fresh check is reused for a few
    /// seconds instead, which halves the round trips of a bulk action.
    client_checked: std::sync::Mutex<Option<std::time::Instant>>,
    /// Serialises background restart-policy reconciliation passes.
    pub restart_policy_lock: Arc<tokio::sync::Mutex<()>>,
    /// A reconciliation pass is already queued; further saves just ride it.
    pub restart_policy_pending: Arc<std::sync::atomic::AtomicBool>,
    /// Restart policy last seen/applied per container id, so a group edit
    /// does not inspect every container on the engine again.
    pub restart_policy_seen: Arc<std::sync::Mutex<HashMap<String, String>>>,
}

/// How long a successful ping vouches for the cached client.
const CLIENT_HEALTH_TTL: std::time::Duration = std::time::Duration::from_secs(3);

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
        let caddy = Arc::new(crate::docker::caddy::CaddySupervisor::new());

        Self {
            client: Arc::new(RwLock::new(client)),
            engine,
            config: Arc::new(RwLock::new(config)),
            build_running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            build_cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            exec_sessions: Arc::new(RwLock::new(HashMap::new())),
            kind: KindPaths::for_app(),
            close_to_tray: Arc::new(std::sync::atomic::AtomicBool::new(close_to_tray)),
            mcp_server: Arc::new(RwLock::new(None)),
            caddy,
            cached_stats: Arc::new(RwLock::new(HashMap::new())),
            ops: Arc::new(crate::ops::OpsRegistry::default()),
            client_checked: std::sync::Mutex::new(None),
            restart_policy_lock: Arc::new(tokio::sync::Mutex::new(())),
            restart_policy_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            restart_policy_seen: Arc::new(std::sync::Mutex::new(HashMap::new())),
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
        let cached = self.client.read().await.clone();
        if let Some(c) = cached {
            let fresh = self
                .client_checked
                .lock()
                .ok()
                .and_then(|g| *g)
                .is_some_and(|t| t.elapsed() < CLIENT_HEALTH_TTL);
            if fresh {
                return Ok(c);
            }
            if c.is_healthy().await {
                self.mark_client_checked();
                return Ok(c);
            }
        }
        if let Ok(mut g) = self.client_checked.lock() {
            *g = None;
        }

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
            Some(ep) => {
                DockerClient::connect_for_endpoint(&ep.kind, &ep.uri, Some(&self.engine.socket()))
                    .map_err(|e| format!("Endpoint '{}' unreachable: {e}", ep.name))?
            }
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
        drop(write_guard);
        self.mark_client_checked();
        Ok(client)
    }

    fn mark_client_checked(&self) {
        if let Ok(mut g) = self.client_checked.lock() {
            *g = Some(std::time::Instant::now());
        }
    }

    /// Stats for one container: the background monitor's sample when it has
    /// one (already correct and free), otherwise a fresh two-cycle sample.
    pub async fn container_stats(
        &self,
        client: &DockerClient,
        id: &str,
    ) -> anyhow::Result<crate::docker::models::ContainerStats> {
        {
            let cache = self.cached_stats.read().await;
            if let Some(s) = cache.get(id) {
                return Ok(s.clone());
            }
            if id.len() >= 12 {
                if let Some((_, s)) = cache.iter().find(|(k, _)| k.starts_with(id)) {
                    return Ok(s.clone());
                }
            }
        }
        client.container_stats(id).await
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
        if let Ok(mut g) = self.client_checked.lock() {
            *g = None;
        }
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
