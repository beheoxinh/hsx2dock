use std::path::Path;

use bollard::Docker;
use futures_util::StreamExt;

use super::models::{
    ContainerItem, ContainerStats, EngineInfo, ImageItem, NetworkItem, VolumeItem,
};

#[derive(Clone)]
pub struct DockerClient {
    docker: Docker,
}

impl DockerClient {
    pub fn connect(socket: Option<&Path>) -> anyhow::Result<Self> {
        let docker = match socket {
            Some(p) => {
                let host = format!("unix://{}", p.display());
                Docker::connect_with_unix(&host, 120, bollard::API_DEFAULT_VERSION)?
            }
            None => Docker::connect_with_socket_defaults()?,
        };
        Ok(Self { docker })
    }

    pub async fn ping(&self) -> anyhow::Result<()> {
        self.docker.ping().await?;
        Ok(())
    }

    pub async fn engine_info(&self) -> anyhow::Result<EngineInfo> {
        let v = self.docker.version().await?;
        let info = self.docker.info().await?;
        Ok(EngineInfo {
            version: v.version.unwrap_or_default(),
            api_version: v.api_version.unwrap_or_default(),
            os: v.os.unwrap_or_default(),
            kernel: v.kernel_version.unwrap_or_default(),
            arch: v.arch.unwrap_or_default(),
            containers_running: info.containers_running.unwrap_or(0),
            containers_stopped: info.containers_stopped.unwrap_or(0),
            containers_paused: info.containers_paused.unwrap_or(0),
            images: info.images.unwrap_or(0),
            storage_driver: info.driver.unwrap_or_default(),
            root_dir: info.docker_root_dir.unwrap_or_default(),
            cgroup_driver: info
                .cgroup_driver
                .map(|d| format!("{:?}", d))
                .unwrap_or_default(),
            n_cpu: info.ncpu.unwrap_or(0),
            mem_total: info.mem_total.unwrap_or(0),
        })
    }

    pub async fn list_containers(&self, all: bool) -> anyhow::Result<Vec<ContainerItem>> {
        let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
            .all(all)
            .build();
        let list = self.docker.list_containers(Some(opts)).await?;
        Ok(list
            .into_iter()
            .map(|c| {
                let id = c.id.clone().unwrap_or_default();
                let names = c.names.unwrap_or_default();
                let name = names
                    .first()
                    .map(|n| n.trim_start_matches('/').to_string())
                    .unwrap_or_default();
                let state = c
                    .state
                    .map(|s| format!("{:?}", s).to_lowercase())
                    .unwrap_or_default();
                ContainerItem {
                    short_id: id.chars().take(12).collect(),
                    id,
                    name,
                    image: c.image.unwrap_or_default(),
                    running: state == "running",
                    state,
                    status: c.status.unwrap_or_default(),
                    created: c.created.unwrap_or(0),
                }
            })
            .collect())
    }

    pub async fn start_container(&self, id: &str) -> anyhow::Result<()> {
        self.docker
            .start_container(id, None::<bollard::query_parameters::StartContainerOptions>)
            .await?;
        Ok(())
    }

    pub async fn stop_container(&self, id: &str) -> anyhow::Result<()> {
        self.docker.stop_container(id, None).await?;
        Ok(())
    }

    pub async fn restart_container(&self, id: &str) -> anyhow::Result<()> {
        self.docker.restart_container(id, None).await?;
        Ok(())
    }

    pub async fn remove_container(&self, id: &str, force: bool) -> anyhow::Result<()> {
        let opts = bollard::query_parameters::RemoveContainerOptionsBuilder::default()
            .force(force)
            .v(true)
            .build();
        self.docker.remove_container(id, Some(opts)).await?;
        Ok(())
    }

    pub async fn container_stats(&self, id: &str) -> anyhow::Result<ContainerStats> {
        let opts = bollard::query_parameters::StatsOptionsBuilder::default()
            .stream(false)
            .one_shot(true)
            .build();
        let mut stream = self.docker.stats(id, Some(opts));
        if let Some(stat) = stream.next().await {
            let stat = stat?;
            let cpu = stat.cpu_stats.as_ref();
            let pre = stat.precpu_stats.as_ref();
            let mut cpu_percent = 0.0f64;
            if let (Some(c), Some(p)) = (cpu, pre) {
                let cpu_total = c
                    .cpu_usage
                    .as_ref()
                    .and_then(|u| u.total_usage)
                    .unwrap_or(0) as f64;
                let pre_total = p
                    .cpu_usage
                    .as_ref()
                    .and_then(|u| u.total_usage)
                    .unwrap_or(0) as f64;
                let sys = c.system_cpu_usage.unwrap_or(0) as f64;
                let pre_sys = p.system_cpu_usage.unwrap_or(0) as f64;
                let cpu_delta = cpu_total - pre_total;
                let sys_delta = sys - pre_sys;
                let online = c
                    .online_cpus
                    .map(|v| v as u64)
                    .or_else(|| {
                        c.cpu_usage
                            .as_ref()
                            .and_then(|u| u.percpu_usage.as_ref().map(|v| v.len() as u64))
                    })
                    .unwrap_or(1)
                    .max(1) as f64;
                if sys_delta > 0.0 && cpu_delta >= 0.0 {
                    cpu_percent = (cpu_delta / sys_delta) * online * 100.0;
                }
            }
            let mem_usage = stat
                .memory_stats
                .as_ref()
                .and_then(|m| m.usage)
                .unwrap_or(0);
            let mem_limit = stat
                .memory_stats
                .as_ref()
                .and_then(|m| m.limit)
                .unwrap_or(0);
            return Ok(ContainerStats {
                cpu_percent: (cpu_percent * 100.0).round() / 100.0,
                mem_usage,
                mem_limit,
            });
        }
        Ok(ContainerStats::default())
    }

    pub async fn container_logs(&self, id: &str, tail: &str) -> anyhow::Result<String> {
        let opts = bollard::query_parameters::LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .tail(tail)
            .timestamps(false)
            .build();
        let mut out = String::new();
        let mut stream = self.docker.logs(id, Some(opts));
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            out.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
        }
        Ok(out)
    }

    pub async fn list_images(&self) -> anyhow::Result<Vec<ImageItem>> {
        let opts = bollard::query_parameters::ListImagesOptionsBuilder::default().build();
        let list = self.docker.list_images(Some(opts)).await?;
        Ok(list
            .into_iter()
            .map(|i| {
                let id = i.id;
                let tags = i.repo_tags;
                ImageItem {
                    short_id: id.chars().take(12).collect(),
                    id,
                    tags,
                    size: i.size,
                    created: i.created,
                }
            })
            .collect())
    }

    pub async fn remove_image(&self, id: &str, force: bool) -> anyhow::Result<()> {
        let opts = bollard::query_parameters::RemoveImageOptionsBuilder::default()
            .force(force)
            .build();
        self.docker.remove_image(id, Some(opts), None).await?;
        Ok(())
    }

    pub async fn prune_images(&self) -> anyhow::Result<(u64, u64)> {
        let opts = bollard::query_parameters::PruneImagesOptionsBuilder::default().build();
        let res = self.docker.prune_images(Some(opts)).await?;
        Ok((
            res.images_deleted.map(|v| v.len() as u64).unwrap_or(0),
            res.space_reclaimed.unwrap_or(0).max(0) as u64,
        ))
    }

    pub async fn pull_image<F>(&self, image: &str, mut on_progress: F) -> anyhow::Result<()>
    where
        F: FnMut(String) + Send,
    {
        let opts = bollard::query_parameters::CreateImageOptionsBuilder::default()
            .from_image(image)
            .build();
        let mut stream = self.docker.create_image(Some(opts), None, None);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if let Some(status) = chunk.status {
                let detail = chunk
                    .progress_detail
                    .as_ref()
                    .and_then(|p| p.current)
                    .map(|c| format!(" ({c})"))
                    .unwrap_or_default();
                on_progress(format!("{status}{detail}"));
            } else if let Some(err) = chunk.error_detail {
                let msg = err
                    .message
                    .unwrap_or_else(|| "image pull failed".to_string());
                anyhow::bail!("{msg}");
            }
        }
        Ok(())
    }

    pub async fn list_volumes(&self) -> anyhow::Result<Vec<VolumeItem>> {
        let opts = bollard::query_parameters::ListVolumesOptionsBuilder::default().build();
        let res = self.docker.list_volumes(Some(opts)).await?;
        Ok(res
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| VolumeItem {
                name: v.name,
                driver: v.driver,
                mountpoint: v.mountpoint,
                created: v.created_at.map(|c| c.to_string()),
            })
            .collect())
    }

    pub async fn create_volume(&self, name: &str, driver: &str) -> anyhow::Result<()> {
        let body = bollard::models::VolumeCreateRequest {
            name: Some(name.to_string()),
            driver: Some(if driver.is_empty() {
                "local".to_string()
            } else {
                driver.to_string()
            }),
            ..Default::default()
        };
        self.docker.create_volume(body).await?;
        Ok(())
    }

    pub async fn remove_volume(&self, name: &str, force: bool) -> anyhow::Result<()> {
        let opts = bollard::query_parameters::RemoveVolumeOptionsBuilder::default()
            .force(force)
            .build();
        self.docker.remove_volume(name, Some(opts)).await?;
        Ok(())
    }

    pub async fn prune_volumes(&self) -> anyhow::Result<u64> {
        let opts = bollard::query_parameters::PruneVolumesOptionsBuilder::default().build();
        let res = self.docker.prune_volumes(Some(opts)).await?;
        Ok(res.volumes_deleted.map(|v| v.len() as u64).unwrap_or(0))
    }

    pub async fn list_networks(&self) -> anyhow::Result<Vec<NetworkItem>> {
        let opts = bollard::query_parameters::ListNetworksOptionsBuilder::default().build();
        let list = self.docker.list_networks(Some(opts)).await?;
        Ok(list
            .into_iter()
            .map(|n| {
                let id = n.id.unwrap_or_default();
                let mut subnet = String::new();
                let mut gateway = String::new();
                if let Some(ipam) = n.ipam {
                    if let Some(cfgs) = ipam.config {
                        if let Some(cfg) = cfgs.first() {
                            subnet = cfg.subnet.clone().unwrap_or_default();
                            gateway = cfg.gateway.clone().unwrap_or_default();
                        }
                    }
                }
                NetworkItem {
                    name: n.name.unwrap_or_default(),
                    driver: n.driver.unwrap_or_default(),
                    id,
                    subnet,
                    gateway,
                }
            })
            .collect())
    }

    pub async fn create_network(&self, name: &str, driver: &str) -> anyhow::Result<()> {
        let body = bollard::models::NetworkCreateRequest {
            name: name.to_string(),
            driver: Some(if driver.is_empty() {
                "bridge".to_string()
            } else {
                driver.to_string()
            }),
            ..Default::default()
        };
        self.docker.create_network(body).await?;
        Ok(())
    }

    pub async fn remove_network(&self, name: &str) -> anyhow::Result<()> {
        self.docker.remove_network(name).await?;
        Ok(())
    }

    pub async fn prune_networks(&self) -> anyhow::Result<u64> {
        let opts = bollard::query_parameters::PruneNetworksOptionsBuilder::default().build();
        let res = self.docker.prune_networks(Some(opts)).await?;
        Ok(res.networks_deleted.map(|v| v.len() as u64).unwrap_or(0))
    }

    pub fn docker(&self) -> &Docker {
        &self.docker
    }
}

impl Default for DockerClient {
    fn default() -> Self {
        Self::connect(None).unwrap_or_else(|_| Self {
            docker: Docker::connect_with_socket_defaults().unwrap_or_else(|_| {
                Docker::connect_with_unix("/var/run/docker.sock", 120, bollard::API_DEFAULT_VERSION)
                    .expect("docker")
            }),
        })
    }
}
