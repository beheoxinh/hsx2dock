//! Stdio transport: the `hsx2dock-mcp` binary's whole reason to exist.
//!
//! An MCP client (Claude Desktop, Claude Code, Cursor, …) launches the binary
//! as a child process and speaks JSON-RPC over its stdin/stdout. There is no
//! network, no port, and no token — the client owning the process *is* the
//! authorization boundary, which is why this is the recommended way in and the
//! HTTP transport in [`super::http`] is opt-in.
//!
//! Everything a tool actually does lives in [`super::server::McpServer`]; this
//! module only wires it to the process's standard streams and translates the
//! shutdown reason into a readable line on stderr.

use std::sync::Arc;

use rmcp::ServiceExt as _;

use crate::mcp::server::McpServer;
use crate::state::AppState;

/// Serves MCP over stdin/stdout until the client disconnects.
///
/// `AppState` is built by the caller (the binary), so this stays testable and
/// free of process-wide setup.
pub async fn serve(state: Arc<AppState>) -> Result<(), String> {
    let server = McpServer::new(state, None);

    // `serve` performs the `initialize` handshake before returning, so a
    // client that cannot agree on a protocol version fails here rather than
    // midway through its first tool call.
    let running = server
        .serve(rmcp::transport::io::stdio())
        .await
        .map_err(|e| format!("MCP handshake failed: {e}"))?;

    let reason = running
        .waiting()
        .await
        .map_err(|e| format!("MCP server task failed: {e}"))?;

    // stderr, never stdout: stdout is the JSON-RPC channel and one stray byte
    // there corrupts the stream.
    eprintln!("hsx2dock-mcp: client disconnected ({reason:?})");
    Ok(())
}
