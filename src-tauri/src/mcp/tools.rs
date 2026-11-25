//! The MCP tool catalogue.
//!
//! One entry per tool: its name, description, JSON Schema, and safety
//! annotations. The catalogue is data, deliberately separate from dispatch
//! ([`crate::mcp::server`]), so that the contract an agent sees can be asserted
//! in unit tests without an engine anywhere near it.
//!
//! # Why the schemas are hand-written
//!
//! rmcp's `#[tool]` macros derive schemas through `schemars`. That would make
//! this crate's public MCP surface depend on a `schemars` major version, and
//! every tool's arguments would need a bespoke `Deserialize` struct — a lot of
//! code to describe shapes the app already defines in `docker::models`. Writing
//! the JSON Schema directly keeps the contract explicit and dependency-free.
//! [`crate::mcp::args`] still validates every call against it.

use serde_json::{json, Map, Value};

/// What a tool does to the world, which decides how the agent should treat it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reads state only.
    Read,
    /// Creates or starts something. Safe to repeat; nothing is lost.
    Write,
    /// Removes data, or can (an arbitrary `container_exec` command, a manifest
    /// that replaces live objects). The only class that carries
    /// `destructiveHint`.
    Destructive,
}

/// One tool as advertised to clients.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub access: Access,
    /// True for tools that act on the bundled engine and must be refused while
    /// a remote endpoint is active (mirrors `AppState::engine_control_guard`).
    pub engine_only: bool,
    pub input_schema: Value,
}

/// The argument every destructive tool must be called with, set to `true`.
pub const CONFIRM_ARG: &str = "confirm";

/// Why a call is refused before it runs, if it is: read-only mode blocks every
/// tool that is not a read, and a destructive tool needs `confirm: true` — a
/// `destructiveHint` alone is only advice a client may ignore.
pub fn access_refusal(access: Access, read_only_mode: bool, confirmed: bool) -> Option<String> {
    if read_only_mode && access != Access::Read {
        return Some(
            "the MCP server is in read-only mode (Hsx2Dock → MCP); only read tools are allowed"
                .into(),
        );
    }
    if access == Access::Destructive && !confirmed {
        return Some(format!(
            "this tool is destructive: ask the user first, then call it again with \
             `{CONFIRM_ARG}: true`"
        ));
    }
    None
}

impl ToolSpec {
    pub fn is_read_only(&self) -> bool {
        self.access == Access::Read
    }

    /// The input schema as advertised: a destructive tool gains the required
    /// `confirm` argument that [`access_refusal`] checks.
    pub fn advertised_schema(&self) -> Map<String, Value> {
        let mut schema = self.input_schema.as_object().cloned().unwrap_or_default();
        if self.access == Access::Destructive {
            if let Some(Value::Object(props)) = schema.get_mut("properties") {
                props.insert(
                    CONFIRM_ARG.into(),
                    json!({
                        "type": "boolean",
                        "const": true,
                        "description": "Must be true. Set it only after the user agreed to this destructive action."
                    }),
                );
            }
            match schema.get_mut("required") {
                Some(Value::Array(required)) => required.push(json!(CONFIRM_ARG)),
                _ => {
                    schema.insert("required".into(), json!([CONFIRM_ARG]));
                }
            }
        }
        schema
    }

    /// The `Tool` value sent in `tools/list`.
    pub fn to_tool(&self) -> rmcp::model::Tool {
        let schema = self.advertised_schema();
        rmcp::model::Tool::new(self.name, self.description, schema)
            .with_title(self.title.to_string())
            .with_annotations(rmcp::model::ToolAnnotations::from_raw(
                Some(self.title.to_string()),
                Some(self.is_read_only()),
                Some(self.access == Access::Destructive),
                Some(self.is_read_only()),
                Some(false),
            ))
    }
}

/// Schema for a tool that takes no arguments.
fn no_args() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

/// Build an object schema from `(name, type-schema, required, description)`.
///
/// A small helper so the catalogue reads as data rather than as a wall of JSON.
fn obj(fields: &[(&str, Value, bool, &str)]) -> Value {
    let mut props = Map::new();
    let mut required = Vec::new();
    for (name, ty, req, desc) in fields {
        let mut schema = ty.clone();
        if let Some(m) = schema.as_object_mut() {
            m.insert("description".into(), json!(desc));
        }
        props.insert((*name).to_string(), schema);
        if *req {
            required.push(json!(name));
        }
    }
    json!({
        "type": "object",
        "properties": Value::Object(props),
        "required": required,
        "additionalProperties": false
    })
}

fn string() -> Value {
    json!({ "type": "string" })
}

fn boolean() -> Value {
    json!({ "type": "boolean" })
}

fn integer() -> Value {
    json!({ "type": "integer", "minimum": 0 })
}

fn string_array() -> Value {
    json!({ "type": "array", "items": { "type": "string" } })
}

fn string_map() -> Value {
    json!({ "type": "object", "additionalProperties": { "type": "string" } })
}

fn object() -> Value {
    json!({ "type": "object" })
}

/// Enum-of-strings schema, for arguments with a fixed set of valid values.
fn enumerated(values: &[&str]) -> Value {
    json!({ "type": "string", "enum": values })
}

/// The container id argument, shared by every per-container tool so the
/// wording an agent sees never drifts between them.
fn id_arg() -> (&'static str, Value, bool, &'static str) {
    (
        "id",
        json!({ "type": "string" }),
        true,
        "Container id or name (a unique name prefix is accepted)",
    )
}

/// Every tool this server exposes.
pub fn catalogue() -> Vec<ToolSpec> {
    let mut tools = vec![
        // ---------------------------------------------------------------- engine
        ToolSpec {
            name: "engine_get_status",
            title: "Get engine status",
            description: "Whether the bundled Docker engine is installed and running, as one of \
                          `Stopped`, `Starting`, `Running`, `Stopping`, `NotInstalled`, `Installing`, \
                          or an error. Call this before other tools to check the engine is up.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_get_info",
            title: "Get engine info",
            description: "Version, API version, OS, architecture and rootless flag of the running \
                          engine. Returns null when the engine is not reachable.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_get_disk_usage",
            title: "Get disk usage",
            description: "Disk used by images, containers, volumes and build cache, with the \
                          reclaimable portion of each.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_start",
            title: "Start engine",
            description: "Start the bundled Docker engine, applying the configured resource caps, \
                          CLI bridge and container-group auto-start. The safe snapshotter-link and \
                          compose-plugin self-repairs run first. Returns the resulting status.",
            access: Access::Write,
            engine_only: true,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_stop",
            title: "Stop engine",
            description: "Stop the bundled Docker engine. Running containers stop with it. Returns \
                          the resulting status.",
            access: Access::Destructive,
            engine_only: true,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_toggle",
            title: "Toggle engine",
            description: "Start the engine if stopped, stop it if running. Returns the resulting \
                          status. Prefer engine_start or engine_stop when the desired state is known.",
            access: Access::Destructive,
            engine_only: true,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_diagnose",
            title: "Diagnose engine",
            description: "Run the full engine health check: install, binaries, data-root \
                          filesystem, containerd snapshotter links, compose plugin, containerd \
                          state, plugin load status, and socket. Returns every check with its \
                          health (`ok`, `warning`, `error`, `missing`), whether it is repairable, \
                          and whether the repair runs automatically at start. Use this when the \
                          engine fails to start or `docker` reports opaque containerd errors.",
            access: Access::Read,
            engine_only: true,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_repair",
            title: "Repair engine",
            description: "Apply repairs to the bundled engine. Repairs never delete images or \
                          containers. Pass `check_ids` with ids from `engine_diagnose` to fix \
                          specific checks, or omit them to repair every repairable failing check. \
                          The engine must be restarted afterwards to pick up most fixes.",
            access: Access::Write,
            engine_only: true,
            input_schema: obj(&[(
                "check_ids",
                string_array(),
                false,
                "Check ids to repair, from `engine_diagnose`. Omit to repair every repairable \
                 failing check.",
            )]),
        },
        ToolSpec {
            name: "engine_get_config",
            title: "Get app config",
            description: "The full persisted application configuration: engine data root, \
                          resource limits, CLI bridge and shim settings, endpoints, registries, \
                          notification opt-outs, container groups, and kind cluster preferences.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_set_config",
            title: "Update app config",
            description: "Merge the given fields into the application configuration and persist \
                          them. Only the keys you pass are changed. Use `engine_get_config` first \
                          to discover the shape.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[(
                "config",
                object(),
                true,
                "A partial config object; keys to change, taken from `engine_get_config`",
            )]),
        },
        ToolSpec {
            name: "engine_get_cli_bridge_status",
            title: "Get CLI bridge status",
            description: "Whether the system `docker` CLI is wired to this app's engine, and \
                          whether this app's engine is the CLI's default context.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_set_cli_bridge",
            title: "Toggle CLI bridge",
            description: "Enable or disable bridging the system `docker` CLI to this app's engine \
                          (a `$XDG_RUNTIME_DIR/docker.sock` symlink plus the `hsx2dock-engine` \
                          context). Returns the new status.",
            access: Access::Write,
            engine_only: true,
            input_schema: obj(&[(
                "enabled",
                boolean(),
                true,
                "true wires the system CLI to this app's engine, false tears the wiring down",
            )]),
        },
        // ---------------------------------------------------------------- containers
        ToolSpec {
            name: "container_list",
            title: "List containers",
            description: "List containers with state, status, image, published ports and compose \
                          labels. Includes stopped containers by default.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[(
                "all",
                boolean(),
                false,
                "Include stopped containers (default true)",
            )]),
        },
        ToolSpec {
            name: "container_inspect",
            title: "Inspect container",
            description: "Full container detail: state, config, environment, mounts, port bindings, \
                          networks, restart policy and resource limits.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[id_arg()]),
        },
        ToolSpec {
            name: "container_logs",
            title: "Get container logs",
            description: "The last N lines of a container's combined stdout and stderr.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[
                id_arg(),
                (
                    "tail",
                    integer(),
                    false,
                    "Number of trailing lines to return (default 200)",
                ),
            ]),
        },
        ToolSpec {
            name: "container_stats",
            title: "Get container stats",
            description: "A one-shot CPU, memory, network, block-IO and PID sample for one \
                          container.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[id_arg()]),
        },
        ToolSpec {
            name: "container_sizes",
            title: "Get container sizes",
            description: "Writable-layer size in bytes for every container, keyed by container id. \
                          Slower than `container_list` because it stats each container.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "container_start",
            title: "Start container",
            description: "Start a stopped container. Returns once the engine reports it running.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[id_arg()]),
        },
        ToolSpec {
            name: "container_stop",
            title: "Stop container",
            description: "Stop a running container, giving its processes time to exit cleanly.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[id_arg()]),
        },
        ToolSpec {
            name: "container_restart",
            title: "Restart container",
            description: "Stop and start a container in one call, keeping its configuration, \
                          volumes and networks.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[id_arg()]),
        },
        ToolSpec {
            name: "container_pause",
            title: "Pause container",
            description: "Pause all processes in a container with the cgroup freezer. The \
                          container keeps its memory and its processes resume where they left off.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[id_arg()]),
        },
        ToolSpec {
            name: "container_unpause",
            title: "Unpause container",
            description: "Resume a paused container, unfreezing every process in it.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[id_arg()]),
        },
        ToolSpec {
            name: "container_rename",
            title: "Rename container",
            description: "Rename a container. The new name must be unique.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                id_arg(),
                (
                    "new_name",
                    string(),
                    true,
                    "The new container name",
                ),
            ]),
        },
        ToolSpec {
            name: "container_remove",
            title: "Remove container",
            description: "Delete a container. Its anonymous volumes and its entry in any container \
                          group go with it; named volumes and the image are kept. A running \
                          container requires `force`, which kills it first.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[
                id_arg(),
                (
                    "force",
                    boolean(),
                    false,
                    "Kill the container first if it is running (default false)",
                ),
            ]),
        },
        ToolSpec {
            name: "container_recreate",
            title: "Recreate container (edit configuration)",
            description: "Edit immutable container configuration (ports, env, mounts, command, etc.) \
                          by recreating the container. Stops and removes the old container, then \
                          starts a new one with the updated RunConfig while preserving its name and group. \
                          Returns the new container id.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[
                id_arg(),
                (
                    "config",
                    object(),
                    true,
                    "Updated RunConfig object. Fields that are not provided or empty keep original values.",
                ),
            ]),
        },
        ToolSpec {
            name: "container_run",
            title: "Run container",
            description: "Create and start a container from an image. The `config` object is the \
                          app's own RunConfig; only `image` is required. Fields: `image`, `name`, \
                          `command` (argv), `entrypoint` (argv), `ports` \
                          (`[{host_ip, host_port, container_port, protocol}]`, ports as strings, \
                          protocol `tcp`/`udp`/`sctp`), `env` (`[{key, value}]`), `volumes` \
                          (`[{kind: \"bind\"|\"volume\", source, target, read_only}]`), \
                          `restart_policy` (`no`/`always`/`unless-stopped`/`on-failure`), \
                          `restart_max_retries`, `detach`, `hostname`, `workdir`, `user`, \
                          `network`, `labels` (`[{key, value}]`), `resources` (`ResourceLimits`: \
                          `memory_mb`, `cpus`, `cpuset_cpus`, `cpu_shares`, `pids_limit` — all \
                          strings), `healthcheck` (`{test: [\"CMD\", ...], interval_s, timeout_s, \
                          retries, start_period_s}`), `cap_add`, `cap_drop`, `extra_hosts`, \
                          `privileged`, `stdin_open`, `tty`, `auto_remove`, `read_only_rootfs`. \
                          Returns the new container id.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "config",
                object(),
                true,
                "RunConfig object; `image` is required. Use `container_inspect` on an existing \
                 container to see a complete example.",
            )]),
        },
        ToolSpec {
            name: "container_update_resources",
            title: "Update container resources",
            description: "Change a running container's CPU and memory limits without recreating it. \
                          The `limits` object uses the app's ResourceLimits fields, all strings: \
                          `memory_mb` (MiB), `cpus` (cores, e.g. `1.5`), `cpuset_cpus` (e.g. \
                          `0-2,4`), `cpu_shares`, `pids_limit`. An empty string means unlimited.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                id_arg(),
                (
                    "limits",
                    object(),
                    true,
                    "ResourceLimits object, e.g. `{\"cpus\": \"1.5\", \"memory_mb\": \"512\"}`",
                ),
            ]),
        },
        ToolSpec {
            name: "container_exec",
            title: "Exec in container",
            description: "Run a command inside a container and return its combined stdout and \
                          stderr. Fails if the command exits non-zero. Best for short, \
                          non-interactive commands (`ls`, `cat`, `env`, `ps`).",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[
                id_arg(),
                (
                    "cmd",
                    string_array(),
                    true,
                    "Command and arguments, e.g. `[\"ls\", \"-la\", \"/\"]`",
                ),
            ]),
        },
        ToolSpec {
            name: "container_prune",
            title: "Prune stopped containers",
            description: "Delete every stopped container except kind cluster nodes. Returns the \
                          number removed.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: no_args(),
        },
        // ---------------------------------------------------------------- container groups
        ToolSpec {
            name: "group_list",
            title: "List container groups",
            description: "The user-defined container groups used for bulk start/stop, each with its \
                          members and auto-start flag.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "group_save",
            title: "Save container groups",
            description: "Replace the stored container groups with the given list. Include existing \
                          groups you want to keep — this is a full replacement, not a merge.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "groups",
                json!({ "type": "array", "items": { "type": "object" } }),
                true,
                "The complete list of container groups",
            )]),
        },
        ToolSpec {
            name: "group_start",
            title: "Start group",
            description: "Start every container in a group. Returns a summary naming the containers \
                          that started and any that failed.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "id",
                string(),
                true,
                "Group id from `group_list`",
            )]),
        },
        ToolSpec {
            name: "group_stop",
            title: "Stop group",
            description: "Stop every container in a group. Returns a summary naming the containers \
                          that stopped and any that failed.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "id",
                string(),
                true,
                "Group id from `group_list`",
            )]),
        },
        // ---------------------------------------------------------------- images
        ToolSpec {
            name: "image_list",
            title: "List images",
            description: "Local images with their repository tags, ids, size and creation time.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "image_history",
            title: "Get image history",
            description: "The layer history of an image: the command that created each layer, its \
                          size and when. Shows how an image was built and where its size comes from.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[(
                "name",
                string(),
                true,
                "Image name or id, e.g. `nginx:latest`",
            )]),
        },
        ToolSpec {
            name: "image_search",
            title: "Search Docker Hub",
            description: "Search Docker Hub for images. Returns matching names, descriptions and \
                          star counts. Does not pull anything.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[(
                "term",
                string(),
                true,
                "Search term, e.g. `postgres`",
            )]),
        },
        ToolSpec {
            name: "image_pull",
            title: "Pull image",
            description: "Pull an image from its registry (authenticating with any saved login). \
                          Returns once the pull finishes; large images can take minutes.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "image",
                string(),
                true,
                "Image reference to pull, e.g. `postgres:16`",
            )]),
        },
        ToolSpec {
            name: "image_push",
            title: "Push image",
            description: "Push an image to a registry, authenticating with any saved login. Returns \
                          once the push finishes.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "image",
                string(),
                true,
                "Image reference to push, e.g. `ghcr.io/owner/app:latest`",
            )]),
        },
        ToolSpec {
            name: "image_tag",
            title: "Tag image",
            description: "Add a repository:tag alias to an image. Useful before pushing.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                (
                    "id",
                    string(),
                    true,
                    "Image id or existing reference",
                ),
                ("repo", string(), true, "Target repository, e.g. `ghcr.io/owner/app`"),
                ("tag", string(), true, "Target tag, e.g. `latest`"),
            ]),
        },
        ToolSpec {
            name: "image_build",
            title: "Build image",
            description: "Build an image from a Dockerfile. The `req` object is the app's \
                          BuildOptions: `context_dir` (required, the build context directory), \
                          `tag` (required, the target image reference), `dockerfile` (path relative \
                          to the context), `target`, `no_cache`, `pull`, and `build_args` \
                          (`[{key, value}]`). Returns once the build finishes. Check the context \
                          with `image_validate_build_context` first if unsure.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "req",
                object(),
                true,
                "BuildOptions object; `context_dir` and `tag` are required",
            )]),
        },
        ToolSpec {
            name: "image_cancel_build",
            title: "Cancel image build",
            description: "Abort the build currently running, if any.",
            access: Access::Write,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "image_validate_build_context",
            title: "Validate build context",
            description: "Check a directory is a usable build context, reporting whether it exists, \
                          whether it has a Dockerfile, and its size.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[(
                "path",
                string(),
                true,
                "Absolute path to the build context directory",
            )]),
        },
        ToolSpec {
            name: "image_preview_build_command",
            title: "Preview build command",
            description: "Render the exact `docker build` command a build request would run, \
                          without running it.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[(
                "req",
                object(),
                true,
                "BuildOptions object; `context_dir` and `tag` are required",
            )]),
        },
        ToolSpec {
            name: "image_remove",
            title: "Remove image",
            description: "Delete an image. Refuses while containers use it unless `force` is set.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[
                (
                    "id",
                    string(),
                    true,
                    "Image id or reference to remove",
                ),
                (
                    "force",
                    boolean(),
                    false,
                    "Remove even if containers reference it (default false)",
                ),
            ]),
        },
        ToolSpec {
            name: "image_prune",
            title: "Prune images",
            description: "Delete dangling (untagged, unreferenced) images. Returns the count and \
                          bytes reclaimed.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: no_args(),
        },
        // ---------------------------------------------------------------- volumes
        ToolSpec {
            name: "volume_list",
            title: "List volumes",
            description: "Named volumes with driver, mountpoint, labels and scope.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "volume_create",
            title: "Create volume",
            description: "Create a named volume, optionally with a driver and driver options (e.g. \
                          `{\"type\": \"nfs\"}`) and labels.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("name", string(), true, "Volume name"),
                ("driver", string(), false, "Volume driver (default `local`)"),
                ("driver_opts", string_map(), false, "Driver options as a string map"),
                ("labels", string_map(), false, "Volume labels"),
            ]),
        },
        ToolSpec {
            name: "volume_remove",
            title: "Remove volume",
            description: "Delete a named volume and its data. Refuses while a container uses it \
                          unless `force` is set. This is unrecoverable.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[
                ("name", string(), true, "Volume name"),
                (
                    "force",
                    boolean(),
                    false,
                    "Remove even if a container uses it (default false)",
                ),
            ]),
        },
        ToolSpec {
            name: "volume_prune",
            title: "Prune volumes",
            description: "Delete every unused volume — data no container references. This is \
                          unrecoverable. Returns the number removed.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: no_args(),
        },
        // ---------------------------------------------------------------- networks
        ToolSpec {
            name: "network_list",
            title: "List networks",
            description: "Networks with driver, scope, subnet, gateway and attached containers.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "network_create",
            title: "Create network",
            description: "Create a network, optionally with a specific subnet, gateway, internal \
                          (no external route) and attachable flags, and labels.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("name", string(), true, "Network name"),
                ("driver", string(), false, "Network driver (default `bridge`)"),
                ("subnet", string(), false, "CIDR subnet, e.g. `172.28.0.0/16`"),
                ("gateway", string(), false, "Gateway address inside the subnet"),
                (
                    "internal",
                    boolean(),
                    false,
                    "Restrict the network to internal traffic (default false)",
                ),
                (
                    "attachable",
                    boolean(),
                    false,
                    "Allow containers to attach at any time, including swarm services (default false)",
                ),
                ("labels", string_map(), false, "Network labels"),
            ]),
        },
        ToolSpec {
            name: "network_remove",
            title: "Remove network",
            description: "Delete a network. Fails while containers are still attached.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[("id", string(), true, "Network id or name")]),
        },
        ToolSpec {
            name: "network_prune",
            title: "Prune networks",
            description: "Delete networks no container uses, skipping the built-in `bridge`, `host` \
                          and `none`. Returns the number removed.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: no_args(),
        },
        // ---------------------------------------------------------------- compose
        ToolSpec {
            name: "compose_list_projects",
            title: "List compose projects",
            description: "Compose projects discovered from container labels, with the path of each \
                          project's compose file and its services.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "compose_up",
            title: "Compose up",
            description: "Bring a compose project up detached from its compose file, creating and \
                          starting its containers and networks. Slow for a project that needs to \
                          pull or build images; consider interrupting on completion.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "file_path",
                string(),
                true,
                "Path to the compose file (or its directory)",
            )]),
        },
        ToolSpec {
            name: "compose_down",
            title: "Compose down",
            description: "Tear a compose project down: stop and remove its containers and networks. \
                          Named volumes are kept.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[(
                "file_path",
                string(),
                true,
                "Path to the compose file (or its directory)",
            )]),
        },
        ToolSpec {
            name: "compose_logs",
            title: "Compose logs",
            description: "Combined logs from every container in a compose project.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[
                (
                    "file_path",
                    string(),
                    true,
                    "Path to the compose file (or its directory)",
                ),
                ("tail", integer(), false, "Number of trailing lines (default 200)"),
            ]),
        },
        ToolSpec {
            name: "compose_project_action",
            title: "Compose project action",
            description: "Start, stop or restart every container of a compose project by project \
                          name — works without the compose file, using the project's labels.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("project", string(), true, "Compose project name"),
                (
                    "action",
                    enumerated(&["start", "stop", "restart"]),
                    true,
                    "Which lifecycle action to apply to the whole project",
                ),
            ]),
        },
        // ---------------------------------------------------------------- kind / kubernetes
        ToolSpec {
            name: "kind_get_status",
            title: "Get kind status",
            description: "Whether the `kind` (Kubernetes in Docker) binary is installed, its \
                          version, and whether an update is available.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "cluster_list",
            title: "List kind clusters",
            description: "Every kind cluster, with its node count, Kubernetes version, state and \
                          per-node resource caps.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "cluster_create",
            title: "Create kind cluster",
            description: "Create a kind Kubernetes cluster. Slow — a single-node cluster takes tens \
                          of seconds and pulls the node image on first use. Poll `cluster_list` \
                          until the cluster reports running.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("name", string(), true, "Cluster name"),
                ("image", string(), false, "Node image; omit for kind's default"),
                (
                    "nodes",
                    integer(),
                    false,
                    "Number of nodes (1–5, default from config)",
                ),
            ]),
        },
        ToolSpec {
            name: "cluster_delete",
            title: "Delete kind cluster",
            description: "Delete a kind cluster and its node containers, releases its kubeconfig \
                          entry and its app-managed state. Any workloads in it are lost.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[("name", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_start",
            title: "Start cluster",
            description: "Start a stopped kind cluster's node containers.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[("name", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_stop",
            title: "Stop cluster",
            description: "Stop a kind cluster's node containers. The cluster keeps its state on \
                          disk and `cluster_start` brings it back.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[("name", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_kubeconfig",
            title: "Get cluster kubeconfig",
            description: "The kubeconfig for a kind cluster, as YAML. Write it to a file to use \
                          `kubectl` against the cluster.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("name", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_load_image",
            title: "Load image into cluster",
            description: "Load a local image into a kind cluster's nodes so pods can use it without \
                          a registry. Slow for large images.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("image", string(), true, "Local image reference to load"),
            ]),
        },
        ToolSpec {
            name: "cluster_nodes",
            title: "Get cluster nodes",
            description: "A kind cluster's nodes with roles, readiness and Kubernetes version.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_pods",
            title: "Get cluster pods",
            description: "Pods in a kind cluster with namespace, phase, restart count, node and \
                          container statuses. Filter by one namespace with `namespace`.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                (
                    "namespace",
                    string(),
                    false,
                    "Limit to one namespace; omit or pass `all` for every namespace",
                ),
            ]),
        },
        ToolSpec {
            name: "cluster_deployments",
            title: "Get cluster deployments",
            description: "Deployments in a kind cluster with ready/desired replica counts.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_statefulsets",
            title: "Get cluster statefulsets",
            description: "StatefulSets in a kind cluster with ready/desired replica counts.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_daemonsets",
            title: "Get cluster daemonsets",
            description: "DaemonSets in a kind cluster with ready/desired counts.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_services",
            title: "Get cluster services",
            description: "Services in a kind cluster with type, cluster IP and exposed ports.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_events",
            title: "Get cluster events",
            description: "Recent events across a kind cluster, newest last. This is the first place \
                          to look when a pod will not start.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_namespaces",
            title: "Get cluster namespaces",
            description: "Namespaces in a kind cluster with their status and age.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_ingresses",
            title: "Get cluster ingresses",
            description: "Ingress rules in a kind cluster with hosts, paths and backends.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_jobs",
            title: "Get cluster jobs",
            description: "Jobs in a kind cluster with completion counts and status.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_cronjobs",
            title: "Get cluster cronjobs",
            description: "CronJobs in a kind cluster with schedule, suspend state and last run.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_pvcs",
            title: "Get cluster PVCs",
            description: "PersistentVolumeClaims in a kind cluster with status, capacity and \
                          storage class.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_pvs",
            title: "Get cluster PVs",
            description: "PersistentVolumes in a kind cluster with capacity, access modes and \
                          claim reference.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_config_resources",
            title: "Get cluster config resources",
            description: "ConfigMaps and Secrets in a kind cluster. Secret values are omitted \
                          unless `include_secrets` is set — leave it off unless the values are \
                          genuinely needed.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                (
                    "include_secrets",
                    boolean(),
                    false,
                    "Include Secret data values (default false)",
                ),
            ]),
        },
        ToolSpec {
            name: "cluster_summary",
            title: "Get cluster summary",
            description: "A one-call overview of a kind cluster: node, pod, workload and namespace \
                          counts plus resource totals.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_metrics",
            title: "Get cluster metrics",
            description: "CPU and memory usage per node and pod, from the metrics-server. Requires \
                          metrics-server to be installed in the cluster.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_metrics_server_status",
            title: "Get metrics-server status",
            description: "Whether metrics-server is installed and ready in a kind cluster. If it \
                          is missing, `cluster_metrics` returns nothing.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_install_metrics_server",
            title: "Install metrics-server",
            description: "Install metrics-server into a kind cluster so `cluster_metrics` returns \
                          data. Slow — it applies upstream manifests and waits for readiness.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_pod_logs",
            title: "Get pod logs",
            description: "Logs from one pod in a kind cluster.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("namespace", string(), true, "Pod namespace"),
                ("pod", string(), true, "Pod name"),
                ("container", string(), false, "Container name for a multi-container pod"),
                ("tail", integer(), false, "Number of trailing lines (default 200)"),
            ]),
        },
        ToolSpec {
            name: "cluster_delete_workload",
            title: "Delete workload",
            description: "Delete a workload (deployment, statefulset, daemonset, job or cronjob) \
                          from a kind cluster. Pods it owns are removed with it.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                (
                    "kind",
                    enumerated(&[
                        "deployment",
                        "statefulset",
                        "daemonset",
                        "job",
                        "cronjob",
                    ]),
                    true,
                    "Workload kind",
                ),
                ("namespace", string(), true, "Workload namespace"),
                ("name", string(), true, "Workload name"),
            ]),
        },
        ToolSpec {
            name: "cluster_restart_workload",
            title: "Restart workload",
            description: "Roll-restart a workload in a kind cluster by patching its pod template.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                (
                    "kind",
                    enumerated(&["deployment", "statefulset", "daemonset"]),
                    true,
                    "Workload kind",
                ),
                ("namespace", string(), true, "Workload namespace"),
                ("name", string(), true, "Workload name"),
            ]),
        },
        ToolSpec {
            name: "cluster_scale_workload",
            title: "Scale workload",
            description: "Set the replica count of a deployment, statefulset or replicaset in a \
                          kind cluster.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                (
                    "kind",
                    enumerated(&["deployment", "statefulset", "replicaset"]),
                    true,
                    "Workload kind",
                ),
                ("namespace", string(), true, "Workload namespace"),
                ("name", string(), true, "Workload name"),
                ("replicas", integer(), true, "Desired replica count"),
            ]),
        },
        ToolSpec {
            name: "cluster_delete_pod",
            title: "Delete pod",
            description: "Delete a pod from a kind cluster. A controller-managed pod is recreated.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("namespace", string(), true, "Pod namespace"),
                ("pod", string(), true, "Pod name"),
            ]),
        },
        ToolSpec {
            name: "cluster_create_namespace",
            title: "Create namespace",
            description: "Create a namespace in a kind cluster.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("name", string(), true, "Namespace name"),
            ]),
        },
        ToolSpec {
            name: "cluster_drain_node",
            title: "Drain node",
            description: "Cordon a kind node and evict its pods, in preparation for stopping it. \
                          Workloads move to other nodes; pods without a controller are lost.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("node", string(), true, "Node name from `cluster_nodes`"),
            ]),
        },
        ToolSpec {
            name: "cluster_set_node_schedulable",
            title: "Set node schedulable",
            description: "Cordon (false) or uncordon (true) a kind node.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("node", string(), true, "Node name from `cluster_nodes`"),
                ("schedulable", boolean(), true, "true to uncordon, false to cordon"),
            ]),
        },
        ToolSpec {
            name: "cluster_apply_yaml",
            title: "Apply YAML to cluster",
            description: "Apply a Kubernetes manifest (YAML or JSON, possibly with several \
                          documents) to a kind cluster.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("yaml", string(), true, "Manifest contents"),
            ]),
        },
        ToolSpec {
            name: "cluster_diff_yaml",
            title: "Diff YAML against cluster",
            description: "Show what applying a manifest would change in a kind cluster \
                          (`kubectl diff`: a server-side dry run, nothing is changed). Returns a \
                          unified diff and whether anything differs.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("yaml", string(), true, "Manifest contents (at most 90 KiB)"),
            ]),
        },
        ToolSpec {
            name: "cluster_selector_logs",
            title: "Logs by label selector",
            description: "Logs of every pod matching a label selector (or the selector of a \
                          workload) in one namespace, all containers, each line \
                          `<timestamp> [pod/<pod>/<container>] <message>`, sorted by time.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("namespace", string(), true, "Namespace"),
                ("selector", string(), false, "Label selector, e.g. `app=web,tier!=db`"),
                ("workload_kind", string(), false, "deployment, statefulset, daemonset, replicaset or job — used when no selector is given"),
                ("workload_name", string(), false, "Workload name, with workload_kind"),
                ("tail", integer(), false, "Lines per container (default 200)"),
                ("since_time", string(), false, "Only lines at or after this RFC 3339 time"),
            ]),
        },
        ToolSpec {
            name: "cluster_access",
            title: "Cluster host access",
            description: "How a kind cluster is reached from the host: its API server port, \
                          whether that port answers, whether the user's kubeconfig holds its \
                          context, and whether kubectl is installed.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_export_kubeconfig_to_host",
            title: "Add cluster to host kubeconfig",
            description: "Merge a kind cluster's context into the user's kubeconfig \
                          ($KUBECONFIG or ~/.kube/config) and make it the current context. \
                          The file is backed up to <file>.hsx2dock.bak first.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_remove_kubeconfig_from_host",
            title: "Remove cluster from host kubeconfig",
            description: "Remove a kind cluster's context, cluster and user entries from the \
                          user's kubeconfig (backed up to <file>.hsx2dock.bak first).",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_get_cluster_prefs",
            title: "Get cluster preferences",
            description: "The stored preferences for a kind cluster: whether it auto-starts with the \
                          engine and its per-node resource caps.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[("cluster", string(), true, "Cluster name")]),
        },
        ToolSpec {
            name: "cluster_set_cluster_prefs",
            title: "Set cluster preferences",
            description: "Update a kind cluster's stored preferences: auto-start with the engine, \
                          and per-node CPU/memory caps.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[
                ("cluster", string(), true, "Cluster name"),
                ("prefs", object(), true, "Preference object to store for the cluster"),
            ]),
        },
        // ---------------------------------------------------------------- endpoints
        ToolSpec {
            name: "endpoint_list",
            title: "List endpoints",
            description: "Configured Docker endpoints — local and remote — with each one's \
                          reachability. The active endpoint decides which engine every other tool \
                          talks to.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "endpoint_get_active",
            title: "Get active endpoint",
            description: "The endpoint currently in use, or null when the bundled local engine is.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "endpoint_test",
            title: "Test endpoint",
            description: "Check a Docker endpoint is reachable before saving it.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[(
                "endpoint",
                object(),
                true,
                "Endpoint to test: `{name, kind, uri}` — `kind` is `unix`, `tcp`, or `ssh`",
            )]),
        },
        ToolSpec {
            name: "endpoint_save",
            title: "Save endpoint",
            description: "Create or update a Docker endpoint. Does not switch to it — call \
                          `endpoint_set_active` afterwards.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[(
                "endpoint",
                object(),
                true,
                "Endpoint to save: `{name, kind, uri}` and an optional `id` to update in place",
            )]),
        },
        ToolSpec {
            name: "endpoint_set_active",
            title: "Set active endpoint",
            description: "Switch every subsequent tool call to the given endpoint. Pass `local` for \
                          the bundled engine. Engine lifecycle and repair tools are unavailable \
                          while a remote endpoint is active.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[(
                "id",
                string(),
                true,
                "Endpoint id from `endpoint_list`, or `local` for the bundled engine",
            )]),
        },
        ToolSpec {
            name: "endpoint_remove",
            title: "Remove endpoint",
            description: "Delete a saved endpoint. Switching back to `local` first is advisable if \
                          it was active.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[("id", string(), true, "Endpoint id")]),
        },
        // ---------------------------------------------------------------- registries
        ToolSpec {
            name: "registry_list",
            title: "List registry logins",
            description: "Saved registry logins (Docker Hub, ghcr.io, GitLab, Harbor, custom) with \
                          their host and which one is the default push target.",
            access: Access::Read,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "registry_test",
            title: "Test registry login",
            description: "Verify registry credentials before saving them.",
            access: Access::Read,
            engine_only: false,
            input_schema: obj(&[(
                "input",
                object(),
                true,
                "Credentials to test: `{id, registry, username, password}`",
            )]),
        },
        ToolSpec {
            name: "registry_save",
            title: "Save registry login",
            description: "Store a registry login, used automatically by `image_pull` and \
                          `image_push`.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "input",
                object(),
                true,
                "Credentials to save: `{id, registry, username, password}`",
            )]),
        },
        ToolSpec {
            name: "registry_set_default",
            title: "Set default registry",
            description: "Choose the login new pushes default to. An empty id clears the choice so \
                          the first saved login is used.",
            access: Access::Write,
            engine_only: false,
            input_schema: obj(&[(
                "id",
                string(),
                false,
                "Registry login id, or empty to clear the default",
            )]),
        },
        ToolSpec {
            name: "registry_remove",
            title: "Remove registry login",
            description: "Delete a saved registry login.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: obj(&[("id", string(), true, "Registry login id")]),
        },
        // ---------------------------------------------------------------- storage / maintenance
        ToolSpec {
            name: "storage_check_path",
            title: "Check data-root path",
            description: "Inspect a candidate engine data root before migrating to it, reporting \
                          whether it exists, is a directory, is empty, and any filesystem problem.",
            access: Access::Read,
            engine_only: true,
            input_schema: obj(&[(
                "path",
                string(),
                true,
                "Absolute path to inspect",
            )]),
        },
        ToolSpec {
            name: "storage_migrate_data_root",
            title: "Migrate engine data root",
            description: "Point the bundled engine at a different data root, optionally moving the \
                          existing images and containers across. Slow for a large store. Stop the \
                          engine first.",
            access: Access::Destructive,
            engine_only: true,
            input_schema: obj(&[
                ("target_path", string(), true, "New data-root directory"),
                (
                    "move_data",
                    boolean(),
                    true,
                    "Move the existing images/containers to the new location",
                ),
                (
                    "clear_old",
                    boolean(),
                    true,
                    "Delete the old data after a successful move (needs move_data=true)",
                ),
                (
                    "confirm_clear_old",
                    boolean(),
                    false,
                    "Required true together with clear_old: confirms the permanent deletion",
                ),
            ]),
        },
        ToolSpec {
            name: "system_prune",
            title: "Prune system",
            description: "Delete unused containers, images, networks and build cache in one pass. \
                          Volumes are not touched. Unrecoverable — check `engine_get_disk_usage` \
                          first to see what it would reclaim.",
            access: Access::Destructive,
            engine_only: false,
            input_schema: no_args(),
        },
        ToolSpec {
            name: "engine_apply_resource_limits",
            title: "Apply engine resource limits",
            description: "Re-apply the configured engine memory and CPU caps to the running \
                          engine's cgroup, without a restart.",
            access: Access::Write,
            engine_only: true,
            input_schema: no_args(),
        },
    ];

    tools.sort_by_key(|t| t.name);
    tools
}

/// Look up one tool by name.
pub fn find(name: &str) -> Option<ToolSpec> {
    catalogue().into_iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn tools() -> Vec<ToolSpec> {
        catalogue()
    }

    #[test]
    fn tool_names_are_unique() {
        let names: Vec<_> = tools().iter().map(|t| t.name).collect();
        let unique: HashSet<_> = names.iter().collect();
        assert_eq!(
            names.len(),
            unique.len(),
            "duplicate tool name in the catalogue"
        );
    }

    #[test]
    fn tool_names_are_snake_case_words() {
        for t in tools() {
            assert!(
                t.name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{} is not snake_case",
                t.name
            );
            assert!(
                !t.name.starts_with('_'),
                "{} starts with an underscore",
                t.name
            );
            assert!(!t.name.ends_with('_'), "{} ends with an underscore", t.name);
        }
    }

    #[test]
    fn every_tool_has_a_usable_description_and_title() {
        for t in tools() {
            assert!(
                t.description.len() >= 30,
                "{} has a description too short to guide a model: {:?}",
                t.name,
                t.description
            );
            assert!(!t.title.trim().is_empty(), "{} has no title", t.name);
            assert!(
                !t.description.contains("  "),
                "{} description has a double space (likely a wrapping mistake)",
                t.name
            );
        }
    }

    #[test]
    fn every_tool_declares_an_object_schema_with_no_unsatisfiable_requirements() {
        for t in tools() {
            let s = t
                .input_schema
                .as_object()
                .unwrap_or_else(|| panic!("{} schema is not an object", t.name));
            assert_eq!(
                s.get("type").and_then(Value::as_str),
                Some("object"),
                "{} schema type must be \"object\"",
                t.name
            );
            let props = s
                .get("properties")
                .and_then(Value::as_object)
                .unwrap_or_else(|| panic!("{} schema has no properties object", t.name));

            // A `required` entry with no matching property can never be
            // satisfied, which would make the tool permanently uncallable.
            let required = s
                .get("required")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for r in &required {
                let name = r.as_str().expect("required entries must be strings");
                assert!(
                    props.contains_key(name),
                    "{} requires '{name}' but does not declare it",
                    t.name
                );
            }

            // Every declared property needs a type for the model to reason about.
            for (name, schema) in props {
                assert!(
                    schema.get("type").is_some() || schema.get("enum").is_some(),
                    "{} property '{name}' declares neither a type nor an enum",
                    t.name
                );
            }

            // Rejecting unknown keys is what makes a typo a clear error rather
            // than a silently ignored argument.
            assert_eq!(
                s.get("additionalProperties"),
                Some(&Value::Bool(false)),
                "{} must reject unknown arguments",
                t.name
            );
        }
    }

    #[test]
    fn every_declared_property_is_described() {
        for t in tools() {
            let props = t.input_schema["properties"].as_object().unwrap();
            for (name, schema) in props {
                let desc = schema.get("description").and_then(Value::as_str);
                assert!(
                    desc.is_some_and(|d| !d.trim().is_empty()),
                    "{} property '{name}' has no description",
                    t.name
                );
            }
        }
    }

    #[test]
    fn reads_are_read_only_and_writes_are_not() {
        for t in tools() {
            let a = t.to_tool();
            let ann = a.annotations.expect("every tool is annotated");
            assert_eq!(
                ann.read_only_hint,
                Some(t.is_read_only()),
                "{} read_only_hint disagrees with its Access class",
                t.name
            );
        }
    }

    #[test]
    fn only_destructive_tools_carry_the_destructive_hint() {
        for t in tools() {
            let ann = t.to_tool().annotations.unwrap();
            assert_eq!(
                ann.destructive_hint,
                Some(t.access == Access::Destructive),
                "{} destructive_hint disagrees with its Access class",
                t.name
            );
        }
    }

    #[test]
    fn listing_and_inspection_tools_are_read_only() {
        // Guards against a new read tool being added as Access::Write by
        // copy-paste, which would make agents ask for confirmation needlessly.
        for t in tools() {
            let reads = t.name.ends_with("_list")
                || t.name.ends_with("_get")
                || t.name.ends_with("_inspect")
                || t.name.ends_with("_status")
                || t.name.ends_with("_history")
                || t.name.ends_with("_search")
                || t.name.ends_with("_diagnose")
                || t.name.ends_with("_metrics")
                || t.name.ends_with("_kubeconfig")
                || t.name.ends_with("_test");
            if reads {
                assert!(
                    t.is_read_only(),
                    "{} looks like a read tool but is not Read",
                    t.name
                );
            }
        }
    }

    #[test]
    fn engine_lifecycle_tools_are_gated_on_the_bundled_engine() {
        // Anything that reaches outside the active client's engine must refuse
        // while a remote endpoint is set, exactly as the GUI does.
        for name in [
            "engine_start",
            "engine_stop",
            "engine_toggle",
            "engine_diagnose",
            "engine_repair",
            "engine_set_cli_bridge",
            "engine_apply_resource_limits",
            "storage_check_path",
            "storage_migrate_data_root",
        ] {
            let t = find(name).unwrap_or_else(|| panic!("{name} missing from the catalogue"));
            assert!(t.engine_only, "{name} must be engine_only");
        }
    }

    #[test]
    fn engine_lifecycle_tools_are_not_read_only() {
        for name in [
            "engine_start",
            "engine_stop",
            "engine_toggle",
            "engine_repair",
            "storage_migrate_data_root",
        ] {
            let t = find(name).unwrap();
            assert!(!t.is_read_only(), "{name} changes engine state");
        }
    }

    #[test]
    fn deleting_and_pruning_tools_are_destructive() {
        for name in [
            "container_remove",
            "container_recreate",
            "container_prune",
            "image_remove",
            "image_prune",
            "volume_remove",
            "volume_prune",
            "network_remove",
            "network_prune",
            "compose_down",
            "cluster_delete",
            "cluster_delete_pod",
            "cluster_delete_workload",
            "cluster_drain_node",
            "system_prune",
            "endpoint_remove",
            "registry_remove",
            "storage_migrate_data_root",
        ] {
            let t = find(name).unwrap_or_else(|| panic!("{name} missing from the catalogue"));
            assert_eq!(
                t.access,
                Access::Destructive,
                "{name} removes data and must be Destructive"
            );
        }
    }

    #[test]
    fn engine_config_and_endpoint_switches_need_confirmation() {
        // Stopping the engine stops every container; a config write or an
        // endpoint switch redirects everything after it.
        for name in [
            "engine_stop",
            "engine_toggle",
            "engine_set_config",
            "endpoint_set_active",
            "endpoint_save",
        ] {
            assert_eq!(find(name).unwrap().access, Access::Destructive, "{name}");
        }
    }

    #[test]
    fn destructive_tools_advertise_a_required_confirm_argument() {
        for t in tools() {
            let schema = Value::Object(t.advertised_schema());
            let required: Vec<&str> = schema["required"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let has = required.contains(&CONFIRM_ARG)
                && schema["properties"][CONFIRM_ARG]["type"] == "boolean";
            assert_eq!(has, t.access == Access::Destructive, "{}", t.name);
        }
    }

    #[test]
    fn access_refusal_enforces_read_only_mode_and_confirmation() {
        assert_eq!(access_refusal(Access::Read, false, false), None);
        assert_eq!(access_refusal(Access::Read, true, false), None);
        assert_eq!(access_refusal(Access::Write, false, false), None);
        assert!(access_refusal(Access::Write, true, true).is_some());
        assert!(access_refusal(Access::Destructive, false, false).is_some());
        assert_eq!(access_refusal(Access::Destructive, false, true), None);
        assert!(access_refusal(Access::Destructive, true, true).is_some());
    }

    #[test]
    fn required_string_arguments_are_non_empty_by_construction() {
        // The schema says `type: string`; `Args::req_str` rejects blank values.
        // This pins the pairing so a schema cannot claim a field is required
        // while the validator would accept an empty string for it.
        for t in tools() {
            let s = &t.input_schema;
            let required = s["required"].as_array().cloned().unwrap_or_default();
            for r in required {
                let name = r.as_str().unwrap();
                let prop = &s["properties"][name];
                let ty = prop.get("type").and_then(Value::as_str);
                assert!(
                    matches!(
                        ty,
                        Some("string")
                            | Some("boolean")
                            | Some("integer")
                            | Some("array")
                            | Some("object")
                    ),
                    "{}: required '{name}' has an unsupported type {ty:?}",
                    t.name
                );
            }
        }
    }

    #[test]
    fn find_resolves_every_catalogued_tool_and_rejects_unknown_ones() {
        for t in tools() {
            assert_eq!(find(t.name).map(|f| f.name), Some(t.name));
        }
        assert!(find("no_such_tool").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn to_tool_preserves_the_contract_an_agent_sees() {
        let t = find("container_start").unwrap();
        let wire = t.to_tool();
        assert_eq!(wire.name, "container_start");
        assert_eq!(wire.title.as_deref(), Some("Start container"));
        assert_eq!(wire.description.as_deref(), Some(t.description));
        assert_eq!(wire.input_schema["type"], "object");
        assert_eq!(wire.input_schema["required"][0], "id");
        assert_eq!(
            wire.input_schema["additionalProperties"],
            Value::Bool(false)
        );
    }

    #[test]
    fn the_catalogue_is_stable_across_calls() {
        // `find` and `list_tools` both rebuild the catalogue; a stable order
        // keeps `tools/list` output byte-identical between calls, which lets
        // clients cache it and keeps diffs honest.
        let a: Vec<_> = tools().iter().map(|t| t.name).collect();
        let b: Vec<_> = tools().iter().map(|t| t.name).collect();
        assert_eq!(a, b);
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(a, sorted, "catalogue must be sorted by name");
    }

    #[test]
    fn the_catalogue_covers_every_domain_the_gui_exposes() {
        // A coverage floor: if a domain vanishes from the catalogue, agents
        // silently lose the ability to manage it.
        for prefix in [
            "engine_",
            "container_",
            "image_",
            "volume_",
            "network_",
            "compose_",
            "cluster_",
            "kind_",
            "endpoint_",
            "registry_",
            "group_",
            "storage_",
            "system_",
        ] {
            assert!(
                tools().iter().any(|t| t.name.starts_with(prefix)),
                "no tool covers the '{prefix}' domain"
            );
        }
    }
}
