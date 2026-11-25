//! The shared local registry: one `registry:2` container (`kind-registry`)
//! that every kind cluster can pull from, following kind's documented
//! "local registry" recipe:
//!  - published on the host at `localhost:5001` (push from the host/engine);
//!  - attached to the `kind` network so nodes reach it as `kind-registry:5000`;
//!  - each node gets `/etc/containerd/certs.d/localhost:5001/hosts.toml`
//!    redirecting `localhost:5001/…` pulls to `http://kind-registry:5000`;
//!  - the `kube-public/local-registry-hosting` ConfigMap (KEP-1755) tells
//!    tools where the registry is.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::kind::local_client;
use crate::docker::kind::CLUSTER_LABEL;
use crate::state::AppState;

pub(crate) const REGISTRY_NAME: &str = "kind-registry";
pub(crate) const REGISTRY_IMAGE: &str = "registry:2";
/// Host port; outside the API range and the ephemeral range pasta drops.
pub(crate) const REGISTRY_HOST_PORT: u16 = 5001;
const REGISTRY_VOLUME: &str = "kind-registry-data";
const KIND_NETWORK: &str = "kind";

/// `localhost:5001`
pub(crate) fn registry_host() -> String {
    format!("localhost:{REGISTRY_HOST_PORT}")
}

/// Whether an image's registry part is the local registry.
pub(crate) fn is_local_registry_host(host: &str) -> bool {
    matches!(
        host,
        h if h == registry_host() || h == format!("127.0.0.1:{REGISTRY_HOST_PORT}")
    )
}

/// What a node's containerd reads for `localhost:5001` pulls.
pub(crate) fn hosts_toml() -> String {
    format!("[host.\"http://{REGISTRY_NAME}:5000\"]\n  capabilities = [\"pull\", \"resolve\"]\n")
}

/// Shell run in each node: writes `hosts.toml` (base64 keeps quoting simple).
pub(crate) fn node_hosts_script() -> String {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(hosts_toml());
    let dir = format!("{}/{}", super::kind_spec::CERTS_D, registry_host());
    format!("mkdir -p '{dir}' && echo '{b64}' | base64 -d > '{dir}/hosts.toml'")
}

/// KEP-1755 ConfigMap advertising the registry to in-cluster tooling.
pub(crate) fn hosting_configmap() -> String {
    format!(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: local-registry-hosting\n  namespace: kube-public\ndata:\n  localRegistryHosting.v1: |\n    host: \"{}\"\n    help: \"https://kind.sigs.k8s.io/docs/user/local-registry/\"\n",
        registry_host()
    )
}

/// `nginx:1.27` → `localhost:5001/nginx:1.27`; `ghcr.io/a/b:t` → `localhost:5001/a/b:t`.
pub(crate) fn local_registry_ref(image: &str) -> Result<String, String> {
    let image = image.trim();
    if image.is_empty() || image.contains(char::is_whitespace) {
        return Err("Image name is required".into());
    }
    let image = image.split('@').next().unwrap_or(image);
    let (first, rest) = image.split_once('/').unwrap_or(("", image));
    let path = if first.contains('.') || first.contains(':') || first == "localhost" {
        rest.to_string()
    } else {
        image.to_string()
    };
    let path = path.strip_prefix("library/").unwrap_or(&path).to_string();
    let last = path.rsplit('/').next().unwrap_or(&path);
    let path = if last.contains(':') { path.clone() } else { format!("{path}:latest") };
    Ok(format!("{}/{path}", registry_host()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryStatus {
    pub exists: bool,
    pub running: bool,
    pub host: String,
    pub image: String,
    /// Clusters whose nodes are on the `kind` network with the registry.
    pub connected_clusters: Vec<String>,
    /// Repositories in the registry's catalog (when it answers).
    pub repositories: Vec<String>,
}

#[tauri::command]
pub async fn registry_status(state: State<'_, Arc<AppState>>) -> Result<RegistryStatus, String> {
    registry_status_core(&state).await
}

pub(crate) async fn registry_status_core(state: &AppState) -> Result<RegistryStatus, String> {
    let client = local_client(state).await?;
    let info = client.docker().inspect_container(REGISTRY_NAME, None).await.ok();
    let running = info
        .as_ref()
        .and_then(|i| i.state.as_ref())
        .and_then(|s| s.running)
        .unwrap_or(false);
    let on_kind = info
        .as_ref()
        .and_then(|i| i.network_settings.as_ref())
        .and_then(|n| n.networks.as_ref())
        .is_some_and(|n| n.contains_key(KIND_NETWORK));
    let mut connected = Vec::new();
    if on_kind {
        for c in super::kind::list_kind_clusters_core(state).await.unwrap_or_default() {
            if c.running && node_wired(&client, &super::kind::control_plane_node(&c.name)).await {
                connected.push(c.name.clone());
            }
        }
    }
    let repositories = if running { catalog().await.unwrap_or_default() } else { Vec::new() };
    Ok(RegistryStatus {
        exists: info.is_some(),
        running,
        host: registry_host(),
        image: REGISTRY_IMAGE.into(),
        connected_clusters: connected,
        repositories,
    })
}

async fn catalog() -> Option<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?;
    let v: serde_json::Value = client
        .get(format!("http://127.0.0.1:{REGISTRY_HOST_PORT}/v2/_catalog?n=500"))
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())?;
    Some(
        v["repositories"]
            .as_array()?
            .iter()
            .filter_map(|r| r.as_str().map(str::to_string))
            .collect(),
    )
}

#[tauri::command]
pub async fn ensure_registry(state: State<'_, Arc<AppState>>) -> Result<RegistryStatus, String> {
    ensure_registry_core(&state).await?;
    registry_status_core(&state).await
}

/// Creates (pulling `registry:2` if needed) and starts the registry. Safe to
/// call repeatedly.
pub(crate) async fn ensure_registry_core(state: &AppState) -> Result<(), String> {
    use futures_util::StreamExt;
    let client = local_client(state).await?;
    let docker = client.docker();
    if let Ok(info) = docker.inspect_container(REGISTRY_NAME, None).await {
        if info.state.and_then(|s| s.running).unwrap_or(false) {
            return Ok(());
        }
        return docker
            .start_container(REGISTRY_NAME, None::<bollard::query_parameters::StartContainerOptions>)
            .await
            .map_err(|e| format!("start {REGISTRY_NAME}: {e}"));
    }
    if docker.inspect_image(REGISTRY_IMAGE).await.is_err() {
        let opts = bollard::query_parameters::CreateImageOptionsBuilder::default()
            .from_image("registry")
            .tag("2")
            .build();
        let mut pull = docker.create_image(Some(opts), None, None);
        while let Some(item) = pull.next().await {
            if let Err(e) = item {
                return Err(format!("pull {REGISTRY_IMAGE}: {e}"));
            }
        }
    }
    let mut ports = HashMap::new();
    ports.insert(
        "5000/tcp".to_string(),
        Some(vec![bollard::models::PortBinding {
            host_ip: Some("127.0.0.1".into()),
            host_port: Some(REGISTRY_HOST_PORT.to_string()),
        }]),
    );
    let mut labels = HashMap::new();
    labels.insert("app.hsx2dock.role".to_string(), "kind-registry".to_string());
    let body = bollard::models::ContainerCreateBody {
        image: Some(REGISTRY_IMAGE.into()),
        labels: Some(labels),
        exposed_ports: Some(vec!["5000/tcp".to_string()]),
        host_config: Some(bollard::models::HostConfig {
            port_bindings: Some(ports),
            binds: Some(vec![format!("{REGISTRY_VOLUME}:/var/lib/registry")]),
            restart_policy: Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let opts = bollard::query_parameters::CreateContainerOptionsBuilder::default()
        .name(REGISTRY_NAME)
        .build();
    docker
        .create_container(Some(opts), body)
        .await
        .map_err(|e| format!("create {REGISTRY_NAME}: {e}"))?;
    docker
        .start_container(REGISTRY_NAME, None::<bollard::query_parameters::StartContainerOptions>)
        .await
        .map_err(|e| format!("start {REGISTRY_NAME}: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn remove_registry(state: State<'_, Arc<AppState>>, delete_data: bool) -> Result<(), String> {
    remove_registry_core(&state, delete_data).await
}

pub(crate) async fn remove_registry_core(state: &AppState, delete_data: bool) -> Result<(), String> {
    let client = local_client(state).await?;
    let docker = client.docker();
    let opts = bollard::query_parameters::RemoveContainerOptionsBuilder::default().force(true).build();
    match docker.remove_container(REGISTRY_NAME, Some(opts)).await {
        Ok(()) => {}
        Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => {}
        Err(e) => return Err(format!("remove {REGISTRY_NAME}: {e}")),
    }
    if delete_data {
        let _ = docker
            .remove_volume(REGISTRY_VOLUME, None::<bollard::query_parameters::RemoveVolumeOptions>)
            .await;
    }
    Ok(())
}

#[tauri::command]
pub async fn connect_registry_to_cluster(
    state: State<'_, Arc<AppState>>,
    cluster: String,
) -> Result<(), String> {
    ensure_registry_core(&state).await?;
    wire_cluster_core(&state, &cluster).await
}

/// Attaches the registry to the `kind` network, writes `hosts.toml` into
/// every node of `cluster` and applies the hosting ConfigMap.
pub(crate) async fn wire_cluster_core(state: &AppState, cluster: &str) -> Result<(), String> {
    let client = local_client(state).await?;
    let docker = client.docker();
    let attached = docker
        .inspect_container(REGISTRY_NAME, None)
        .await
        .map_err(|e| format!("{REGISTRY_NAME} is not created: {e}"))?
        .network_settings
        .and_then(|n| n.networks)
        .is_some_and(|n| n.contains_key(KIND_NETWORK));
    if !attached {
        docker
            .connect_network(
                KIND_NETWORK,
                bollard::models::NetworkConnectRequest {
                    container: REGISTRY_NAME.into(),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| format!("connect {REGISTRY_NAME} to the kind network: {e}"))?;
    }
    let mut filters = HashMap::new();
    filters.insert("label".to_string(), vec![format!("{CLUSTER_LABEL}={cluster}")]);
    let opts = bollard::query_parameters::ListContainersOptionsBuilder::default()
        .filters(&filters)
        .build();
    let nodes = docker.list_containers(Some(opts)).await.map_err(|e| e.to_string())?;
    if nodes.is_empty() {
        return Err(format!("cluster '{cluster}' has no running nodes"));
    }
    let script = node_hosts_script();
    for n in nodes {
        let Some(id) = n.id else { continue };
        client
            .exec_capture(&id, vec!["sh".into(), "-c".into(), script.clone()])
            .await
            .map_err(|e| format!("configure node registry: {e}"))?;
    }
    super::kind::apply_yaml_core(state, cluster, &hosting_configmap()).await?;
    super::kind::invalidate_kind_clusters_cache().await;
    Ok(())
}

/// Starts the registry when a cluster wired to it powers on: the engine stops
/// every container when it shuts down, and pushes or pulls through
/// `localhost:5001` would otherwise fail until someone starts it by hand. A
/// removed registry is left removed.
pub(crate) async fn start_for_cluster(state: &AppState, cluster: &str) {
    let Ok(client) = local_client(state).await else { return };
    let running = match client.docker().inspect_container(REGISTRY_NAME, None).await {
        Ok(info) => info.state.and_then(|s| s.running).unwrap_or(false),
        Err(_) => return,
    };
    if running || !node_wired(&client, &super::kind::control_plane_node(cluster)).await {
        return;
    }
    if let Err(e) = ensure_registry_core(state).await {
        tracing::warn!("could not start the local registry for {cluster}: {e}");
    }
}

/// Whether a node already has the registry `hosts.toml` (cheap marker check).
pub(crate) async fn node_wired(
    client: &crate::docker::client::DockerClient,
    node: &str,
) -> bool {
    let path = format!("{}/{}/hosts.toml", super::kind_spec::CERTS_D, registry_host());
    client
        .exec_capture(node, vec!["sh".into(), "-c".into(), format!("test -f '{path}' && echo yes")])
        .await
        .map(|o| o.trim() == "yes")
        .unwrap_or(false)
}

#[tauri::command]
pub async fn push_to_local_registry(
    app: tauri::AppHandle,
    state: State<'_, Arc<AppState>>,
    image: String,
) -> Result<String, String> {
    push_to_local_registry_core(&state, Some(&app), &image).await
}

/// Tags `image` as `localhost:5001/…` and pushes it; returns the new ref.
pub(crate) async fn push_to_local_registry_core(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    image: &str,
) -> Result<String, String> {
    ensure_registry_core(state).await?;
    let target = local_registry_ref(image)?;
    let (repo, tag) = target.rsplit_once(':').ok_or("bad target ref")?;
    let client = local_client(state).await?;
    let opts = bollard::query_parameters::TagImageOptionsBuilder::default()
        .repo(repo)
        .tag(tag)
        .build();
    client
        .docker()
        .tag_image(image.trim(), Some(opts))
        .await
        .map_err(|e| format!("tag {image} as {target}: {e}"))?;
    super::images::push_image_core(state, app, target.clone()).await?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_are_rewritten_into_the_local_registry() {
        assert_eq!(local_registry_ref("nginx:1.27").unwrap(), "localhost:5001/nginx:1.27");
        assert_eq!(local_registry_ref("nginx").unwrap(), "localhost:5001/nginx:latest");
        assert_eq!(local_registry_ref("me/app").unwrap(), "localhost:5001/me/app:latest");
        assert_eq!(local_registry_ref("ghcr.io/a/b:v1").unwrap(), "localhost:5001/a/b:v1");
        assert_eq!(local_registry_ref("docker.io/library/redis:7").unwrap(), "localhost:5001/redis:7");
        assert_eq!(local_registry_ref("localhost:5001/x:1").unwrap(), "localhost:5001/x:1");
        assert_eq!(local_registry_ref("reg:5000/team/app").unwrap(), "localhost:5001/team/app:latest");
        assert!(local_registry_ref(" ").is_err());
    }

    #[test]
    fn node_script_and_configmap_point_at_the_registry() {
        assert!(hosts_toml().contains("http://kind-registry:5000"));
        let s = node_hosts_script();
        assert!(s.contains("/etc/containerd/certs.d/localhost:5001"));
        assert!(s.contains("base64 -d"));
        let cm = hosting_configmap();
        assert!(cm.contains("namespace: kube-public") && cm.contains("host: \"localhost:5001\""));
        assert!(is_local_registry_host("localhost:5001") && !is_local_registry_host("docker.io"));
    }
}
