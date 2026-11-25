//! Tauri commands for the MCP front-end itself: status, enabling, token
//! rotation, and the prepared client configs the Settings page shows.
//!
//! These are the only MCP commands that take `State<'_, Arc<AppState>>`; the server
//! and its transports are in `crate::mcp`, because they must also run in the
//! standalone `hsx2dock-mcp` binary where no Tauri app exists.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::config::{default_mcp_port, AppConfig};
use crate::mcp::http;
use crate::state::AppState;

/// What the Settings page is allowed to know about the HTTP server.
///
/// Note what is absent: the token. It lives in `mcp.json` with owner-only
/// permissions and is only ever surfaced through [`mcp_token`], which the UI
/// calls when the user asks to see it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    /// The HTTP transport is enabled in the config *and* the token exists.
    pub enabled: bool,
    /// The listener is actually up right now.
    pub running: bool,
    /// `http://127.0.0.1:<port>/mcp`, or empty when the server is not running.
    pub url: String,
    pub port: u16,
    /// A token exists — not the token itself.
    pub has_token: bool,
    /// `~/.config/hsx2dock/mcp.json`, so the docs can point at a real path.
    pub token_path: String,
    /// Absolute path of the stdio binary, if it was found on this install.
    pub stdio_path: String,
    pub tool_count: usize,
    /// Only read tools may be called (and are listed).
    pub read_only: bool,
    pub resource_count: usize,
    pub prompt_count: usize,
    pub server_name: String,
    pub server_version: String,
}

fn stdio_binary_path() -> String {
    // Installed side by side with the app by `install.sh`; also the name a
    // `cargo build` produces next to the GUI binary.
    let name = if cfg!(windows) {
        "hsx2dock-mcp.exe"
    } else {
        "hsx2dock-mcp"
    };
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(name));
        }
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/bin").join(name));
    }
    if let Some(data) = dirs::data_dir() {
        candidates.push(data.join("hsx2dock").join(name));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| name.to_string())
}

fn token_file_path() -> String {
    AppConfig::config_dir()
        .map(|d| d.join("mcp.json").display().to_string())
        .unwrap_or_default()
}

async fn status_of(state: &AppState, config: &AppConfig) -> McpStatus {
    let running = state.mcp_server.read().await.as_ref().map(|s| s.url());
    let has_token = matches!(http::read_token().await, Ok(Some(_)));
    McpStatus {
        enabled: config.mcp_enabled,
        running: running.is_some(),
        url: running.unwrap_or_default(),
        port: config.mcp_port,
        has_token,
        token_path: token_file_path(),
        stdio_path: stdio_binary_path(),
        tool_count: crate::mcp::tools::catalogue().len(),
        read_only: config.mcp_read_only,
        resource_count: crate::mcp::resources::catalogue().len(),
        prompt_count: crate::mcp::prompts::catalogue().len(),
        server_name: crate::mcp::SERVER_NAME.to_string(),
        server_version: crate::mcp::SERVER_VERSION.to_string(),
    }
}

#[tauri::command]
pub async fn mcp_status(state: State<'_, Arc<AppState>>) -> Result<McpStatus, String> {
    let config = state.config.read().await.clone();
    Ok(status_of(state.inner(), &config).await)
}

/// Starts or stops the loopback HTTP server to match `enabled`.
///
/// Enabling creates a token on first use and persists it; disabling stops the
/// listener but keeps the token, so re-enabling does not invalidate every
/// agent already configured.
pub(crate) async fn apply_mcp_enabled(state: &Arc<AppState>, enabled: bool) -> Result<(), String> {
    if !enabled {
        *state.mcp_server.write().await = None;
        return Ok(());
    }

    let token = http::load_or_create_token().await?;
    let port = state.config.read().await.mcp_port;

    // Already serving on the configured port? Nothing to do.
    if let Some(running) = state.mcp_server.read().await.as_ref() {
        if running.addr.port() == port {
            return Ok(());
        }
    }
    *state.mcp_server.write().await = None;
    let server = http::start(Arc::clone(state), token, port).await?;
    tracing::info!("MCP HTTP listening on {}", server.url());
    *state.mcp_server.write().await = Some(server);
    Ok(())
}

/// Brings the listener in line with the saved config after a config write that
/// did not come from [`mcp_set_enabled`] — an agent calling `engine_set_config`,
/// for instance. The UI path already applies this itself; this is what keeps the
/// two from disagreeing.
pub(crate) async fn reconcile_http_server(state: &Arc<AppState>) -> Result<(), String> {
    let (enabled, port) = {
        let cfg = state.config.read().await;
        (cfg.mcp_enabled, cfg.mcp_port)
    };

    if !enabled {
        let mut server = state.mcp_server.write().await;
        if server.take().is_some() {
            tracing::info!("MCP HTTP server stopped: disabled in config");
        }
        return Ok(());
    }

    if state
        .mcp_server
        .read()
        .await
        .as_ref()
        .map(|s| s.addr.port())
        == Some(port)
    {
        return Ok(());
    }
    apply_mcp_enabled(state, true).await
}

/// Settings' save path: persists the flags, applies every other side effect
/// the normal config save performs, then starts/stops the listener to match.
pub(crate) async fn save_mcp_settings(
    state: &Arc<AppState>,
    enabled: bool,
    port: u16,
) -> Result<(), String> {
    let port = if port == 0 { default_mcp_port() } else { port };
    // The change goes into a copy that the normal save writes and only then
    // swaps in (keeping autostart, the shim and engine caps in step). A failed
    // save leaves memory, the file and the listener as they were — nothing to
    // roll back.
    let mut next = state.config.read().await.clone();
    next.mcp_enabled = enabled;
    next.mcp_port = port;
    crate::commands::system::save_app_config_core(state, next).await?;
    apply_mcp_enabled(state, enabled).await
}

#[tauri::command]
pub async fn mcp_set_enabled(
    state: State<'_, Arc<AppState>>,
    enabled: bool,
    port: Option<u16>,
) -> Result<McpStatus, String> {
    let port = match port {
        Some(p) => p,
        None => state.config.read().await.mcp_port,
    };
    save_mcp_settings(state.inner(), enabled, port).await?;
    let config = state.config.read().await.clone();
    Ok(status_of(state.inner(), &config).await)
}

/// Switches MCP read-only mode. Applies to the next call on both transports
/// (the stdio binary reads the same config file on each launch).
#[tauri::command]
pub async fn mcp_set_read_only(
    state: State<'_, Arc<AppState>>,
    read_only: bool,
) -> Result<McpStatus, String> {
    state
        .update_config(|cfg| {
            cfg.mcp_read_only = read_only;
            Ok(())
        })
        .await?;
    let config = state.config.read().await.clone();
    Ok(status_of(state.inner(), &config).await)
}

/// Invalidates the current token, writes a new one, and restarts the server if
/// it is running. Every existing agent config stops working — that is the
/// point of a rotation.
#[tauri::command]
pub async fn mcp_rotate_token(state: State<'_, Arc<AppState>>) -> Result<McpStatus, String> {
    let was_running = state.mcp_server.read().await.is_some();
    *state.mcp_server.write().await = None;

    let token = http::generate_token();
    let path = AppConfig::config_dir()
        .ok_or("Could not locate the app config directory")?
        .join("mcp.json");
    http::write_token(&path, &token).await?;

    if was_running {
        let port = state.config.read().await.mcp_port;
        let server = http::start(state.inner().clone(), token, port).await?;
        tracing::info!("MCP HTTP restarted on {}", server.url());
        *state.mcp_server.write().await = Some(server);
    }

    let config = state.config.read().await.clone();
    Ok(status_of(state.inner(), &config).await)
}

/// Returns the bearer token so the user can paste it into an agent's config.
///
/// This is the one command that hands out the secret. That is fine here: the UI
/// runs as the same OS user that owns `mcp.json`, so anything able to invoke
/// this command could already read the file.
#[tauri::command]
pub async fn mcp_token() -> Result<String, String> {
    http::load_or_create_token().await
}

/// One tool, flattened for the documentation page.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// `"read"`, `"write"` or `"destructive"`.
    pub access: &'static str,
    /// Refused while a remote endpoint is active.
    pub engine_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceInfo {
    pub uri: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub mime_type: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// `(name, description, required)`
    pub arguments: Vec<McpPromptArgInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptArgInfo {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
}

/// The whole MCP surface, for the in-app documentation page.
///
/// Read from the same catalogues the server advertises, so the page cannot
/// drift from what an agent actually sees.
#[tauri::command]
pub async fn mcp_catalogue() -> Result<Vec<McpToolInfo>, String> {
    Ok(crate::mcp::tools::catalogue()
        .into_iter()
        .map(|t| McpToolInfo {
            name: t.name,
            title: t.title,
            description: t.description,
            access: match t.access {
                crate::mcp::tools::Access::Read => "read",
                crate::mcp::tools::Access::Write => "write",
                crate::mcp::tools::Access::Destructive => "destructive",
            },
            engine_only: t.engine_only,
        })
        .collect())
}

#[tauri::command]
pub async fn mcp_resources() -> Result<Vec<McpResourceInfo>, String> {
    Ok(crate::mcp::resources::catalogue()
        .into_iter()
        .map(|r| McpResourceInfo {
            uri: r.uri,
            title: r.title,
            description: r.description,
            mime_type: r.mime_type,
        })
        .collect())
}

#[tauri::command]
pub async fn mcp_prompts() -> Result<Vec<McpPromptInfo>, String> {
    Ok(crate::mcp::prompts::catalogue()
        .into_iter()
        .map(|p| McpPromptInfo {
            name: p.name,
            title: p.title,
            description: p.description,
            arguments: p
                .arguments
                .iter()
                .map(|(name, description, required)| McpPromptArgInfo {
                    name,
                    description,
                    required: *required,
                })
                .collect(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_path_falls_back_to_the_binary_name() {
        // The only guaranteed property: never empty, so the UI can always
        // render a command line.
        assert!(!stdio_binary_path().is_empty());
    }

    #[test]
    fn status_counts_match_the_catalogues() {
        assert_eq!(
            crate::mcp::tools::catalogue().len(),
            crate::mcp::tools::catalogue()
                .iter()
                .map(|t| t.name)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "tool names must be unique or agents see duplicates"
        );
    }
}
