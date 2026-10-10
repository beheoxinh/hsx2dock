use std::io::Write;
use std::path::Path;

use bollard::Docker;
use futures_util::StreamExt;

use super::models::{
    ContainerDetails, ContainerItem, ContainerStats, DfReport, EngineInfo, ImageHistoryEntry,
    ImageItem, NetworkItem, RunConfig, VolumeItem,
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

    /// Real daemon health check: the Engine API must answer.
    /// A present-but-stale socket file alone does not count as healthy.
    pub async fn is_healthy(&self) -> bool {
        self.docker.ping().await.map(|_| true).unwrap_or(false)
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
                    ports: c
                        .ports
                        .as_deref()
                        .map(Self::format_ports_static)
                        .unwrap_or_default(),
                }
            })
            .collect())
    }

    /// Format container port bindings as `PUBLIC:PRIVATE, ...`.
    pub fn format_ports_static(ports: &[bollard::models::PortSummary]) -> String {
        ports
            .iter()
            .map(|p| match p.public_port {
                Some(pub_port) => format!("{pub_port}:{}", p.private_port),
                None => format!("{}", p.private_port),
            })
            .collect::<Vec<_>>()
            .join(", ")
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

    pub async fn tag_image(&self, id: &str, repo: &str, tag: &str) -> anyhow::Result<()> {
        let opts = bollard::query_parameters::TagImageOptionsBuilder::default()
            .repo(repo)
            .tag(tag)
            .build();
        self.docker.tag_image(id, Some(opts)).await?;
        Ok(())
    }

    pub async fn push_image<F>(&self, name: &str, mut on_progress: F) -> anyhow::Result<()>
    where
        F: FnMut(String) + Send,
    {
        let mut stream = self.docker.push_image(
            name,
            None::<bollard::query_parameters::PushImageOptions>,
            None,
        );
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(info) => {
                    let status = info.status.unwrap_or_default();
                    if !status.is_empty() {
                        on_progress(status);
                    } else if let Some(err) = info.error_detail.and_then(|e| e.message) {
                        anyhow::bail!("{err}");
                    }
                }
                Err(e) => anyhow::bail!("{e}"),
            }
        }
        Ok(())
    }

    pub async fn image_history(&self, name: &str) -> anyhow::Result<Vec<ImageHistoryEntry>> {
        let list = self.docker.image_history(name).await?;
        Ok(list
            .into_iter()
            .map(|h| {
                let id = h.id.clone();
                ImageHistoryEntry {
                    short_id: id.chars().take(12).collect(),
                    id,
                    created: h.created,
                    created_by: h.created_by,
                    tags: h.tags,
                    size: h.size,
                }
            })
            .collect())
    }

    pub async fn search_images(&self, term: &str) -> anyhow::Result<Vec<(String, String, u64)>> {
        let opts = bollard::query_parameters::SearchImagesOptionsBuilder::default()
            .term(term)
            .build();
        let list = self.docker.search_images(opts).await?;
        Ok(list
            .into_iter()
            .map(|r| {
                (
                    r.name.unwrap_or_default(),
                    r.description.unwrap_or_default(),
                    r.star_count.unwrap_or(0) as u64,
                )
            })
            .collect())
    }

    pub async fn build_image<F>(
        &self,
        context_dir: &std::path::Path,
        dockerfile: &str,
        tag: &str,
        mut on_progress: F,
    ) -> anyhow::Result<()>
    where
        F: FnMut(String) + Send,
    {
        let tarball = build_context_tarball(context_dir)?;
        let dockerfile = if dockerfile.trim().is_empty() {
            "Dockerfile".to_string()
        } else {
            dockerfile.trim().to_string()
        };
        let mut builder = bollard::query_parameters::BuildImageOptionsBuilder::default()
            .dockerfile(&dockerfile)
            .rm(true);
        if !tag.trim().is_empty() {
            builder = builder.t(tag.trim());
        }
        let opts = builder.build();
        let body = bollard::body_full(bytes::Bytes::from(tarball));
        let mut stream = self.docker.build_image(opts, None, Some(body));
        while let Some(chunk) = stream.next().await {
            let info = chunk.map_err(|e| anyhow::anyhow!("{e}"))?;
            if let Some(err) = info.error_detail.and_then(|e| e.message) {
                anyhow::bail!("{err}");
            }
            if let Some(line) = info.stream {
                let line = line.trim().to_string();
                if !line.is_empty() {
                    on_progress(line);
                }
            } else if let Some(status) = info.status {
                if !status.trim().is_empty() {
                    on_progress(status);
                }
            }
        }
        Ok(())
    }

    pub async fn system_df(&self) -> anyhow::Result<DfReport> {
        let df = self
            .docker
            .df(None::<bollard::query_parameters::DataUsageOptions>)
            .await?;
        Ok(DfReport {
            images_count: df
                .image_usage
                .as_ref()
                .and_then(|u| u.total_count)
                .unwrap_or(0) as usize,
            images_size: df
                .image_usage
                .as_ref()
                .and_then(|u| u.total_size)
                .unwrap_or(0)
                .max(0) as u64,
            containers_count: df
                .container_usage
                .as_ref()
                .and_then(|u| u.total_count)
                .unwrap_or(0) as usize,
            containers_size: df
                .container_usage
                .as_ref()
                .and_then(|u| u.total_size)
                .unwrap_or(0)
                .max(0) as u64,
            volumes_count: df
                .volume_usage
                .as_ref()
                .and_then(|u| u.total_count)
                .unwrap_or(0) as usize,
            volumes_size: df
                .volume_usage
                .as_ref()
                .and_then(|u| u.total_size)
                .unwrap_or(0)
                .max(0) as u64,
        })
    }

    pub async fn system_prune(&self) -> anyhow::Result<u64> {
        let (mut reclaimed, _) = self.prune_images().await.unwrap_or((0, 0));
        reclaimed += self
            .docker
            .prune_containers(None::<bollard::query_parameters::PruneContainersOptions>)
            .await
            .map(|r| r.space_reclaimed.unwrap_or(0).max(0) as u64)
            .unwrap_or(0);
        reclaimed += self
            .docker
            .prune_networks(None::<bollard::query_parameters::PruneNetworksOptions>)
            .await
            .map(|_| 0u64)
            .unwrap_or(0);
        reclaimed += self.prune_volumes().await.unwrap_or(0);
        let _ = reclaimed;
        // Recompute via df for an accurate total.
        let df = self.system_df().await.unwrap_or_default();
        Ok(df.images_size + df.containers_size + df.volumes_size)
    }

    pub async fn pause_container(&self, id: &str) -> anyhow::Result<()> {
        self.docker.pause_container(id).await?;
        Ok(())
    }

    pub async fn unpause_container(&self, id: &str) -> anyhow::Result<()> {
        self.docker.unpause_container(id).await?;
        Ok(())
    }

    pub async fn rename_container(&self, id: &str, name: &str) -> anyhow::Result<()> {
        let opts = bollard::query_parameters::RenameContainerOptionsBuilder::default()
            .name(name)
            .build();
        self.docker.rename_container(id, opts).await?;
        Ok(())
    }

    pub async fn inspect_container(&self, id: &str) -> anyhow::Result<ContainerDetails> {
        let info = self
            .docker
            .inspect_container(
                id,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await?;
        Ok(container_details_from_inspect(&info))
    }

    /// Create (and optionally start) a container from a [`RunConfig`].
    /// Returns the new container id.
    pub async fn create_container(&self, cfg: &RunConfig, start: bool) -> anyhow::Result<String> {
        cfg.validate().map_err(anyhow::Error::msg)?;
        let mut port_bindings: std::collections::HashMap<
            String,
            Option<Vec<bollard::models::PortBinding>>,
        > = std::collections::HashMap::new();
        let mut exposed_ports: Vec<String> = Vec::new();
        for p in &cfg.ports {
            // Validated as HOST:CONTAINER above.
            let parts: Vec<&str> = p.split(':').collect();
            let (host, container) = (parts[0].trim(), parts[1].trim());
            let container_key = if container.contains('/') {
                container.to_string()
            } else {
                format!("{container}/tcp")
            };
            exposed_ports.push(container_key.clone());
            port_bindings.insert(
                container_key,
                Some(vec![bollard::models::PortBinding {
                    host_ip: None,
                    host_port: Some(host.to_string()),
                }]),
            );
        }
        let binds: Vec<String> = cfg.volumes.clone();
        let restart_name = match cfg.restart_policy.as_str() {
            "always" => bollard::models::RestartPolicyNameEnum::ALWAYS,
            "unless-stopped" => bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED,
            "on-failure" => bollard::models::RestartPolicyNameEnum::ON_FAILURE,
            _ => bollard::models::RestartPolicyNameEnum::NO,
        };
        let host_config = bollard::models::HostConfig {
            binds: if binds.is_empty() { None } else { Some(binds) },
            port_bindings: if port_bindings.is_empty() {
                None
            } else {
                Some(port_bindings)
            },
            restart_policy: Some(bollard::models::RestartPolicy {
                name: Some(restart_name),
                maximum_retry_count: None,
            }),
            ..Default::default()
        };
        let body = bollard::models::ContainerCreateBody {
            image: Some(cfg.image.clone()),
            cmd: if cfg.command.is_empty() {
                None
            } else {
                Some(cfg.command.clone())
            },
            env: if cfg.env.is_empty() {
                None
            } else {
                Some(cfg.env.clone())
            },
            exposed_ports: if exposed_ports.is_empty() {
                None
            } else {
                Some(exposed_ports)
            },
            host_config: Some(host_config),
            ..Default::default()
        };
        let opts = if cfg.name.trim().is_empty() {
            None
        } else {
            Some(
                bollard::query_parameters::CreateContainerOptionsBuilder::default()
                    .name(cfg.name.trim())
                    .build(),
            )
        };
        let res = self.docker.create_container(opts, body).await?;
        if start || cfg.detach {
            let _ = self
                .docker
                .start_container(
                    &res.id,
                    None::<bollard::query_parameters::StartContainerOptions>,
                )
                .await;
        }
        Ok(res.id)
    }

    /// Fetch the last `tail` lines of a container's logs (decoded text).
    pub async fn logs_tail(&self, id: &str, tail: &str) -> anyhow::Result<String> {
        self.container_logs(id, tail).await
    }

    /// Stream container logs line-by-line; `follow=true` keeps the
    /// stream open. Decoded lines (multiplex framing already stripped
    /// by bollard) are split on newlines before invoking the callback.
    pub async fn logs_stream<F>(
        &self,
        id: &str,
        tail: &str,
        follow: bool,
        mut on_line: F,
    ) -> anyhow::Result<()>
    where
        F: FnMut(String) + Send,
    {
        let opts = bollard::query_parameters::LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .tail(tail)
            .follow(follow)
            .timestamps(false)
            .build();
        let mut stream = self.docker.logs(id, Some(opts));
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let text = chunk.to_string();
            for line in text.split_inclusive('\n') {
                if !line.is_empty() {
                    on_line(line.to_string());
                }
            }
        }
        Ok(())
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

fn container_details_from_inspect(
    info: &bollard::models::ContainerInspectResponse,
) -> ContainerDetails {
    let id = info.id.clone().unwrap_or_default();
    let name = info
        .name
        .clone()
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_string();
    let state = info
        .state
        .as_ref()
        .and_then(|s| s.status)
        .map(|s| format!("{s:?}").to_lowercase())
        .unwrap_or_default();
    let config = info.config.as_ref();
    let command = info
        .path
        .clone()
        .into_iter()
        .chain(info.args.clone().unwrap_or_default())
        .collect::<Vec<_>>()
        .join(" ");
    let mounts = info
        .mounts
        .as_ref()
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    format!(
                        "{}:{}",
                        m.source.clone().unwrap_or_default(),
                        m.destination.clone().unwrap_or_default()
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let networks = info
        .network_settings
        .as_ref()
        .and_then(|n| n.networks.clone())
        .map(|nets| nets.keys().cloned().collect())
        .unwrap_or_default();
    ContainerDetails {
        short_id: id.chars().take(12).collect(),
        id,
        name,
        image: config
            .and_then(|c| c.image.clone())
            .or_else(|| info.image.clone())
            .unwrap_or_default(),
        state,
        status: String::new(),
        created: String::new(),
        command,
        env: config.and_then(|c| c.env.clone()).unwrap_or_default(),
        mounts,
        networks,
        ports: String::new(),
        restart_policy: info
            .host_config
            .as_ref()
            .and_then(|h| h.restart_policy.clone())
            .and_then(|r| r.name)
            .map(|n| format!("{n:?}").to_lowercase())
            .unwrap_or_default(),
    }
}

/// Pack `dir` (recursively) into a gzipped tarball for the `/build` endpoint.
///
/// Symlinks are skipped, hidden `.dockerignore` semantics are out of scope:
/// everything under the directory is included. Fails when `dir` is not a
/// readable directory.
fn build_context_tarball(dir: &std::path::Path) -> anyhow::Result<Vec<u8>> {
    if !dir.is_dir() {
        anyhow::bail!("build context is not a directory: {}", dir.display());
    }
    let mut tar_data = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_data);
        append_dir_all(&mut builder, dir, dir)?;
        builder.into_inner()?;
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&tar_data)?;
    Ok(gz.finish()?)
}

fn append_dir_all(
    builder: &mut tar::Builder<&mut Vec<u8>>,
    root: &std::path::Path,
    dir: &std::path::Path,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);
        if path.is_symlink() {
            continue;
        } else if path.is_dir() {
            builder.append_dir(rel, &path)?;
            append_dir_all(builder, root, &path)?;
        } else if path.is_file() {
            builder.append_path_with_name(&path, rel)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_config_validation() {
        let mut cfg = RunConfig::default();
        assert!(cfg.validate().is_err());
        cfg.image = "nginx:latest".to_string();
        assert!(cfg.validate().is_ok());
        cfg.ports = vec!["bad".to_string()];
        assert!(cfg.validate().is_err());
        cfg.ports = vec!["8080:80".to_string()];
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn parse_lines_trims_and_skips_empty() {
        let out = RunConfig::parse_lines("  a=1\n\n  \n b=2\n");
        assert_eq!(out, vec!["a=1".to_string(), "b=2".to_string()]);
    }

    #[test]
    fn format_ports_joins() {
        let ports = vec![
            bollard::models::PortSummary {
                private_port: 80,
                public_port: Some(8080),
                ..Default::default()
            },
            bollard::models::PortSummary {
                private_port: 443,
                public_port: None,
                ..Default::default()
            },
        ];
        assert_eq!(DockerClient::format_ports_static(&ports), "8080:80, 443");
    }
}
