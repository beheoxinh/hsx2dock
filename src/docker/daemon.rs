use crate::config::AppConfig;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use tokio::fs as tfs;

use super::models::DaemonState;

/// A socket path is usable when it exists and accepts a unix connection.
/// Plain files (e.g. leftover fixtures) must not count as a live daemon.
fn is_usable_socket(path: &PathBuf) -> bool {
    path.exists() && std::os::unix::net::UnixStream::connect(path).is_ok()
}

#[derive(Debug, Clone)]
pub struct DaemonManager {
    config: AppConfig,
    socket_candidates: Vec<PathBuf>,
}

impl DaemonManager {
    pub fn new(config: AppConfig) -> Self {
        let mut socket_candidates = Vec::new();
        let preferred = config.prefer_socket.trim();
        if !preferred.is_empty() {
            socket_candidates.push(PathBuf::from(preferred));
        }
        socket_candidates.push(PathBuf::from(format!(
            "{}/.docker/desktop/docker.sock",
            std::env::var("HOME").unwrap_or_default()
        )));
        socket_candidates.push(PathBuf::from("/var/run/docker.sock"));
        Self {
            config,
            socket_candidates,
        }
    }

    pub fn detect_sockets(&self) -> Vec<PathBuf> {
        self.socket_candidates
            .iter()
            .filter(|p| is_usable_socket(p))
            .cloned()
            .collect()
    }

    /// Validate the configured preferred socket: empty means "auto", otherwise
    /// it must be an absolute path. Returns the cleaned value or an error
    /// message for the Preferences dialog.
    pub fn validate_preferred_socket(raw: &str) -> Result<String, String> {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            return Ok(trimmed);
        }
        if !trimmed.starts_with('/') {
            return Err("Preferred socket must be an absolute path".into());
        }
        Ok(trimmed)
    }

    /// Socket candidates currently configured (preferred first).
    pub fn socket_candidates(&self) -> &[PathBuf] {
        &self.socket_candidates
    }

    pub fn active_socket(&self) -> Option<PathBuf> {
        self.detect_sockets().first().cloned()
    }

    pub fn state_from_socket(&self) -> DaemonState {
        match self.active_socket() {
            Some(_) => DaemonState::Running,
            None => DaemonState::Stopped,
        }
    }

    pub fn try_start_unit(&self) -> io::Result<bool> {
        if self.config.daemon_unit.trim().is_empty() {
            return Ok(false);
        }
        for unit in self.unit_candidates() {
            let status = Command::new("systemctl")
                .arg("--user")
                .arg("start")
                .arg(&unit)
                .status()?;
            if status.success() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Units to try when starting/stopping the daemon: the configured unit
    /// first, then the well-known `docker.service` fallback.
    fn unit_candidates(&self) -> Vec<String> {
        let mut units = Vec::new();
        let configured = self.config.daemon_unit.trim().to_string();
        if !configured.is_empty() {
            units.push(configured);
        }
        if !units.iter().any(|u| u == "docker.service") {
            units.push("docker.service".to_string());
        }
        units
    }

    pub fn try_stop_unit(&self) -> io::Result<bool> {
        if self.config.daemon_unit.trim().is_empty() {
            return Ok(false);
        }
        let mut stopped = false;
        for unit in self.unit_candidates() {
            let status = Command::new("systemctl")
                .arg("--user")
                .arg("stop")
                .arg(&unit)
                .status()?;
            stopped = stopped || status.success();
        }
        Ok(stopped)
    }

    pub fn is_unit_active(&self) -> io::Result<bool> {
        let unit = &self.config.daemon_unit;
        if unit.is_empty() {
            return Ok(false);
        }
        let status = Command::new("systemctl")
            .arg("--user")
            .arg("is-active")
            .arg(unit)
            .output()?;
        let out = String::from_utf8_lossy(&status.stdout);
        Ok(out.trim().contains("active"))
    }

    pub async fn ensure_config_dir(&self) -> io::Result<()> {
        if let Some(dir) = crate::config::AppConfig::config_dir() {
            tfs::create_dir_all(dir).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_sockets_no_false_positive() {
        let cfg = AppConfig::default();
        let dm = DaemonManager::new(cfg);
        let socks = dm.detect_sockets();
        // should only return existing sockets
        for s in &socks {
            assert!(s.exists());
        }
    }

    #[test]
    fn preferred_socket_wins_when_present() {
        let dir = std::env::temp_dir().join(format!("dm-sock-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let preferred = dir.join("preferred.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&preferred).expect("fixture socket");
        let cfg = AppConfig {
            prefer_socket: preferred.to_string_lossy().to_string(),
            ..AppConfig::default()
        };
        let dm = DaemonManager::new(cfg);
        assert_eq!(dm.active_socket(), Some(preferred.clone()));
        assert!(dm.detect_sockets().first() == Some(&preferred));
        assert!(matches!(dm.state_from_socket(), DaemonState::Running));
        drop(_listener);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_state_from_socket_logic() {
        let dm = DaemonManager::new(AppConfig::default());
        let st = dm.state_from_socket();
        match st {
            DaemonState::Running => assert!(dm.active_socket().is_some()),
            DaemonState::Stopped => assert!(dm.active_socket().is_none()),
            _ => (),
        }
    }
}
