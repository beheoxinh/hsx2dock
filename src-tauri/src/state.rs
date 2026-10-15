use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::AppConfig;
use crate::docker::client::DockerClient;
use crate::docker::engine::{EngineManager, EnginePaths};
use crate::docker::models::DaemonState;

pub struct AppState {
    pub client: Arc<RwLock<Option<DockerClient>>>,
    pub engine: Arc<EngineManager>,
    pub config: Arc<RwLock<AppConfig>>,
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

        Self {
            client: Arc::new(RwLock::new(client)),
            engine,
            config: Arc::new(RwLock::new(config)),
        }
    }

    pub fn daemon_state(&self) -> DaemonState {
        if !self.engine.is_installed() {
            DaemonState::NotInstalled
        } else if self.engine.is_running() || self.engine.socket_live() {
            DaemonState::Running
        } else {
            DaemonState::Stopped
        }
    }

    pub async fn get_client(&self) -> Result<DockerClient, String> {
        let read_guard = self.client.read().await;
        if let Some(ref c) = *read_guard {
            if c.is_healthy().await {
                return Ok(c.clone());
            }
        }
        drop(read_guard);

        // Try reconnecting to engine socket
        let mut write_guard = self.client.write().await;
        let socket = self.engine.socket();
        match DockerClient::connect(Some(&socket)) {
            Ok(c) => {
                *write_guard = Some(c.clone());
                Ok(c)
            }
            Err(e) => Err(format!("Docker daemon not reachable: {e}")),
        }
    }
}
