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
    /// Whether a secret is stored — never the secret itself.
    pub has_password: bool,
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

fn to_status(cfg: &RegistryConfig) -> RegistryStatus {
    RegistryStatus {
        id: cfg.id.clone(),
        name: cfg.name.clone(),
        host: cfg.host.clone(),
        username: cfg.username.clone(),
        insecure: cfg.insecure,
        has_password: !cfg.auth.is_empty(),
    }
}

#[tauri::command]
pub async fn list_registries(state: State<'_, AppState>) -> Result<Vec<RegistryStatus>, String> {
    let cfg = state.config.read().await;
    Ok(cfg.registries.iter().map(to_status).collect())
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
    let outcome = probe_registry(&host, &input.username, &input.password, input.insecure).await;
    Ok(RegistryTestResult {
        ok: outcome.0,
        message: outcome.1,
        latency_ms: started.elapsed().as_millis() as u64,
    })
}

/// URL candidates to probe for a registry's v2 API. `docker.io` maps to the
/// real registry host — `docker.io/v2/` would hit the marketing site and
/// report a bogus success.
pub fn probe_urls(host: &str, insecure: bool) -> Vec<String> {
    let effective = match host {
        "docker.io" | "index.docker.io" => "registry-1.docker.io",
        other => other,
    };
    if insecure {
        // Self-signed TLS first, then plain HTTP (docker's
        // --insecure-registry means "allow HTTP", so cover both
        // personal-Harbor cases).
        vec![
            format!("https://{effective}/v2/"),
            format!("http://{effective}/v2/"),
        ]
    } else {
        vec![format!("https://{effective}/v2/")]
    }
}

async fn probe_registry(
    host: &str,
    username: &str,
    password: &str,
    insecure: bool,
) -> (bool, String) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .danger_accept_invalid_certs(insecure)
        .build();
    let client = match client {
        Ok(c) => c,
        Err(e) => return (false, format!("Failed to build HTTP client: {e}")),
    };

    let urls = probe_urls(host, insecure);

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
            format!("{} ({})", &tail[..end], full.split(':').last().unwrap_or("").trim())
        }
        None => full,
    }
}

/// Upserts a login. An empty password on edit keeps the stored secret.
#[tauri::command]
pub async fn save_registry_auth(
    state: State<'_, AppState>,
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
            Some(prev) => prev.auth.clone(),
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
    };

    match cfg.registries.iter_mut().find(|r| r.id == entry.id) {
        Some(slot) => *slot = entry,
        None => cfg.registries.push(entry),
    }
    state.save_config().await
}

#[tauri::command]
pub async fn remove_registry_auth(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut cfg = state.config.write().await;
    let before = cfg.registries.len();
    cfg.registries.retain(|r| r.id != id);
    if cfg.registries.len() == before {
        return Err("No registry login with that id".into());
    }
    drop(cfg);
    state.save_config().await
}

#[cfg(test)]
mod tests {
    use super::*;

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
            probe_urls("docker.io", false),
            vec!["https://registry-1.docker.io/v2/".to_string()]
        );
        assert_eq!(
            probe_urls("ghcr.io", false),
            vec!["https://ghcr.io/v2/".to_string()]
        );
        assert_eq!(
            probe_urls("harbor.lan:8443", true),
            vec![
                "https://harbor.lan:8443/v2/".to_string(),
                "http://harbor.lan:8443/v2/".to_string(),
            ]
        );
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
        };
        assert_eq!(
            cfg.credentials().password.as_deref(),
            Some(""),
            "broken stored auth must not panic and must yield an empty password"
        );
    }
}
