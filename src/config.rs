use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_auto_start() -> bool {
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
    /// engine. Off by default: it writes outside the app's own dirs.
    #[serde(default)]
    pub cli_bridge_enabled: bool,
    /// Also run `docker context use docker-manager-engine` so plain `docker`
    /// works without `--context`. Ignored unless `cli_bridge_enabled`.
    #[serde(default)]
    pub cli_bridge_set_default: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_start_daemon: default_auto_start(),
            compose_file: String::new(),
            cli_bridge_enabled: false,
            cli_bridge_set_default: false,
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
                return Self::load_from(&path).await;
            }
        }
        Ok(Self::default())
    }

    pub async fn load_from(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = tokio::fs::read_to_string(path).await?;
        Ok(toml::from_str(&content)?)
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
    fn missing_fields_fall_back_to_defaults() {
        // Legacy/partial config.toml files must still parse. Unknown keys from
        // older versions (daemon_unit, prefer_socket) are simply ignored.
        let cfg: AppConfig = toml::from_str("").expect("empty file parses");
        assert_eq!(cfg, AppConfig::default());

        let cfg: AppConfig =
            toml::from_str("daemon_unit = \"docker.service\"\nprefer_socket = \"/tmp/x.sock\"\n")
                .expect("legacy keys are ignored, not fatal");
        assert_eq!(cfg, AppConfig::default());

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
        };
        cfg.save_to(&path).await.expect("save works");
        let loaded = AppConfig::load_from(&path).await.expect("load works");
        assert_eq!(loaded, cfg);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
