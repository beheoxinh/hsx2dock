//! MCP (Model Context Protocol) front-end.
//!
//! Exposes the whole app to AI agents: containers, images, volumes, networks,
//! compose projects, kind/Kubernetes clusters, engine lifecycle and
//! self-repair, endpoints, the CLI bridge, and application config.
//!
//! Two front-ends share one implementation: an in-process server (served over
//! stdio by the `hsx2dock-mcp` binary, or streamable HTTP when enabled in
//! Settings). Both build on [`server::McpServer`], which holds an
//! [`Arc<AppState>`] and therefore sees the same engine, config, and endpoints
//! the GUI does.
//!
//! Why `Arc<AppState>` rather than the `#[tauri::command]` layer: Tauri's
//! `State<'r, T>` wraps a private `&'r T` and cannot be constructed outside the
//! IPC dispatcher, so commands are unreachable from here. The MCP tools call
//! the same `AppState` methods those commands call, which also keeps the
//! authorization rules identical (see `tools::ToolSpec::engine_only`).

pub mod args;
pub mod http;
pub mod prompts;
pub mod resources;
pub mod server;
pub mod stdio;
pub mod tools;

/// Identity advertised to clients on `initialize`.
pub const SERVER_NAME: &str = "hsx2dock";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Instructions shown to the agent on connect. Kept short: it is prepended to
/// the model's context on every session, so it earns its tokens or it goes.
pub const INSTRUCTIONS: &str = "\
Hsx2Dock controls a bundled rootless Docker engine (and optional remote \
endpoints) plus kind/Kubernetes clusters.

Conventions:
- Containers are addressed by id or name; a unique name prefix works.
- `*_list` / `*_get` / `*_inspect` tools are read-only.
- Destructive tools (remove, prune, exec, apply…) require `confirm: true`; ask \
the user before setting it. In read-only mode only read tools are listed.
- Engine lifecycle and self-repair tools only apply to the bundled engine. If a \
remote endpoint is active they fail by design — call `endpoint_list` and \
`endpoint_set_active` to switch back first.
- Long-running operations (pull, build, compose up, cluster create) return once \
the operation is accepted; poll the matching `*_list` tool for progress.

Start with `engine_get_status` and `engine_diagnose` to see whether the engine \
is running and healthy before issuing other calls.";
