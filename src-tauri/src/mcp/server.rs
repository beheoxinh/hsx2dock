//! The MCP server: one `ServerHandler` over the app's own state.
//!
//! # Why dispatch does not call the Tauri commands
//!
//! Every `#[tauri::command]` takes `State<'_, Arc<AppState>>`, and a `State` can
//! only be built by Tauri's IPC dispatcher — it is a borrow handed in by the
//! macro. The MCP server runs on a different transport (stdio or HTTP) with no
//! IPC involved, so it cannot call those functions. It holds an
//! `Arc<AppState>` and performs the same operations directly.
//!
//! That duplication is deliberate but bounded: anything with real logic lives
//! in an app-free `pub(crate)` core in `commands::*` (the image pull/push/build
//! and kind cluster cores), and this module is the thin dispatch layer over
//! them plus `DockerClient`. When a command gains logic, put it in a core and
//! call it from both sides.
//!
//! # Safety
//!
//! Tools that drive the *bundled* engine's lifecycle are marked `engine_only`
//! in the catalogue and refuse while a remote endpoint is active, mirroring
//! `AppState::engine_control_guard`. Destructive tools carry
//! `destructiveHint: true`. Neither of those is advisory: the checks are
//! enforced here, in [`McpServer::call`].

use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ListPromptsResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, ResourceContents, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer};
use serde_json::{json, Value};

use super::args::Args;
use super::tools::{Access, ToolSpec};
use crate::commands::kind;
use crate::commands::kind_access;
use crate::config::AppConfig;
use crate::docker::models::{BuildOptions, DaemonState, EndpointInput, ResourceLimits, RunConfig};
use crate::state::AppState;

/// The MCP front-end over a running app.
///
/// `app` is present when the server was started by the GUI (so long operations
/// can emit the same progress events the UI listens for) and `None` in the
/// standalone `hsx2dock-mcp` binary.
#[derive(Clone)]
pub struct McpServer {
    pub state: Arc<AppState>,
    pub app: Option<tauri::AppHandle>,
}

impl McpServer {
    pub fn new(state: Arc<AppState>, app: Option<tauri::AppHandle>) -> Self {
        Self { state, app }
    }

    fn app_ref(&self) -> Option<&tauri::AppHandle> {
        self.app.as_ref()
    }
}

/// Turns a handler result into the MCP result the client renders.
///
/// Success is structured JSON (`structuredContent` plus a text rendering for
/// clients that ignore it); failure is a *tool* error, not a protocol error,
/// so the agent actually sees the message.
fn ok(value: Value) -> CallToolResult {
    CallToolResult::structured(value)
}

/// An optional list of strings; anything else reads as empty.
fn opt_strings(args: &Args, key: &str) -> Vec<String> {
    match args.opt_value(key) {
        Some(Value::Array(items)) => {
            items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        }
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s],
        _ => Vec::new(),
    }
}

/// An optional boolean flag (absent = false).
fn opt_flag(args: &Args, key: &str) -> bool {
    matches!(args.opt_value(key), Some(Value::Bool(true)))
}

fn fail(message: impl Into<String>) -> CallToolResult {
    let message = message.into();
    CallToolResult::structured_error(json!({ "error": message }))
}

/// `Result<T, String>` → tool result, mapping the error into `fail`.
fn done<T: serde::Serialize>(result: Result<T, String>) -> CallToolResult {
    match result {
        Ok(v) => match serde_json::to_value(v) {
            Ok(value) => ok(value),
            Err(e) => fail(format!("could not serialize result: {e}")),
        },
        Err(e) => fail(e),
    }
}

/// For tools whose only meaningful output is "it worked".
fn done_unit(result: Result<(), String>) -> CallToolResult {
    match result {
        Ok(()) => ok(json!({ "ok": true })),
        Err(e) => fail(e),
    }
}

/// Extracts the object a tool takes as one argument (a `RunConfig`,
/// `BuildOptions`, `ResourceLimits`, …) and deserializes it.
fn object_arg<T: serde::de::DeserializeOwned>(args: &Args, key: &str) -> Result<T, String> {
    let value = args.req_object(key).map_err(|e| e.to_string())?;
    serde_json::from_value(value)
        .map_err(|e| format!("argument '{key}' does not match the expected object: {e}"))
}

/// Wraps a list-valued resource so a daemon error becomes a readable
/// `{"<key>": [], "unavailable": "..."}` instead of a missing resource.
///
/// The distinction matters to an agent: "no resource at this URI" means the
/// address is wrong and it should stop asking; `unavailable` means the address
/// is right and the engine needs starting.
fn unavailable_on_error<T: serde::Serialize>(key: &str, result: Result<T, String>) -> Value {
    match result {
        Ok(items) => json!({ key: items }),
        Err(e) => json!({ key: [], "unavailable": e }),
    }
}

/// Directories (under home unless absolute) an agent may not use as, or
/// inside, a build context: a build sends the whole context to the daemon, and
/// a `COPY` then puts it in an image the agent can read back.
const AGENT_DENIED_DIRS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".password-store",
    ".local/share/keyrings",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".config/hsx2dock",
    ".config/gcloud",
    ".config/gh",
    "/etc",
    "/root",
    "/proc",
    "/sys",
    "/dev",
    "/boot",
];

/// Whether an agent may read `path` as a build context. The path is resolved
/// first (symlinks, `..`), so a link into a denied directory is caught. The
/// filesystem root and the home directory itself are refused as too broad.
fn agent_path_allowed(path: &std::path::Path) -> Result<(), String> {
    let home = dirs::home_dir();
    agent_path_allowed_in(path, home.as_deref())
}

fn agent_path_allowed_in(
    path: &std::path::Path,
    home: Option<&std::path::Path>,
) -> Result<(), String> {
    let resolved = std::fs::canonicalize(path)
        .map_err(|e| format!("cannot resolve '{}': {e}", path.display()))?;
    let refuse = || {
        Err(format!(
            "'{}' is off limits to agents (credentials or system files); pick a project directory",
            path.display()
        ))
    };
    if resolved.parent().is_none() || home.is_some_and(|h| resolved == h) {
        return refuse();
    }
    for denied in AGENT_DENIED_DIRS {
        let dir = if denied.starts_with('/') {
            std::path::PathBuf::from(denied)
        } else if let Some(h) = home {
            h.join(denied)
        } else {
            continue;
        };
        // Inside a denied dir, or a context that would contain one.
        if resolved.starts_with(&dir) || dir.starts_with(&resolved) {
            return refuse();
        }
    }
    Ok(())
}

/// [`agent_path_allowed`] for the context and the Dockerfile of a build.
fn agent_build_paths_allowed(req: &BuildOptions) -> Result<(), String> {
    let context = std::path::Path::new(&req.context_dir);
    agent_path_allowed(context)?;
    if !req.dockerfile.trim().is_empty() {
        let dockerfile = std::path::Path::new(&req.dockerfile);
        let full = if dockerfile.is_absolute() {
            dockerfile.to_path_buf()
        } else {
            context.join(dockerfile)
        };
        if full.exists() {
            agent_path_allowed(&full)?;
        }
    }
    Ok(())
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
        )
        .with_server_info(rmcp::model::Implementation::new(
            super::SERVER_NAME,
            super::SERVER_VERSION,
        ))
        .with_instructions(super::INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        // In read-only mode only what may actually be called is listed.
        let read_only_mode = self.read_only_mode().await;
        Ok(ListToolsResult::with_all_items(
            super::tools::catalogue()
                .iter()
                .filter(|t| !read_only_mode || t.is_read_only())
                .map(ToolSpec::to_tool)
                .collect(),
        ))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult::with_all_items(
            super::resources::catalogue()
                .iter()
                .map(super::resources::ResourceSpec::to_resource)
                .collect(),
        ))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult::with_all_items(
            super::prompts::catalogue()
                .iter()
                .map(super::prompts::PromptSpec::to_prompt)
                .collect(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        match self.read_resource_body(&request.uri).await {
            Some(text) => {
                // Report the type the catalogue advertises, not a fixed one:
                // `hsx2dock://guide` is markdown, and a client that renders by
                // mime type would otherwise show it as raw JSON.
                let mime = super::resources::catalogue()
                    .iter()
                    .find(|r| r.uri == request.uri)
                    .map(|r| r.mime_type)
                    .unwrap_or("application/json");
                Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
                    vec![ResourceContents::text(text, request.uri).with_mime_type(mime)],
                )))
            }
            None => Err(McpError::resource_not_found(
                format!("no resource at '{}'", request.uri),
                None,
            )),
        }
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResponse, McpError> {
        let empty = serde_json::Map::new();
        let args = request.arguments.as_ref().unwrap_or(&empty);
        match super::prompts::messages(&request.name, args) {
            Some(messages) => Ok(rmcp::model::GetPromptResponse::Complete(
                rmcp::model::GetPromptResult::new(messages),
            )),
            None => Err(McpError::invalid_params(
                format!("no prompt named '{}'", request.name),
                None,
            )),
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(spec) = super::tools::find(&request.name) else {
            return Err(McpError::invalid_params(
                format!("no tool named '{}'", request.name),
                None,
            ));
        };
        // `confirm` is consumed here, not passed on: it is a gate, not an
        // argument of the tool itself.
        let mut raw = request.arguments;
        let confirmed = raw
            .as_mut()
            .and_then(|m| m.remove(super::tools::CONFIRM_ARG))
            .is_some_and(|v| v.as_bool() == Some(true));
        let read_only_mode = self.read_only_mode().await;
        if let Some(reason) = super::tools::access_refusal(spec.access, read_only_mode, confirmed) {
            return Ok(CallToolResponse::Complete(fail(reason)));
        }
        let args = Args::new(raw);

        // Enforced, not advisory: a tool that touches the bundled engine's own
        // files must not run against a remote endpoint.
        if spec.engine_only {
            if let Err(e) = self.state.engine_control_guard().await {
                return Ok(CallToolResponse::Complete(fail(e)));
            }
        }

        Ok(CallToolResponse::Complete(self.call(&spec, &args).await))
    }
}

// ==================== dispatch ====================

impl McpServer {
    /// Routes one validated call to its handler.
    ///
    /// Split from [`ServerHandler::call_tool`] so the match reads as the
    /// contract it is: one arm per catalogue entry, in catalogue order.
    async fn call(&self, spec: &ToolSpec, args: &Args) -> CallToolResult {
        let state = &self.state;
        match spec.name {
            // ------------------------------------------------ engine
            "engine_get_status" => ok(json!({ "state": state.daemon_state() })),
            "engine_get_info" => done(self.engine_info().await),
            "engine_get_disk_usage" => done(self.disk_usage().await),
            // `engine_start` on a running engine used to *stop* it (and the
            // reverse): all three shared the toggle. Start/stop are now
            // idempotent and only act when the engine is in the other state.
            "engine_start" => done(self.set_engine_running(Some(true)).await),
            "engine_stop" => done(self.set_engine_running(Some(false)).await),
            "engine_toggle" => done(self.set_engine_running(None).await),
            "engine_diagnose" => ok(json!(self.diagnose())),
            "engine_repair" => {
                let ids = args
                    .opt_value("check_ids")
                    .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok());
                ok(json!(self.repair(ids)))
            }
            "engine_get_config" => ok(json!(self.app_config().await)),
            "engine_set_config" => match object_arg::<AppConfig>(args, "config") {
                Ok(config) => done_unit(self.save_config(config).await),
                Err(e) => fail(e),
            },
            "engine_get_cli_bridge_status" => ok(json!(self.cli_bridge_status().await)),
            "engine_set_cli_bridge" => match args.req_bool("enabled") {
                Ok(enabled) => done(self.set_cli_bridge(enabled).await),
                Err(e) => fail(e.to_string()),
            },
            "engine_apply_resource_limits" => done(self.apply_resource_limits().await),

            // ------------------------------------------------ containers
            "container_list" => match args.bool_or("all", true) {
                Ok(all) => done(self.list_containers(all).await),
                Err(e) => fail(e.to_string()),
            },
            "container_inspect" => {
                self.with_id(args, |id| async move {
                    let client = state.get_client().await?;
                    client
                        .inspect_container(&id)
                        .await
                        .map_err(|e| e.to_string())
                })
                .await
            }
            "container_logs" => match (args.req_str("id"), args.opt_u64("tail")) {
                (Ok(id), Ok(tail)) => done(
                    self.container_logs(&id, tail.unwrap_or(200).min(5000) as usize)
                        .await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "container_stats" => {
                self.with_id(args, |id| async move {
                    let client = state.get_client().await?;
                    state.container_stats(&client, &id).await.map_err(|e| e.to_string())
                })
                .await
            }
            "container_sizes" => done(self.container_sizes().await),
            "container_start" | "container_stop" | "container_restart" | "container_pause"
            | "container_unpause" => self.container_power(spec.name, args).await,
            "container_rename" => self.rename_container(args).await,
            "container_remove" => match (args.req_str("id"), args.bool_or("force", false)) {
                (Ok(id), Ok(force)) => done_unit(self.remove_container(&id, force).await),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "container_recreate" => {
                match (args.req_str("id"), object_arg::<RunConfig>(args, "config")) {
                    (Ok(id), Ok(config)) => done(
                        crate::commands::containers::recreate_container_core(
                            &self.state,
                            &id,
                            config,
                        )
                        .await
                        .map(|new_id| json!({ "id": new_id })),
                    ),
                    (Err(e), _) => fail(e.to_string()),
                    (_, Err(e)) => fail(e),
                }
            }
            "container_run" => match object_arg::<RunConfig>(args, "config") {
                Ok(config) => done_unit(self.run_container(config).await),
                Err(e) => fail(e),
            },
            "container_update_resources" => {
                match (
                    args.req_str("id"),
                    object_arg::<ResourceLimits>(args, "limits"),
                ) {
                    (Ok(id), Ok(limits)) => done_unit(self.update_limits(&id, &limits).await),
                    (Err(e), _) => fail(e.to_string()),
                    (_, Err(e)) => fail(e),
                }
            }
            "container_exec" => match (args.req_str("id"), args.req_str_vec("cmd")) {
                (Ok(id), Ok(cmd)) => self.exec(&id, cmd).await,
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "container_prune" => done(self.prune_containers().await),

            // ------------------------------------------------ groups
            "group_list" => ok(json!(self.groups().await)),
            "group_save" => match args.opt_value("groups") {
                Some(v) => match serde_json::from_value(v) {
                    Ok(groups) => done_unit(self.save_groups(groups).await),
                    Err(e) => fail(format!("argument 'groups' is not a group list: {e}")),
                },
                None => fail("missing required argument 'groups'"),
            },
            "group_start" | "group_stop" => match args.req_str("id") {
                Ok(id) => {
                    let start = spec.name == "group_start";
                    done(self.set_group_power(&id, start).await)
                }
                Err(e) => fail(e.to_string()),
            },

            // ------------------------------------------------ images
            "image_list" => done(self.list_images().await),
            "image_history" => match args.req_str("name") {
                Ok(name) => done(self.image_history(&name).await),
                Err(e) => fail(e.to_string()),
            },
            "image_search" => match args.req_str("term") {
                Ok(term) => done(self.search_images(&term).await),
                Err(e) => fail(e.to_string()),
            },
            "image_pull" => match args.req_str("image") {
                Ok(image) => done_unit(
                    crate::commands::images::pull_image_core(state, self.app_ref(), image).await,
                ),
                Err(e) => fail(e.to_string()),
            },
            "image_push" => match args.req_str("image") {
                Ok(image) => done_unit(
                    crate::commands::images::push_image_core(state, self.app_ref(), image).await,
                ),
                Err(e) => fail(e.to_string()),
            },
            "image_tag" => match (
                args.req_str("id"),
                args.req_str("repo"),
                args.req_str("tag"),
            ) {
                (Ok(id), Ok(repo), Ok(tag)) => done_unit(self.tag_image(&id, &repo, &tag).await),
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => fail(e.to_string()),
            },
            "image_build" => match object_arg::<BuildOptions>(args, "req") {
                Ok(req) => match agent_build_paths_allowed(&req) {
                    Ok(()) => done_unit(
                        crate::commands::images::build_image_core(state, self.app_ref(), req)
                            .await,
                    ),
                    Err(e) => fail(e),
                },
                Err(e) => fail(e),
            },
            "image_cancel_build" => done_unit(self.cancel_build()),
            "image_validate_build_context" => match args.req_str("path") {
                Ok(path) => done(
                    agent_path_allowed(std::path::Path::new(&path))
                        .and_then(|()| crate::commands::images::validate_build_context_core(&path)),
                ),
                Err(e) => fail(e.to_string()),
            },
            "image_preview_build_command" => match object_arg::<BuildOptions>(args, "req") {
                Ok(req) => done(crate::commands::images::preview_build_command_core(req)),
                Err(e) => fail(e),
            },
            "image_remove" => match (args.req_str("id"), args.bool_or("force", false)) {
                (Ok(id), Ok(force)) => done_unit(self.remove_image(&id, force).await),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "image_prune" => done(self.prune_images().await),

            // ------------------------------------------------ volumes
            "volume_list" => done(crate::commands::volumes::list_volumes_core(state).await),
            "volume_create" => done_unit(self.create_volume(args).await),
            "volume_remove" => match (args.req_str("name"), args.bool_or("force", false)) {
                (Ok(name), Ok(force)) => done_unit(
                    crate::commands::volumes::remove_volume_core(state, &name, force).await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "volume_prune" => done(crate::commands::volumes::prune_volumes_core(state).await),

            // ------------------------------------------------ networks
            "network_list" => done(crate::commands::networks::list_networks_core(state).await),
            "network_create" => done_unit(self.create_network(args).await),
            "network_remove" => match args.req_str("id") {
                Ok(id) => {
                    done_unit(crate::commands::networks::remove_network_core(state, &id).await)
                }
                Err(e) => fail(e.to_string()),
            },
            "network_prune" => done(crate::commands::networks::prune_networks_core(state).await),

            // ------------------------------------------------ compose
            "compose_list_projects" => {
                done(crate::commands::compose::list_compose_projects_core(state).await)
            }
            "compose_up" => match args.req_str("file_path") {
                Ok(path) => {
                    let req = crate::commands::compose::UpRequest {
                        profiles: opt_strings(args, "profiles"),
                        env_file: args.opt_str("env_file").ok().flatten(),
                        skip_override: opt_flag(args, "skip_override"),
                        options: crate::docker::compose::UpOptions {
                            force_recreate: opt_flag(args, "force_recreate"),
                            remove_orphans: opt_flag(args, "remove_orphans"),
                            services: opt_strings(args, "services"),
                        },
                    };
                    done(crate::commands::compose::compose_up_core(&path, req, None).await)
                }
                Err(e) => fail(e.to_string()),
            },
            "compose_down" => match args.req_str("file_path") {
                Ok(path) => {
                    let opts = crate::docker::compose::DownOptions {
                        remove_volumes: opt_flag(args, "remove_volumes"),
                        remove_orphans: opt_flag(args, "remove_orphans"),
                    };
                    done(crate::commands::compose::compose_down_core(&path, opts, None).await)
                }
                Err(e) => fail(e.to_string()),
            },
            "compose_logs" => match (args.req_str("file_path"), args.opt_u64("tail")) {
                (Ok(path), Ok(tail)) => done(
                    crate::commands::compose::compose_logs_core(
                        &path,
                        tail.unwrap_or(200) as usize,
                        &opt_strings(args, "services"),
                    )
                    .await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "compose_service_action" => {
                match (args.req_str("project"), args.req_str("service"), args.req_str("action"), args.opt_u64("replicas")) {
                    (Ok(project), Ok(service), Ok(action), Ok(replicas)) => {
                        let file = args.opt_str("file_path").ok().flatten();
                        done(
                            crate::commands::compose::compose_service_action_core(
                                &project,
                                file.as_deref(),
                                &service,
                                &action,
                                replicas.map(|n| n.min(u32::MAX as u64) as u32),
                                None,
                            )
                            .await,
                        )
                    }
                    (Err(e), ..) | (_, Err(e), ..) | (_, _, Err(e), _) | (.., Err(e)) => {
                        fail(e.to_string())
                    }
                }
            }
            "compose_config" => match args.req_str("file_path") {
                Ok(path) => {
                    let content = args.opt_str("content").ok().flatten();
                    let env = args.opt_str("env_file").ok().flatten();
                    done(
                        crate::commands::compose::compose_validate_core(&path, content, env.as_deref())
                            .await,
                    )
                }
                Err(e) => fail(e.to_string()),
            },
            "compose_read_file" => match args.req_str("file_path") {
                Ok(path) => done(crate::commands::compose::compose_read_file(path).await),
                Err(e) => fail(e.to_string()),
            },
            "compose_save_file" => match (args.req_str("file_path"), args.req_str("content")) {
                (Ok(path), Ok(content)) => {
                    let original = match args.opt_str("original").ok().flatten() {
                        Some(o) => Ok(o),
                        // No conflict check asked for: compare with what is there now.
                        None => crate::commands::compose::compose_read_file(path.clone()).await,
                    };
                    match original {
                        Ok(original) => done_unit(
                            crate::commands::compose::compose_save_file(path, content, original).await,
                        ),
                        Err(e) => fail(e),
                    }
                }
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "compose_env_get" => match args.req_str("file_path") {
                Ok(path) => {
                    let env = args.opt_str("env_file").ok().flatten();
                    let reveal = opt_flag(args, "reveal");
                    match crate::commands::compose::compose_read_env(path, env).await {
                        Ok(info) => {
                            let values: serde_json::Map<String, serde_json::Value> =
                                crate::commands::compose_extra::env_entries(&info.content)
                                    .into_iter()
                                    .map(|(k, v)| {
                                        let shown = if !reveal
                                            && !v.is_empty()
                                            && crate::commands::compose_extra::is_secret_key(&k)
                                        {
                                            "***".to_string()
                                        } else {
                                            v
                                        };
                                        (k, json!(shown))
                                    })
                                    .collect();
                            ok(json!({ "path": info.path, "exists": info.exists, "values": values }))
                        }
                        Err(e) => fail(e),
                    }
                }
                Err(e) => fail(e.to_string()),
            },
            "compose_env_set" => match (args.req_str("file_path"), args.req_str_map("set")) {
                (Ok(path), Ok(set)) => {
                    let env = args.opt_str("env_file").ok().flatten();
                    let unset = opt_strings(args, "unset");
                    let set: std::collections::BTreeMap<String, String> = set.into_iter().collect();
                    let res = async {
                        let info = crate::commands::compose::compose_read_env(path.clone(), env.clone()).await?;
                        let next = crate::commands::compose_extra::env_apply(&info.content, &set, &unset)?;
                        let original = info.exists.then_some(info.content);
                        crate::commands::compose::compose_save_env(path, env, next, original).await
                    }
                    .await;
                    done_unit(res)
                }
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "compose_project_pin" | "compose_project_unpin" => match args.req_str("file_path") {
                Ok(path) => {
                    let pin = spec.name == "compose_project_pin";
                    done(crate::commands::compose::compose_pin_core(state, &path, pin).await)
                }
                Err(e) => fail(e.to_string()),
            },
            "compose_scan" => match (args.req_str("dir"), args.opt_u64("depth")) {
                (Ok(dir), Ok(depth)) => done(
                    crate::commands::compose::compose_scan_core(&dir, depth.map(|d| d.min(6) as u32))
                        .await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "compose_project_action" => match (args.req_str("project"), args.req_str("action")) {
                (Ok(project), Ok(action)) => done(
                    crate::commands::compose::compose_project_action_core(&project, &action).await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },

            // ------------------------------------------------ kind / clusters
            "kind_get_status" => ok(json!(crate::docker::kind::kind_status(&state.kind))),
            "cluster_list" => done(kind::list_kind_clusters_core(state).await),
            "cluster_create" => {
                let parsed = (|| -> Result<_, crate::mcp::args::ArgError> {
                    Ok((
                        args.req_str("name")?,
                        args.opt_str("image")?,
                        args.opt_u64("nodes")?,
                        args.opt_str("config")?,
                    ))
                })();
                match parsed {
                    Ok((name, image, nodes, config)) => {
                        let spec = match args.opt_value("spec") {
                            Some(v) => match serde_json::from_value::<
                                crate::commands::kind_spec::ClusterSpec,
                            >(v)
                            {
                                Ok(s) => s,
                                Err(e) => return fail(format!("spec: {e}")),
                            },
                            None => crate::commands::kind_spec::ClusterSpec::simple(
                                image,
                                nodes.map(|n| n as u32),
                            ),
                        };
                        done_unit(
                            kind::create_kind_cluster_core(state, self.app_ref(), name, spec, config)
                                .await,
                        )
                    }
                    Err(e) => fail(e.to_string()),
                }
            }
            "cluster_add_worker" => match args.req_str("cluster") {
                Ok(cluster) => done(
                    crate::commands::kind_nodes::add_cluster_worker_core(state, self.app_ref(), &cluster)
                        .await,
                ),
                Err(e) => fail(e.to_string()),
            },
            "cluster_remove_node" => match (args.req_str("cluster"), args.req_str("node")) {
                (Ok(cluster), Ok(node)) => done_unit(
                    crate::commands::kind_nodes::remove_cluster_node_core(
                        state,
                        self.app_ref(),
                        &cluster,
                        &node,
                    )
                    .await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_preview_config" => {
                let spec = match args.opt_value("spec") {
                    Some(v) => match serde_json::from_value::<crate::commands::kind_spec::ClusterSpec>(v) {
                        Ok(s) => s,
                        Err(e) => return fail(format!("spec: {e}")),
                    },
                    None => crate::commands::kind_spec::ClusterSpec::default(),
                };
                match args.opt_str("config") {
                    Ok(config) => ok(json!(
                        crate::commands::kind_spec::preview_cluster_config_core(state, &spec, config).await
                    )),
                    Err(e) => fail(e.to_string()),
                }
            }
            "cluster_node_images" => ok(json!(
                crate::commands::kind_spec::list_kind_node_images_core(state).await
            )),
            "local_registry_status" => done(crate::commands::kind_registry::registry_status_core(state).await),
            "local_registry_start" => match crate::commands::kind_registry::ensure_registry_core(state).await {
                Ok(()) => done(crate::commands::kind_registry::registry_status_core(state).await),
                Err(e) => fail(e),
            },
            "local_registry_remove" => done_unit(
                crate::commands::kind_registry::remove_registry_core(
                    state,
                    args.bool_or("delete_data", false).unwrap_or(false),
                )
                .await,
            ),
            "local_registry_push" => match args.req_str("image") {
                Ok(image) => done(
                    crate::commands::kind_registry::push_to_local_registry_core(
                        state,
                        self.app_ref(),
                        &image,
                    )
                    .await,
                ),
                Err(e) => fail(e.to_string()),
            },
            "cluster_connect_registry" => match args.req_str("cluster") {
                Ok(cluster) => {
                    let r = async {
                        crate::commands::kind_registry::ensure_registry_core(state).await?;
                        crate::commands::kind_registry::wire_cluster_core(state, &cluster).await
                    }
                    .await;
                    done_unit(r)
                }
                Err(e) => fail(e.to_string()),
            },
            // ------------------------------------------- phase 3–5 cluster tools
            "portforward_list" => match args.opt_str("cluster") {
                Ok(c) => ok(json!(crate::commands::kind_portforward::list_port_forwards_core(c.as_deref()))),
                Err(e) => fail(e.to_string()),
            },
            "portforward_start" => match camel_args::<crate::commands::kind_portforward::ForwardSpec>(args) {
                Ok(spec) => done(crate::commands::kind_portforward::start_port_forward_core(self.state.clone(), spec).await),
                Err(e) => fail(e),
            },
            "portforward_stop" => match args.req_str("id") {
                Ok(id) => done_unit(crate::commands::kind_portforward::stop_port_forward_core(
                    &id,
                    args.bool_or("forget", false).unwrap_or(false),
                )),
                Err(e) => fail(e.to_string()),
            },
            "addon_list" => match args.req_str("cluster") {
                Ok(c) => done(crate::commands::kind_addons::list_cluster_addons_core(state, &c).await),
                Err(e) => fail(e.to_string()),
            },
            "addon_install" | "addon_uninstall" => match (args.req_str("cluster"), args.req_str("addon")) {
                (Ok(c), Ok(a)) => done_unit(
                    crate::commands::kind_addons::addon_op_wait(state, self.app_ref(), &c, &a, spec.name == "addon_install").await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "dashboard_token" => match args.req_str("cluster") {
                Ok(c) => done(crate::commands::kind_addons::dashboard_token_core(state, &c).await),
                Err(e) => fail(e.to_string()),
            },
            "ingress_list" => match args.req_str("cluster") {
                Ok(c) => done(crate::commands::kind_addons::list_cluster_ingresses_core(state, &c).await),
                Err(e) => fail(e.to_string()),
            },
            "ingress_expose" => match (args.req_str("cluster"), args.req_str("host")) {
                (Ok(c), Ok(h)) => done_unit(
                    crate::commands::kind_addons::expose_ingress_host_core(
                        state,
                        &c,
                        &h,
                        args.bool_or("expose", true).unwrap_or(true),
                    )
                    .await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_link_service" => match camel_args::<crate::commands::kind_links::LinkRequest>(args) {
                Ok(req) => done(crate::commands::kind_links::link_cluster_service_core(state, req).await),
                Err(e) => fail(e),
            },
            "cluster_service_links" => match args.req_str("cluster") {
                Ok(c) => done(crate::commands::kind_links::list_service_links_core(state, &c).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_unlink_service" | "cluster_check_link" => {
                match (args.req_str("cluster"), args.req_str("namespace"), args.req_str("name")) {
                    (Ok(c), Ok(ns), Ok(n)) => {
                        if spec.name == "cluster_unlink_service" {
                            done_unit(crate::commands::kind_links::unlink_cluster_service_core(state, &c, &ns, &n).await)
                        } else {
                            done(crate::commands::kind_links::check_service_link_core(state, &c, &ns, &n).await)
                        }
                    }
                    (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => fail(e.to_string()),
                }
            }
            "helm_list" => match args.req_str("cluster") {
                Ok(c) => done(crate::commands::kind_helm::helm_list_releases_core(state, &c).await),
                Err(e) => fail(e.to_string()),
            },
            "helm_release" => match (args.req_str("cluster"), args.req_str("namespace"), args.req_str("name")) {
                (Ok(c), Ok(ns), Ok(n)) => done(crate::commands::kind_helm::helm_release_detail_core(state, &c, &ns, &n).await),
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => fail(e.to_string()),
            },
            "helm_install" => match camel_args::<crate::commands::kind_helm::HelmInstallRequest>(args) {
                Ok(req) => done(crate::commands::kind_helm::helm_install_core(state, req).await),
                Err(e) => fail(e),
            },
            "helm_rollback" => match (
                args.req_str("cluster"),
                args.req_str("namespace"),
                args.req_str("name"),
                args.opt_u64("revision"),
            ) {
                (Ok(c), Ok(ns), Ok(n), Ok(Some(r))) => done_unit(
                    crate::commands::kind_helm::helm_rollback_core(state, &c, &ns, &n, r as u32).await,
                ),
                (_, _, _, Ok(None)) => fail("missing required argument 'revision'".to_string()),
                (Err(e), _, _, _) | (_, Err(e), _, _) | (_, _, Err(e), _) | (_, _, _, Err(e)) => fail(e.to_string()),
            },
            "helm_uninstall" => match (args.req_str("cluster"), args.req_str("namespace"), args.req_str("name")) {
                (Ok(c), Ok(ns), Ok(n)) => done_unit(crate::commands::kind_helm::helm_uninstall_core(state, &c, &ns, &n).await),
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => fail(e.to_string()),
            },
            "helm_repo_list" => done(crate::commands::kind_helm::helm_repo_list().await),
            "helm_repo_add" => match (args.req_str("name"), args.req_str("url")) {
                (Ok(n), Ok(u)) => done_unit(crate::commands::kind_helm::helm_repo_add_core(&n, &u).await),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "helm_repo_remove" => match args.req_str("name") {
                Ok(n) => done_unit(crate::commands::kind_helm::helm_repo_remove(n).await),
                Err(e) => fail(e.to_string()),
            },
            "helm_repo_update" => done(crate::commands::kind_helm::helm_repo_update().await),
            "helm_search" => match args.opt_str("query") {
                Ok(q) => done(
                    crate::commands::kind_helm::helm_search_core(
                        q.as_deref().unwrap_or(""),
                        args.bool_or("versions", false).unwrap_or(false),
                    )
                    .await,
                ),
                Err(e) => fail(e.to_string()),
            },
            "helm_show_values" => match (args.req_str("chart"), args.opt_str("version")) {
                (Ok(c), Ok(v)) => done(crate::commands::kind_helm::helm_show_values(c, v).await),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_snapshot_list" => match args.opt_str("cluster") {
                Ok(c) => done(crate::commands::kind_snapshots::list_cluster_snapshots_core(c.as_deref())),
                Err(e) => fail(e.to_string()),
            },
            "cluster_snapshot_create" => match (args.req_str("cluster"), args.opt_str("note")) {
                (Ok(c), Ok(note)) => done(
                    crate::commands::kind_snapshots::create_cluster_snapshot_core(state, self.app_ref(), &c, note).await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_snapshot_delete" => match (args.req_str("cluster"), args.req_str("id")) {
                (Ok(c), Ok(id)) => done_unit(crate::commands::kind_snapshots::delete_cluster_snapshot_core(state, &c, &id).await),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_snapshot_restore" | "cluster_clone" => {
                let new_name = if spec.name == "cluster_clone" { args.req_str("new_name").map(Some) } else { Ok(None) };
                match (args.req_str("cluster"), args.req_str("id"), new_name) {
                    (Ok(cluster), Ok(id), Ok(new_name)) => done(
                        crate::commands::kind_snapshots::restore_cluster_snapshot_core(
                            state,
                            self.app_ref(),
                            crate::commands::kind_snapshots::RestoreRequest { cluster, id, new_name },
                        )
                        .await,
                    ),
                    (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => fail(e.to_string()),
                }
            }
            "cluster_alerts" => match args.opt_str("cluster") {
                Ok(c) => ok(json!(crate::commands::kind_watch::scan_cluster_alerts(state, c.as_deref()).await)),
                Err(e) => fail(e.to_string()),
            },
            "cluster_history" => match (args.req_str("cluster"), args.opt_str("range")) {
                (Ok(c), Ok(r)) => done(crate::commands::kind_watch::cluster_history_core(&c, r.as_deref().unwrap_or("1h"))),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_delete" => match args.req_str("name") {
                Ok(name) => {
                    done_unit(kind::delete_kind_cluster_core(state, self.app_ref(), name).await)
                }
                Err(e) => fail(e.to_string()),
            },
            "cluster_start" | "cluster_stop" => match args.req_str("name") {
                Ok(name) => {
                    let start = spec.name == "cluster_start";
                    done_unit(kind::set_cluster_power(self.app_ref(), state, &name, start).await)
                }
                Err(e) => fail(e.to_string()),
            },
            "cluster_kubeconfig" => match args.req_str("name") {
                Ok(name) => done(kind::get_kind_kubeconfig_core(state, name).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_load_image" => match (args.req_str("cluster"), args.req_str("image")) {
                (Ok(cluster), Ok(image)) => done_unit(
                    kind::load_image_to_cluster_core(state, self.app_ref(), cluster, image).await,
                ),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_nodes" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::get_cluster_nodes_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_pods" => match (args.req_str("cluster"), args.opt_str("namespace")) {
                (Ok(cluster), Ok(namespace)) => {
                    done(kind::get_cluster_pods_core(state, cluster, namespace).await)
                }
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_events" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::get_cluster_events_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_deployments" | "cluster_statefulsets" | "cluster_daemonsets" => {
                let want = match spec.name {
                    "cluster_deployments" => "Deployment",
                    "cluster_statefulsets" => "StatefulSet",
                    _ => "DaemonSet",
                };
                match args.req_str("cluster") {
                    Ok(cluster) => done(
                        kind::get_cluster_workloads_core(state, cluster)
                            .await
                            .map(|ws| {
                                ws.into_iter()
                                    .filter(|w| w.kind.eq_ignore_ascii_case(want))
                                    .collect::<Vec<_>>()
                            }),
                    ),
                    Err(e) => fail(e.to_string()),
                }
            }
            "cluster_jobs" | "cluster_cronjobs" => {
                let resource = if spec.name == "cluster_jobs" {
                    "jobs"
                } else {
                    "cronjobs"
                };
                match args.req_str("cluster") {
                    Ok(cluster) => {
                        done(kind::get_cluster_jobs_core(state, cluster, resource).await)
                    }
                    Err(e) => fail(e.to_string()),
                }
            }
            "cluster_services" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::get_cluster_services_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_namespaces" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::get_cluster_namespaces_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_ingresses" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::get_cluster_ingresses_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_pvcs" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::get_cluster_pvcs_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_pvs" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::get_cluster_pvs_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_config_resources" => {
                match (
                    args.req_str("cluster"),
                    args.bool_or("include_secrets", false),
                ) {
                    (Ok(cluster), Ok(include_secrets)) => done(
                        kind::get_cluster_config_resources_core(
                            state,
                            cluster,
                            Some(include_secrets),
                        )
                        .await,
                    ),
                    (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
                }
            }
            "cluster_summary" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::get_cluster_summary_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_metrics" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::get_cluster_metrics_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_metrics_server_status" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::metrics_server_status_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_install_metrics_server" => match args.req_str("cluster") {
                Ok(cluster) => done(kind::install_metrics_server_core(state, cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_pod_logs" => {
                match (
                    args.req_str("cluster"),
                    args.req_str("namespace"),
                    args.req_str("pod"),
                    args.opt_str("container"),
                    args.opt_u64("tail"),
                ) {
                    (Ok(cluster), Ok(namespace), Ok(pod), Ok(_container), Ok(tail)) => done(
                        kind::get_pod_logs_core(
                            state,
                            cluster,
                            namespace,
                            pod,
                            tail.map(|t| t as u32),
                        )
                        .await,
                    ),
                    (Err(e), _, _, _, _)
                    | (_, Err(e), _, _, _)
                    | (_, _, Err(e), _, _)
                    | (_, _, _, Err(e), _)
                    | (_, _, _, _, Err(e)) => fail(e.to_string()),
                }
            }
            "cluster_delete_workload" | "cluster_restart_workload" => {
                match (
                    args.req_str("cluster"),
                    args.req_str("kind"),
                    args.req_str("namespace"),
                    args.req_str("name"),
                ) {
                    (Ok(cluster), Ok(kind_name), Ok(namespace), Ok(name)) => {
                        if spec.name == "cluster_delete_workload" {
                            done_unit(
                                kind::delete_k8s_resource_core(
                                    state,
                                    cluster,
                                    kind_name,
                                    Some(namespace),
                                    name,
                                )
                                .await,
                            )
                        } else {
                            done_unit(
                                kind::restart_workload_core(
                                    state, cluster, kind_name, namespace, name,
                                )
                                .await,
                            )
                        }
                    }
                    (Err(e), _, _, _)
                    | (_, Err(e), _, _)
                    | (_, _, Err(e), _)
                    | (_, _, _, Err(e)) => fail(e.to_string()),
                }
            }
            "cluster_scale_workload" => {
                match (
                    args.req_str("cluster"),
                    args.req_str("kind"),
                    args.req_str("namespace"),
                    args.req_str("name"),
                    args.opt_u64("replicas"),
                ) {
                    (Ok(cluster), Ok(kind), Ok(namespace), Ok(name), Ok(replicas)) => done_unit(
                        kind::scale_workload_core(
                            state,
                            cluster,
                            kind,
                            namespace,
                            name,
                            replicas.unwrap_or(1) as u32,
                        )
                        .await,
                    ),
                    (Err(e), _, _, _, _)
                    | (_, Err(e), _, _, _)
                    | (_, _, Err(e), _, _)
                    | (_, _, _, Err(e), _)
                    | (_, _, _, _, Err(e)) => fail(e.to_string()),
                }
            }
            "cluster_delete_pod" => {
                match (
                    args.req_str("cluster"),
                    args.req_str("namespace"),
                    args.req_str("pod"),
                ) {
                    (Ok(cluster), Ok(namespace), Ok(pod)) => {
                        done_unit(kind::delete_pod_core(state, cluster, namespace, pod, None).await)
                    }
                    (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => fail(e.to_string()),
                }
            }
            "cluster_create_namespace" => match (args.req_str("cluster"), args.req_str("name")) {
                (Ok(cluster), Ok(name)) => {
                    done_unit(kind::create_namespace_core(state, cluster, name).await)
                }
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_drain_node" => match (args.req_str("cluster"), args.req_str("node")) {
                (Ok(cluster), Ok(node)) => done(kind::drain_node_core(state, cluster, node).await),
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_set_node_schedulable" => match (
                args.req_str("cluster"),
                args.req_str("node"),
                args.req_bool("schedulable"),
            ) {
                (Ok(cluster), Ok(node), Ok(schedulable)) => done_unit(
                    kind::set_node_schedulable_core(state, cluster, node, schedulable).await,
                ),
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => fail(e.to_string()),
            },
            "cluster_apply_yaml" => match (args.req_str("cluster"), args.req_str("yaml")) {
                (Ok(cluster), Ok(yaml)) => {
                    done(kind_access::apply_cluster_yaml_core(state, &cluster, &yaml).await)
                }
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_diff_yaml" => match (args.req_str("cluster"), args.req_str("yaml")) {
                (Ok(cluster), Ok(yaml)) => {
                    done(kind_access::diff_cluster_yaml_core(state, &cluster, &yaml).await)
                }
                (Err(e), _) | (_, Err(e)) => fail(e.to_string()),
            },
            "cluster_selector_logs" => {
                let parsed = (|| -> Result<_, crate::mcp::args::ArgError> {
                    Ok((
                        args.req_str("cluster")?,
                        args.req_str("namespace")?,
                        args.opt_str("selector")?,
                        args.opt_str("workload_kind")?.zip(args.opt_str("workload_name")?),
                        args.opt_u64("tail")?.map(|t| t.min(u32::MAX as u64) as u32),
                        args.opt_str("since_time")?,
                    ))
                })();
                match parsed {
                    Ok((cluster, ns, selector, workload, tail, since)) => done(
                        kind_access::get_selector_logs_core(
                            state, &cluster, &ns, selector, workload, tail, since,
                        )
                        .await,
                    ),
                    Err(e) => fail(e.to_string()),
                }
            }
            "cluster_access" => match args.req_str("cluster") {
                Ok(cluster) => done(kind_access::get_cluster_access_core(state, &cluster).await),
                Err(e) => fail(e.to_string()),
            },
            "cluster_export_kubeconfig_to_host" => match args.req_str("cluster") {
                Ok(cluster) => {
                    done(kind_access::export_kubeconfig_to_host_core(state, &cluster).await)
                }
                Err(e) => fail(e.to_string()),
            },
            "cluster_remove_kubeconfig_from_host" => match args.req_str("cluster") {
                Ok(cluster) => {
                    done(kind_access::remove_kubeconfig_from_host_core(state, &cluster).await)
                }
                Err(e) => fail(e.to_string()),
            },
            "cluster_get_cluster_prefs" => match args.req_str("cluster") {
                Ok(cluster) => ok(json!(kind::get_cluster_prefs_core(state, &cluster).await)),
                Err(e) => fail(e.to_string()),
            },
            "cluster_set_cluster_prefs" => {
                match (args.req_str("cluster"), args.opt_value("prefs")) {
                    (Ok(cluster), Some(v)) => match serde_json::from_value(v) {
                        Ok(prefs) => {
                            done_unit(kind::set_cluster_prefs_core(state, &cluster, prefs).await)
                        }
                        Err(e) => fail(format!("argument 'prefs' is not a prefs object: {e}")),
                    },
                    (Err(e), _) => fail(e.to_string()),
                    (_, None) => fail("missing required argument 'prefs'"),
                }
            }

            // ------------------------------------------------ endpoints
            "endpoint_list" => done(self.list_endpoints().await),
            "endpoint_get_active" => done(self.active_endpoint().await),
            "endpoint_test" => match args.opt_value("endpoint") {
                Some(v) => match serde_json::from_value::<EndpointInput>(v) {
                    Ok(ep) => done(crate::commands::endpoints::test_endpoint(ep).await),
                    Err(e) => fail(format!(
                        "argument 'endpoint' is not an endpoint object: {e}"
                    )),
                },
                None => fail("missing required argument 'endpoint'"),
            },
            "endpoint_save" => match args.opt_value("endpoint") {
                Some(v) => match serde_json::from_value::<EndpointInput>(v) {
                    Ok(ep) => done_unit(self.save_endpoint(ep).await),
                    Err(e) => fail(format!(
                        "argument 'endpoint' is not an endpoint object: {e}"
                    )),
                },
                None => fail("missing required argument 'endpoint'"),
            },
            "endpoint_set_active" => match args.req_str("id") {
                Ok(id) => done_unit(self.set_active_endpoint(id).await),
                Err(e) => fail(e.to_string()),
            },
            "endpoint_remove" => match args.req_str("id") {
                Ok(id) => done_unit(self.remove_endpoint(&id).await),
                Err(e) => fail(e.to_string()),
            },

            // ------------------------------------------------ registries
            "registry_list" => done(self.list_registries().await),
            "registry_test" => match args.opt_value("input") {
                Some(v) => match serde_json::from_value(v) {
                    Ok(input) => done(crate::commands::registries::test_registry_auth(input).await),
                    Err(e) => fail(format!("argument 'input' is not a registry login: {e}")),
                },
                None => fail("missing required argument 'input'"),
            },
            "registry_save" => match args.opt_value("input") {
                Some(v) => match serde_json::from_value(v) {
                    Ok(input) => done_unit(self.save_registry(input).await),
                    Err(e) => fail(format!("argument 'input' is not a registry login: {e}")),
                },
                None => fail("missing required argument 'input'"),
            },
            "registry_set_default" => match args.opt_str("id") {
                Ok(id) => done_unit(self.set_default_registry(id.unwrap_or_default()).await),
                Err(e) => fail(e.to_string()),
            },
            "registry_remove" => match args.req_str("id") {
                Ok(id) => done_unit(self.remove_registry(&id).await),
                Err(e) => fail(e.to_string()),
            },

            // ------------------------------------------------ storage / system
            "storage_check_path" => match args.req_str("path") {
                Ok(path) => {
                    done(crate::commands::system::check_data_root_path_core(state, path).await)
                }
                Err(e) => fail(e.to_string()),
            },
            "storage_migrate_data_root" => match (
                args.req_str("target_path"),
                args.bool_or("move_data", true),
                args.bool_or("clear_old", false),
                args.bool_or("confirm_clear_old", false),
            ) {
                // Deleting the old data root is irreversible, so an agent has to
                // say so explicitly; a bare clear_old is refused.
                (Ok(_), Ok(_), Ok(true), Ok(false)) => fail(
                    "clear_old deletes the old data root permanently; pass confirm_clear_old=true to allow it"
                        .to_string(),
                ),
                (Ok(target_path), Ok(move_data), Ok(clear_old), Ok(_)) => done_unit(
                    crate::commands::system::migrate_data_root_core(
                        state,
                        target_path,
                        move_data,
                        clear_old,
                    )
                    .await,
                ),
                (Err(e), _, _, _)
                | (_, Err(e), _, _)
                | (_, _, Err(e), _)
                | (_, _, _, Err(e)) => fail(e.to_string()),
            },
            "system_prune" => done(self.prune_system().await),

            other => fail(format!(
                "tool '{other}' is advertised but has no handler — this is a bug in \
                 hsx2dock; please report it"
            )),
        }
    }
}

// ==================== helpers ====================
//
// Each of these is the MCP-side body of one or more Tauri commands. Where the
// command has real logic it lives in a `*_core` in `commands::*` and both
// sides call it; what is repeated here is the one-or-two-line glue.

impl McpServer {
    async fn engine_info(&self) -> Result<Option<crate::docker::models::EngineInfo>, String> {
        let client = match self.state.get_client().await {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };
        Ok(client.engine_info().await.ok())
    }

    async fn disk_usage(&self) -> Result<crate::docker::models::DfReport, String> {
        crate::commands::system::disk_usage_core(&self.state, false).await
    }

    async fn prune_system(&self) -> Result<u64, String> {
        let client = self.state.get_client().await?;
        client.system_prune().await.map_err(|e| e.to_string())
    }

    /// Mirrors `commands::system::toggle_engine`, including the resource-limit
    /// pass after a start so an agent-started engine obeys the saved caps.
    /// `want`: `Some(true)` start, `Some(false)` stop, `None` toggle.
    async fn set_engine_running(&self, want: Option<bool>) -> Result<DaemonState, String> {
        let state = &self.state;
        state.engine_control_guard().await?;
        let _lifecycle = crate::runtime::claim_engine_lifecycle()?;
        let current = state.daemon_state();
        let running = current == DaemonState::Running;
        if want == Some(running) {
            return Ok(current);
        }
        match current {
            DaemonState::Running => {
                let engine = state.engine.clone();
                crate::runtime::engine_op(move || engine.stop())
                    .await?
                    .map_err(|e| e.to_string())?;
                state.invalidate_client();
                Ok(state.daemon_state())
            }
            DaemonState::Stopped | DaemonState::NotInstalled => {
                let engine = state.engine.clone();
                crate::runtime::engine_op(move || engine.start())
                    .await?
                    .map_err(|e| e.to_string())?;
                let _ = state.get_client().await;
                if let Err(e) = crate::commands::system::apply_engine_limits_now(state).await {
                    tracing::warn!("could not apply engine resource limits: {e}");
                }
                Ok(state.daemon_state())
            }
            other => Ok(other),
        }
    }

    fn diagnose(&self) -> crate::docker::doctor::EngineDoctorReport {
        crate::docker::doctor::diagnose(&self.state.engine.paths())
    }

    /// Same selection rules as `commands::system::repair_engine`: `None` or an
    /// empty list means every auto-repairable check.
    fn repair(&self, check_ids: Option<Vec<String>>) -> crate::docker::doctor::RepairReport {
        let auto = crate::docker::doctor::auto_repairable();
        let ids: Vec<&str> = match check_ids.as_deref() {
            Some(requested) if !requested.is_empty() => {
                requested.iter().map(String::as_str).collect()
            }
            _ => auto,
        };
        let report = crate::docker::doctor::repair(&self.state.engine.paths(), &ids);
        for (id, reason) in &report.failed {
            tracing::warn!("engine repair failed for {id}: {reason}");
        }
        report
    }

    /// The saved config with registry secrets reduced to `has_password`.
    async fn app_config(&self) -> Value {
        let cfg = self.state.config.read().await;
        let mut value = serde_json::to_value(&*cfg).unwrap_or_else(|_| json!({}));
        if let Some(obj) = value.as_object_mut() {
            let logins: Vec<Value> = cfg
                .registries
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "name": r.name,
                        "host": r.host,
                        "username": r.username,
                        "insecure": r.insecure,
                        "allow_http": r.allow_http,
                        "has_password": !r.auth.is_empty(),
                    })
                })
                .collect();
            obj.insert("registries".into(), Value::Array(logins));
        }
        value
    }

    /// Saves through the same core the Settings page uses, so an agent write
    /// cannot skip the autostart entry, the data-root sync, the shim refresh or
    /// the engine-limits pass — and so the two paths cannot drift apart.
    ///
    /// The MCP settings are the one exception: they are kept as they are. An
    /// agent must not be able to switch off read-only mode, move the listener
    /// or turn it off under its own feet — those stay the user's call in the app.
    async fn save_config(&self, mut config: AppConfig) -> Result<(), String> {
        // The stdio server's memory may predate a change made in the app; the
        // save path copies these fields from memory, so refresh them first.
        self.sync_mcp_settings_from_disk().await;
        {
            let current = self.state.config.read().await;
            config.mcp_enabled = current.mcp_enabled;
            config.mcp_port = current.mcp_port;
            config.mcp_read_only = current.mcp_read_only;
        }
        crate::commands::system::save_app_config_core(&self.state, config).await
    }

    /// The user's MCP settings as saved on disk, copied into this process's
    /// config. The stdio server is its own process and loaded the config once;
    /// a switch the user flips in the app must reach it on the next call. An
    /// unreadable file leaves memory as it is.
    async fn sync_mcp_settings_from_disk(&self) {
        let Some(path) = AppConfig::config_path().filter(|p| p.exists()) else {
            return;
        };
        let Ok(disk) = AppConfig::load_from(&path).await else {
            return;
        };
        let mut cfg = self.state.config.write().await;
        cfg.mcp_enabled = disk.mcp_enabled;
        cfg.mcp_port = disk.mcp_port;
        cfg.mcp_read_only = disk.mcp_read_only;
    }

    /// Read-only mode as currently saved (see [`Self::sync_mcp_settings_from_disk`]).
    async fn read_only_mode(&self) -> bool {
        self.sync_mcp_settings_from_disk().await;
        self.state.config.read().await.mcp_read_only
    }

    async fn cli_bridge_status(&self) -> crate::commands::system::CliBridgeStatus {
        let config = self.state.config.read().await;
        let engine_socket = &self.state.engine.paths().socket;
        let link_state = crate::docker::cli_bridge::xdg_symlink_state(engine_socket);
        crate::commands::system::CliBridgeStatus {
            enabled: config.cli_bridge_enabled,
            link_state,
        }
    }

    async fn set_cli_bridge(
        &self,
        enabled: bool,
    ) -> Result<crate::commands::system::CliBridgeStatus, String> {
        self.state
            .update_config(|config| {
                config.cli_bridge_enabled = enabled;
                Ok(())
            })
            .await?;
        let link_state = crate::commands::system::apply_cli_bridge(&self.state).await;
        let config = self.state.config.read().await;
        Ok(crate::commands::system::CliBridgeStatus {
            enabled: config.cli_bridge_enabled,
            link_state,
        })
    }

    async fn apply_resource_limits(&self) -> Result<String, String> {
        crate::commands::system::apply_engine_limits_now(&self.state).await
    }

    async fn list_containers(
        &self,
        all: bool,
    ) -> Result<Vec<crate::docker::models::ContainerItem>, String> {
        let client = self.state.get_client().await?;
        client.list_containers(all).await.map_err(|e| e.to_string())
    }

    /// Runs `f` with the `id` argument, returning its result as a tool result.
    async fn with_id<F, Fut, T>(&self, args: &Args, f: F) -> CallToolResult
    where
        F: FnOnce(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
        T: serde::Serialize,
    {
        match args.req_str("id") {
            Ok(id) => done(f(id).await),
            Err(e) => fail(e.to_string()),
        }
    }

    async fn container_logs(&self, id: &str, tail: usize) -> Result<String, String> {
        let client = self.state.get_client().await?;
        client
            .container_logs(id, &tail.to_string(), false)
            .await
            .map_err(|e| e.to_string())
    }

    async fn container_sizes(&self) -> Result<std::collections::HashMap<String, u64>, String> {
        let client = self.state.get_client().await?;
        client.container_sizes().await.map_err(|e| e.to_string())
    }

    async fn container_power(&self, tool: &str, args: &Args) -> CallToolResult {
        let id = match args.req_str("id") {
            Ok(id) => id,
            Err(e) => return fail(e.to_string()),
        };
        let client = match self.state.get_client().await {
            Ok(c) => c,
            Err(e) => return fail(e),
        };
        let result = match tool {
            "container_start" => client.start_container(&id).await,
            "container_stop" => client.stop_container(&id).await,
            "container_restart" => client.restart_container(&id).await,
            "container_pause" => client.pause_container(&id).await,
            _ => client.unpause_container(&id).await,
        };
        done_unit(result.map_err(|e| e.to_string()))
    }

    async fn rename_container(&self, args: &Args) -> CallToolResult {
        let (id, new_name) = match (args.req_str("id"), args.req_str("new_name")) {
            (Ok(id), Ok(name)) => (id, name),
            (Err(e), _) | (_, Err(e)) => return fail(e.to_string()),
        };
        done_unit(
            crate::commands::containers::rename_container_core(&self.state, &id, &new_name).await,
        )
    }

    async fn remove_container(&self, id: &str, force: bool) -> Result<(), String> {
        let client = self.state.get_client().await?;
        client
            .remove_container(id, force)
            .await
            .map_err(|e| e.to_string())
    }

    async fn run_container(&self, config: RunConfig) -> Result<(), String> {
        config.validate().map_err(|e| e.to_string())?;
        let client = self.state.get_client().await?;
        client
            .create_container(&config, true)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn update_limits(&self, id: &str, limits: &ResourceLimits) -> Result<(), String> {
        let client = self.state.get_client().await?;
        client
            .update_container_limits(id, limits)
            .await
            .map_err(|e| e.to_string())
    }

    async fn exec(&self, id: &str, cmd: Vec<String>) -> CallToolResult {
        let client = match self.state.get_client().await {
            Ok(c) => c,
            Err(e) => return fail(e),
        };
        match client.exec_capture(id, cmd).await {
            Ok(out) => ok(json!({ "output": out })),
            Err(e) => fail(format!("{e:#}")),
        }
    }

    async fn prune_containers(&self) -> Result<u64, String> {
        let client = self.state.get_client().await?;
        client.prune_containers().await.map_err(|e| e.to_string())
    }

    async fn groups(&self) -> Vec<crate::config::ContainerGroup> {
        self.state.config.read().await.container_groups.clone()
    }

    async fn save_groups(&self, groups: Vec<crate::config::ContainerGroup>) -> Result<(), String> {
        crate::commands::groups::save_container_groups_core(&self.state, groups).await
    }

    async fn set_group_power(&self, id: &str, start: bool) -> Result<String, String> {
        if start {
            crate::commands::groups::start_container_group_core(&self.state, id).await
        } else {
            crate::commands::groups::stop_container_group_core(&self.state, id).await
        }
    }

    async fn list_images(&self) -> Result<Vec<crate::docker::models::ImageItem>, String> {
        let client = self.state.get_client().await?;
        let configured = {
            let cfg = self.state.config.read().await;
            cfg.kind_default_image.clone()
        };
        let configured = Some(configured.as_str()).filter(|s| !s.is_empty());
        client
            .list_images_visible(configured)
            .await
            .map_err(|e| e.to_string())
    }

    async fn image_history(
        &self,
        name: &str,
    ) -> Result<Vec<crate::docker::models::ImageHistoryEntry>, String> {
        let client = self.state.get_client().await?;
        client.image_history(name).await.map_err(|e| e.to_string())
    }

    async fn search_images(&self, term: &str) -> Result<Vec<Value>, String> {
        let client = self.state.get_client().await?;
        let results = client
            .search_images(term)
            .await
            .map_err(|e| e.to_string())?;
        Ok(results
            .into_iter()
            .map(|(name, description, star_count)| {
                json!({ "name": name, "description": description, "star_count": star_count })
            })
            .collect())
    }

    async fn tag_image(&self, id: &str, repo: &str, tag: &str) -> Result<(), String> {
        let client = self.state.get_client().await?;
        client
            .tag_image(id, repo, tag)
            .await
            .map_err(|e| e.to_string())
    }

    fn cancel_build(&self) -> Result<(), String> {
        use std::sync::atomic::Ordering::Relaxed;
        if !self.state.build_running.load(Relaxed) {
            return Err("No build in progress".to_string());
        }
        self.state.build_cancel.store(true, Relaxed);
        Ok(())
    }

    async fn remove_image(&self, id: &str, force: bool) -> Result<(), String> {
        crate::commands::images::remove_image_core(&self.state, id, force).await
    }

    async fn prune_images(&self) -> Result<Value, String> {
        let client = self.state.get_client().await?;
        let (deleted, reclaimed) = client.prune_images().await.map_err(|e| e.to_string())?;
        Ok(json!({ "deleted": deleted, "reclaimed_bytes": reclaimed }))
    }

    async fn create_volume(&self, args: &Args) -> Result<(), String> {
        let name = args.req_str("name").map_err(|e| e.to_string())?;
        let driver = args
            .opt_str("driver")
            .map_err(|e| e.to_string())?
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| "local".to_string());
        let driver_opts = match args.opt_value("driver_opts") {
            Some(v) => serde_json::from_value(v)
                .map_err(|e| format!("argument 'driver_opts' must be a string map: {e}"))?,
            None => Default::default(),
        };
        let labels = match args.opt_value("labels") {
            Some(v) => serde_json::from_value(v)
                .map_err(|e| format!("argument 'labels' must be a string map: {e}"))?,
            None => Default::default(),
        };
        let client = self.state.get_client().await?;
        client
            .create_volume(&name, &driver, driver_opts, labels)
            .await
            .map_err(|e| e.to_string())
    }

    async fn create_network(&self, args: &Args) -> Result<(), String> {
        let name = args.req_str("name").map_err(|e| e.to_string())?;
        let driver = args
            .opt_str("driver")
            .map_err(|e| e.to_string())?
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| "bridge".to_string());
        let subnet = args.opt_str("subnet").map_err(|e| e.to_string())?;
        let gateway = args.opt_str("gateway").map_err(|e| e.to_string())?;
        let internal = args.bool_or("internal", false).map_err(|e| e.to_string())?;
        let attachable = args
            .bool_or("attachable", true)
            .map_err(|e| e.to_string())?;
        let labels = match args.opt_value("labels") {
            Some(v) => serde_json::from_value(v)
                .map_err(|e| format!("argument 'labels' must be a string map: {e}"))?,
            None => Default::default(),
        };
        let client = self.state.get_client().await?;
        client
            .create_network(
                &name,
                &driver,
                subnet.as_deref().unwrap_or_default(),
                gateway.as_deref().unwrap_or_default(),
                internal,
                attachable,
                labels,
            )
            .await
            .map_err(|e| e.to_string())
    }

    async fn list_endpoints(&self) -> Result<Vec<crate::docker::models::EndpointStatus>, String> {
        crate::commands::endpoints::list_endpoints_core(&self.state).await
    }

    async fn active_endpoint(
        &self,
    ) -> Result<Option<crate::docker::models::EndpointStatus>, String> {
        crate::commands::endpoints::get_active_endpoint_core(&self.state).await
    }

    async fn save_endpoint(&self, ep: EndpointInput) -> Result<(), String> {
        crate::commands::endpoints::save_endpoint_core(&self.state, ep).await.map(|_| ())
    }

    async fn remove_endpoint(&self, id: &str) -> Result<(), String> {
        crate::commands::endpoints::remove_endpoint_core(&self.state, id).await.map(|_| ())
    }

    /// Same core as the Settings command, so an agent switch also re-points the
    /// CLI bridge and tells the window (`endpoint-changed`).
    async fn set_active_endpoint(&self, id: String) -> Result<(), String> {
        crate::commands::endpoints::set_active_endpoint_core(&self.state, self.app_ref(), id).await
    }

    async fn list_registries(
        &self,
    ) -> Result<Vec<crate::commands::registries::RegistryStatus>, String> {
        crate::commands::registries::list_registries_core(&self.state).await
    }

    async fn save_registry(
        &self,
        input: crate::commands::registries::RegistryInput,
    ) -> Result<(), String> {
        crate::commands::registries::save_registry_auth_core(&self.state, input).await
    }

    async fn set_default_registry(&self, id: String) -> Result<(), String> {
        crate::commands::registries::set_default_registry_core(&self.state, &id).await
    }

    async fn remove_registry(&self, id: &str) -> Result<(), String> {
        crate::commands::registries::remove_registry_auth_core(&self.state, id).await
    }

    /// Resolves one resource URI to its text body.
    ///
    /// Resources mirror the read tools but are pull-based: a client can read
    /// `hsx2dock://containers` without spending a tool call. Errors are turned
    /// into a JSON body rather than `None` so a client that asks for
    /// `hsx2dock://engine/status` while the engine is down gets the reason.
    async fn read_resource_body(&self, uri: &str) -> Option<String> {
        let state = &self.state;
        let to_text = |v: Value| Some(v.to_string());
        let joined = |prefix: &str| {
            let prefix = prefix.to_string();
            move |v: Value| Some(serde_json::json!({ prefix.as_str(): v }).to_string())
        };
        match uri {
            "hsx2dock://guide" => Some(super::INSTRUCTIONS.to_string()),
            "hsx2dock://tool-catalogue" => {
                let tools: Vec<Value> = super::tools::catalogue()
                    .into_iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "title": t.title,
                            "description": t.description,
                            "access": match t.access {
                                Access::Read => "read",
                                Access::Write => "write",
                                Access::Destructive => "destructive",
                            },
                            "engine_only": t.engine_only,
                            "input_schema": t.input_schema,
                        })
                    })
                    .collect();
                to_text(Value::Array(tools))
            }
            "hsx2dock://engine/status" => to_text(json!({
                "state": state.daemon_state(),
                "installed": state.engine.is_installed(),
                "installed_version": state.engine.installed_version(),
                "socket": state.engine.socket().display().to_string(),
                "socket_live": state.engine.socket_live(),
            })),
            "hsx2dock://engine/doctor" => match state.engine_control_guard().await {
                Ok(()) => to_text(json!(self.diagnose())),
                Err(e) => to_text(json!({ "unavailable": e })),
            },
            // Resources that need a live daemon report *why* they are empty
            // rather than vanishing: a stopped engine must not look like a
            // resource that does not exist, or an agent reads it as a bad URI
            // and gives up instead of starting the engine.
            "hsx2dock://containers" => to_text(unavailable_on_error(
                "containers",
                self.list_containers(true).await,
            )),
            "hsx2dock://images" => {
                to_text(unavailable_on_error("images", self.list_images().await))
            }
            "hsx2dock://volumes" => to_text(unavailable_on_error(
                "volumes",
                crate::commands::volumes::list_volumes_core(state).await,
            )),
            "hsx2dock://networks" => to_text(unavailable_on_error(
                "networks",
                crate::commands::networks::list_networks_core(state).await,
            )),
            "hsx2dock://clusters" => to_text(unavailable_on_error(
                "clusters",
                kind::list_kind_clusters_core(state).await,
            )),
            "hsx2dock://groups" => joined("groups")(json!(self.groups().await)),
            "hsx2dock://config" => Some(self.app_config().await.to_string()),
            "hsx2dock://endpoints" => {
                let active = state.config.read().await.active_endpoint.clone();
                let list = match self.list_endpoints().await {
                    Ok(list) => json!(list),
                    Err(e) => json!({ "error": e }),
                };
                to_text(json!({
                    "active": active.unwrap_or_else(|| "local".to_string()),
                    "endpoints": list,
                }))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resource whose backing daemon is down must still resolve. Reporting
    /// "no resource at this URI" instead would read as a bad address to an
    /// agent, which then never learns that starting the engine fixes it.
    #[test]
    fn unavailable_on_error_keeps_the_uri_and_names_the_cause() {
        let value = unavailable_on_error::<Vec<String>>("containers", Err("daemon down".into()));
        assert_eq!(value["containers"], json!([]));
        assert_eq!(value["unavailable"], json!("daemon down"));
    }

    #[test]
    fn unavailable_on_error_is_a_plain_list_when_the_call_worked() {
        let value = unavailable_on_error("containers", Ok::<_, String>(vec!["a"]));
        assert_eq!(value["containers"], json!(["a"]));
        assert!(
            value.get("unavailable").is_none(),
            "a working read must not claim to be unavailable"
        );
    }

    /// The guide is markdown and is rendered as such by clients; the rest are
    /// JSON. A single hardcoded mime type in `read_resource` used to make the
    /// guide render as raw JSON.
    #[test]
    fn every_resource_advertises_a_usable_mime_type() {
        for spec in super::super::resources::catalogue() {
            assert!(
                matches!(spec.mime_type, "text/markdown" | "application/json"),
                "{} has unexpected mime type {}",
                spec.uri,
                spec.mime_type
            );
        }
        let guide = super::super::resources::catalogue()
            .into_iter()
            .find(|r| r.uri == "hsx2dock://guide")
            .expect("the guide is the first thing an agent reads");
        assert_eq!(guide.mime_type, "text/markdown");
    }
}

#[cfg(test)]
mod agent_path_tests {
    use super::agent_path_allowed_in;

    #[test]
    fn credential_and_system_dirs_are_refused_but_projects_are_not() {
        let tmp = std::env::temp_dir().join(format!("hsx2-agent-path-{}", std::process::id()));
        let home = tmp.join("home");
        let project = home.join("code/app");
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&ssh).unwrap();
        let link = home.join("code/sneaky");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&ssh, &link).unwrap();
        let h = Some(home.as_path());

        assert!(agent_path_allowed_in(&project, h).is_ok());
        assert!(agent_path_allowed_in(&ssh, h).is_err(), "credentials dir");
        assert!(agent_path_allowed_in(&link, h).is_err(), "symlink into credentials");
        assert!(agent_path_allowed_in(&home, h).is_err(), "home itself is too broad");
        assert!(agent_path_allowed_in(&home.join("code/.."), h).is_err(), "dot-dot to home");
        assert!(agent_path_allowed_in(std::path::Path::new("/"), h).is_err());
        assert!(agent_path_allowed_in(std::path::Path::new("/etc"), h).is_err());
        assert!(agent_path_allowed_in(&home.join("missing"), h).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

/// Deserialises a tool's snake_case arguments into a camelCase request type.
fn camel_args<T: serde::de::DeserializeOwned>(args: &crate::mcp::args::Args) -> Result<T, String> {
    let mut m = serde_json::Map::new();
    for (k, v) in args.raw() {
        if v.is_null() {
            continue;
        }
        let mut out = String::new();
        let mut up = false;
        for ch in k.chars() {
            if ch == '_' {
                up = true;
            } else if up {
                out.extend(ch.to_uppercase());
                up = false;
            } else {
                out.push(ch);
            }
        }
        m.insert(out, v.clone());
    }
    serde_json::from_value(serde_json::Value::Object(m)).map_err(|e| format!("invalid arguments: {e}"))
}
