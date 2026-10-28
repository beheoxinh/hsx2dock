use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_auto_start() -> bool {
    true
}

/// Bridge flags default to **on**: a fresh install wires the system `docker`
/// CLI to the app's own engine without any extra step.
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_auto_start")]
    pub auto_start_daemon: bool,
    #[serde(default)]
    pub compose_file: String,
    /// Create/repair `$XDG_RUNTIME_DIR/docker.sock -> engine socket` (and the
    /// `hsx2dock-engine` context) so the system `docker` CLI reaches our
    /// engine. On by default: it is the feature this app exists for.
    #[serde(default = "default_true")]
    pub cli_bridge_enabled: bool,
    /// Also run `docker context use hsx2dock-engine` so plain `docker`
    /// works without `--context`. Ignored unless `cli_bridge_enabled`.
    /// On by default, so a fresh install "just works".
    #[serde(default = "default_true")]
    pub cli_bridge_set_default: bool,
    /// Context the CLI was using *before* the bridge took over as default.
    /// Restored on teardown, so we never clobber the user's own context
    /// (e.g. `desktop-linux`) by blindly falling back to `default`.
    #[serde(default)]
    pub cli_bridge_prev_context: String,
    /// Custom storage directory for containers, images, and volumes.
    /// If empty, defaults to `$XDG_DATA_HOME/hsx2dock/data`.
    #[serde(default)]
    pub data_root: String,
    /// UI Theme preference: "system" | "dark" | "light"
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Closing the window hides it to the tray instead of quitting. The app
    /// (and the engine it supervises) keeps running in the background; only
    /// the tray's Quit exits. On by default: a desktop Docker manager is
    /// expected to live in the tray like Docker Desktop does.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// Register an XDG autostart entry so the app (with its engine) comes up
    /// at login. Off by default — enabling it is an explicit opt-in.
    #[serde(default)]
    pub auto_start_on_boot: bool,
    /// Launched by the autostart entry, start hidden in the tray instead of
    /// showing the window. Ignored unless `auto_start_on_boot` is on.
    #[serde(default)]
    pub start_minimized: bool,
    /// Which docker CLI the app runs: "auto" | "system" | "bundled".
    ///
    /// `auto` prefers the newer of the system and bundled binaries, so a
    /// machine with no Docker install still works and a machine with a newer
    /// Docker keeps using it.
    #[serde(default = "default_cli_mode")]
    pub cli_mode: String,
    /// Install `~/.local/bin/docker` pointing at the resolved CLI, so `docker`
    /// works in a terminal even when nothing was installed system-wide.
    #[serde(default)]
    pub cli_shim_enabled: bool,
    /// Bumped when a default changes in a way old files must be migrated onto.
    /// `0` = written before the CLI bridge became on-by-default.
    #[serde(default)]
    pub schema_version: u32,
    /// Saved remote daemons. The local bundled engine is implicit and is
    /// never stored here.
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
    /// Id of the active endpoint. `None` = the local bundled engine.
    #[serde(default)]
    pub active_endpoint: Option<String>,
    /// Shell opened by the container terminal and the Debug action.
    #[serde(default = "default_shell")]
    pub terminal_shell: String,
    /// Font size (px) used by terminal panes.
    #[serde(default = "default_terminal_font_size")]
    pub terminal_font_size: u32,
    /// Saved registry logins (Docker Hub, ghcr.io, GitLab, Harbor, custom).
    #[serde(default)]
    pub registries: Vec<RegistryConfig>,
    /// Id of the login new pushes/pulls default to. Empty = the first saved
    /// login, preserving the pre-selection behaviour.
    #[serde(default)]
    pub default_registry: String,
    /// Desktop-notification opt-outs, driven by the tray's Notifications submenu.
    #[serde(default)]
    pub notifications: NotificationPrefs,
    /// Default node count pre-filled in the kind "Create cluster" form (1–5).
    #[serde(default = "default_kind_nodes")]
    pub kind_default_nodes: u32,
    /// Default node image for kind clusters; empty = kind's bundled default.
    #[serde(default)]
    pub kind_default_image: String,
    /// User-defined container groups: bulk start/stop with an optional
    /// auto-start-on-app-open flag (Docker Desktop-style groups).
    #[serde(default)]
    pub container_groups: Vec<ContainerGroup>,
    /// Engine memory cap in MiB applied to the engine's cgroup on start.
    /// `0` = unlimited. Keeps the bundled daemon from starving the host.
    #[serde(default)]
    pub engine_memory_limit_mb: u64,
    /// Engine CPU cap in cores (e.g. `2.5`) applied with the memory cap.
    /// Empty or `0` = unlimited.
    #[serde(default)]
    pub engine_cpu_cores: String,
    /// Master switch for auto-starting kind clusters with the engine.
    #[serde(default)]
    pub kind_auto_start: bool,
    /// Seconds to wait between cluster starts so several clusters don't
    /// hammer the host at once. `0` = back-to-back.
    #[serde(default = "default_kind_start_delay")]
    pub kind_auto_start_delay: u32,
    /// Per-cluster prefs (auto-start flag, per-node resource caps), keyed by
    /// cluster name. Clusters without an entry use the defaults (manual
    /// start, unlimited nodes).
    #[serde(default)]
    pub kind_cluster_prefs: std::collections::BTreeMap<String, KindClusterPrefs>,
    // ---- MCP (Model Context Protocol) ----
    /// Serve the app to AI agents over loopback HTTP. Off by default: the
    /// stdio binary needs no setting, and this one opens a listening socket.
    #[serde(default)]
    pub mcp_enabled: bool,
    /// Port for the loopback HTTP transport. `0` = pick a free port on start.
    #[serde(default = "default_mcp_port")]
    pub mcp_port: u16,
    /// Automatically prune dangling / untagged images when viewing the Images page or on background tick.
    #[serde(default)]
    pub auto_prune_dangling_images: bool,
    /// Automatically prune isolated / unused volumes when viewing the Volumes page.
    #[serde(default)]
    pub auto_prune_isolated_volumes: bool,
    // ---- Caddy Gateway Preferences ----
    #[serde(default)]
    pub gateway_enabled: bool,
    #[serde(default)]
    pub gateway_auto_start: bool,
    #[serde(default = "default_gateway_http_port")]
    pub gateway_http_port: u16,
    #[serde(default = "default_gateway_https_port")]
    pub gateway_https_port: u16,
    #[serde(default)]
    pub gateway_routes: Vec<crate::docker::caddy::GatewayRoute>,
}

fn default_gateway_http_port() -> u16 {
    80
}

fn default_gateway_https_port() -> u16 {
    443
}

fn default_kind_start_delay() -> u32 {
    5
}

/// Deliberately obscure: it only has to beat other dev servers on the machine,
/// and a stable number means a saved agent config survives an app restart.
pub(crate) fn default_mcp_port() -> u16 {
    45123
}

/// Per-cluster preferences persisted in the app config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct KindClusterPrefs {
    /// Start this cluster together with the engine (staggered, capped).
    #[serde(default)]
    pub auto_start: bool,
    /// Per-node CPU cap in millicores (`0`/unset = unlimited).
    #[serde(default)]
    pub node_cpu_m: u32,
    /// Per-node memory cap in MiB (`0`/unset = unlimited).
    #[serde(default)]
    pub node_memory_mb: u32,
}

/// A named set of containers that can be started/stopped together, optionally
/// auto-started when the app (and its engine) comes up.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContainerGroup {
    /// Stable id used by the UI and auto-start; generated on save.
    pub id: String,
    /// Display name, unique among groups.
    pub name: String,
    /// Member container ids at save time. Members that no longer exist are
    /// skipped at start/stop time rather than failing the whole group.
    #[serde(default)]
    pub container_ids: Vec<String>,
    /// Start this group automatically when the engine comes up.
    #[serde(default)]
    pub auto_start: bool,
}

/// Which tray-driven desktop notifications to surface. Every category defaults
/// to on: a fresh install should tell the user when long-running work ends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPrefs {
    /// Image build finished (success or failure).
    #[serde(default = "default_true")]
    pub build_done: bool,
    /// Pull/push transfer finished.
    #[serde(default = "default_true")]
    pub transfer_done: bool,
    /// Engine started, stopped, or died.
    #[serde(default = "default_true")]
    pub engine_state: bool,
    /// Kind cluster created or deleted.
    #[serde(default = "default_true")]
    pub kind_events: bool,
    /// The window was closed into the tray (the app and engine keep running).
    #[serde(default = "default_true")]
    pub tray_hidden: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            build_done: default_true(),
            transfer_done: default_true(),
            engine_state: default_true(),
            kind_events: default_true(),
            tray_hidden: default_true(),
        }
    }
}

/// A saved registry login. Any host works: Docker Hub, ghcr.io,
/// registry.gitlab.com, a personal Harbor, or a community registry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegistryConfig {
    /// Stable id used by the UI; generated on save.
    pub id: String,
    /// Display name, e.g. "GitHub ghcr".
    pub name: String,
    /// Registry host, e.g. `ghcr.io`, `harbor.example.com:8443`.
    pub host: String,
    pub username: String,
    /// base64(username:password) — the same obfuscation docker's own
    /// `config.json` uses. Not encryption; the config file is user-private.
    pub auth: String,
    /// Accept self-signed TLS certs (personal Harbor installs).
    #[serde(default)]
    pub insecure: bool,
}

impl RegistryConfig {
    /// Builds the bollard credential for this login. `auth` holds
    /// base64(username:password).
    pub fn credentials(&self) -> bollard::auth::DockerCredentials {
        use base64::Engine as _;
        let password = base64::engine::general_purpose::STANDARD
            .decode(self.auth.as_bytes())
            .ok()
            .and_then(|raw| {
                let s = String::from_utf8(raw).ok()?;
                let (_, pass) = s.split_once(':')?;
                Some(pass.to_string())
            })
            .unwrap_or_default();
        bollard::auth::DockerCredentials {
            username: Some(self.username.clone()),
            password: Some(password),
            serveraddress: Some(server_address_for_host(&self.host)),
            ..Default::default()
        }
    }
}

/// Docker's canonical server address for an index-style host. The daemon
/// matches saved auth against this string for Docker Hub pulls/pushes.
pub fn server_address_for_host(host: &str) -> String {
    if matches!(
        host,
        "docker.io" | "index.docker.io" | "registry-1.docker.io"
    ) {
        "https://index.docker.io/v1/".to_string()
    } else {
        format!("https://{host}")
    }
}

/// A saved remote daemon (tcp or ssh). The local engine needs no entry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EndpointConfig {
    /// Stable id used by the UI; generated on save.
    pub id: String,
    pub name: String,
    /// `"tcp"` or `"ssh"`.
    pub kind: String,
    /// `host:2375`, `tcp://host:2375`, `user@host`, `ssh://user@host`.
    pub uri: String,
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_cli_mode() -> String {
    "auto".to_string()
}

fn default_shell() -> String {
    "sh".to_string()
}

fn default_terminal_font_size() -> u32 {
    12
}

fn default_kind_nodes() -> u32 {
    1
}

/// Current config schema. See [`AppConfig::schema_version`].
pub const CONFIG_SCHEMA_VERSION: u32 = 3;

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_start_daemon: default_auto_start(),
            compose_file: String::new(),
            cli_bridge_enabled: default_true(),
            cli_bridge_set_default: default_true(),
            cli_bridge_prev_context: String::new(),
            data_root: String::new(),
            theme: default_theme(),
            close_to_tray: true,
            auto_start_on_boot: false,
            start_minimized: false,
            cli_mode: default_cli_mode(),
            cli_shim_enabled: false,
            schema_version: CONFIG_SCHEMA_VERSION,
            endpoints: Vec::new(),
            active_endpoint: None,
            terminal_shell: default_shell(),
            terminal_font_size: default_terminal_font_size(),
            registries: Vec::new(),
            default_registry: String::new(),
            notifications: NotificationPrefs::default(),
            kind_default_nodes: default_kind_nodes(),
            kind_default_image: String::new(),
            container_groups: Vec::new(),
            engine_memory_limit_mb: 0,
            engine_cpu_cores: String::new(),
            kind_auto_start: false,
            kind_auto_start_delay: default_kind_start_delay(),
            kind_cluster_prefs: Default::default(),
            mcp_enabled: false,
            mcp_port: default_mcp_port(),
            auto_prune_dangling_images: false,
            auto_prune_isolated_volumes: false,
            gateway_enabled: false,
            gateway_auto_start: false,
            gateway_http_port: default_gateway_http_port(),
            gateway_https_port: default_gateway_https_port(),
            gateway_routes: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("hsx2dock"))
    }

    pub fn config_path() -> Option<PathBuf> {
        Self::config_dir().map(|d| d.join("config.toml"))
    }

    /// Engine CPU cap in cores; empty or `0` is unlimited (reported as 0.0).
    pub fn engine_cpu_limit(&self) -> Result<f64, String> {
        let raw = self.engine_cpu_cores.trim();
        if raw.is_empty() {
            return Ok(0.0);
        }
        let cores: f64 = raw
            .parse()
            .map_err(|_| format!("Invalid engine CPU limit '{raw}' (expected e.g. 2.5)"))?;
        if !cores.is_finite() || cores < 0.0 {
            return Err(format!("Invalid engine CPU limit '{raw}'"));
        }
        Ok(cores)
    }

    pub async fn load() -> anyhow::Result<Self> {
        if let Some(path) = Self::config_path() {
            if path.exists() {
                let (cfg, stale) = Self::load_and_migrate(&path).await?;
                // Persist a one-time migration so it does not re-run and so
                // the file is not misleadingly still showing the old values.
                if stale {
                    let _ = cfg.save().await;
                }
                return Ok(cfg);
            }
        }
        Ok(Self::default())
    }

    pub async fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        Ok(Self::load_and_migrate(path).await?.0)
    }

    /// Reads a config file and upgrades it, also reporting whether the file
    /// predated the current schema. The caller needs that flag *before* the
    /// migration is applied — checking `schema_version` afterwards would always
    /// be false, which is why the upgrade was never persisted before.
    async fn load_and_migrate(path: &std::path::Path) -> anyhow::Result<(Self, bool)> {
        let content = tokio::fs::read_to_string(path).await?;
        let mut cfg: AppConfig = toml::from_str(&content)?;
        let stale = cfg.schema_version < CONFIG_SCHEMA_VERSION;
        cfg.migrate();
        Ok((cfg, stale))
    }

    /// One-time upgrades for files written by older versions. Idempotent: only
    /// acts when the file predates the flag it changes.
    fn migrate(&mut self) {
        if self.schema_version < CONFIG_SCHEMA_VERSION {
            // Before this schema the CLI bridge shipped off-by-default. A user
            // who never touched it has `false` stored, which we now flip on —
            // the app's whole point is wiring `docker` to its engine.
            self.cli_bridge_enabled = true;
            self.cli_bridge_set_default = true;
            // Schema 2 makes close-to-tray the default. Files written by the
            // first release lack the key (already flipped by the serde default)
            // or stored `false` from the toggle — both move to the new default
            // so the installed app behaves like a fresh one.
            self.close_to_tray = true;
            // Schema 3 adds the MCP settings. They default to "off, no token",
            // so nothing needs rewriting here — bumping the version is what
            // makes the app rewrite the file with the new keys.
            self.schema_version = CONFIG_SCHEMA_VERSION;
        }
    }

    /// Drops settings that only mean something together. "Start minimized" is
    /// driven by the autostart entry, so it is meaningless (and misleading in
    /// the UI) once "Auto startup on boot" is off.
    pub fn normalize(&mut self) {
        if !self.auto_start_on_boot {
            self.start_minimized = false;
        }
        // A zero port means "pick one at start"; anything else must be a real
        // port, and the server only ever binds it on loopback.
        if self.mcp_port == 0 {
            self.mcp_port = default_mcp_port();
        }
    }

    pub async fn save(&self) -> anyhow::Result<()> {
        if let Some(dir) = Self::config_dir() {
            tokio::fs::create_dir_all(&dir).await?;
            if let Some(path) = Self::config_path() {
                self.save_to(&path).await?;
            }
        }
        Ok(())
    }

    pub async fn save_to(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let content = toml::to_string_pretty(self)?;
        tokio::fs::write(path, content).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_ship_ready_to_start_daemon() {
        let cfg = AppConfig::default();
        assert!(cfg.auto_start_daemon);
        assert!(cfg.compose_file.is_empty());
    }

    #[test]
    fn desktop_integration_defaults_and_normalization() {
        let cfg = AppConfig::default();
        // Tray-first out of the box, but starting at login stays an opt-in.
        assert!(cfg.close_to_tray);
        assert!(!cfg.auto_start_on_boot);
        assert!(!cfg.start_minimized);

        // Start-minimized only makes sense as a companion of autostart.
        let mut cfg = AppConfig::default();
        cfg.auto_start_on_boot = true;
        cfg.start_minimized = true;
        cfg.normalize();
        assert!(cfg.start_minimized);

        cfg.auto_start_on_boot = false;
        cfg.normalize();
        assert!(!cfg.start_minimized);
    }

    #[test]
    fn bridge_is_on_by_default() {
        // A fresh install must wire `docker` to the engine with no extra step.
        let cfg = AppConfig::default();
        assert!(cfg.cli_bridge_enabled);
        assert!(cfg.cli_bridge_set_default);
        assert!(cfg.cli_bridge_prev_context.is_empty());
        // Missing keys (older or hand-written config) resolve the same way.
        let parsed: AppConfig = toml::from_str("").expect("empty file parses");
        assert!(parsed.cli_bridge_enabled);
        assert!(parsed.cli_bridge_set_default);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // Legacy/partial config.toml files must still parse. Unknown keys from
        // older versions (daemon_unit, prefer_socket) are simply ignored.
        let cfg: AppConfig = toml::from_str("").expect("empty file parses");
        // `toml::from_str` skips migration, so only compare the fields that
        // `deserialize` itself is responsible for.
        assert_eq!(
            cfg.auto_start_daemon,
            AppConfig::default().auto_start_daemon
        );
        assert!(cfg.cli_bridge_enabled);
        assert!(cfg.cli_bridge_set_default);
        // Closing the window hides to the tray unless the user turns it off.
        assert!(cfg.close_to_tray);

        let cfg: AppConfig =
            toml::from_str("daemon_unit = \"docker.service\"\nprefer_socket = \"/tmp/x.sock\"\n")
                .expect("legacy keys are ignored, not fatal");
        assert_eq!(
            cfg.auto_start_daemon,
            AppConfig::default().auto_start_daemon
        );
        assert!(cfg.cli_bridge_enabled);
        assert!(cfg.close_to_tray);

        let cfg: AppConfig =
            toml::from_str("auto_start_daemon = false\n").expect("partial file parses");
        assert!(!cfg.auto_start_daemon);
    }

    #[tokio::test]
    async fn migration_is_persisted_and_reported_once() {
        let dir = std::env::temp_dir().join(format!("dm-cfg-mig-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");
        tokio::fs::write(&path, "close_to_tray = false\n")
            .await
            .expect("seeds a legacy file");

        let (cfg, stale) = AppConfig::load_and_migrate(&path).await.expect("loads");
        assert!(
            stale,
            "a file below the current schema must report itself stale"
        );
        assert!(cfg.close_to_tray);
        cfg.save_to(&path).await.expect("persists the upgrade");

        let raw = tokio::fs::read_to_string(&path).await.expect("reads back");
        assert!(raw.contains("close_to_tray = true"));
        assert!(raw.contains(&format!("schema_version = {CONFIG_SCHEMA_VERSION}")));

        // The upgrade is one-shot: the rewritten file is current.
        let (_, stale_again) = AppConfig::load_and_migrate(&path).await.expect("loads");
        assert!(!stale_again);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn round_trips_through_disk() {
        let dir = std::env::temp_dir().join(format!("dm-cfg-rt-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");
        let cfg = AppConfig {
            auto_start_daemon: false,
            compose_file: "/tmp/docker-compose.yml".to_string(),
            cli_bridge_enabled: true,
            cli_bridge_set_default: true,
            cli_bridge_prev_context: "desktop-linux".to_string(),
            data_root: "/mnt/storage/docker".to_string(),
            theme: "dark".to_string(),
            close_to_tray: true,
            auto_start_on_boot: false,
            start_minimized: false,
            cli_mode: "auto".to_string(),
            cli_shim_enabled: false,
            schema_version: CONFIG_SCHEMA_VERSION,
            endpoints: Vec::new(),
            active_endpoint: None,
            terminal_shell: default_shell(),
            terminal_font_size: default_terminal_font_size(),
            registries: Vec::new(),
            default_registry: String::new(),
            notifications: NotificationPrefs::default(),
            kind_default_nodes: default_kind_nodes(),
            kind_default_image: String::new(),
            container_groups: Vec::new(),
            engine_memory_limit_mb: 0,
            engine_cpu_cores: String::new(),
            kind_auto_start: false,
            kind_auto_start_delay: default_kind_start_delay(),
            kind_cluster_prefs: Default::default(),
            mcp_enabled: false,
            mcp_port: default_mcp_port(),
            auto_prune_dangling_images: false,
            auto_prune_isolated_volumes: false,
            gateway_enabled: false,
            gateway_auto_start: false,
            gateway_http_port: default_gateway_http_port(),
            gateway_https_port: default_gateway_https_port(),
            gateway_routes: Vec::new(),
        };
        cfg.save_to(&path).await.expect("save works");
        let loaded = AppConfig::load_from(&path).await.expect("load works");
        assert_eq!(loaded, cfg);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_config_migrates_bridge_on() {
        // A file written before the bridge was on-by-default carries
        // `false`; loading it must flip both flags and stamp the schema.
        let mut cfg: AppConfig = toml::from_str(
            "cli_bridge_enabled = false\ncli_bridge_set_default = false\nclose_to_tray = false\n",
        )
        .expect("parses");
        assert!(!cfg.cli_bridge_enabled);
        assert!(!cfg.close_to_tray);
        cfg.migrate();
        assert!(cfg.cli_bridge_enabled);
        assert!(cfg.cli_bridge_set_default);
        // Schema 2 promotes close-to-tray to the default, including for a
        // stored `false` written by the first build of the toggle.
        assert!(cfg.close_to_tray);
        assert_eq!(cfg.schema_version, CONFIG_SCHEMA_VERSION);
        // Second run is a no-op.
        cfg.migrate();
        assert_eq!(cfg.schema_version, CONFIG_SCHEMA_VERSION);
    }

    #[test]
    fn notification_prefs_default_all_on() {
        // Every category ships enabled; the Settings UI only turns them off.
        let p = NotificationPrefs::default();
        assert!(
            p.build_done && p.transfer_done && p.engine_state && p.kind_events && p.tray_hidden
        );
        // Configs written before this section existed deserialize to all-on.
        let from_toml: NotificationPrefs = toml::from_str("").expect("empty table parses");
        assert_eq!(from_toml, NotificationPrefs::default());
    }

    #[test]
    fn container_groups_and_engine_limits_round_trip() {
        let mut cfg = AppConfig::default();
        assert!(cfg.container_groups.is_empty());
        assert_eq!(cfg.engine_memory_limit_mb, 0);
        assert_eq!(cfg.engine_cpu_limit().unwrap(), 0.0);

        cfg.container_groups.push(ContainerGroup {
            id: "g1".into(),
            name: "Stack A".into(),
            container_ids: vec!["abc".into()],
            auto_start: true,
        });
        cfg.engine_memory_limit_mb = 2048;
        cfg.engine_cpu_cores = "1.5".to_string();
        let toml = toml::to_string(&cfg).expect("serializes");
        let back: AppConfig = toml::from_str(&toml).expect("parses");
        assert_eq!(back.container_groups, cfg.container_groups);
        assert_eq!(back.engine_memory_limit_mb, 2048);
        assert_eq!(back.engine_cpu_limit().unwrap(), 1.5);

        // Files written before these keys existed parse to "off / empty".
        let legacy: AppConfig = toml::from_str("auto_start_daemon = true\n").expect("parses");
        assert!(legacy.container_groups.is_empty());
        assert_eq!(legacy.engine_memory_limit_mb, 0);
        assert_eq!(legacy.engine_cpu_cores, "");
        assert_eq!(legacy.engine_cpu_limit().unwrap(), 0.0);

        // Garbage CPU input errors instead of panicking or being ignored.
        let mut bad = AppConfig::default();
        bad.engine_cpu_cores = "lots".into();
        assert!(bad.engine_cpu_limit().is_err());
        bad.engine_cpu_cores = "-2".into();
        assert!(bad.engine_cpu_limit().is_err());
    }
}
