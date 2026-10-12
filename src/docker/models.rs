use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum DaemonState {
    #[default]
    Stopped,
    Starting,
    /// A stop was requested and the socket is still winding down.
    Stopping,
    Running,
    NotInstalled,
    /// A bundled-engine download/install is in flight.
    Installing,
    /// Engine probe failed with a human-readable reason (e.g. ping error).
    Error(String),
}

#[derive(Debug, Clone, Default)]
pub struct ContainerItem {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub created: i64,
    pub running: bool,
    pub ports: String,
}

#[derive(Debug, Clone, Default)]
pub struct ImageItem {
    pub id: String,
    pub short_id: String,
    pub tags: Vec<String>,
    pub size: i64,
    pub created: i64,
}

#[derive(Debug, Clone, Default)]
pub struct VolumeItem {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub created: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NetworkItem {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub subnet: String,
    pub gateway: String,
}

#[derive(Debug, Clone, Default)]
pub struct ContainerStats {
    pub cpu_percent: f64,
    pub mem_usage: u64,
    pub mem_limit: u64,
}

impl ContainerStats {
    pub fn mem_percent(&self) -> f64 {
        if self.mem_limit == 0 {
            0.0
        } else {
            (self.mem_usage as f64 / self.mem_limit as f64) * 100.0
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EngineInfo {
    pub version: String,
    pub api_version: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub containers_running: i64,
    pub containers_stopped: i64,
    pub containers_paused: i64,
    pub images: i64,
    pub storage_driver: String,
    pub root_dir: String,
    pub cgroup_driver: String,
    pub n_cpu: i64,
    pub mem_total: i64,
}

/// Full detail snapshot for one container, built from `inspect_container`.
#[derive(Debug, Clone, Default)]
pub struct ContainerDetails {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub created: String,
    pub command: String,
    pub env: Vec<String>,
    pub mounts: Vec<String>,
    pub networks: Vec<String>,
    pub ports: String,
    pub restart_policy: String,
}

/// Parameters for creating + starting a container (Run wizard).
#[derive(Debug, Clone, Default)]
pub struct RunConfig {
    pub image: String,
    pub name: String,
    pub command: Vec<String>,
    /// `HOST:CONTAINER` entries, e.g. `8080:80`.
    pub ports: Vec<String>,
    /// `KEY=VALUE` entries.
    pub env: Vec<String>,
    /// `HOST_PATH:CONTAINER_PATH` entries.
    pub volumes: Vec<String>,
    /// one of: no, always, unless-stopped, on-failure
    pub restart_policy: String,
    pub detach: bool,
}

impl RunConfig {
    /// Validate user input; returns a human-readable error.
    pub fn validate(&self) -> Result<(), String> {
        if self.image.trim().is_empty() {
            return Err("Image is required".to_string());
        }
        for p in &self.ports {
            let parts: Vec<&str> = p.split(':').collect();
            if parts.len() != 2
                || parts[0].trim().is_empty()
                || parts[1].trim().is_empty()
                || parts[0].parse::<u16>().is_err()
                || parts[1]
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .parse::<u16>()
                    .is_err()
            {
                return Err(format!(
                    "Invalid port mapping '{p}' (expected HOST:CONTAINER)"
                ));
            }
        }
        for v in &self.volumes {
            let parts: Vec<&str> = v.split(':').collect();
            if parts.len() != 2 || parts[0].trim().is_empty() || parts[1].trim().is_empty() {
                return Err(format!(
                    "Invalid volume mapping '{v}' (expected HOST:CONTAINER)"
                ));
            }
        }
        for e in &self.env {
            if !e.contains('=') || e.starts_with('=') {
                return Err(format!("Invalid env entry '{e}' (expected KEY=VALUE)"));
            }
        }
        Ok(())
    }

    /// Parse a multiline text area into trimmed non-empty lines.
    pub fn parse_lines(text: &str) -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }
}

/// One entry of `docker history`.
#[derive(Debug, Clone, Default)]
pub struct ImageHistoryEntry {
    pub id: String,
    pub short_id: String,
    pub created: i64,
    pub created_by: String,
    pub tags: Vec<String>,
    pub size: i64,
}

/// Aggregated `docker system df` numbers for the dashboard.
#[derive(Debug, Clone, Default)]
pub struct DfReport {
    pub images_count: usize,
    pub images_size: u64,
    pub containers_count: usize,
    pub containers_size: u64,
    pub volumes_count: usize,
    pub volumes_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(image: &str, ports: &[&str], env: &[&str]) -> RunConfig {
        RunConfig {
            image: image.to_string(),
            ports: ports.iter().map(|s| s.to_string()).collect(),
            env: env.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn validate_rejects_empty_image() {
        assert!(cfg("", &[], &[]).validate().is_err());
        assert!(cfg("  ", &[], &[]).validate().is_err());
    }

    #[test]
    fn validate_accepts_host_port_mapping() {
        assert!(cfg("nginx:latest", &["8080:80"], &["K=V"])
            .validate()
            .is_ok());
    }

    #[test]
    fn validate_rejects_bad_ports_and_env() {
        assert!(cfg("img", &["no-colon"], &[]).validate().is_err());
        assert!(cfg("img", &["a:b:c"], &[]).validate().is_err());
        assert!(cfg("img", &[], &["NOEQUALS"]).validate().is_err());
    }

    #[test]
    fn parse_lines_trims_and_skips_empty() {
        assert_eq!(
            RunConfig::parse_lines("  a\n\n  b  \n"),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(RunConfig::parse_lines("  \n ").is_empty());
    }
}
