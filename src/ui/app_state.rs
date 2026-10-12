use std::cell::{Cell, RefCell};
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
use crate::docker::engine::EngineManager;
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
    /// Engine download/install progress: label + optional 0.0–1.0 fraction.
    EngineProgress(String, Option<f64>),
    /// Engine install state may have changed (engine downloaded/removed):
    /// drop the cached probe and re-run daemon detection on the main loop.
    EngineChanged,
}

/// Shared application state: config, daemon manager, docker client factory and
/// the event bus that bridges Tokio workers to the GTK main loop.
pub struct AppState {
    pub config: RefCell<AppConfig>,
    pub engine: EngineManager,
    client: RefCell<Option<DockerClient>>,
    /// Cached `docker` CLI presence probe + timestamp (monotonic ms).
    cli_probe: RefCell<(bool, u64)>,
    /// Monotonic ms when the last explicit start was requested (Starting grace).
    starting_since: RefCell<Option<u64>>,
    /// Monotonic ms when the last explicit stop was requested (Stopping grace).
    stopping_since: RefCell<Option<u64>>,
    /// True while a bundled-engine download/install is in flight.
    installing: Cell<bool>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
}

impl AppState {
    pub fn new(config: AppConfig) -> Rc<Self> {
        let (tx, rx) = async_channel::unbounded();
        let engine = EngineManager::new(crate::docker::engine::EnginePaths::new());
        Rc::new(Self {
            config: RefCell::new(config),
            engine,
            client: RefCell::new(None),
            cli_probe: RefCell::new((false, 0)),
            starting_since: RefCell::new(None),
            stopping_since: RefCell::new(None),
            installing: Cell::new(false),
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
        if self.installing.get() {
            return DaemonState::Installing;
        }
        if !self.engine.is_installed() {
            return DaemonState::NotInstalled;
        }
        let socket_state = crate::docker::daemon::state_from_socket(self.engine.socket());
        if matches!(socket_state, DaemonState::Stopped) {
            *self.stopping_since.borrow_mut() = None;
        }
        if matches!(socket_state, DaemonState::Running) {
            *self.starting_since.borrow_mut() = None;
            return socket_state;
        }
        if self.start_in_grace_period() {
            return DaemonState::Starting;
        }
        if self.stop_in_grace_period() && !matches!(socket_state, DaemonState::Stopped) {
            return DaemonState::Stopping;
        }
        socket_state
    }

    /// Whether a stop was requested within the last 30s.
    fn stop_in_grace_period(&self) -> bool {
        match *self.stopping_since.borrow() {
            Some(at) => now_ms().saturating_sub(at) < 30_000,
            None => false,
        }
    }

    /// Mark a bundled-engine download/install as in flight (or finished).
    pub fn set_installing(&self, installing: bool) {
        self.installing.set(installing);
    }

    /// Whether the bundled engine is downloaded and ready to run.
    fn cli_available_cached(&self) -> bool {
        let now = now_ms();
        let (cached, at) = *self.cli_probe.borrow();
        if now.saturating_sub(at) < 2_000 && at != 0 {
            return cached;
        }
        let available = self.engine.is_installed();
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
        *self.stopping_since.borrow_mut() = None;
    }

    /// Record an explicit stop request, clearing any pending start grace.
    fn mark_stop_requested(&self) {
        *self.stopping_since.borrow_mut() = Some(now_ms());
        *self.starting_since.borrow_mut() = None;
    }

    /// Active unix socket path when the daemon is reachable.
    pub fn active_socket(&self) -> Option<PathBuf> {
        let p = self.engine.socket().to_path_buf();
        if crate::docker::daemon::is_usable_socket(&p) {
            Some(p)
        } else {
            None
        }
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
        *self.config.borrow_mut() = config;
        self.reset_client();
        self.refresh_daemon();
    }

    /// Mutate the stored config in place and persist it in the background.
    ///
    /// Unlike [`apply_config`](Self::apply_config) this does not rebuild the
    /// daemon manager or re-poll — use it for flags that only affect UI-side
    /// behaviour (e.g. the Docker CLI bridge toggle).
    pub fn update_config(&self, f: impl FnOnce(&mut AppConfig)) {
        let snapshot = {
            let mut cfg = self.config.borrow_mut();
            f(&mut cfg);
            cfg.clone()
        };
        spawn(async move {
            if let Err(e) = snapshot.save().await {
                tracing::warn!("failed to save config: {e}");
            }
        });
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
        let engine = self.engine.clone();
        self.notify(AppEvent::DaemonState(DaemonState::Starting));
        spawn(async move {
            let result = crate::runtime::engine_op(move || engine.start())
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("engine op: {e}")));
            match result {
                Ok(()) => {
                    let _ = tx.send(AppEvent::DaemonState(DaemonState::Running)).await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AppEvent::Error(format!("engine autostart failed: {e}")))
                        .await;
                    let _ = tx.send(AppEvent::DaemonState(DaemonState::Stopped)).await;
                }
            }
        });
    }

    /// Start or stop the bundled engine.
    pub fn toggle_daemon(&self) {
        tracing::info!("toggle_daemon called");
        let tx = self.sender();
        if !self.engine.is_installed() {
            self.notify(AppEvent::Toast(
                "Engine is not downloaded yet — see the Engine page".into(),
            ));
            return;
        }
        // Branch on engine process + socket liveness rather than guessing.
        let start = !self.engine.is_running() && self.active_socket().is_none();
        if start {
            if !self.cli_available_cached() {
                self.notify(AppEvent::DaemonState(DaemonState::NotInstalled));
                self.notify(AppEvent::Toast(
                    "Docker CLI is not installed — see the Engine page guide".into(),
                ));
                return;
            }
            self.mark_start_requested();
        } else {
            self.mark_stop_requested();
        }
        self.notify(AppEvent::DaemonState(if start {
            DaemonState::Starting
        } else {
            DaemonState::Stopping
        }));
        let engine = self.engine.clone();
        spawn(async move {
            let runner = engine.clone();
            let result =
                crate::runtime::engine_op(
                    move || {
                        if start {
                            runner.start()
                        } else {
                            runner.stop()
                        }
                    },
                )
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("engine op: {e}")));
            let msg = match result {
                Ok(()) if start => "Engine started".to_string(),
                Ok(()) => "Engine stopped".to_string(),
                Err(e) => format!("engine {e}"),
            };
            let probe = engine.clone();
            let live = tokio::task::spawn_blocking(move || {
                crate::docker::daemon::is_usable_socket(probe.socket())
            })
            .await
            .unwrap_or(false);
            if start && !live {
                let _ = tx
                    .send(AppEvent::DaemonState(DaemonState::Error(
                        "Engine did not come up — see the Engine page log".into(),
                    )))
                    .await;
            } else {
                let _ = tx
                    .send(AppEvent::DaemonState(
                        crate::docker::daemon::state_from_socket(engine.socket()),
                    ))
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
            self.error("Engine is not running");
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
            None => self.toast("Engine is not running"),
        }
    }

    /// True when a cached client (or a connectable socket) exists.
    pub fn has_client(&self) -> bool {
        self.client().is_some()
    }
}
