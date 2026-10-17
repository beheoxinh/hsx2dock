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
    /// `docker-manager-engine` context) so the system `docker` CLI reaches our
    /// engine. On by default: it is the feature this app exists for.
    #[serde(default = "default_true")]
    pub cli_bridge_enabled: bool,
    /// Also run `docker context use docker-manager-engine` so plain `docker`
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
    /// If empty, defaults to `$XDG_DATA_HOME/docker-manager/data`.
    #[serde(default)]
    pub data_root: String,
    /// UI Theme preference: "system" | "dark" | "light"
    #[serde(default = "default_theme")]
    pub theme: String,
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
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_cli_mode() -> String {
    "auto".to_string()
}

/// Current config schema. See [`AppConfig::schema_version`].
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

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
            cli_mode: default_cli_mode(),
            cli_shim_enabled: false,
            schema_version: CONFIG_SCHEMA_VERSION,
        }
    }
}

impl AppConfig {
    pub fn config_dir() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("docker-manager"))
    }

    pub fn config_path() -> Option<PathBuf> {
        Self::config_dir().map(|d| d.join("config.toml"))
    }

    pub async fn load() -> anyhow::Result<Self> {
        if let Some(path) = Self::config_path() {
            if path.exists() {
                let cfg = Self::load_from(&path).await?;
                // Persist a one-time migration so it does not re-run and so
                // the file is not misleadingly still showing the old values.
                if cfg.schema_version < CONFIG_SCHEMA_VERSION {
                    let _ = cfg.save().await;
                }
                return Ok(cfg);
            }
        }
        Ok(Self::default())
    }

    pub async fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = tokio::fs::read_to_string(path).await?;
        let mut cfg: AppConfig = toml::from_str(&content)?;
        cfg.migrate();
        Ok(cfg)
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
            self.schema_version = CONFIG_SCHEMA_VERSION;
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

        let cfg: AppConfig =
            toml::from_str("daemon_unit = \"docker.service\"\nprefer_socket = \"/tmp/x.sock\"\n")
                .expect("legacy keys are ignored, not fatal");
        assert_eq!(
            cfg.auto_start_daemon,
            AppConfig::default().auto_start_daemon
        );
        assert!(cfg.cli_bridge_enabled);

        let cfg: AppConfig =
            toml::from_str("auto_start_daemon = false\n").expect("partial file parses");
        assert!(!cfg.auto_start_daemon);
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
            cli_mode: "auto".to_string(),
            cli_shim_enabled: false,
            schema_version: CONFIG_SCHEMA_VERSION,
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
        let mut cfg: AppConfig =
            toml::from_str("cli_bridge_enabled = false\ncli_bridge_set_default = false\n")
                .expect("parses");
        assert!(!cfg.cli_bridge_enabled);
        cfg.migrate();
        assert!(cfg.cli_bridge_enabled);
        assert!(cfg.cli_bridge_set_default);
        assert_eq!(cfg.schema_version, CONFIG_SCHEMA_VERSION);
        // Second run is a no-op.
        cfg.migrate();
        assert_eq!(cfg.schema_version, CONFIG_SCHEMA_VERSION);
    }
}
