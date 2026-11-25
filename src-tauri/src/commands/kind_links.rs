//! Cross-cluster service links: a selector-less Service in the target cluster
//! whose EndpointSlice points at the source cluster's NodePort (node IP on
//! the shared `kind` network) or MetalLB LoadBalancer IP. Pods in the target
//! reach `<name>.<namespace>.svc` as if the service were local.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use super::kind::{control_plane_node, kubectl_json, local_client};
use crate::state::AppState;

const LINK_LABEL: &str = "hsx2dock.io/link";
const SOURCE_ANNOTATION: &str = "hsx2dock.io/link-source";
const MODE_ANNOTATION: &str = "hsx2dock.io/link-mode";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkRequest {
    pub source_cluster: String,
    pub source_namespace: String,
    pub source_service: String,
    pub target_cluster: String,
    pub target_namespace: String,
    /// Service name in the target; defaults to the source service name.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LinkPort {
    pub name: String,
    pub port: u16,
    pub remote_port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceLink {
    pub namespace: String,
    pub name: String,
    /// `cluster/namespace/service` of the source.
    pub source: String,
    pub address: String,
    pub ports: Vec<LinkPort>,
    /// How the address was resolved: NodePort or LoadBalancer.
    pub mode: String,
}

/// Remote address and ports a link should target, from the source Service.
/// LoadBalancer IPs win (they survive node changes); NodePort otherwise.
pub(crate) fn resolve_target(svc: &serde_json::Value, node_ip: &str) -> Result<(String, String, Vec<LinkPort>), String> {
    let ty = svc["spec"]["type"].as_str().unwrap_or("ClusterIP");
    let ports = svc["spec"]["ports"].as_array().cloned().unwrap_or_default();
    if ports.is_empty() {
        return Err("the source service has no ports".into());
    }
    let lb_ip = svc["status"]["loadBalancer"]["ingress"]
        .as_array()
        .and_then(|a| a.iter().find_map(|i| i["ip"].as_str()))
        .map(str::to_string);
    let mk = |p: &serde_json::Value, remote: u16, i: usize| LinkPort {
        name: p["name"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| format!("p{i}")),
        port: p["port"].as_u64().unwrap_or(0) as u16,
        remote_port: remote,
        protocol: p["protocol"].as_str().unwrap_or("TCP").to_string(),
    };
    if ty == "LoadBalancer" {
        if let Some(ip) = lb_ip {
            let ports = ports.iter().enumerate().map(|(i, p)| mk(p, p["port"].as_u64().unwrap_or(0) as u16, i)).collect();
            return Ok((ip, "LoadBalancer".into(), ports));
        }
    }
    if ty == "NodePort" || ty == "LoadBalancer" {
        let mut out = Vec::new();
        for (i, p) in ports.iter().enumerate() {
            let np = p["nodePort"].as_u64().ok_or("a service port has no nodePort yet")? as u16;
            out.push(mk(p, np, i));
        }
        return Ok((node_ip.to_string(), "NodePort".into(), out));
    }
    Err(format!(
        "the source service is {ty}; only NodePort or LoadBalancer services are reachable from another cluster (install MetalLB or change the type)"
    ))
}

pub(crate) fn link_manifest(
    name: &str,
    namespace: &str,
    source: &str,
    address: &str,
    mode: &str,
    ports: &[LinkPort],
) -> String {
    let svc_ports: Vec<_> = ports
        .iter()
        .map(|p| serde_json::json!({"name": p.name, "port": p.port, "targetPort": p.remote_port, "protocol": p.protocol}))
        .collect();
    let ep_ports: Vec<_> = ports
        .iter()
        .map(|p| serde_json::json!({"name": p.name, "port": p.remote_port, "protocol": p.protocol}))
        .collect();
    let labels = serde_json::json!({ LINK_LABEL: "true" });
    let svc = serde_json::json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": {"name": name, "namespace": namespace, "labels": labels,
                     "annotations": {SOURCE_ANNOTATION: source, MODE_ANNOTATION: mode}},
        "spec": {"ports": svc_ports}
    });
    let slice = serde_json::json!({
        "apiVersion": "discovery.k8s.io/v1", "kind": "EndpointSlice",
        "metadata": {"name": format!("{name}-link"), "namespace": namespace,
                     "labels": {"kubernetes.io/service-name": name,
                                "endpointslice.kubernetes.io/managed-by": "hsx2dock",
                                LINK_LABEL: "true"}},
        "addressType": "IPv4",
        "ports": ep_ports,
        "endpoints": [{"addresses": [address], "conditions": {"ready": true}}]
    });
    // JSON is valid YAML; `---` separates the two documents.
    format!("{}\n---\n{}\n", svc, slice)
}

fn check_k8s_name(n: &str) -> Result<(), String> {
    let ok = !n.is_empty()
        && n.len() <= 63
        && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !n.starts_with('-')
        && !n.ends_with('-');
    if ok { Ok(()) } else { Err(format!("'{n}' is not a valid Kubernetes name")) }
}

async fn node_internal_ip(state: &AppState, cluster: &str) -> Result<String, String> {
    let node = kubectl_json(state, cluster, &["get", "node", &control_plane_node(cluster), "-o", "json"]).await?;
    node["status"]["addresses"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["type"] == "InternalIP" && x["address"].as_str().is_some_and(|s| s.contains('.'))))
        .and_then(|x| x["address"].as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{cluster} has no IPv4 node address"))
}

#[tauri::command]
pub async fn link_cluster_service(state: State<'_, Arc<AppState>>, req: LinkRequest) -> Result<ServiceLink, String> {
    link_cluster_service_core(&state, req).await
}

pub(crate) async fn link_cluster_service_core(state: &AppState, req: LinkRequest) -> Result<ServiceLink, String> {
    super::kind::check_cluster_name(&req.source_cluster)?;
    super::kind::check_cluster_name(&req.target_cluster)?;
    let name = req.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| req.source_service.clone());
    for n in [&name, &req.source_namespace, &req.source_service, &req.target_namespace] {
        check_k8s_name(n)?;
    }
    if req.source_cluster == req.target_cluster {
        return Err("Source and target are the same cluster; use the service directly".into());
    }
    let svc = kubectl_json(
        state,
        &req.source_cluster,
        &["get", "service", &req.source_service, "-n", &req.source_namespace, "-o", "json"],
    )
    .await?;
    let node_ip = node_internal_ip(state, &req.source_cluster).await?;
    let (address, mode, ports) = resolve_target(&svc, &node_ip)?;
    // Refuse to overwrite a real (non-link) service.
    if let Ok(existing) = kubectl_json(
        state,
        &req.target_cluster,
        &["get", "service", &name, "-n", &req.target_namespace, "-o", "json", "--ignore-not-found"],
    )
    .await
    {
        if existing.is_object() && existing["metadata"]["labels"][LINK_LABEL].as_str() != Some("true") {
            return Err(format!("{}/{name} already exists in {} and is not a link", req.target_namespace, req.target_cluster));
        }
    }
    let ns_yaml = format!("apiVersion: v1\nkind: Namespace\nmetadata:\n  name: {}\n", req.target_namespace);
    let _ = super::kind::apply_yaml_core(state, &req.target_cluster, &ns_yaml).await;
    let source = format!("{}/{}/{}", req.source_cluster, req.source_namespace, req.source_service);
    let yaml = link_manifest(&name, &req.target_namespace, &source, &address, &mode, &ports);
    super::kind::apply_yaml_core(state, &req.target_cluster, &yaml).await?;
    Ok(ServiceLink { namespace: req.target_namespace, name, source, address, ports, mode })
}

#[tauri::command]
pub async fn list_service_links(state: State<'_, Arc<AppState>>, cluster: String) -> Result<Vec<ServiceLink>, String> {
    list_service_links_core(&state, &cluster).await
}

pub(crate) async fn list_service_links_core(state: &AppState, cluster: &str) -> Result<Vec<ServiceLink>, String> {
    super::kind::check_cluster_name(cluster)?;
    let label = format!("{LINK_LABEL}=true");
    let svcs = kubectl_json(state, cluster, &["get", "service", "-A", "-l", &label, "-o", "json"]).await?;
    let slices = kubectl_json(state, cluster, &["get", "endpointslices", "-A", "-l", &label, "-o", "json"]).await?;
    let empty = vec![];
    let slices = slices["items"].as_array().unwrap_or(&empty);
    Ok(svcs["items"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .map(|s| {
            let ns = s["metadata"]["namespace"].as_str().unwrap_or("").to_string();
            let name = s["metadata"]["name"].as_str().unwrap_or("").to_string();
            let slice = slices.iter().find(|e| {
                e["metadata"]["namespace"].as_str() == Some(&ns)
                    && e["metadata"]["labels"]["kubernetes.io/service-name"].as_str() == Some(&name)
            });
            let address = slice
                .and_then(|e| e["endpoints"][0]["addresses"][0].as_str())
                .unwrap_or("")
                .to_string();
            let ports = s["spec"]["ports"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|p| LinkPort {
                            name: p["name"].as_str().unwrap_or("").into(),
                            port: p["port"].as_u64().unwrap_or(0) as u16,
                            remote_port: p["targetPort"].as_u64().unwrap_or(0) as u16,
                            protocol: p["protocol"].as_str().unwrap_or("TCP").into(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            ServiceLink {
                source: s["metadata"]["annotations"][SOURCE_ANNOTATION].as_str().unwrap_or("").into(),
                namespace: ns,
                name,
                address,
                ports,
                mode: s["metadata"]["annotations"][MODE_ANNOTATION].as_str().unwrap_or("").into(),
            }
        })
        .collect())
}

#[tauri::command]
pub async fn unlink_cluster_service(
    state: State<'_, Arc<AppState>>,
    cluster: String,
    namespace: String,
    name: String,
) -> Result<(), String> {
    unlink_cluster_service_core(&state, &cluster, &namespace, &name).await
}

pub(crate) async fn unlink_cluster_service_core(
    state: &AppState,
    cluster: &str,
    namespace: &str,
    name: &str,
) -> Result<(), String> {
    super::kind::check_cluster_name(cluster)?;
    check_k8s_name(namespace)?;
    check_k8s_name(name)?;
    let svc = kubectl_json(state, cluster, &["get", "service", name, "-n", namespace, "-o", "json"]).await?;
    if svc["metadata"]["labels"][LINK_LABEL].as_str() != Some("true") {
        return Err(format!("{namespace}/{name} is not a service link"));
    }
    super::kind::kubectl_raw(state, cluster, &["delete", "service", name, "-n", namespace, "--ignore-not-found"]).await?;
    super::kind::kubectl_raw(
        state,
        cluster,
        &["delete", "endpointslice", &format!("{name}-link"), "-n", namespace, "--ignore-not-found"],
    )
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkCheck {
    pub port: u16,
    pub ok: bool,
    pub message: String,
}

#[tauri::command]
pub async fn check_service_link(
    state: State<'_, Arc<AppState>>,
    cluster: String,
    namespace: String,
    name: String,
) -> Result<Vec<LinkCheck>, String> {
    check_service_link_core(&state, &cluster, &namespace, &name).await
}

/// TCP connect from the target cluster's node to every linked endpoint.
pub(crate) async fn check_service_link_core(
    state: &AppState,
    cluster: &str,
    namespace: &str,
    name: &str,
) -> Result<Vec<LinkCheck>, String> {
    let links = list_service_links_core(state, cluster).await?;
    let link = links
        .into_iter()
        .find(|l| l.namespace == namespace && l.name == name)
        .ok_or_else(|| format!("{namespace}/{name} is not a service link"))?;
    let ip: std::net::Ipv4Addr = link.address.parse().map_err(|_| "the link has no endpoint address")?;
    let client = local_client(state).await?;
    let node = control_plane_node(cluster);
    let mut out = Vec::new();
    for p in link.ports.iter().filter(|p| p.protocol == "TCP") {
        let script = format!(
            "if timeout 3 bash -c '</dev/tcp/{ip}/{}' 2>/dev/null; then echo OK; else echo FAIL; fi",
            p.remote_port
        );
        let r = client.exec_capture(&node, vec!["sh".into(), "-c".into(), script]).await;
        let (ok, message) = match r {
            Ok(s) if s.trim_end().ends_with("OK") => (true, format!("{ip}:{} reachable", p.remote_port)),
            Ok(_) => (false, format!("{ip}:{} did not accept a connection within 3s", p.remote_port)),
            Err(e) => (false, e.to_string()),
        };
        out.push(LinkCheck { port: p.port, ok, message });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodeport_and_loadbalancer_targets() {
        let np = serde_json::json!({"spec": {"type": "NodePort", "ports": [{"name": "http", "port": 80, "nodePort": 30080}]}});
        let (a, m, p) = resolve_target(&np, "172.18.0.2").unwrap();
        assert_eq!((a.as_str(), m.as_str(), p[0].remote_port, p[0].port), ("172.18.0.2", "NodePort", 30080, 80));

        let lb = serde_json::json!({"spec": {"type": "LoadBalancer", "ports": [{"port": 443, "nodePort": 31443}]},
            "status": {"loadBalancer": {"ingress": [{"ip": "172.18.255.200"}]}}});
        let (a, m, p) = resolve_target(&lb, "x").unwrap();
        assert_eq!((a.as_str(), m.as_str(), p[0].remote_port, p[0].name.as_str()), ("172.18.255.200", "LoadBalancer", 443, "p0"));

        // Pending LB falls back to its NodePort.
        let pending = serde_json::json!({"spec": {"type": "LoadBalancer", "ports": [{"port": 443, "nodePort": 31443}]}});
        assert_eq!(resolve_target(&pending, "n").unwrap().1, "NodePort");

        let cip = serde_json::json!({"spec": {"type": "ClusterIP", "ports": [{"port": 80}]}});
        assert!(resolve_target(&cip, "n").is_err());
    }

    #[test]
    fn manifest_has_matching_port_names() {
        let ports = vec![LinkPort { name: "http".into(), port: 80, remote_port: 30080, protocol: "TCP".into() }];
        let y = link_manifest("api", "default", "a/default/api", "172.18.0.2", "NodePort", &ports);
        let docs: Vec<serde_json::Value> = y.split("\n---\n").map(|d| serde_json::from_str(d.trim()).unwrap()).collect();
        assert_eq!(docs[0]["spec"]["ports"][0]["name"], docs[1]["ports"][0]["name"]);
        assert_eq!(docs[1]["metadata"]["labels"]["kubernetes.io/service-name"], "api");
        assert!(docs[0]["spec"]["selector"].is_null());
    }

    #[test]
    fn names_are_validated() {
        assert!(check_k8s_name("my-svc").is_ok());
        assert!(check_k8s_name("My_svc").is_err());
        assert!(check_k8s_name("a;rm").is_err());
    }
}
