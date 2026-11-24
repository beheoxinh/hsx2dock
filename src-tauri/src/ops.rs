//! Live registry of container lifecycle operations (start/stop/restart).
//!
//! Group starts used to run as one long blocking command: the UI had no way to
//! tell which members were still waiting, and a 30-container group froze the
//! card for the whole pass. Every bulk path now records each member here as
//! `queued` → `running` → removed, and mirrors each transition to the webview
//! as a `container-op` event (and the group as a `group-op` event). The UI can
//! also re-hydrate from [`OpsRegistry::snapshot`] after a page switch.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter};

/// How many Docker lifecycle calls a bulk operation keeps in flight at once.
/// High enough that a big group is not serialised behind one slow container,
/// low enough that dockerd (and the disk) is not stampeded.
pub const BULK_CONCURRENCY: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OpPhase {
    /// Waiting for a concurrency slot.
    Queued,
    /// The Docker call is in flight.
    Running,
    /// Finished successfully (only sent as an event, never stored).
    Done,
    /// Finished with an error (only sent as an event, never stored).
    Failed,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContainerOp {
    pub id: String,
    /// `start` | `stop` | `restart`.
    pub op: String,
    pub phase: OpPhase,
    /// The group that scheduled it, when it came from a bulk action.
    pub group: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GroupOp {
    pub group: String,
    /// `start` | `stop`.
    pub op: String,
    pub phase: OpPhase,
    pub total: usize,
    pub completed: usize,
    pub summary: Option<String>,
}

#[derive(Default)]
pub struct OpsRegistry {
    containers: Mutex<HashMap<String, ContainerOp>>,
    groups: Mutex<HashMap<String, GroupOp>>,
    app: OnceLock<AppHandle>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OpsSnapshot {
    pub containers: Vec<ContainerOp>,
    pub groups: Vec<GroupOp>,
}

impl OpsRegistry {
    /// Wires the registry to the webview. Before this (and in the standalone
    /// MCP binary) transitions are tracked but not broadcast.
    pub fn attach(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }

    fn emit<T: serde::Serialize + Clone>(&self, event: &str, payload: &T) {
        if let Some(app) = self.app.get() {
            let _ = app.emit(event, payload.clone());
        }
    }

    /// True when another operation already owns this container.
    pub fn is_busy(&self, id: &str) -> bool {
        self.containers
            .lock()
            .map(|m| m.contains_key(id))
            .unwrap_or(false)
    }

    /// Claims `id` for `op`. Returns false when it is already being handled,
    /// so two clicks (or a tray click during auto-start) never double-issue.
    pub fn claim(&self, id: &str, op: &str, group: Option<&str>) -> bool {
        let entry = ContainerOp {
            id: id.to_string(),
            op: op.to_string(),
            phase: OpPhase::Queued,
            group: group.map(str::to_string),
            error: None,
        };
        {
            let Ok(mut map) = self.containers.lock() else {
                return false;
            };
            if map.contains_key(id) {
                return false;
            }
            map.insert(id.to_string(), entry.clone());
        }
        self.emit("container-op", &entry);
        true
    }

    pub fn running(&self, id: &str) {
        let entry = {
            let Ok(mut map) = self.containers.lock() else {
                return;
            };
            let Some(e) = map.get_mut(id) else { return };
            e.phase = OpPhase::Running;
            e.clone()
        };
        self.emit("container-op", &entry);
    }

    /// Releases `id` and broadcasts the outcome.
    pub fn finish(&self, id: &str, result: Result<(), String>) {
        let entry = {
            let Ok(mut map) = self.containers.lock() else {
                return;
            };
            map.remove(id)
        };
        let Some(mut entry) = entry else { return };
        match result {
            Ok(()) => entry.phase = OpPhase::Done,
            Err(e) => {
                entry.phase = OpPhase::Failed;
                entry.error = Some(e);
            }
        }
        self.emit("container-op", &entry);
    }

    /// Registers a group pass. Returns false when the same group is already
    /// being started or stopped.
    pub fn begin_group(&self, group: &str, op: &str, total: usize) -> bool {
        let entry = GroupOp {
            group: group.to_string(),
            op: op.to_string(),
            phase: OpPhase::Running,
            total,
            completed: 0,
            summary: None,
        };
        {
            let Ok(mut map) = self.groups.lock() else {
                return false;
            };
            if map.contains_key(group) {
                return false;
            }
            map.insert(group.to_string(), entry.clone());
        }
        self.emit("group-op", &entry);
        true
    }

    pub fn group_is_busy(&self, group: &str) -> bool {
        self.groups
            .lock()
            .map(|m| m.contains_key(group))
            .unwrap_or(false)
    }

    pub fn group_progress(&self, group: &str) {
        let entry = {
            let Ok(mut map) = self.groups.lock() else {
                return;
            };
            let Some(e) = map.get_mut(group) else { return };
            e.completed = (e.completed + 1).min(e.total);
            e.clone()
        };
        self.emit("group-op", &entry);
    }

    pub fn end_group(&self, group: &str, result: &Result<String, String>) {
        let entry = {
            let Ok(mut map) = self.groups.lock() else {
                return;
            };
            map.remove(group)
        };
        let Some(mut entry) = entry else { return };
        entry.completed = entry.total;
        match result {
            Ok(s) => {
                entry.phase = OpPhase::Done;
                entry.summary = Some(s.clone());
            }
            Err(e) => {
                entry.phase = OpPhase::Failed;
                entry.summary = Some(e.clone());
            }
        }
        self.emit("group-op", &entry);
    }

    pub fn snapshot(&self) -> OpsSnapshot {
        OpsSnapshot {
            containers: self
                .containers
                .lock()
                .map(|m| m.values().cloned().collect())
                .unwrap_or_default(),
            groups: self
                .groups
                .lock()
                .map(|m| m.values().cloned().collect())
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_container_can_only_be_claimed_once() {
        let ops = OpsRegistry::default();
        assert!(ops.claim("a", "start", None));
        assert!(!ops.claim("a", "stop", None));
        assert!(ops.is_busy("a"));
        ops.running("a");
        assert_eq!(ops.snapshot().containers[0].phase, OpPhase::Running);
        ops.finish("a", Ok(()));
        assert!(!ops.is_busy("a"));
        assert!(ops.claim("a", "stop", None));
    }

    #[test]
    fn group_progress_is_capped_and_cleared() {
        let ops = OpsRegistry::default();
        assert!(ops.begin_group("g", "start", 1));
        assert!(!ops.begin_group("g", "start", 1));
        ops.group_progress("g");
        ops.group_progress("g");
        assert_eq!(ops.snapshot().groups[0].completed, 1);
        ops.end_group("g", &Ok("1 started".into()));
        assert!(!ops.group_is_busy("g"));
    }
}
