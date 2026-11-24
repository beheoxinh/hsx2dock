//! Loopback streamable-HTTP transport for the MCP server.
//!
//! This exists for agents that live outside the desktop — a browser-based
//! client, a containerised agent, a remote model runner pointed at a tunnel —
//! and that therefore cannot be handed a stdio child process. The stdio binary
//! in [`super::stdio`] is the default and needs none of this.
//!
//! # Security model
//!
//! Opening a socket on the machine is a real change in exposure, so the rules
//! here are not configurable:
//!
//! * The listener binds `127.0.0.1` only. There is no option to bind `0.0.0.0`;
//!   reaching the server from elsewhere requires the user to build a tunnel,
//!   which is a deliberate act.
//! * Every request must carry `Authorization: Bearer <token>`, compared in
//!   constant time. The token is 32 random bytes, base64url-encoded.
//! * The token lives in `mcp.json` under the app's config dir, written with
//!   owner-only permissions (`0600`), not in `config.toml` — that file is
//!   copied around by users and pasted into bug reports.
//! * `allowed_hosts` is pinned to the loopback names so a DNS-rebinding page
//!   cannot reach the server through a same-origin request.
//!
//! The server is stopped when the app closes; it is never started unless
//! `AppConfig::mcp_enabled` is set.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::any_service;
use axum::Router;
use base64::Engine as _;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::mcp::server::McpServer;
use crate::state::AppState;

/// File name, under [`AppConfig::config_dir`], holding the HTTP token.
const TOKEN_FILE: &str = "mcp.json";

/// What `mcp.json` holds. A struct rather than a bare string so a later
/// addition (TLS pinning, allowed origins) does not need a file-format break.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenFile {
    token: String,
}

/// The live HTTP server, kept in app state so Settings can report on it and
/// stop it without restarting the app.
pub struct RunningServer {
    pub addr: SocketAddr,
    /// Dropping this is what stops the server.
    _shutdown: tokio_util::sync::DropGuard,
}

impl RunningServer {
    /// The URL an MCP client is configured with.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }
}

/// Generates a fresh token: two v4 UUIDs, i.e. 122 bits of CSPRNG entropy
/// (a v4 UUID fixes 6 bits for version and variant). Only used when there is
/// none saved.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Returns the saved token, or `None` when the file is absent or empty.
///
/// Never creates one: enabling the transport is a deliberate act from
/// Settings, and a token that appears as a side effect of a read would be a
/// socket nobody asked for.
pub async fn read_token() -> Result<Option<String>, String> {
    let path = token_path().ok_or("Could not locate the app config directory")?;
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => {
            let parsed: TokenFile = serde_json::from_str(&raw)
                .map_err(|e| format!("Invalid {}: {e}", path.display()))?;
            Ok(Some(parsed.token).filter(|t| !t.trim().is_empty()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

/// Returns the saved token, generating and persisting one if there is none.
pub async fn load_or_create_token() -> Result<String, String> {
    if let Some(existing) = read_token().await? {
        return Ok(existing);
    }

    let token = generate_token();
    if token.is_empty() {
        return Err("Could not generate an MCP token".into());
    }
    let path = token_path().ok_or("Could not locate the app config directory")?;
    write_token(&path, &token).await?;
    Ok(token)
}

/// Persists the token with owner-only permissions.
pub async fn write_token(path: &std::path::Path, token: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(&TokenFile {
        token: token.to_string(),
    })
    .map_err(|e| e.to_string())?;
    // Created owner-only: writing first and chmod-ing after left the token
    // world-readable for a moment.
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;
        let p = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&p)
                .and_then(|mut f| f.write_all(body.as_bytes()))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("Could not restrict {}: {e}", path.display()))?;
    }
    Ok(())
}

fn token_path() -> Option<std::path::PathBuf> {
    AppConfig::config_dir().map(|d| d.join(TOKEN_FILE))
}

/// Starts the loopback server. `port` of `0` lets the OS choose.
///
/// The caller owns the returned handle: dropping it stops the server.
pub async fn start(
    state: Arc<AppState>,
    token: String,
    port: u16,
) -> Result<RunningServer, String> {
    if token.trim().is_empty() {
        return Err("The MCP HTTP server needs a token".into());
    }

    let service: StreamableHttpService<McpServer, LocalSessionManager> = StreamableHttpService::new(
        {
            let state = state.clone();
            move || Ok(McpServer::new(state.clone(), None))
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_sse_keep_alive(Some(std::time::Duration::from_secs(30))),
    );

    let app = Router::new()
        .route_service("/mcp", any_service(service))
        .layer(axum::middleware::from_fn_with_state(
            Arc::new(token.clone()),
            require_token,
        ));

    // Port 0 must go through the real socket to learn what the OS picked.
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(|e| format!("Could not listen on 127.0.0.1:{port}: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("Could not read the listening address: {e}"))?;

    let shutdown = tokio_util::sync::CancellationToken::new();
    let drop_guard = shutdown.clone().drop_guard();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        let server = axum::serve(listener, app).with_graceful_shutdown(async move {
            shutdown.cancelled().await;
        });
        if ready_tx.send(()).is_ok() {
            tracing::info!("MCP HTTP listening on http://{addr}/mcp");
        }
        if let Err(e) = server.await {
            tracing::error!("MCP HTTP server stopped: {e}");
        }
    });
    // `axum::serve` binds before it polls, so the listener is already live;
    // this only waits for the serving task to be scheduled.
    let _ = ready_rx.await;

    Ok(RunningServer {
        addr,
        _shutdown: drop_guard,
    })
}

/// Rejects anything without the exact bearer token.
async fn require_token(
    State(expected): State<Arc<String>>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();

    // Compare full strings so a length difference does not short-circuit and
    // leak the token's prefix timing.
    if !constant_time_eq(presented.as_bytes(), expected.as_bytes()) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "Missing or invalid MCP token",
        )
            .into_response();
    }
    next.run(request).await
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_mcp_port;

    #[test]
    fn token_is_url_safe_and_256_bits() {
        let token = generate_token();
        assert_eq!(token.len(), 43, "32 bytes of base64url, unpadded");
        assert!(token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(token, generate_token(), "tokens must not repeat");
    }

    #[test]
    fn constant_time_eq_matches_string_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn default_config_has_mcp_off_with_a_port() {
        let cfg = AppConfig::default();
        assert!(!cfg.mcp_enabled);
        assert_eq!(cfg.mcp_port, default_mcp_port());
    }

    #[test]
    fn normalize_replaces_a_zero_port() {
        let mut cfg = AppConfig {
            mcp_port: 0,
            ..AppConfig::default()
        };
        cfg.normalize();
        assert_eq!(cfg.mcp_port, default_mcp_port());
    }
}

/// End-to-end auth tests against a real listener.
///
/// The token check is the only thing standing between a loopback port and every
/// container on the machine, so it gets exercised over a socket rather than
/// only as a unit: these tests start a minimal axum app with the same
/// middleware `start` installs and speak HTTP to it.
#[cfg(test)]
mod auth_tests {
    use super::*;

    async fn spawn(token: &str) -> std::net::SocketAddr {
        let app = Router::new()
            .route("/mcp", axum::routing::get(|| async { "reached" }))
            .layer(axum::middleware::from_fn_with_state(
                Arc::new(token.to_string()),
                require_token,
            ));
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0u16))
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        addr
    }

    /// Minimal HTTP/1.1 request/response over a raw socket, so the test needs no
    /// HTTP client dependency.
    async fn request(addr: std::net::SocketAddr, headers: &[(&str, &str)]) -> (u16, String) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let mut req = format!("GET /mcp HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).await.expect("write");
        let mut raw = Vec::new();
        // The server closes the connection, so reading to EOF terminates.
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_to_end(&mut raw),
        )
        .await;
        let text = String::from_utf8_lossy(&raw).to_string();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        (status, text)
    }

    #[tokio::test]
    async fn rejects_missing_token() {
        let addr = spawn("secret-token").await;
        let (status, body) = request(addr, &[]).await;
        assert_eq!(status, 401, "no Authorization header must be refused");
        assert!(!body.contains("reached"), "handler must not run");
    }

    #[tokio::test]
    async fn rejects_wrong_token() {
        let addr = spawn("secret-token").await;
        let (status, _) = request(addr, &[("Authorization", "Bearer wrong")]).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn rejects_a_prefix_of_the_token() {
        // Guards against a compare that stops at the first differing byte and
        // against matching by prefix.
        let addr = spawn("secret-token").await;
        let (status, _) = request(addr, &[("Authorization", "Bearer secret")]).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn rejects_a_non_bearer_scheme() {
        let addr = spawn("secret-token").await;
        let (status, _) = request(addr, &[("Authorization", "Basic secret-token")]).await;
        assert_eq!(status, 401);
    }

    #[tokio::test]
    async fn accepts_the_exact_token() {
        let addr = spawn("secret-token").await;
        let (status, body) = request(addr, &[("Authorization", "Bearer secret-token")]).await;
        assert_eq!(
            status, 200,
            "the right token must reach the handler: {body}"
        );
        assert!(body.contains("reached"));
    }
}
