use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_daemon_unit() -> String {
    "docker.service".to_string()
}

fn default_auto_start() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_auto_start")]
    pub auto_start_daemon: bool,
    #[serde(default = "default_daemon_unit")]
    pub daemon_unit: String,
    #[serde(default)]
    pub prefer_socket: String,
    #[serde(default)]
    pub compose_file: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            auto_start_daemon: default_auto_start(),
            daemon_unit: default_daemon_unit(),
            prefer_socket: String::new(),
            compose_file: String::new(),
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
        assert_eq!(cfg.daemon_unit, "docker.service");
        assert!(cfg.prefer_socket.is_empty());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // Legacy/partial config.toml files must still parse.
        let cfg: AppConfig = toml::from_str("").expect("empty file parses");
        assert_eq!(cfg, AppConfig::default());

        let cfg: AppConfig =
            toml::from_str("daemon_unit = \"docker.service\"\n").expect("partial file parses");
        assert!(!cfg.daemon_unit.is_empty());
        assert_eq!(cfg.daemon_unit, "docker.service");
        assert!(cfg.auto_start_daemon);
        assert!(cfg.prefer_socket.is_empty());
    }

    #[tokio::test]
    async fn save_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("dm-cfg-test-{}", std::process::id()));
        let path = dir.join("config.toml");
        let cfg = AppConfig {
            auto_start_daemon: false,
            daemon_unit: "custom.service".to_string(),
            prefer_socket: "/tmp/test-docker.sock".to_string(),
            compose_file: "/tmp/docker-compose.yml".to_string(),
        };
        cfg.save_to(&path).await.expect("save works");
        let loaded = AppConfig::load_from(&path).await.expect("load works");
        assert_eq!(loaded, cfg);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn legacy_file_migrates_on_load() {
        // A config written by an older version (only daemon_unit set)
        // loads with the new defaults filling the gaps.
        let dir = std::env::temp_dir().join(format!("dm-cfg-legacy-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");
        std::fs::write(&path, "daemon_unit = \"docker.service\"\n").expect("fixture written");
        let loaded = AppConfig::load_from(&path).await.expect("legacy loads");
        assert_eq!(loaded.daemon_unit, "docker.service");
        assert!(loaded.auto_start_daemon);
        assert!(loaded.prefer_socket.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
