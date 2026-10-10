use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Monotonic-ish wall clock in ms (sufficient for short grace windows).
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

use async_channel::{Receiver, Sender};

use crate::config::AppConfig;
use crate::docker::client::DockerClient;
use crate::docker::daemon::DaemonManager;
use crate::docker::models::{
    ContainerItem, ContainerStats, DaemonState, DfReport, EngineInfo, ImageItem, NetworkItem,
    VolumeItem,
};
use crate::runtime::spawn;

/// Scope of a [`AppEvent::Busy`] completion: only the affected view reloads
/// instead of refreshing every tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshScope {
    Daemon,
    Containers,
    Images,
    Volumes,
    Networks,
    Compose,
    All,
}

/// Events flowing from background Tokio tasks back to the GTK main loop.
#[derive(Debug, Clone)]
pub enum AppEvent {
    DaemonState(DaemonState),
    EngineInfo(Box<EngineInfo>),
    DiskUsage(Box<DfReport>),
    Containers(Vec<ContainerItem>),
    ContainerStats(String, ContainerStats),
    Images(Vec<ImageItem>),
    Volumes(Vec<VolumeItem>),
    Networks(Vec<NetworkItem>),
    PullDone(Result<(), String>),
    Toast(String),
    Error(String),
    Busy(RefreshScope),
    /// Engine install state may have changed (CLI installed/removed):
    /// drop the cached CLI probe and re-run daemon detection on the main loop.
    EngineChanged,
}

/// Shared application state: config, daemon manager, docker client factory and
/// the event bus that bridges Tokio workers to the GTK main loop.
pub struct AppState {
    pub config: RefCell<AppConfig>,
    pub daemon: RefCell<DaemonManager>,
    client: RefCell<Option<DockerClient>>,
    /// Cached `docker` CLI presence probe + timestamp (monotonic ms).
    cli_probe: RefCell<(bool, u64)>,
    /// Monotonic ms when the last explicit start was requested (Starting grace).
    starting_since: RefCell<Option<u64>>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Rc<Self> {
        let (tx, rx) = async_channel::unbounded();
        let daemon = DaemonManager::new(config.clone());
        Rc::new(Self {
            config: RefCell::new(config),
            daemon: RefCell::new(daemon),
            client: RefCell::new(None),
            cli_probe: RefCell::new((false, 0)),
            starting_since: RefCell::new(None),
            tx,
            rx,
        })
    }

    pub fn sender(&self) -> Sender<AppEvent> {
        self.tx.clone()
    }

    pub fn receiver(&self) -> Receiver<AppEvent> {
        self.rx.clone()
    }

    /// Current daemon state, derived from socket presence.
    ///
    /// When the `docker` CLI is absent the engine cannot be managed locally,
    /// so `NotInstalled` wins over socket probing. The CLI check is cached
    /// for ~30s so the 2s poll loop never spawns a process per tick.
    /// A recent explicit start (≤30s) keeps `Starting` until the socket/API
    /// proves `Running`, so the UI never flickers to Stopped mid-start.
    pub fn daemon_state(&self) -> DaemonState {
        if !self.cli_available_cached() {
            return DaemonState::NotInstalled;
        }
        let socket_state = self.daemon.borrow().state_from_socket();
        if matches!(socket_state, DaemonState::Running) {
            *self.starting_since.borrow_mut() = None;
            return socket_state;
        }
        if self.start_in_grace_period() {
            return DaemonState::Starting;
        }
        socket_state
    }

    /// Cheap cached `docker` CLI presence check (TTL ~30s).
    fn cli_available_cached(&self) -> bool {        let now = now_ms();
        let (cached, at) = *self.cli_probe.borrow();
        if now.saturating_sub(at) < 30_000 && at != 0 {
            return cached;
        }
        let available = crate::docker::install::cli_available();
        *self.cli_probe.borrow_mut() = (available, now);
        available
    }

    /// Whether a start was requested within the last 30s.
    fn start_in_grace_period(&self) -> bool {
        match *self.starting_since.borrow() {
            Some(t) => now_ms().saturating_sub(t) < 30_000,
            None => false,
        }
    }

    /// Drop the cached `docker` CLI probe so the next `daemon_state()` call
    /// re-probes the real CLI. Called on the GTK main loop after an
    /// (un)install completes (`AppEvent::EngineChanged`).
    pub fn invalidate_cli_cache(&self) {
        *self.cli_probe.borrow_mut() = (false, 0);
    }

    /// Record an explicit start request (arms the Starting grace window).
    fn mark_start_requested(&self) {
        *self.starting_since.borrow_mut() = Some(now_ms());
    }

    /// Active unix socket path when the daemon is reachable.
    pub fn active_socket(&self) -> Option<PathBuf> {
        self.daemon.borrow().active_socket()
    }

    /// Create (and cache) a client bound to the currently active socket.
    pub fn client(&self) -> Option<DockerClient> {
        let socket = self.active_socket()?;
        let mut guard = self.client.borrow_mut();
        if guard.is_none() {
            *guard = DockerClient::connect(Some(socket.as_path())).ok();
        }
        guard.clone()
    }

    /// Drop the cached client (e.g. after the daemon socket changed).
    pub fn reset_client(&self) {
        *self.client.borrow_mut() = None;
    }

    /// Apply a new config live: replace the stored config, rebuild the daemon
    /// manager (socket candidates derive from it), drop the cached client and
    /// re-poll daemon state so the UI reflects the change immediately.
    pub fn apply_config(&self, config: AppConfig) {
        *self.config.borrow_mut() = config.clone();
        *self.daemon.borrow_mut() = DaemonManager::new(config);
        self.reset_client();
        self.refresh_daemon();
    }

    pub fn notify(&self, event: AppEvent) {
        let _ = self.tx.send_blocking(event);
    }

    pub fn toast(&self, message: impl Into<String>) {
        self.notify(AppEvent::Toast(message.into()));
    }

    pub fn error(&self, message: impl Into<String>) {
        self.notify(AppEvent::Error(message.into()));
    }

    /// Refresh daemon state + engine info in the background.
    ///
    /// The socket existing is not enough: a stale socket file reports a dead
    /// daemon as running. Ping the API first and only report `Running` when
    /// the daemon actually answers; otherwise fall back to `Stopped` so the
    /// UI never claims a live connection it does not have.
    pub fn refresh_daemon(&self) {
        let tx = self.sender();
        let state = self.daemon_state();
        if matches!(state, DaemonState::NotInstalled) {
            let _ = tx.send_blocking(AppEvent::DaemonState(state));
            return;
        }
        if matches!(state, DaemonState::Starting) {
            // Within the start grace window the socket may not exist yet;
            // keep the transient badge without spamming error toasts.
            let _ = tx.send_blocking(AppEvent::DaemonState(state));
            return;
        }
        if !matches!(state, DaemonState::Running) {
            let _ = tx.send_blocking(AppEvent::DaemonState(state));
            return;
        }
        self.reset_client();
        let Some(client) = self.client() else {
            let _ = tx.send_blocking(AppEvent::DaemonState(DaemonState::Stopped));
            self.error("Docker socket detected but the API is unreachable");
            return;
        };
        spawn(async move {
            if !client.is_healthy().await {
                let _ = tx.send(AppEvent::DaemonState(DaemonState::Stopped)).await;
                let _ = tx
                    .send(AppEvent::Error(
                        "Docker socket detected but the API is unreachable".into(),
                    ))
                    .await;
                return;
            }
            match client.ping().await {
                Ok(()) => {
                    let _ = tx.send(AppEvent::DaemonState(DaemonState::Running)).await;
                    match client.engine_info().await {
                        Ok(info) => {
                            let _ = tx.send(AppEvent::EngineInfo(Box::new(info))).await;
                            match client.system_df().await {
                                Ok(df) => {
                                    let _ = tx.send(AppEvent::DiskUsage(Box::new(df))).await;
                                }
                                Err(e) => {
                                    let _ =
                                        tx.send(AppEvent::Error(format!("disk usage: {e}"))).await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(AppEvent::Error(format!("engine info: {e}"))).await;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::DaemonState(DaemonState::Stopped)).await;
                    let _ = tx
                        .send(AppEvent::Error(format!("daemon ping failed: {e}")))
                        .await;
                }
            }
        });
    }

    /// Auto-start the daemon unit once when enabled and no socket is live.
    pub fn ensure_daemon_autostart(&self) {
        if !self.config.borrow().auto_start_daemon {
            return;
        }
        if self.active_socket().is_some() {
            return;
        }
        if !self.cli_available_cached() {
            self.notify(AppEvent::DaemonState(DaemonState::NotInstalled));
            return;
        }
        self.mark_start_requested();
        let tx = self.sender();
        let daemon = self.daemon.borrow().clone();
        self.notify(AppEvent::DaemonState(DaemonState::Starting));
        spawn(async move {
            match daemon.try_start_unit() {
                Ok(true) => {
                    let _ = tx
                        .send(AppEvent::DaemonState(daemon.state_from_socket()))
                        .await;
                }
                Ok(false) => {
                    let _ = tx
                        .send(AppEvent::Error(
                            "No systemd unit configured — start Docker manually".into(),
                        ))
                        .await;
                    let _ = tx
                        .send(AppEvent::DaemonState(daemon.state_from_socket()))
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppEvent::Error(format!("autostart failed: {e}")))
                        .await;
                    let _ = tx
                        .send(AppEvent::DaemonState(daemon.state_from_socket()))
                        .await;
                }
            }
        });
    }

    /// Start or stop the configured systemd user unit.
    pub fn toggle_daemon(&self) {
        let tx = self.sender();
        let unit = self.config.borrow().daemon_unit.clone();
        if unit.trim().is_empty() {
            self.notify(AppEvent::Toast(
                "No systemd unit configured — set one in Preferences".into(),
            ));
            return;
        }
        // Branch on the live systemd unit state rather than socket presence:
        // a stale socket file can otherwise invert start/stop.
        let start = !matches!(self.daemon.borrow().is_unit_active(), Ok(true));
        if start {
            if !self.cli_available_cached() {
                self.notify(AppEvent::DaemonState(DaemonState::NotInstalled));
                self.notify(AppEvent::Toast(
                    "Docker CLI is not installed — see the Engine page guide".into(),
                ));
                return;
            }
            self.mark_start_requested();
        }
        self.notify(AppEvent::DaemonState(if start {
            DaemonState::Starting
        } else {
            DaemonState::Stopped
        }));
        let daemon = self.daemon.borrow().clone();
        spawn(async move {
            // systemctl blocks: run it on a blocking thread so neither the GTK
            // main loop nor the Tokio async workers stall.
            let result = tokio::task::spawn_blocking(move || {
                if start {
                    daemon.try_start_unit()
                } else {
                    daemon.try_stop_unit()
                }
            })
            .await
            .unwrap_or_else(|e| Err(std::io::Error::other(format!("task join: {e}"))));
            let msg = match result {
                Ok(true) => {
                    if start {
                        format!("Started {unit}")
                    } else {
                        format!("Stopped {unit}")
                    }
                }
                Ok(false) => "No systemd unit configured — start Docker manually".to_string(),
                Err(e) => format!("systemctl failed: {e}"),
            };
            let socket_live = tokio::task::spawn_blocking(|| {
                DaemonManager::new(crate::config::AppConfig::default()).active_socket()
            })
            .await
            .unwrap_or(None)
            .is_some();
            if start && !socket_live {
                let _ = tx
                    .send(AppEvent::DaemonState(DaemonState::Error(
                        "Docker did not come up within 30s — check `systemctl status docker.service`".into(),
                    )))
                    .await;
            }
            let _ = tx.send(AppEvent::Toast(msg)).await;
            let _ = tx.send(AppEvent::Busy(RefreshScope::Daemon)).await;
        });
    }

    /// Run `f` on the Tokio runtime with a connected client.
    ///
    /// The closure receives the client and the event sender; it must return the
    /// future to execute. When the daemon is down the closure is never called
    /// and an error event is emitted instead.
    pub fn with_client<F, Fut>(&self, f: F)
    where
        F: FnOnce(DockerClient, Sender<AppEvent>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        if !self.has_client() {
            self.error("Docker daemon is not running");
            return;
        }
        self.with_client_or_toast(f);
    }

    /// Run `f` on the Tokio runtime, toasting on failure instead of erroring.
    ///
    /// Unlike [`with_client`](Self::with_client), a missing daemon produces a
    /// transient toast (no error styling) — use this for user-initiated action
    /// buttons where a quiet heads-up beats an error banner.
    pub fn with_client_or_toast<F, Fut>(&self, f: F)
    where
        F: FnOnce(DockerClient, Sender<AppEvent>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let tx = self.sender();
        match self.client() {
            Some(client) => spawn(async move { f(client, tx).await }),
            None => self.toast("Docker daemon is not running"),
        }
    }

    /// True when a cached client (or a connectable socket) exists.
    pub fn has_client(&self) -> bool {
        self.client().is_some()
    }
}
