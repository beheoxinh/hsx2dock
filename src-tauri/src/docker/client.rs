use std::io::Write;
use std::path::Path;

use bollard::Docker;
use futures_util::StreamExt;

use super::models::{
    limits_from_host_config, BuildOptions, BuildStep, ContainerDetails, ContainerItem,
    ContainerStats, DfReport, EngineInfo, EnvVar, Healthcheck, ImageHistoryEntry, ImageItem,
    NetworkItem, PortMapping, PortProtocol, PublishedPort, ResourceLimits, RunConfig, VolumeItem,
    VolumeMount,
};

#[derive(Clone)]
pub struct DockerClient {
    docker: Docker,
}

/// A stopped container is a prune candidate unless it is a kind node — those
/// carry the cluster and must survive every cleanup pass.
fn pruneable_container(state: &str, kind_cluster: Option<&str>) -> bool {
    kind_cluster.is_none() && matches!(state, "exited" | "dead" | "created")
}

/// kind's network and Docker's built-ins are never prune candidates.
fn pruneable_network(name: &str, protected: bool) -> bool {
    !protected && !matches!(name, "bridge" | "host" | "none")
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

    /// Connects to an explicit endpoint.
    ///
    /// `kind` is `"local"`, `"tcp"` or `"ssh"`; the local kind needs the
    /// engine socket. TLS-terminated daemons are not supported yet, so a
    /// failure here is reported rather than silently retried over plain HTTP.
    pub fn connect_for_endpoint(
        kind: &str,
        uri: &str,
        socket: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let docker = match kind {
            "local" => match socket {
                Some(p) => Docker::connect_with_unix(
                    &format!("unix://{}", p.display()),
                    120,
                    bollard::API_DEFAULT_VERSION,
                )?,
                None => Docker::connect_with_socket_defaults()?,
            },
            "tcp" => {
                let host = normalize_tcp_uri(uri)?;
                Docker::connect_with_http(&host, 120, bollard::API_DEFAULT_VERSION)?
            }
            "ssh" => {
                let host = normalize_ssh_uri(uri)?;
                // No explicit keypair: bollard falls back to the agent and the
                // user's default identity files, exactly like the ssh CLI.
                Docker::connect_with_ssh(&host, 120, bollard::API_DEFAULT_VERSION, None)?
            }
            other => anyhow::bail!("Unsupported endpoint kind: {other}"),
        };
        Ok(Self { docker })
    }

    /// The daemon's reported version, used to label a reachable endpoint.
    pub async fn version_string(&self) -> anyhow::Result<String> {
        let v = self.docker.version().await?;
        Ok(v.version.unwrap_or_default())
    }

    /// Real daemon health check: the Engine API must answer.
    /// A present-but-stale socket file alone does not count as healthy.
    pub async fn is_healthy(&self) -> bool {
        self.docker.ping().await.map(|_| true).unwrap_or(false)
    }

    pub async fn ping(&self) -> anyhow::Result<()> {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.docker.ping())
            .await
            .map_err(|_| anyhow::anyhow!("docker ping timed out after 5s"))??;
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

    /// One-shot disk usage per container (bytes written to its RW layer).
    /// `size=true` makes the daemon walk each container's layer, so this is
    /// deliberately separate from the cheap `list_containers` the tick uses.
    pub async fn container_sizes(&self) -> anyhow::Result<std::collections::HashMap<String, u64>> {
        let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
            .all(true)
            .size(true)
            .build();
        let list = self.docker.list_containers(Some(opts)).await?;
        Ok(list
            .into_iter()
            .filter_map(|c| {
                let id = c.id?;
                let bytes = c.size_rw.unwrap_or(0).max(0) as u64;
                Some((id, bytes))
            })
            .collect())
    }

    /// One-shot exec: runs `cmd` inside `container`, returns combined stdout+stderr.
    /// Errors on non-zero exit. Used for kubectl queries inside kind nodes.
    pub async fn exec_capture(&self, container: &str, cmd: Vec<String>) -> anyhow::Result<String> {
        use anyhow::Context;
        use bollard::container::LogOutput;
        use bollard::exec::StartExecResults;
        use bollard::models::ExecConfig;

        let exec = self
            .docker
            .create_exec(
                container,
                ExecConfig {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(cmd),
                    ..Default::default()
                },
            )
            .await
            .with_context(|| format!("create exec in {container}"))?;
        let id = exec.id;
        let started = self
            .docker
            .start_exec(&id, None)
            .await
            .with_context(|| format!("start exec in {container}"))?;
        let StartExecResults::Attached { mut output, .. } = started else {
            anyhow::bail!("engine started this exec detached");
        };
        let mut out: Vec<u8> = Vec::new();
        while let Some(frame) = output.next().await {
            match frame {
                Ok(LogOutput::StdOut { message })
                | Ok(LogOutput::StdErr { message })
                | Ok(LogOutput::Console { message }) => out.extend_from_slice(&message),
                Ok(_) => {}
                Err(e) => anyhow::bail!("exec stream error: {e}"),
            }
        }
        let inspect = self.docker.inspect_exec(&id).await?;
        if let Some(code) = inspect.exit_code {
            if code != 0 {
                anyhow::bail!(
                    "command exited with {code}: {}",
                    String::from_utf8_lossy(&out).trim()
                );
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Applies CPU/memory caps to an existing (running or stopped) container.
    /// `cpu_m` in millicores, `mem_mb` in MiB; `None` leaves the field alone.
    /// Used to cap kind node containers so clusters can't starve the host.
    pub async fn update_container_caps(
        &self,
        id: &str,
        cpu_m: Option<u32>,
        mem_mb: Option<u32>,
    ) -> anyhow::Result<()> {
        use bollard::models::ContainerUpdateBody;

        let mut body = ContainerUpdateBody::default();
        if let Some(m) = cpu_m.filter(|v| *v > 0) {
            // quota = cores × 100_000 µs per 100 ms period; millicores → ×100
            body.cpu_quota = Some((m as i64) * 100);
        }
        if let Some(mb) = mem_mb.filter(|v| *v > 0) {
            let bytes = mb as i64 * 1024 * 1024;
            body.memory = Some(bytes);
            // memory_swap must be >= memory or some daemons reject the update
            body.memory_swap = Some(bytes * 2);
        }
        self.docker.update_container(id, body).await?;
        Ok(())
    }

    /// Rewrites a container's restart policy. kind nodes are created with
    /// `on-failure:1`, which lets Docker bring a cluster back on its own when
    /// the engine starts; the app pins them to `"no"` so only its own
    /// auto-start logic decides what runs.
    pub async fn update_restart_policy(&self, id: &str, policy: &str) -> anyhow::Result<()> {
        use bollard::models::{ContainerUpdateBody, RestartPolicy, RestartPolicyNameEnum as B};
        let name = match policy {
            "always" => B::ALWAYS,
            "unless-stopped" => B::UNLESS_STOPPED,
            "on-failure" => B::ON_FAILURE,
            _ => B::NO,
        };
        let body = ContainerUpdateBody {
            restart_policy: Some(RestartPolicy {
                name: Some(name),
                maximum_retry_count: None,
            }),
            ..Default::default()
        };
        self.docker.update_container(id, body).await?;
        Ok(())
    }

    pub async fn list_containers(&self, all: bool) -> anyhow::Result<Vec<ContainerItem>> {
        let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
            .all(all)
            .build();
        let list = self.docker.list_containers(Some(opts)).await?;
        Ok(list
            .into_iter()
            .map(|c| {
                let labels = c.labels.unwrap_or_default();
                let compose_project = labels
                    .get("com.docker.compose.project")
                    .filter(|s| !s.is_empty())
                    .cloned();
                let compose_service = labels
                    .get("com.docker.compose.service")
                    .filter(|s| !s.is_empty())
                    .cloned();
                let kind_cluster = labels
                    .get("io.x-k8s.kind.cluster")
                    .filter(|s| !s.is_empty())
                    .cloned();
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
                    published_ports: c
                        .ports
                        .as_deref()
                        .map(Self::published_ports_static)
                        .unwrap_or_default(),
                    compose_project,
                    compose_service,
                    kind_cluster,
                }
            })
            .collect())
    }

    /// Format container port bindings the way `docker ps` does, e.g.
    /// `0.0.0.0:8080->80/tcp, 127.0.0.1:5432->5432/tcp`, or `443/tcp` for a
    /// port that is only exposed. The bind address is kept on purpose: the UI
    /// flags mappings that are reachable beyond loopback.
    pub fn format_ports_static(ports: &[bollard::models::PortSummary]) -> String {
        ports
            .iter()
            .map(|p| {
                let proto = p.typ.as_ref().map(|t| t.to_string()).unwrap_or_default();
                let target = if proto.is_empty() {
                    p.private_port.to_string()
                } else {
                    format!("{}/{}", p.private_port, proto)
                };
                match p.public_port {
                    Some(pub_port) => match p.ip.as_deref().filter(|ip| !ip.is_empty()) {
                        Some(ip) => format!("{ip}:{pub_port}->{target}"),
                        None => format!("{pub_port}->{target}"),
                    },
                    None => target,
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Structured counterpart of [`Self::format_ports_static`]: only bindings
    /// that are actually published on the host, which is what the exposure
    /// check in the UI cares about.
    pub fn published_ports_static(ports: &[bollard::models::PortSummary]) -> Vec<PublishedPort> {
        ports
            .iter()
            .filter_map(|p| {
                let host_port = p.public_port?;
                Some(PublishedPort {
                    host_ip: p.ip.clone().unwrap_or_default(),
                    host_port,
                    container_port: p.private_port,
                    protocol: p.typ.as_ref().map(|t| t.to_string()).unwrap_or_default(),
                })
            })
            .collect()
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

    /// Remove stopped containers, never kind cluster nodes.
    ///
    /// The Docker prune API has no reliable negative label filter, and a
    /// stopped kind node is exactly what a "stopped containers" prune would
    /// eat — deleting it destroys the cluster. Selection happens here instead.
    pub async fn prune_containers(&self) -> anyhow::Result<u64> {
        let mut removed = 0u64;
        for container in self.list_containers(true).await? {
            if !pruneable_container(&container.state, container.kind_cluster.as_deref()) {
                continue;
            }
            let opts = bollard::query_parameters::RemoveContainerOptionsBuilder::default()
                .v(true)
                .build();
            if self
                .docker
                .remove_container(&container.id, Some(opts))
                .await
                .is_ok()
            {
                removed += 1;
            }
        }
        Ok(removed)
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
                net_rx: stat
                    .networks
                    .as_ref()
                    .map(|n| n.iter().map(|(_, s)| s.rx_bytes.unwrap_or(0)).sum())
                    .unwrap_or(0),
                net_tx: stat
                    .networks
                    .as_ref()
                    .map(|n| n.iter().map(|(_, s)| s.tx_bytes.unwrap_or(0)).sum())
                    .unwrap_or(0),
            });
        }
        Ok(ContainerStats::default())
    }

    pub async fn container_logs(&self, id: &str, tail: &str, timestamps: bool) -> anyhow::Result<String> {
        let opts = bollard::query_parameters::LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .tail(tail)
            .timestamps(timestamps)
            .build();
        let mut out = String::new();
        let mut stream = self.docker.logs(id, Some(opts));
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            out.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
        }
        Ok(out)
    }

    /// Every image, including kind's node images. Callers that render lists
    /// for the user should use [`Self::list_images_visible`] so core images
    /// stay out of reach.
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

    /// Summaries of kind node containers — every one carries
    /// `io.x-k8s.kind.cluster`, whatever image it was created from.
    async fn kind_node_containers(&self) -> anyhow::Result<Vec<bollard::models::ContainerSummary>> {
        let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
            .all(true)
            .build();
        Ok(self
            .docker
            .list_containers(Some(opts))
            .await?
            .into_iter()
            .filter(|c| {
                c.labels
                    .as_ref()
                    .map(|l| l.contains_key(crate::docker::kind::CLUSTER_LABEL))
                    .unwrap_or(false)
            })
            .collect())
    }

    /// Image IDs (without the `sha256:` prefix) that kind node containers were
    /// created from. A node image may be retagged or referenced only by digest,
    /// so the tag list alone cannot identify it — the nodes themselves are the
    /// proof that the image is load-bearing.
    pub async fn kind_node_image_ids(&self) -> anyhow::Result<std::collections::HashSet<String>> {
        Ok(self
            .kind_node_containers()
            .await?
            .into_iter()
            .filter_map(|c| c.image_id)
            .map(|id| id.trim_start_matches("sha256:").to_string())
            .collect())
    }

    /// Images the user may see and delete: kind node images are hidden because
    /// removing one breaks every cluster that uses it.
    pub async fn list_images_visible(
        &self,
        configured_kind_image: Option<&str>,
    ) -> anyhow::Result<Vec<ImageItem>> {
        // Container-derived IDs catch retagged / digest-only node images that
        // the tag check cannot see. A failure here only degrades to the tag
        // check rather than taking the whole image list down.
        let kind_images = self.kind_node_image_ids().await.unwrap_or_default();
        Ok(self
            .list_images()
            .await?
            .into_iter()
            .filter(|i| {
                !crate::docker::kind::is_kind_image(&i.tags, configured_kind_image)
                    && !kind_images.contains(i.id.trim_start_matches("sha256:"))
            })
            .collect())
    }

    /// Tags (repo:tag) currently attached to an image, used to decide whether
    /// removing it would take a kind node image with it.
    pub async fn image_repo_tags(&self, id: &str) -> anyhow::Result<Vec<String>> {
        let info = self.docker.inspect_image(id).await?;
        Ok(info.repo_tags.unwrap_or_default())
    }

    pub async fn remove_image(&self, id: &str, force: bool) -> anyhow::Result<()> {
        let opts = bollard::query_parameters::RemoveImageOptionsBuilder::default()
            .force(force)
            .build();
        self.docker.remove_image(id, Some(opts), None).await?;
        Ok(())
    }

    pub async fn prune_images(&self) -> anyhow::Result<(u64, u64)> {
        // Dangling-only for two reasons: it matches the UI copy ("unused
        // images") and it can never touch a tagged kind node image.
        let filters =
            std::collections::HashMap::from([("dangling".to_string(), vec!["true".to_string()])]);
        let opts = bollard::query_parameters::PruneImagesOptionsBuilder::default()
            .filters(&filters)
            .build();
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

        // Kind node containers mount /var as an anonymous Docker volume without
        // io.x-k8s.kind* labels. Inspect the nodes to discover every volume they
        // attach so those can be marked protected as well.
        let mut kind_volume_names = std::collections::HashSet::new();
        if let Ok(containers) = self.kind_node_containers().await {
            for c in containers {
                for m in c.mounts.into_iter().flatten() {
                    if let Some(name) = m.name.filter(|n| !n.is_empty()) {
                        kind_volume_names.insert(name);
                    }
                }
            }
        }

        Ok(res
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| {
                let is_kind_labelled = v.labels.keys().any(|k| k.starts_with("io.x-k8s.kind"));
                let is_kind_mounted = kind_volume_names.contains(&v.name);
                VolumeItem {
                    protected: is_kind_labelled || is_kind_mounted,
                    name: v.name,
                    driver: v.driver,
                    mountpoint: v.mountpoint,
                    created: v.created_at.map(|c| c.to_string()),
                    // Filled in by the `list_volumes` command after walking the
                    // mountpoint on disk; the raw API list leaves them unknown.
                    size: None,
                    restricted_dirs: 0,
                }
            })
            .collect())
    }

    pub async fn create_volume(
        &self,
        name: &str,
        driver: &str,
        driver_opts: std::collections::HashMap<String, String>,
        labels: std::collections::HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let body = bollard::models::VolumeCreateRequest {
            name: Some(name.to_string()),
            driver: Some(if driver.is_empty() {
                "local".to_string()
            } else {
                driver.to_string()
            }),
            driver_opts: if driver_opts.is_empty() {
                None
            } else {
                Some(driver_opts)
            },
            labels: if labels.is_empty() {
                None
            } else {
                Some(labels)
            },
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
        // Hand-rolled rather than the API call so protected (kind-managed)
        // volumes can never be selected. In-use volumes fail to remove and are
        // skipped, which matches Docker's "unused volumes" semantics.
        let mut removed = 0u64;
        for volume in self.list_volumes().await? {
            if volume.protected {
                continue;
            }
            if self.remove_volume(&volume.name, false).await.is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub async fn list_networks(&self) -> anyhow::Result<Vec<NetworkItem>> {
        let opts = bollard::query_parameters::ListNetworksOptionsBuilder::default().build();
        let list = self.docker.list_networks(Some(opts)).await?;
        Ok(list
            .into_iter()
            .map(|n| {
                let id = n.id.unwrap_or_default();
                let name = n.name.unwrap_or_default();
                let protected = name == crate::docker::kind::NETWORK_NAME
                    || n.labels
                        .as_ref()
                        .map(|labels| labels.keys().any(|k| k.starts_with("io.x-k8s.kind")))
                        .unwrap_or(false);
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
                    name,
                    driver: n.driver.unwrap_or_default(),
                    id,
                    subnet,
                    gateway,
                    protected,
                }
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_network(
        &self,
        name: &str,
        driver: &str,
        subnet: &str,
        gateway: &str,
        internal: bool,
        attachable: bool,
        labels: std::collections::HashMap<String, String>,
    ) -> anyhow::Result<()> {
        let ipam = if !subnet.trim().is_empty() || !gateway.trim().is_empty() {
            Some(bollard::models::Ipam {
                driver: Some("default".to_string()),
                config: Some(vec![bollard::models::IpamConfig {
                    subnet: if subnet.trim().is_empty() {
                        None
                    } else {
                        Some(subnet.trim().to_string())
                    },
                    gateway: if gateway.trim().is_empty() {
                        None
                    } else {
                        Some(gateway.trim().to_string())
                    },
                    ..Default::default()
                }]),
                options: None,
            })
        } else {
            None
        };

        let body = bollard::models::NetworkCreateRequest {
            name: name.to_string(),
            driver: Some(if driver.is_empty() {
                "bridge".to_string()
            } else {
                driver.to_string()
            }),
            internal: Some(internal),
            attachable: Some(attachable),
            ipam,
            labels: if labels.is_empty() {
                None
            } else {
                Some(labels)
            },
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
        // Skip kind's network and Docker's built-ins; attempt the rest and
        // ignore "network is in use" failures.
        let mut removed = 0u64;
        for network in self.list_networks().await? {
            if !pruneable_network(&network.name, network.protected) {
                continue;
            }
            if self.docker.remove_network(&network.id).await.is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
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

    /// Builds an image from a local context and streams every build step.
    ///
    /// `cancel` is polled on each stream item; flipping it drops the stream,
    /// which aborts the build on the daemon. Returns the built image id when
    /// the daemon reports one (`BuildInfo::aux`).
    pub async fn build_image<F>(
        &self,
        options: &BuildOptions,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        mut on_step: F,
    ) -> anyhow::Result<Option<String>>
    where
        F: FnMut(BuildStep) + Send,
    {
        let (context, dockerfile) = options.validate().map_err(anyhow::Error::msg)?;
        let tarball = build_context_tarball(&context)?;

        let mut builder = bollard::query_parameters::BuildImageOptionsBuilder::default()
            .dockerfile(&dockerfile)
            .rm(true)
            .forcerm(true);
        let tag = options.tag.trim();
        if !tag.is_empty() {
            builder = builder.t(tag);
        }
        if let Some(target) = options
            .target
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            builder = builder.target(target);
        }
        if options.no_cache {
            builder = builder.nocache(true);
        }
        if options.pull {
            builder = builder.pull("true");
        }
        let args = options.build_arg_pairs();
        if !args.is_empty() {
            builder = builder.buildargs(
                &args
                    .into_iter()
                    .collect::<std::collections::HashMap<_, _>>(),
            );
        }

        let opts = builder.build();
        let body = bollard::body_full(bytes::Bytes::from(tarball));
        let mut stream = self.docker.build_image(opts, None, Some(body));
        let mut image_id: Option<String> = None;

        while let Some(chunk) = stream.next().await {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Ok(image_id);
            }
            let info = chunk.map_err(|e| anyhow::anyhow!("{e}"))?;

            if let Some(err) = info.error_detail.and_then(|e| e.message) {
                let err = err.trim().to_string();
                if !err.is_empty() {
                    anyhow::bail!("{err}");
                }
            }
            if let Some(id) = info.aux.and_then(|aux| aux.id) {
                image_id = Some(id);
            }

            let (current, total) = info
                .progress_detail
                .as_ref()
                .map(|d| {
                    (
                        d.current.map(|v| v.max(0) as u64),
                        d.total.map(|v| v.max(0) as u64),
                    )
                })
                .unwrap_or((None, None));

            if let Some(line) = info.stream {
                let line = line.trim_end().to_string();
                if !line.trim().is_empty() {
                    on_step(BuildStep {
                        status: line.clone(),
                        log: Some(line),
                        current,
                        total,
                        is_error: false,
                    });
                }
            } else if let Some(status) = info.status {
                if !status.trim().is_empty() {
                    on_step(BuildStep {
                        status,
                        log: None,
                        current,
                        total,
                        is_error: false,
                    });
                }
            }
        }
        Ok(image_id)
    }

    pub async fn system_df(&self) -> anyhow::Result<DfReport> {
        let df = self
            .docker
            .df(None::<bollard::query_parameters::DataUsageOptions>)
            .await?;

        let images_size = df
            .image_usage
            .as_ref()
            .and_then(|u| u.total_size)
            .unwrap_or(0)
            .max(0) as u64;

        let containers_size = df
            .container_usage
            .as_ref()
            .and_then(|u| u.total_size)
            .unwrap_or(0)
            .max(0) as u64;

        let volumes_size = df
            .volume_usage
            .as_ref()
            .and_then(|u| u.total_size)
            .unwrap_or(0)
            .max(0) as u64;

        let build_cache_size = df
            .build_cache_usage
            .as_ref()
            .and_then(|u| u.total_size)
            .unwrap_or(0)
            .max(0) as u64;

        let img_reclaimable = df
            .image_usage
            .as_ref()
            .and_then(|u| u.reclaimable)
            .unwrap_or(0)
            .max(0) as u64;

        let cnt_reclaimable = df
            .container_usage
            .as_ref()
            .and_then(|u| u.reclaimable)
            .unwrap_or(0)
            .max(0) as u64;

        let vol_reclaimable = df
            .volume_usage
            .as_ref()
            .and_then(|u| u.reclaimable)
            .unwrap_or(0)
            .max(0) as u64;

        let cache_reclaimable = df
            .build_cache_usage
            .as_ref()
            .and_then(|u| u.reclaimable)
            .unwrap_or(0)
            .max(0) as u64;

        let total_reclaimable =
            img_reclaimable + cnt_reclaimable + vol_reclaimable + cache_reclaimable;

        Ok(DfReport {
            images_count: df
                .image_usage
                .as_ref()
                .and_then(|u| u.total_count)
                .unwrap_or(0) as usize,
            images_size,
            containers_count: df
                .container_usage
                .as_ref()
                .and_then(|u| u.total_count)
                .unwrap_or(0) as usize,
            containers_size,
            volumes_count: df
                .volume_usage
                .as_ref()
                .and_then(|u| u.total_count)
                .unwrap_or(0) as usize,
            volumes_size,
            build_cache_size,
            total_reclaimable,
        })
    }

    pub async fn system_prune(&self) -> anyhow::Result<u64> {
        let (mut reclaimed, _) = self.prune_images().await.unwrap_or((0, 0));
        // Container/network/volume pruning is hand-rolled so kind's node
        // containers, its network and any protected volumes survive.
        let _ = self.prune_containers().await.unwrap_or(0);
        let _ = self.prune_networks().await.unwrap_or(0);
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

    /// Unix seconds when the container was (last) started, if it ever was.
    /// Used to tell containers Docker restored on engine start apart from the
    /// ones that were already running when the app launched.
    pub async fn container_started_at(&self, id: &str) -> Option<i64> {
        let info = self
            .docker
            .inspect_container(
                id,
                None::<bollard::query_parameters::InspectContainerOptions>,
            )
            .await
            .ok()?;
        let started = info.state.and_then(|s| s.started_at)?;
        chrono::DateTime::parse_from_rfc3339(&started)
            .ok()
            .map(|d| d.timestamp())
    }

    /// Create (and optionally start) a container from a [`RunConfig`].
    /// Returns the new container id.
    pub async fn create_container(&self, cfg: &RunConfig, start: bool) -> anyhow::Result<String> {
        cfg.validate()
            .map_err(|e| anyhow::anyhow!("Invalid container configuration: {e}"))?;

        let port_bindings = cfg.port_bindings();
        let exposed_ports = cfg.exposed_ports();
        let binds: Vec<String> = cfg.volumes.iter().map(|v| v.to_spec()).collect();

        let mut labels_map = std::collections::HashMap::new();
        for l in &cfg.labels {
            labels_map.insert(l.key.clone(), l.value.clone());
        }

        let host_config = bollard::models::HostConfig {
            binds: if binds.is_empty() { None } else { Some(binds) },
            port_bindings: if port_bindings.is_empty() {
                None
            } else {
                Some(port_bindings)
            },
            restart_policy: Some(cfg.restart_policy_body()),
            memory: cfg
                .resources
                .memory_bytes()
                .map_err(|e| anyhow::anyhow!("Invalid memory limit: {e}"))?,
            nano_cpus: cfg
                .resources
                .nano_cpus()
                .map_err(|e| anyhow::anyhow!("Invalid CPU limit: {e}"))?,
            cpu_shares: cfg
                .resources
                .cpu_shares_value()
                .map_err(|e| anyhow::anyhow!("Invalid CPU shares: {e}"))?,
            cpuset_cpus: if cfg.resources.cpuset_cpus.trim().is_empty() {
                None
            } else {
                Some(cfg.resources.cpuset_cpus.trim().to_string())
            },
            pids_limit: cfg
                .resources
                .pids_limit_value()
                .map_err(|e| anyhow::anyhow!("Invalid PIDs limit: {e}"))?,
            cap_add: if cfg.cap_add.is_empty() {
                None
            } else {
                Some(cfg.cap_add.clone())
            },
            cap_drop: if cfg.cap_drop.is_empty() {
                None
            } else {
                Some(cfg.cap_drop.clone())
            },
            extra_hosts: if cfg.extra_hosts.is_empty() {
                None
            } else {
                Some(cfg.extra_hosts.clone())
            },
            network_mode: if cfg.network.trim().is_empty() {
                None
            } else {
                Some(cfg.network.trim().to_string())
            },
            privileged: Some(cfg.privileged),
            readonly_rootfs: Some(cfg.read_only_rootfs),
            auto_remove: Some(cfg.auto_remove),
            ..Default::default()
        };
        let body = bollard::models::ContainerCreateBody {
            image: Some(cfg.image.trim().to_string()),
            hostname: if cfg.hostname.trim().is_empty() {
                None
            } else {
                Some(cfg.hostname.trim().to_string())
            },
            working_dir: if cfg.workdir.trim().is_empty() {
                None
            } else {
                Some(cfg.workdir.trim().to_string())
            },
            user: if cfg.user.trim().is_empty() {
                None
            } else {
                Some(cfg.user.trim().to_string())
            },
            cmd: if cfg.command.is_empty() {
                None
            } else {
                Some(cfg.command.clone())
            },
            entrypoint: if cfg.entrypoint.is_empty() {
                None
            } else {
                Some(cfg.entrypoint.clone())
            },
            env: if cfg.env_list().is_empty() {
                None
            } else {
                Some(cfg.env_list())
            },
            labels: if labels_map.is_empty() {
                None
            } else {
                Some(labels_map)
            },
            tty: Some(cfg.tty),
            open_stdin: Some(cfg.stdin_open),
            attach_stdin: Some(cfg.stdin_open && !cfg.detach),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            healthcheck: cfg.healthcheck_body(),
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
    pub async fn logs_tail(&self, id: &str, tail: &str, timestamps: bool) -> anyhow::Result<String> {
        self.container_logs(id, tail, timestamps).await
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

    /// Applies new resource limits to an existing container via the engine's
    /// update endpoint. Limits can be changed but not removed — the engine
    /// treats zero/absent values as "leave unchanged".
    pub async fn update_container_limits(
        &self,
        id: &str,
        limits: &ResourceLimits,
    ) -> anyhow::Result<()> {
        let body = crate::docker::models::update_body_from_limits(limits)
            .map_err(|e| anyhow::anyhow!(e))?;
        self.docker.update_container(id, body).await?;
        Ok(())
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
            .map(|n| crate::docker::models::RestartPolicy::from_bollard_name(&n).to_string())
            .unwrap_or_default(),
        command_vec: info
            .path
            .clone()
            .into_iter()
            .chain(info.args.clone().unwrap_or_default())
            .collect(),
        entrypoint: config
            .and_then(|c| c.entrypoint.clone())
            .unwrap_or_default(),
        port_mappings: port_mappings_from_inspect(info),
        env_pairs: config
            .and_then(|c| c.env.clone())
            .unwrap_or_default()
            .iter()
            .map(|e| {
                EnvVar::parse(e).unwrap_or_else(|_| EnvVar {
                    key: e.clone(),
                    value: String::new(),
                })
            })
            .collect(),
        volume_mounts: info
            .host_config
            .as_ref()
            .and_then(|h| h.binds.clone())
            .unwrap_or_default()
            .iter()
            .filter_map(|b| VolumeMount::parse(b).ok())
            .collect(),
        labels: config
            .and_then(|c| c.labels.clone())
            .unwrap_or_default()
            .iter()
            .map(|(k, v)| EnvVar::new(k, v))
            .collect(),
        hostname: config.and_then(|c| c.hostname.clone()).unwrap_or_default(),
        workdir: config
            .and_then(|c| c.working_dir.clone())
            .unwrap_or_default(),
        user: config.and_then(|c| c.user.clone()).unwrap_or_default(),
        network: info
            .host_config
            .as_ref()
            .and_then(|h| h.network_mode.clone())
            .unwrap_or_default(),
        resources: ResourceLimits {
            memory_mb: info
                .host_config
                .as_ref()
                .and_then(|h| h.memory)
                .filter(|m| *m > 0)
                .map(|m| (m / (1024 * 1024)).to_string())
                .unwrap_or_default(),
            cpus: info
                .host_config
                .as_ref()
                .and_then(|h| h.nano_cpus)
                .filter(|c| *c > 0)
                .map(|c| {
                    let cores = c as f64 / 1_000_000_000.0;
                    if (cores.fract()).abs() < f64::EPSILON {
                        format!("{}", cores as i64)
                    } else {
                        format!("{cores}")
                    }
                })
                .unwrap_or_default(),
            cpuset_cpus: info
                .host_config
                .as_ref()
                .and_then(|h| h.cpuset_cpus.clone())
                .unwrap_or_default(),
            cpu_shares: info
                .host_config
                .as_ref()
                .and_then(|h| h.cpu_shares)
                .filter(|v| *v > 0)
                .map(|v| v.to_string())
                .unwrap_or_default(),
            pids_limit: info
                .host_config
                .as_ref()
                .and_then(|h| h.pids_limit)
                .filter(|v| *v > 0)
                .map(|v| v.to_string())
                .unwrap_or_default(),
            restart_policy: info
                .host_config
                .as_ref()
                .and_then(|h| h.restart_policy.clone())
                .and_then(|r| r.name)
                .map(|n| crate::docker::models::RestartPolicy::from_bollard_name(&n).to_string())
                .unwrap_or_default(),
            restart_max_retries: info
                .host_config
                .as_ref()
                .and_then(|h| h.restart_policy.clone())
                .and_then(|r| r.maximum_retry_count)
                .filter(|v| *v > 0)
                .map(|v| v.to_string())
                .unwrap_or_default(),
        },
        restart_max_retries: info
            .host_config
            .as_ref()
            .and_then(|h| h.restart_policy.clone())
            .and_then(|r| r.maximum_retry_count)
            .filter(|v| *v > 0)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        healthcheck: healthcheck_from_inspect(config),
        privileged: info
            .host_config
            .as_ref()
            .and_then(|h| h.privileged)
            .unwrap_or(false),
        tty: config.and_then(|c| c.tty).unwrap_or(false),
        stdin_open: config.and_then(|c| c.open_stdin).unwrap_or(false),
        auto_remove: info
            .host_config
            .as_ref()
            .and_then(|h| h.auto_remove)
            .unwrap_or(false),
        read_only_rootfs: info
            .host_config
            .as_ref()
            .and_then(|h| h.readonly_rootfs)
            .unwrap_or(false),
        limits: limits_from_host_config(info.host_config.as_ref()),
    }
}

/// Published ports from `inspect`, rendered back into structured mappings.
fn port_mappings_from_inspect(
    info: &bollard::models::ContainerInspectResponse,
) -> Vec<PortMapping> {
    let Some(net) = info.network_settings.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (spec, binding) in net.ports.clone().unwrap_or_default().iter() {
        // `spec` is `"80/tcp"`.
        let (container_port, proto) = spec
            .split_once('/')
            .map(|(p, pr)| (p.to_string(), pr.to_string()))
            .unwrap_or_else(|| (spec.clone(), "tcp".to_string()));
        let protocol = PortProtocol::parse(&proto).unwrap_or_default();
        match binding {
            Some(bindings) if !bindings.is_empty() => {
                for b in bindings {
                    out.push(PortMapping {
                        host_ip: b.host_ip.clone().unwrap_or_default(),
                        host_port: b.host_port.clone().unwrap_or_default(),
                        container_port: container_port.clone(),
                        protocol,
                    });
                }
            }
            _ => out.push(PortMapping {
                host_ip: String::new(),
                host_port: String::new(),
                container_port,
                protocol,
            }),
        }
    }
    out.sort_by(|a, b| {
        a.container_port
            .cmp(&b.container_port)
            .then(a.host_port.cmp(&b.host_port))
    });
    out
}

/// Health check from `inspect`, mapped back into the form's shape.
fn healthcheck_from_inspect(config: Option<&bollard::models::ContainerConfig>) -> Healthcheck {
    let Some(hc) = config.and_then(|c| c.healthcheck.clone()) else {
        return Healthcheck::default();
    };
    let test = hc.test.clone().unwrap_or_default();
    if test.is_empty() || test == ["NONE"] {
        return Healthcheck::default();
    }
    Healthcheck {
        test,
        interval_s: hc
            .interval
            .filter(|v| *v > 0)
            .map(ns_to_duration)
            .unwrap_or_default(),
        timeout_s: hc
            .timeout
            .filter(|v| *v > 0)
            .map(ns_to_duration)
            .unwrap_or_default(),
        retries: hc
            .retries
            .filter(|v| *v > 0)
            .map(|v| v.to_string())
            .unwrap_or_default(),
        start_period_s: hc
            .start_period
            .filter(|v| *v > 0)
            .map(ns_to_duration)
            .unwrap_or_default(),
    }
}

/// Render nanoseconds back into a compact Go-style duration string.
fn ns_to_duration(ns: i64) -> String {
    const S: i64 = 1_000_000_000;
    const MS: i64 = 1_000_000;
    if ns >= S && ns % S == 0 {
        format!("{}s", ns / S)
    } else {
        format!("{}ms", ns / MS)
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

fn normalize_tcp_uri(uri: &str) -> anyhow::Result<String> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        anyhow::bail!("TCP URI cannot be empty");
    }
    if trimmed.starts_with("tcp://")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
    {
        return Ok(trimmed.to_string());
    }
    if trimmed.contains("://") {
        anyhow::bail!("Unsupported scheme for TCP endpoint: {trimmed}");
    }
    if trimmed.contains(":") {
        return Ok(format!("tcp://{trimmed}"));
    }
    Ok(format!("tcp://{trimmed}:2375"))
}

fn normalize_ssh_uri(uri: &str) -> anyhow::Result<String> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        anyhow::bail!("SSH URI cannot be empty");
    }
    if trimmed.starts_with("ssh://") {
        return Ok(trimmed.to_string());
    }
    if trimmed.contains("://") {
        anyhow::bail!("Unsupported scheme for SSH endpoint: {trimmed}");
    }
    Ok(format!("ssh://{trimmed}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::models::{EnvVar, PortMapping};

    #[test]
    fn kind_resources_are_never_prune_candidates() {
        // Stopped containers are candidates, except kind nodes.
        assert!(pruneable_container("exited", None));
        assert!(pruneable_container("created", None));
        assert!(!pruneable_container("exited", Some("dev")));
        assert!(!pruneable_container("running", None));
        // Networks: kind and the built-ins stay.
        assert!(!pruneable_network("kind", true));
        assert!(!pruneable_network("bridge", false));
        assert!(!pruneable_network("host", false));
        assert!(!pruneable_network("none", false));
        assert!(pruneable_network("my-app_net", false));
    }

    #[test]
    fn run_config_validation() {
        let mut cfg = RunConfig::default();
        assert!(cfg.validate().is_err(), "image is required");
        cfg.image = "nginx:latest".to_string();
        assert!(cfg.validate().is_ok());
        cfg.ports
            .push(PortMapping::parse("bad").unwrap_or_default());
        assert!(cfg.validate().is_err());
        cfg.ports.clear();
        cfg.ports.push(PortMapping::parse("8080:80").unwrap());
        assert!(cfg.validate().is_ok());
        cfg.env.push(EnvVar::parse("a=1").unwrap());
        cfg.env.push(EnvVar::parse("b=2").unwrap());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn run_config_accepts_a_partial_wizard_payload() {
        // The Run wizard posts only the fields it edits. Everything else must
        // fall back to Default instead of failing the whole invocation with
        // "missing field ..." — that made the port-forwarding form unusable.
        let cfg: RunConfig = serde_json::from_value(serde_json::json!({
            "image": "nginx:latest",
            "ports": [{ "container_port": "80" }]
        }))
        .expect("partial payload deserializes");

        assert_eq!(cfg.ports.len(), 1);
        assert_eq!(cfg.ports[0].container_port, "80");
        // Omitted knobs keep the engine's own defaults.
        assert_eq!(cfg.ports[0].host_ip, "");
        assert_eq!(cfg.ports[0].host_port, "");
        assert_eq!(cfg.ports[0].protocol, PortProtocol::Tcp);
        assert!(cfg.command.is_empty());
        assert!(!cfg.auto_remove);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn parse_lines_trims_and_skips_empty() {
        let out = RunConfig::parse_lines("  a=1\n\n  \n b=2\n");
        assert_eq!(out, vec!["a=1".to_string(), "b=2".to_string()]);
    }

    #[test]
    fn format_ports_joins_like_docker_ps() {
        let ports = vec![
            bollard::models::PortSummary {
                ip: Some("0.0.0.0".to_string()),
                private_port: 80,
                public_port: Some(8080),
                typ: Some(bollard::models::PortSummaryTypeEnum::TCP),
            },
            bollard::models::PortSummary {
                ip: Some("127.0.0.1".to_string()),
                private_port: 5432,
                public_port: Some(5432),
                typ: Some(bollard::models::PortSummaryTypeEnum::TCP),
            },
            bollard::models::PortSummary {
                private_port: 443,
                public_port: None,
                typ: Some(bollard::models::PortSummaryTypeEnum::TCP),
                ..Default::default()
            },
        ];
        assert_eq!(
            DockerClient::format_ports_static(&ports),
            "0.0.0.0:8080->80/tcp, 127.0.0.1:5432->5432/tcp, 443/tcp"
        );
    }

    #[test]
    fn published_ports_keep_only_host_bindings() {
        let ports = vec![
            bollard::models::PortSummary {
                ip: Some(String::new()),
                private_port: 80,
                public_port: Some(8080),
                typ: Some(bollard::models::PortSummaryTypeEnum::TCP),
            },
            bollard::models::PortSummary {
                private_port: 443,
                public_port: None,
                ..Default::default()
            },
            bollard::models::PortSummary {
                ip: Some("127.0.0.1".to_string()),
                private_port: 5353,
                public_port: Some(5353),
                typ: Some(bollard::models::PortSummaryTypeEnum::UDP),
            },
        ];

        let published = DockerClient::published_ports_static(&ports);
        // Exposed-only ports are not published, so they carry no exposure risk.
        assert_eq!(published.len(), 2);
        // An empty bind address is preserved: the UI treats it as "every
        // interface", which is what the engine means by it.
        assert_eq!(published[0].host_ip, "");
        assert_eq!(published[0].host_port, 8080);
        assert_eq!(published[1].host_ip, "127.0.0.1");
        assert_eq!(published[1].protocol, "udp");
    }

    #[test]
    fn tcp_uri_normalization_adds_scheme_and_default_port() {
        assert_eq!(
            normalize_tcp_uri("localhost:2375").unwrap(),
            "tcp://localhost:2375"
        );
        assert_eq!(
            normalize_tcp_uri(" 10.0.0.5:2376 ").unwrap(),
            "tcp://10.0.0.5:2376"
        );
        assert_eq!(
            normalize_tcp_uri("docker.example.com").unwrap(),
            "tcp://docker.example.com:2375"
        );
        // An explicit scheme is passed through untouched.
        assert_eq!(
            normalize_tcp_uri("tcp://host:2375").unwrap(),
            "tcp://host:2375"
        );
        assert_eq!(
            normalize_tcp_uri("http://host:9000").unwrap(),
            "http://host:9000"
        );
    }

    #[test]
    fn tcp_uri_normalization_rejects_an_empty_or_foreign_value() {
        assert!(normalize_tcp_uri("   ").is_err());
        assert!(normalize_tcp_uri("unix:///var/run/docker.sock").is_err());
        assert!(normalize_tcp_uri("ssh://user@host").is_err());
    }

    #[test]
    fn ssh_uri_normalization_adds_the_scheme() {
        assert_eq!(normalize_ssh_uri("user@host").unwrap(), "ssh://user@host");
        assert_eq!(
            normalize_ssh_uri("ssh://user@host:22").unwrap(),
            "ssh://user@host:22"
        );
        assert!(normalize_ssh_uri("").is_err());
        assert!(normalize_ssh_uri("tcp://host:2375").is_err());
    }
}
