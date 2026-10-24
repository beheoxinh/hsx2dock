//! The MCP resource catalogue.
//!
//! Resources are the *read* half of the agent's contract: a client can pull
//! the current state of the machine without spending a tool call, and the
//! `hsx2dock://guide` resource is what makes an agent useful on its first
//! turn — it carries the same instructions a tool-only client would never see.
//!
//! Every entry resolves to a handler in [`crate::mcp::server`]; this module
//! only declares what exists, so the list can be asserted without an engine.

use rmcp::model::Resource;

/// One addressable resource.
#[derive(Debug, Clone, Copy)]
pub struct ResourceSpec {
    pub uri: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub mime_type: &'static str,
}

impl ResourceSpec {
    pub fn to_resource(&self) -> Resource {
        Resource::new(self.uri, self.name)
            .with_title(self.title.to_string())
            .with_description(self.description.to_string())
            .with_mime_type(self.mime_type.to_string())
    }
}

/// Everything this server exposes as a resource.
pub fn catalogue() -> Vec<ResourceSpec> {
    vec![
        ResourceSpec {
            uri: "hsx2dock://guide",
            name: "guide",
            title: "Hsx2Dock agent guide",
            description: "How to drive Hsx2Dock: the engine's lifecycle, which tools are \
                          read-only, what must never be called while a remote endpoint is \
                          active, and the shape of the objects tools take as arguments.",
            mime_type: "text/markdown",
        },
        ResourceSpec {
            uri: "hsx2dock://engine/status",
            name: "engine-status",
            title: "Engine status",
            description: "Daemon state, socket path, installed version and whether the socket \
                          is live. The first thing to read when anything looks broken.",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://engine/doctor",
            name: "engine-doctor",
            title: "Engine doctor report",
            description: "Every diagnostic check with its status and repair hint. Unavailable \
                          while a remote endpoint is active (the checks describe the bundled \
                          engine's own files).",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://containers",
            name: "containers",
            title: "Containers",
            description: "Every container, running and stopped, with image, state, ports and \
                          compose project labels.",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://images",
            name: "images",
            title: "Images",
            description: "Local images with tags, size and creation date.",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://volumes",
            name: "volumes",
            title: "Volumes",
            description: "Named volumes with driver and mountpoint.",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://networks",
            name: "networks",
            title: "Networks",
            description: "Docker networks with driver, scope and attached containers.",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://clusters",
            name: "clusters",
            title: "Kind clusters",
            description: "Kind (Kubernetes-in-Docker) clusters with node counts and readiness.",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://groups",
            name: "groups",
            title: "Container groups",
            description: "Saved container groups and their members.",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://config",
            name: "config",
            title: "App configuration",
            description: "The persisted settings: data root, CLI mode and bridge state, \
                          theme, engine caps, kind defaults and notification opt-outs. \
                          Registry logins are summarised — secrets are never included.",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://endpoints",
            name: "endpoints",
            title: "Endpoints",
            description: "The local bundled engine plus every saved remote endpoint, with \
                          which one is active and whether it is reachable.",
            mime_type: "application/json",
        },
        ResourceSpec {
            uri: "hsx2dock://tool-catalogue",
            name: "tool-catalogue",
            title: "Tool catalogue",
            description: "Every tool with its arguments, safety class and whether it is \
                          engine-only. Useful for planning a multi-step task.",
            mime_type: "application/json",
        },
    ]
}
