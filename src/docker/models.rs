use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum DaemonState {
    #[default]
    Stopped,
    Starting,
    Running,
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
