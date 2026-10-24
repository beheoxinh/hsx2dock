//! MCP prompts: ready-made briefs a client can render for the user.
//!
//! A prompt is a starting message, not a tool: it tells the agent what a
//! particular job looks like when done well. They exist because the same
//! three jobs (diagnose the engine, audit container health, audit a cluster)
//! are what this app is actually for, and an agent asked "fix my docker" has
//! no idea which of 109 tools to start with.
//!
//! The text here is the single source of truth for the `hsx2dock://guide`
//! resource too, so an agent that only reads resources still gets the same
//! operating rules.

use rmcp::model::{Prompt, PromptArgument, PromptMessage, Role};

/// One prompt as advertised to clients.
#[derive(Debug, Clone, Copy)]
pub struct PromptSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// `(name, description, required)`
    pub arguments: &'static [(&'static str, &'static str, bool)],
}

impl PromptSpec {
    pub fn to_prompt(&self) -> Prompt {
        let args = self
            .arguments
            .iter()
            .map(|(name, desc, required)| {
                PromptArgument::new(*name)
                    .with_description(*desc)
                    .with_required(*required)
            })
            .collect::<Vec<_>>();
        Prompt::new(self.name, Some(self.description), Some(args))
            .with_title(self.title.to_string())
    }
}

pub fn catalogue() -> Vec<PromptSpec> {
    vec![
        PromptSpec {
            name: "diagnose-engine",
            title: "Diagnose the Docker engine",
            description: "Walk the engine from status to a concrete diagnosis, and repair only \
                          what is safe to repair automatically.",
            arguments: &[(
                "symptom",
                "What the user reported, if anything (e.g. \"containers won't start\"). \
                 Optional.",
                false,
            )],
        },
        PromptSpec {
            name: "container-health",
            title: "Audit container health",
            description: "Check every container for restart loops, missing health checks, \
                          published-on-all-interfaces ports and unbounded resource use.",
            arguments: &[],
        },
        PromptSpec {
            name: "cluster-audit",
            title: "Audit a kind cluster",
            description: "Check a cluster's nodes, workloads, events and metrics, and report \
                          what is unhealthy before changing anything.",
            arguments: &[(
                "cluster",
                "Cluster name. Omit to audit every cluster.",
                false,
            )],
        },
        PromptSpec {
            name: "plan-environment",
            title: "Plan and stand up an environment",
            description: "Turn a description of an environment into a concrete plan using this \
                          app's tools, then execute it in dependency order.",
            arguments: &[(
                "description",
                "What the environment should contain, in the user's words.",
                true,
            )],
        },
    ]
}

/// The shared operating rules, used by the guide resource and by every prompt.
pub const OPERATING_RULES: &str = "\
Operating rules for this server:

1. Start with `engine_get_status`. If it is not `Running`, nothing else will
   work; start it with `engine_start` (or `engine_toggle`) and poll
   `engine_get_status` until it reports `Running`.
2. Read before you write. `*_list`, `*_get_*` and `*_inspect` are read-only;
   `container_inspect` is the fastest way to learn the shape of a `RunConfig`
   before calling `container_run`.
3. Engine-lifecycle and repair tools (`engine_start`, `engine_stop`,
   `engine_toggle`, `engine_diagnose`, `engine_repair`,
   `engine_apply_resource_limits`, `engine_set_cli_bridge`) refuse while a
   remote endpoint is active, because they act on the bundled engine's own
   files. Check `endpoint_get_active` first if that error appears.
4. Long operations (`image_pull`, `image_push`, `image_build`,
   `cluster_create`, `compose_up`, `storage_migrate_data_root`) return when
   the work is accepted or finished — a success response means it completed.
   Follow up with the matching `*_list` rather than assuming.
5. Destructive tools are marked as such in their annotations: `*_remove`,
   `*_prune`, `cluster_delete`, `cluster_delete_workload`,
   `cluster_delete_pod`, `cluster_drain_node`, `storage_migrate_data_root`.
   Confirm the target exists and is the intended one before calling them.
6. Prefer the narrowest tool. `container_logs` with a small `tail` beats
   `container_inspect` when you only need the last few lines, and
   `cluster_summary` beats five separate cluster calls when you need an
   overview.
7. Every tool returns structured JSON on success and a readable error
   message on failure. An error is the engine's own message — read it before
   retrying; a retry of the same failing call will fail the same way.";

pub fn messages(
    name: &str,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Option<Vec<PromptMessage>> {
    let text = match name {
        "diagnose-engine" => {
            let symptom = args
                .get("symptom")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let mut t = String::from(
                "Diagnose this machine's Docker engine.\n\n\
                 Do this in order:\n\
                 1. `engine_get_status` — record the exact state.\n\
                 2. If it is installed, `engine_get_info` and `engine_get_disk_usage` for \
                 version and space pressure.\n\
                 3. `engine_diagnose` — read every check, including the ones that passed; \
                 a failing check usually names its own cause.\n\
                 4. Only if a check is auto-repairable, call `engine_repair` (optionally with \
                 the specific `check_ids`). Never repair a check that reports manual \
                 intervention.\n\
                 5. Re-run `engine_diagnose` to confirm the repair took, then `engine_get_status`.\n\n\
                 Report: what was wrong, what you changed, and what still needs the user.\n",
            );
            if let Some(s) = symptom {
                t.push_str(&format!("\nThe user reported: {s}\n"));
            }
            t.push('\n');
            t.push_str(OPERATING_RULES);
            t
        }
        "container-health" => format!(
            "Audit container health on this machine.\n\n\
             Do this in order:\n\
             1. `container_list` with `all: true` and `container_sizes`.\n\
             2. For anything restarting, exited unexpectedly or marked unhealthy, call \
             `container_inspect` and `container_logs` (tail 100) — read the exit code and the \
             last error before forming a theory.\n\
             3. Flag containers publishing ports on every interface; the app marks these in \
             its own UI and they are a real exposure risk.\n\
             4. Check `container_stats` for containers under memory pressure, and \
             `container_inspect` for ones with no memory/CPU limit at all.\n\
             5. Summarise as a table: container, problem, evidence, suggested fix. Do not \
             restart or remove anything without being asked.\n\n\
             {OPERATING_RULES}"
        ),
        "cluster-audit" => {
            let cluster = args
                .get("cluster")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let target = match cluster {
                Some(c) => format!("the cluster `{c}`"),
                None => "every kind cluster (`cluster_list` first)".to_string(),
            };
            format!(
                "Audit {target}.\n\n\
                 Per cluster, in this order:\n\
                 1. `cluster_summary` — API server readiness and version.\n\
                 2. `cluster_nodes` — every node `Ready`, none unschedulable unexpectedly.\n\
                 3. `cluster_pods` — non-Running pods, restart counts, crash loops.\n\
                 4. `cluster_events` — read newest first; `Warning` events usually name the \
                 failing object.\n\
                 5. `cluster_metrics` (and `cluster_metrics_server_status` if it errors) for \
                 pressure; `cluster_deployments`/`cluster_statefulsets` for replica shortfalls.\n\
                 6. `cluster_pod_logs` for anything crash-looping — one pod is enough to see \
                 the cause.\n\n\
                 Report findings only. Scaling, deleting pods and draining nodes are \
                 destructive; ask before doing any of them.\n\n\
                 {OPERATING_RULES}"
            )
        }
        "plan-environment" => {
            let description = args
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            format!(
                "Plan and stand up this environment: {description}\n\n\
                 Work in this order:\n\
                 1. `engine_get_status` — the engine must be running first.\n\
                 2. Inspect what already exists (`container_list`, `image_list`, `volume_list`, \
                 `network_list`) so you reuse rather than duplicate.\n\
                 3. Write the plan as a numbered list: networks and volumes first, then images \
                 (`image_pull`/`image_build`), then containers (`container_run`) or a compose \
                 file (`compose_up`).\n\
                 4. Execute in that order, checking each step's result before the next.\n\
                 5. Finish with `container_list` to confirm everything is running, and report \
                 each container's name, image and published ports.\n\n\
                 {OPERATING_RULES}"
            )
        }
        _ => return None,
    };
    Some(vec![PromptMessage::new(
        Role::User,
        rmcp::model::ContentBlock::text(text),
    )])
}
