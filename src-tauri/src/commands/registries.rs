//! Registry login management — Docker Hub, ghcr.io, GitLab Container
//! Registry, personal Harbor, and any community registry that speaks the
//! Docker Registry v2 API.
//!
//! Logins live in `AppConfig.registries` (passwords stored as
//! base64(username:password), the same obfuscation docker's own
//! `config.json` uses) and are matched to pull/push operations by the
//! registry host extracted from the image reference.

use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

use crate::config::RegistryConfig;
use crate::state::AppState;

#[derive(Clone, Serialize)]
pub struct RegistryStatus {
    pub id: String,
    pub name: String,
    pub host: String,
    pub username: String,
    pub insecure: bool,
    pub allow_http: bool,
    /// Whether a secret is stored — never the secret itself.
    pub has_password: bool,
    /// True for the login new pushes default to.
    pub is_default: bool,
}

#[derive(Clone, Deserialize)]
pub struct RegistryInput {
    /// Empty for a new login.
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub username: String,
    /// Plain password or token; stored base64-encoded. Empty on edit = keep
    /// the stored secret.
    pub password: String,
    pub insecure: bool,
    /// See [`RegistryConfig::allow_http`].
    #[serde(default)]
    pub allow_http: bool,
}

#[derive(Clone, Serialize)]
pub struct RegistryTestResult {
    pub ok: bool,
    pub message: String,
    pub latency_ms: u64,
}

/// Normalizes a registry host: trims, lowercases, strips a scheme and a
/// trailing slash. `"HTTPS://GHCR.IO/"` → `"ghcr.io"`.
pub fn normalize_host(raw: &str) -> String {
    let mut h = raw.trim().to_lowercase();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = h.strip_prefix(scheme) {
            h = rest.to_string();
            break;
        }
    }
    h.trim_end_matches('/').trim().to_string()
}

/// Encodes `username:password` the way registry auth expects.
pub fn encode_auth(username: &str, password: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"))
}

fn to_status(cfg: &RegistryConfig, default_id: &str) -> RegistryStatus {
    RegistryStatus {
        id: cfg.id.clone(),
        name: cfg.name.clone(),
        host: cfg.host.clone(),
        username: cfg.username.clone(),
        insecure: cfg.insecure,
        allow_http: cfg.allow_http,
        has_password: !cfg.auth.is_empty(),
        is_default: !default_id.is_empty() && cfg.id == default_id,
    }
}

/// Resolves the login new operations should use: the explicit default when it
/// still exists, else the first saved login. Returns `None` with no logins.
pub fn default_registry(cfg: &crate::config::AppConfig) -> Option<&RegistryConfig> {
    if !cfg.default_registry.is_empty() {
        if let Some(found) = cfg.registries.iter().find(|r| r.id == cfg.default_registry) {
            return Some(found);
        }
    }
    cfg.registries.first()
}

#[tauri::command]
pub async fn list_registries(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<RegistryStatus>, String> {
    list_registries_core(&state).await
}

/// App-free core of [`list_registries`], shared with the MCP server.
pub(crate) async fn list_registries_core(state: &AppState) -> Result<Vec<RegistryStatus>, String> {
    let cfg = state.config.read().await;
    let default_id = default_registry(&cfg).map(|r| r.id.as_str()).unwrap_or("");
    Ok(cfg
        .registries
        .iter()
        .map(|r| to_status(r, default_id))
        .collect())
}

/// Marks one saved login as the default for new pushes. An empty id clears
/// the selection, which falls back to the first saved login.
#[tauri::command]
pub async fn set_default_registry(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<(), String> {
    set_default_registry_core(&state, &id).await
}

/// App-free core of [`set_default_registry`].
pub(crate) async fn set_default_registry_core(state: &AppState, id: &str) -> Result<(), String> {
    let mut cfg = state.config.write().await;
    if !id.is_empty() && !cfg.registries.iter().any(|r| r.id == id) {
        return Err("No registry login with that id".into());
    }
    let mut next = cfg.clone();
    next.default_registry = id.to_string();
    next.save().await.map_err(|e| e.to_string())?;
    *cfg = next;
    Ok(())
}

/// Probes `GET /v2/` with the given credentials. Every registry that speaks
/// the Docker v2 API answers 200 with valid auth and 401 without it.
#[tauri::command]
pub async fn test_registry_auth(input: RegistryInput) -> Result<RegistryTestResult, String> {
    let host = normalize_host(&input.host);
    if host.is_empty() {
        return Err("Registry host is required".into());
    }
    let started = Instant::now();
    let outcome = probe_registry(
        &host,
        &input.username,
        &input.password,
        input.insecure,
        input.allow_http,
    )
    .await;
    Ok(RegistryTestResult {
        ok: outcome.0,
        message: outcome.1,
        latency_ms: started.elapsed().as_millis() as u64,
    })
}

/// URL candidates to probe for a registry's v2 API. `docker.io` maps to the
/// real registry host — `docker.io/v2/` would hit the marketing site and
/// report a bogus success.
///
/// Basic-auth credentials must not travel in clear text to a remote host by
/// accident, so plain HTTP is offered for a loopback registry with `insecure`
/// (a local dev registry), and for a remote one only with the explicit
/// `allow_http` opt-in. A self-signed registry stays on HTTPS with `insecure`.
pub fn probe_urls(host: &str, insecure: bool, allow_http: bool) -> Vec<String> {
    let effective = match host {
        "docker.io" | "index.docker.io" => "registry-1.docker.io",
        other => other,
    };
    let mut urls = vec![format!("https://{effective}/v2/")];
    if (insecure && is_loopback_host(effective)) || (allow_http && effective == host) {
        urls.push(format!("http://{effective}/v2/"));
    }
    urls
}

/// True for `localhost`, `*.localhost` and loopback IP literals, with or
/// without a port: `localhost:5000`, `127.0.0.1:5000`, `[::1]:5000`.
fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(bracketed) = host.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        match host.rsplit_once(':') {
            Some((name, _port)) if !name.contains(':') => name,
            _ => host,
        }
    };
    let lower = name.to_ascii_lowercase();
    lower == "localhost"
        || lower.ends_with(".localhost")
        || lower
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

async fn probe_registry(
    host: &str,
    username: &str,
    password: &str,
    insecure: bool,
    allow_http: bool,
) -> (bool, String) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .danger_accept_invalid_certs(insecure)
        .build();
    let client = match client {
        Ok(c) => c,
        Err(e) => return (false, format!("Failed to build HTTP client: {e}")),
    };

    let urls = probe_urls(host, insecure, allow_http);

    let mut last_error = String::from("no URL attempted");
    for url in urls {
        let mut req = client.get(&url);
        if !username.is_empty() {
            req = req.basic_auth(username, Some(password));
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return (
                        true,
                        format!("Authenticated at {host} ({})", status.as_u16()),
                    );
                }
                if status.as_u16() == 401 {
                    return (
                        false,
                        format!("Invalid username or password for {host} (401)"),
                    );
                }
                if status.as_u16() == 403 {
                    return (
                        false,
                        format!(
                            "Credentials rejected with 403 at {host} — logged in but not allowed here"
                        ),
                    );
                }
                if status.as_u16() == 404 {
                    return (
                        false,
                        format!("{host} answered 404 on /v2/ — not a Docker registry endpoint"),
                    );
                }
                last_error = format!("{host} answered {}", status.as_u16());
            }
            Err(e) => {
                last_error = format!("Cannot reach {host}: {}", sanitize_reqwest_error(&e));
            }
        }
    }
    (false, last_error)
}

/// Strips URLs out of reqwest errors so messages stay readable.
fn sanitize_reqwest_error(err: &reqwest::Error) -> String {
    let full = err.to_string();
    // reqwest messages embed the full URL; keep the host, drop the path noise.
    match full.find("http") {
        Some(pos) => {
            let tail = &full[pos..];
            let end = tail
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(tail.len());
            format!(
                "{} ({})",
                &tail[..end],
                full.split(':').next_back().unwrap_or("").trim()
            )
        }
        None => full,
    }
}

/// Upserts a login. An empty password on edit keeps the stored secret.
#[tauri::command]
pub async fn save_registry_auth(
    state: State<'_, Arc<AppState>>,
    input: RegistryInput,
) -> Result<(), String> {
    save_registry_auth_core(&state, input).await
}

/// App-free core of [`save_registry_auth`].
pub(crate) async fn save_registry_auth_core(
    state: &AppState,
    input: RegistryInput,
) -> Result<(), String> {
    let host = normalize_host(&input.host);
    if host.is_empty() {
        return Err("Registry host is required".into());
    }
    let username = input.username.trim().to_string();
    if username.is_empty() {
        return Err("Username is required".into());
    }

    let mut cfg = state.config.write().await;
    let existing = input
        .id
        .as_deref()
        .and_then(|id| cfg.registries.iter().find(|r| r.id == id));

    let auth = if input.password.is_empty() {
        match existing {
            // The stored secret belongs to the host it was saved for. Moving
            // the login to another host needs the secret typed again.
            Some(prev) if normalize_host(&prev.host) == host => prev.auth.clone(),
            Some(_) => {
                return Err(format!(
                    "Enter the password or token again: the saved one belongs to a different registry than {host}"
                ))
            }
            None => return Err("Password or access token is required".into()),
        }
    } else {
        encode_auth(&username, &input.password)
    };

    let id = match existing {
        Some(prev) => prev.id.clone(),
        None => {
            let base = if input.name.trim().is_empty() {
                host.replace(['.', ':', '/'], "-")
            } else {
                input
                    .name
                    .trim()
                    .to_lowercase()
                    .replace(|c: char| !c.is_ascii_alphanumeric(), "-")
            };
            let mut id = base.trim_matches('-').to_string();
            if id.is_empty() {
                id = "registry".to_string();
            }
            // Avoid collisions when the same host is saved twice.
            let mut candidate = id.clone();
            let mut n = 2;
            while cfg.registries.iter().any(|r| r.id == candidate) {
                candidate = format!("{id}-{n}");
                n += 1;
            }
            candidate
        }
    };

    let entry = RegistryConfig {
        id,
        name: if input.name.trim().is_empty() {
            host.clone()
        } else {
            input.name.trim().to_string()
        },
        host,
        username,
        auth,
        insecure: input.insecure,
        allow_http: input.allow_http,
    };

    // Save a copy first: on a write error memory must keep the old logins.
    let mut next = cfg.clone();
    match next.registries.iter_mut().find(|r| r.id == entry.id) {
        Some(slot) => *slot = entry,
        None => next.registries.push(entry),
    }
    next.save().await.map_err(|e| e.to_string())?;
    crate::docker::engine::set_insecure_registries(insecure_registry_hosts(&next.registries));
    *cfg = next;
    Ok(())
}

/// Hosts the engine must reach over plain HTTP (`--insecure-registry`): every
/// login that opted in with `allow_http`, deduplicated. Applied on the next
/// engine start.
pub fn insecure_registry_hosts(registries: &[RegistryConfig]) -> Vec<String> {
    let mut hosts: Vec<String> = registries
        .iter()
        .filter(|r| r.allow_http)
        .map(|r| normalize_host(&r.host))
        .filter(|h| !h.is_empty() && !matches!(h.as_str(), "docker.io" | "index.docker.io"))
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

#[tauri::command]
pub async fn remove_registry_auth(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<(), String> {
    remove_registry_auth_core(&state, &id).await
}

/// App-free core of [`remove_registry_auth`].
pub(crate) async fn remove_registry_auth_core(state: &AppState, id: &str) -> Result<(), String> {
    let mut cfg = state.config.write().await;
    let before = cfg.registries.len();
    let mut next = cfg.clone();
    next.registries.retain(|r| r.id != id);
    if next.registries.len() == before {
        return Err("No registry login with that id".into());
    }
    // The removed login may have been the default; clear it so the resolver
    // falls back to the first remaining login instead of a dangling id.
    if next.default_registry == id {
        next.default_registry.clear();
    }
    next.save().await.map_err(|e| e.to_string())?;
    crate::docker::engine::set_insecure_registries(insecure_registry_hosts(&next.registries));
    *cfg = next;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn normalize_host_strips_scheme_case_and_slash() {
        assert_eq!(normalize_host("HTTPS://GHCR.IO/"), "ghcr.io");
        assert_eq!(normalize_host(" http://localhost:5000 "), "localhost:5000");
        assert_eq!(
            normalize_host("harbor.example.com:8443/"),
            "harbor.example.com:8443"
        );
    }

    #[test]
    fn probe_urls_map_docker_hub_to_the_real_registry_host() {
        assert_eq!(
            probe_urls("docker.io", false, false),
            vec!["https://registry-1.docker.io/v2/".to_string()]
        );
        assert_eq!(
            probe_urls("ghcr.io", false, false),
            vec!["https://ghcr.io/v2/".to_string()]
        );
        // `insecure` alone never gives a remote registry plain HTTP.
        assert_eq!(
            probe_urls("harbor.lan:8443", true, false),
            vec!["https://harbor.lan:8443/v2/".to_string()]
        );
        // Plain HTTP is tried for a loopback registry with `insecure`...
        assert_eq!(
            probe_urls("localhost:5000", true, false),
            vec![
                "https://localhost:5000/v2/".to_string(),
                "http://localhost:5000/v2/".to_string(),
            ]
        );
        assert_eq!(probe_urls("127.0.0.1:5000", true, false).len(), 2);
        assert_eq!(probe_urls("localhost:5000", false, false).len(), 1);
        // ...and for a remote one only with the explicit opt-in.
        assert_eq!(
            probe_urls("harbor.lan:5000", false, true),
            vec![
                "https://harbor.lan:5000/v2/".to_string(),
                "http://harbor.lan:5000/v2/".to_string(),
            ]
        );
        // Docker Hub is never probed over HTTP.
        assert_eq!(probe_urls("docker.io", false, true).len(), 1);
    }

    #[test]
    fn insecure_registry_hosts_lists_only_opted_in_logins() {
        let reg = |host: &str, allow_http: bool| RegistryConfig {
            id: host.into(),
            name: host.into(),
            host: host.into(),
            username: "u".into(),
            auth: String::new(),
            insecure: false,
            allow_http,
        };
        let hosts = insecure_registry_hosts(&[
            reg("ghcr.io", false),
            reg("HTTP://Harbor.LAN:5000/", true),
            reg("harbor.lan:5000", true),
            reg("docker.io", true),
        ]);
        assert_eq!(hosts, vec!["harbor.lan:5000".to_string()]);
    }

    #[test]
    fn encode_auth_round_trips_through_bollard_credentials() {
        let cfg = RegistryConfig {
            id: "t".into(),
            name: "Test".into(),
            host: "ghcr.io".into(),
            username: "octocat".into(),
            auth: encode_auth("octocat", "s3cr3t:with:colons"),
            insecure: false,
            allow_http: false,
        };
        let creds = cfg.credentials();
        assert_eq!(creds.username.as_deref(), Some("octocat"));
        assert_eq!(creds.password.as_deref(), Some("s3cr3t:with:colons"));
        assert_eq!(creds.serveraddress.as_deref(), Some("https://ghcr.io"));
    }

    #[test]
    fn docker_hub_credentials_use_the_index_address() {
        let cfg = RegistryConfig {
            id: "t".into(),
            name: "Hub".into(),
            host: "docker.io".into(),
            username: "user".into(),
            auth: encode_auth("user", "pw"),
            insecure: false,
            allow_http: false,
        };
        assert_eq!(
            cfg.credentials().serveraddress.as_deref(),
            Some("https://index.docker.io/v1/")
        );
    }

    #[test]
    fn malformed_auth_decodes_to_empty_password() {
        let cfg = RegistryConfig {
            id: "t".into(),
            name: "Broken".into(),
            host: "example.com".into(),
            username: "user".into(),
            auth: "not-base64!!".into(),
            insecure: false,
            allow_http: false,
        };
        assert_eq!(
            cfg.credentials().password.as_deref(),
            Some(""),
            "broken stored auth must not panic and must yield an empty password"
        );
    }

    #[test]
    fn default_registry_falls_back_to_the_first_login() {
        let reg = |id: &str| RegistryConfig {
            id: id.into(),
            name: id.into(),
            host: format!("{id}.example.com"),
            username: "user".into(),
            auth: encode_auth("user", "pw"),
            insecure: false,
            allow_http: false,
        };
        let mut cfg = AppConfig {
            registries: vec![reg("first"), reg("second")],
            ..AppConfig::default()
        };

        assert_eq!(default_registry(&cfg).map(|r| r.id.as_str()), Some("first"));

        cfg.default_registry = "second".into();
        assert_eq!(
            default_registry(&cfg).map(|r| r.id.as_str()),
            Some("second")
        );

        // A login removed outside the command leaves a dangling id; the
        // resolver must not hand back nothing while logins still exist.
        cfg.default_registry = "gone".into();
        assert_eq!(default_registry(&cfg).map(|r| r.id.as_str()), Some("first"));

        cfg.registries.clear();
        assert!(default_registry(&cfg).is_none());
    }
}
