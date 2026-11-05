//! Engine self-diagnosis and repair.
//!
//! Docker's failures are famously opaque — a dangling symlink in the data root
//! surfaces as `Unimplemented: unknown service
//! containerd.services.leases.v1.Leases`, and the only clue lives in the
//! engine log. This module turns those conditions into named checks with a
//! health state, a human explanation, and a repair action, so the app can fix
//! itself instead of asking the user to read dockerd's log.
//!
//! Two entry points:
//!
//! * [`diagnose`] — read-only. Every check is a filesystem/process probe.
//! * [`repair`] — applies the fix for the requested checks.
//!
//! [`auto_repair`] is the subset that is safe to run unattended before every
//! engine start: it never deletes user data, so a fresh data root, a moved
//! storage location, or a dangling compat link all recover on their own.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::engine::{
    self, compose_plugin_dir, dir_supports_d_type, EnginePaths, SNAPSHOTTER_DTYPE_PLUGINS,
};

/// Stable ids for every check. The UI addresses repairs by these, so they are
/// part of the frontend contract and must not be renamed casually.
pub const CHECK_ENGINE_INSTALL: &str = "engine-install";
pub const CHECK_ENGINE_BINARIES: &str = "engine-binaries";
pub const CHECK_DATA_ROOT_FS: &str = "data-root-fs";
pub const CHECK_SNAPSHOTTER_LINKS: &str = "snapshotter-links";
pub const CHECK_COMPOSE_PLUGIN: &str = "compose-plugin";
pub const CHECK_CONTAINERD_STATE: &str = "containerd-state";
pub const CHECK_CONTAINERD_PLUGINS: &str = "containerd-plugins";
pub const CHECK_SOCKET: &str = "engine-socket";
pub const CHECK_NET_HELPER: &str = "net-helper";

/// How a check (or a containerd plugin) is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    /// Working as intended.
    Ok,
    /// Works, but something is unusual — worth showing, not worth fixing.
    Warning,
    /// Broken; the engine cannot do its job until this is fixed.
    Error,
    /// Not present yet (fresh install, or a component that was never set up).
    Missing,
}

impl Health {
    /// Rank used to pick the worst state for the card summary.
    fn rank(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Missing => 1,
            Self::Warning => 2,
            Self::Error => 3,
        }
    }
}

/// One diagnosed check.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineCheck {
    /// Stable id — the UI sends this back to [`repair`].
    pub id: String,
    pub label: String,
    pub health: Health,
    /// One line the user can act on; never a raw error dump.
    pub detail: String,
    /// Filesystem path the check looks at, when there is one.
    pub path: Option<String>,
    /// True when [`repair`] can fix this without user data loss.
    pub repairable: bool,
    /// True when [`auto_repair`] applies it before every engine start.
    pub auto: bool,
}

/// One containerd plugin as reported by the engine log.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContainerdPlugin {
    pub id: String,
    pub health: Health,
    pub detail: String,
}

/// Everything the Settings card renders in one round trip.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EngineDoctorReport {
    /// Worst health across every check — drives the card's colour.
    pub health: Health,
    pub checks: Vec<EngineCheck>,
    /// Plugin rows parsed from the engine log; empty until the engine has run.
    pub plugins: Vec<ContainerdPlugin>,
    /// Data root actually in use, so the card can name it.
    pub data_root: String,
    /// True when the engine is currently answering on its socket.
    pub engine_running: bool,
}

/// What a repair run changed.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RepairReport {
    /// Check ids that were fixed.
    pub repaired: Vec<String>,
    /// `(check id, reason)` for checks that could not be fixed.
    pub failed: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Diagnosis
// ---------------------------------------------------------------------------

/// Run every check and report. Read-only: never mutates the filesystem.
pub fn diagnose(paths: &EnginePaths) -> EngineDoctorReport {
    let mut checks = vec![
        check_engine_install(paths),
        check_engine_binaries(paths),
        check_data_root_fs(paths),
        check_snapshotter_links(paths),
        check_compose_plugin(paths),
        check_containerd_state(paths),
        check_socket(paths),
        check_net_helper(paths),
    ];

    let plugins = read_containerd_plugins(&paths.log_file);
    checks.push(check_containerd_plugin_failures(&plugins));

    let health = checks
        .iter()
        .map(|c| c.health)
        .max_by_key(|h| h.rank())
        .unwrap_or(Health::Ok);

    EngineDoctorReport {
        health,
        checks,
        plugins,
        data_root: paths.data_root.display().to_string(),
        engine_running: engine::is_usable_socket(&paths.socket),
    }
}

/// Rootless networking needs a user-space net helper (`pasta` preferred, else
/// `slirp4netns`). Without one the engine cannot start at all — flag it here so
/// the Doctor card names the fix instead of the start button failing with a
/// rootlesskit error.
fn check_net_helper(paths: &EnginePaths) -> EngineCheck {
    let engine_installed = paths.current_dir.join(".complete").is_file();
    let (health, detail) = if !engine::missing_net_helper(paths) {
        (Health::Ok, "A rootless network helper (pasta or slirp4netns) is available.".into())
    } else if engine_installed {
        (
            Health::Error,
            "No rootless network helper found. Install `pasta` (preferred, \
             e.g. `sudo dnf install passt`) or `slirp4netns`, then start the engine."
                .into(),
        )
    } else {
        (
            Health::Missing,
            "No rootless network helper yet. Install `pasta` (preferred, \
             e.g. `sudo dnf install passt`) or `slirp4netns` before starting the engine."
                .into(),
        )
    };
    EngineCheck {
        id: CHECK_NET_HELPER.into(),
        label: "Rootless network helper".into(),
        health,
        detail,
        path: None,
        // Installing a system package is the user's call; nothing on disk to fix.
        repairable: false,
        auto: false,
    }
}

fn check_engine_install(paths: &EnginePaths) -> EngineCheck {
    let marker = paths.current_dir.join(".complete");
    let installed = marker.is_file();
    EngineCheck {
        id: CHECK_ENGINE_INSTALL.into(),
        label: "Engine installation".into(),
        health: if installed {
            Health::Ok
        } else {
            Health::Missing
        },
        detail: if installed {
            match engine::installed_engine_version(paths) {
                Some(v) => format!("Docker Engine {v} is installed."),
                None => "Engine is installed.".into(),
            }
        } else {
            "No engine is installed yet. Install one from the Docker Engine card.".into()
        },
        path: Some(paths.current_dir.display().to_string()),
        // Reinstalling needs a download; the update flow owns that.
        repairable: false,
        auto: false,
    }
}

/// Every binary dockerd needs at runtime. A partial extract (interrupted
/// download, an antivirus quarantine, a moved folder) leaves the engine
/// installed but unable to start.
const REQUIRED_BINARIES: [&str; 5] = ["dockerd", "containerd", "rootlesskit", "runc", "docker"];

fn check_engine_binaries(paths: &EnginePaths) -> EngineCheck {
    let bin = paths.bin_dir();
    let missing: Vec<&str> = REQUIRED_BINARIES
        .iter()
        .copied()
        .filter(|name| !bin.join(name).is_file())
        .collect();

    if missing.is_empty() {
        return EngineCheck {
            id: CHECK_ENGINE_BINARIES.into(),
            label: "Engine binaries".into(),
            health: Health::Ok,
            detail: format!(
                "All {} required binaries are present.",
                REQUIRED_BINARIES.len()
            ),
            path: Some(bin.display().to_string()),
            repairable: false,
            auto: false,
        };
    }

    EngineCheck {
        id: CHECK_ENGINE_BINARIES.into(),
        label: "Engine binaries".into(),
        health: if paths.current_dir.join(".complete").is_file() {
            // The marker says installed but the files are gone: a real fault.
            Health::Error
        } else {
            Health::Missing
        },
        detail: format!(
            "Missing: {}. Reinstall the engine to restore them.",
            missing.join(", ")
        ),
        path: Some(bin.display().to_string()),
        repairable: false,
        auto: false,
    }
}

fn check_data_root_fs(paths: &EnginePaths) -> EngineCheck {
    let root = &paths.data_root;
    let supports = dir_supports_d_type(root);

    let (health, detail) = match supports {
        Some(true) => (
            Health::Ok,
            "The filesystem reports directory entry types; containerd snapshotters work here."
                .to_string(),
        ),
        Some(false) => (
            Health::Warning,
            "This filesystem does not report d_type, so containerd's snapshotter directories are \
             redirected to a compatible location. This is handled automatically."
                .to_string(),
        ),
        None => (
            Health::Warning,
            "Could not determine the filesystem type. The engine will report a clear error if it \
             matters."
                .to_string(),
        ),
    };

    EngineCheck {
        id: CHECK_DATA_ROOT_FS.into(),
        label: "Storage filesystem".into(),
        health,
        detail,
        path: Some(root.display().to_string()),
        repairable: false,
        auto: false,
    }
}

/// A snapshotter link that does not resolve is the exact condition that makes
/// every pull and every container start fail with the lease error.
fn check_snapshotter_links(paths: &EnginePaths) -> EngineCheck {
    let daemon = paths.data_root.join("containerd").join("daemon");
    let mut dangling: Vec<&str> = Vec::new();
    let mut linked = 0usize;

    for plugin in SNAPSHOTTER_DTYPE_PLUGINS {
        let link = daemon.join(plugin);
        let Ok(meta) = std::fs::symlink_metadata(&link) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            linked += 1;
            if !link.exists() {
                dangling.push(plugin);
            }
        }
    }

    if !dangling.is_empty() {
        return EngineCheck {
            id: CHECK_SNAPSHOTTER_LINKS.into(),
            label: "Snapshotter links".into(),
            health: Health::Error,
            detail: format!(
                "{} points at a location that no longer exists. Every image pull and container \
                 start fails until this is repaired.",
                dangling.join(", ")
            ),
            path: Some(daemon.display().to_string()),
            repairable: true,
            auto: true,
        };
    }

    EngineCheck {
        id: CHECK_SNAPSHOTTER_LINKS.into(),
        label: "Snapshotter links".into(),
        health: Health::Ok,
        detail: if linked > 0 {
            format!("{linked} snapshotter directories are redirected and resolve correctly.")
        } else {
            "No redirection needed on this filesystem.".into()
        },
        path: Some(daemon.display().to_string()),
        // True regardless of the current state: the fields describe the check,
        // not this diagnosis — the UI shows a Repair button only when broken.
        repairable: true,
        auto: true,
    }
}

fn check_compose_plugin(paths: &EnginePaths) -> EngineCheck {
    let canonical = compose_plugin_dir(paths).join("docker-compose");
    let legacy = paths.current_dir.join("cli-plugins").join("docker-compose");

    if canonical.is_file() {
        return EngineCheck {
            id: CHECK_COMPOSE_PLUGIN.into(),
            label: "Compose plugin".into(),
            health: Health::Ok,
            detail: "Docker Compose is installed where the engine looks for it.".into(),
            path: Some(canonical.display().to_string()),
            repairable: true,
            auto: true,
        };
    }

    if legacy.is_file() {
        return EngineCheck {
            id: CHECK_COMPOSE_PLUGIN.into(),
            label: "Compose plugin".into(),
            health: Health::Warning,
            detail: "Compose is installed in the legacy location and will be moved automatically."
                .into(),
            path: Some(legacy.display().to_string()),
            repairable: true,
            auto: true,
        };
    }

    EngineCheck {
        id: CHECK_COMPOSE_PLUGIN.into(),
        label: "Compose plugin".into(),
        health: Health::Missing,
        detail: "Docker Compose is not installed. Install it from the Compose card.".into(),
        path: Some(canonical.display().to_string()),
        repairable: false,
        auto: false,
    }
}

/// A containerd state dir that exists but has no metadata is what a partial
/// copy or a hard kill leaves behind; the engine boots and then fails oddly.
fn check_containerd_state(paths: &EnginePaths) -> EngineCheck {
    let daemon = paths.data_root.join("containerd").join("daemon");
    let meta = daemon.join("io.containerd.metadata.v1.bolt");

    if !daemon.is_dir() {
        return EngineCheck {
            id: CHECK_CONTAINERD_STATE.into(),
            label: "Containerd state".into(),
            health: Health::Missing,
            detail: "Not created yet — the engine builds it on first start.".into(),
            path: Some(daemon.display().to_string()),
            repairable: false,
            auto: false,
        };
    }

    let entries = std::fs::read_dir(&daemon)
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0);

    if entries == 0 {
        return EngineCheck {
            id: CHECK_CONTAINERD_STATE.into(),
            label: "Containerd state".into(),
            health: Health::Warning,
            detail: "The state directory is empty; the engine will initialise it on next start."
                .into(),
            path: Some(daemon.display().to_string()),
            repairable: false,
            auto: false,
        };
    }

    EngineCheck {
        id: CHECK_CONTAINERD_STATE.into(),
        label: "Containerd state".into(),
        health: if meta.is_dir() || meta.exists() {
            Health::Ok
        } else {
            Health::Warning
        },
        detail: if meta.exists() {
            "Containerd metadata is present.".into()
        } else {
            "Containerd has state but no metadata yet; the next start recreates it.".into()
        },
        path: Some(daemon.display().to_string()),
        repairable: false,
        auto: false,
    }
}

fn check_socket(paths: &EnginePaths) -> EngineCheck {
    let live = engine::is_usable_socket(&paths.socket);
    let exists = paths.socket.exists();

    let (health, detail) = if live {
        (
            Health::Ok,
            "The engine is answering on its socket.".to_string(),
        )
    } else if exists {
        (
            Health::Warning,
            "A stale socket file is present but nothing is listening; it is cleared on the next \
             start."
                .to_string(),
        )
    } else {
        (Health::Missing, "The engine is not running.".to_string())
    };

    EngineCheck {
        id: CHECK_SOCKET.into(),
        label: "Engine socket".into(),
        health,
        detail,
        path: Some(paths.socket.display().to_string()),
        repairable: false,
        auto: false,
    }
}

/// Rolls the per-plugin parse into one check, so the summary card shows
/// "2 plugins failed to load" without the user opening the plugin list.
fn check_containerd_plugin_failures(plugins: &[ContainerdPlugin]) -> EngineCheck {
    let failed: Vec<&ContainerdPlugin> = plugins
        .iter()
        .filter(|p| p.health == Health::Error)
        .collect();

    let warnings: Vec<&ContainerdPlugin> = plugins
        .iter()
        .filter(|p| p.health == Health::Warning)
        .collect();

    if plugins.is_empty() {
        return EngineCheck {
            id: CHECK_CONTAINERD_PLUGINS.into(),
            label: "Containerd plugins".into(),
            health: Health::Missing,
            detail: "No plugin information yet — start the engine once to populate it.".into(),
            path: None,
            repairable: false,
            auto: false,
        };
    }

    if failed.is_empty() {
        let detail = if warnings.is_empty() {
            format!("All {} plugins loaded cleanly.", plugins.len())
        } else {
            format!(
                "{} plugins active ({} optional skipped: {}).",
                plugins.len() - warnings.len(),
                warnings.len(),
                warnings
                    .iter()
                    .map(|p| p.id.replace("io.containerd.snapshotter.v1.", "").replace("io.containerd.internal.v1.", ""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        return EngineCheck {
            id: CHECK_CONTAINERD_PLUGINS.into(),
            label: "Containerd plugins".into(),
            health: Health::Ok,
            detail,
            path: None,
            repairable: false,
            auto: false,
        };
    }

    EngineCheck {
        id: CHECK_CONTAINERD_PLUGINS.into(),
        label: "Containerd plugins".into(),
        health: Health::Error,
        detail: format!(
            "{} critical plugin{} failed to load: {}. Running engine repair to restore snapshotter links and restarting usually clears this.",
            failed.len(),
            if failed.len() == 1 { "" } else { "s" },
            failed
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        path: None,
        // The repair is "fix the links, then restart" — handled by the caller.
        repairable: true,
        auto: false,
    }
}

// ---------------------------------------------------------------------------
// Engine log → plugin status
// ---------------------------------------------------------------------------

/// Parse containerd's plugin lines out of the engine log.
///
/// containerd logs one line per plugin at startup:
/// `level=info msg="loading plugin" id=io.containerd.grpc.v1.leases`
/// `level=warning msg="failed to load plugin" error="..." id=...`
/// `level=info msg="skip loading plugin" error="..." id=...`
///
/// Only the last occurrence of each id counts, so a log that spans several
/// boots reports the current state rather than a stale failure.
pub fn read_containerd_plugins(log_file: &Path) -> Vec<ContainerdPlugin> {
    let Ok(text) = std::fs::read_to_string(log_file) else {
        return Vec::new();
    };
    parse_containerd_plugins(&text)
}

/// Plugins containerd itself treats as optional. They are skipped on most
/// installs (`tracing endpoint not configured`, no `mkfs.erofs` in `$PATH`), so
/// a skip is the normal state, not a fault.
fn is_optional_plugin(id: &str) -> bool {
    matches!(
        id,
        "io.containerd.tracing.processor.v1.otlp"
            | "io.containerd.internal.v1.tracing"
            | "io.containerd.differ.v1.erofs"
            | "io.containerd.server.v1.grpc-tcp"
            | "io.containerd.server.v1.metrics"
    ) || id.starts_with("io.containerd.snapshotter.v1.blockfile")
        || id.starts_with("io.containerd.snapshotter.v1.btrfs")
        || id.starts_with("io.containerd.snapshotter.v1.erofs")
        || id.starts_with("io.containerd.snapshotter.v1.zfs")
        || id.starts_with("io.containerd.snapshotter.v1.devmapper")
}

/// A failure that is expected because the engine runs rootless or on an
/// incompatible filesystem/kernel configuration for optional snapshotters,
/// rather than an engine fault.
///
/// Cases:
/// - `io.containerd.internal.v1.opt`: attempts `/opt/containerd` (permission denied in rootless).
/// - `io.containerd.snapshotter.v1.btrfs`: "not supported on this platform: skip plugin" on ext4/xfs.
/// - `io.containerd.snapshotter.v1.devmapper`: "devmapper not supported: skip plugin" or "devmapper not configured".
/// - `io.containerd.snapshotter.v1.zfs` / `erofs`: filesystem missing or kernel module absent.
fn is_expected_rootless_failure(id: &str, line: &str) -> bool {
    if id == "io.containerd.internal.v1.opt" && line.contains("permission denied") {
        return true;
    }

    if is_optional_plugin(id)
        && (line.contains("not supported")
            || line.contains("skip plugin")
            || line.contains("not configured")
            || line.contains("no such file or directory"))
    {
        return true;
    }

    false
}

/// Pure form of [`read_containerd_plugins`], so the parsing is unit-testable.
pub fn parse_containerd_plugins(log: &str) -> Vec<ContainerdPlugin> {
    use std::collections::BTreeMap;

    let mut latest: BTreeMap<String, ContainerdPlugin> = BTreeMap::new();

    for line in log.lines() {
        if !line.contains("plugin") {
            continue;
        }
        let Some(id) = field(line, "id") else {
            continue;
        };
        if !id.starts_with("io.containerd.") {
            continue;
        }

        let plugin = if line.contains("failed to load plugin") {
            ContainerdPlugin {
                id: id.to_string(),
                health: if is_expected_rootless_failure(id, line) {
                    // Rootless containerd cannot create `/opt/containerd`, and
                    // the trace/erofs plugins are optional. Calling these
                    // "broken" would send the user chasing a repair that does
                    // nothing — the engine works fine without them.
                    Health::Warning
                } else {
                    Health::Error
                },
                detail: field(line, "error").unwrap_or("failed to load").to_string(),
            }
        } else if line.contains("skip loading plugin") {
            ContainerdPlugin {
                id: id.to_string(),
                health: if is_optional_plugin(id) {
                    Health::Ok
                } else {
                    Health::Warning
                },
                detail: field(line, "error").unwrap_or("skipped").to_string(),
            }
        } else if line.contains("loading plugin") {
            ContainerdPlugin {
                id: id.to_string(),
                health: Health::Ok,
                detail: "loaded".to_string(),
            }
        } else {
            continue;
        };

        latest.insert(id.to_string(), plugin);
    }

    latest.into_values().collect()
}

/// Value of a `key=value` pair in a logfmt line, with optional quotes.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    if let Some(stripped) = rest.strip_prefix('"') {
        let end = stripped.find('"')?;
        Some(&stripped[..end])
    } else {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        Some(&rest[..end])
    }
}

// ---------------------------------------------------------------------------
// Repair
// ---------------------------------------------------------------------------

/// Checks that [`auto_repair`] applies before every engine start.
pub fn auto_repairable() -> Vec<&'static str> {
    vec![CHECK_SNAPSHOTTER_LINKS, CHECK_COMPOSE_PLUGIN]
}

/// Apply the safe fixes. Called before every engine start, so a fresh data
/// root, a moved storage location, or a dangling compat link heals itself.
///
/// Never deletes user data: it only creates directories, rewrites broken
/// symlinks, and copies the compose plugin into place.
pub fn auto_repair(paths: &EnginePaths) -> RepairReport {
    repair(paths, &auto_repairable())
}

/// Apply the fixes for `ids`, collecting what worked and what did not.
///
/// Unknown ids are ignored rather than failing the whole run: the frontend may
/// be a version behind the backend.
pub fn repair(paths: &EnginePaths, ids: &[&str]) -> RepairReport {
    let mut report = RepairReport::default();

    for id in ids {
        let result = match *id {
            CHECK_SNAPSHOTTER_LINKS => engine::ensure_snapshotter_compat(&paths.data_root),
            CHECK_COMPOSE_PLUGIN => engine::ensure_compose_layout(paths).map(|_| ()),
            // The plugin-failure repair is the snapshotter fix plus a restart;
            // the caller restarts, so only the filesystem part happens here.
            CHECK_CONTAINERD_PLUGINS => engine::ensure_snapshotter_compat(&paths.data_root),
            other => {
                report.failed.push((
                    other.to_string(),
                    "This check has no automatic repair.".into(),
                ));
                continue;
            }
        };

        match result {
            Ok(()) => report.repaired.push((*id).to_string()),
            Err(e) => report.failed.push(((*id).to_string(), e.to_string())),
        }
    }

    report
}

/// Data-root sanity used by the "move storage" wizard: the target must not be
/// the current root and must be writable.
pub fn preflight_data_root(target: &Path) -> Result<()> {
    engine::check_data_root_filesystem(target)
}

/// Default data root when the user has never chosen one.
pub fn default_data_root() -> PathBuf {
    EnginePaths::new().data_root
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = r#"
time="1" level=info msg="loading plugin" id=io.containerd.grpc.v1.healthcheck type=io.containerd.grpc.v1
time="2" level=warning msg="failed to load plugin" error="mkdir /x: file exists" id=io.containerd.snapshotter.v1.overlayfs type=io.containerd.snapshotter.v1
time="3" level=info msg="skip loading plugin" error="no scratch file generator: skip plugin" id=io.containerd.snapshotter.v1.blockfile type=io.containerd.snapshotter.v1
time="4" level=info msg="loading plugin" id=io.containerd.grpc.v1.leases type=io.containerd.grpc.v1
"#;

    #[test]
    fn plugin_log_reports_loaded_failed_and_skipped() {
        let plugins = parse_containerd_plugins(LOG);
        let by_id = |id: &str| {
            plugins
                .iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("{id} missing"))
        };

        assert_eq!(
            by_id("io.containerd.grpc.v1.healthcheck").health,
            Health::Ok
        );
        assert_eq!(by_id("io.containerd.grpc.v1.leases").health, Health::Ok);

        let overlay = by_id("io.containerd.snapshotter.v1.overlayfs");
        assert_eq!(overlay.health, Health::Error);
        assert_eq!(overlay.detail, "mkdir /x: file exists");

        let blockfile = by_id("io.containerd.snapshotter.v1.blockfile");
        // A skipped optional plugin is the normal state, not a warning: the
        // engine does not use it, and flagging it would send the user chasing a
        // repair that changes nothing.
        assert_eq!(blockfile.health, Health::Ok);
        assert!(blockfile.detail.contains("no scratch file generator"));
    }

    #[test]
    fn a_rootless_permission_denial_is_not_reported_as_broken() {
        // Straight from a real rootless engine log: containerd cannot create
        // its scratch dir under /opt and carries on. This must not read as a
        // fault, or every install shows a false error forever.
        let log = r#"
time="1" level=warning msg="failed to load plugin" error="mkdir /opt/containerd: permission denied" id=io.containerd.internal.v1.opt type=io.containerd.internal.v1
"#;
        let plugins = parse_containerd_plugins(log);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].health, Health::Warning);
        assert!(!plugins[0].detail.is_empty());

        // And it does not roll up into a failing "Containerd plugins" check.
        let check = check_containerd_plugin_failures(&plugins);
        assert_eq!(check.health, Health::Ok);
    }

    #[test]
    fn platform_unsupported_snapshotters_are_treated_as_warnings() {
        // When running on ext4/xfs without btrfs/devmapper support, containerd
        // skips those snapshotters with "not supported on this platform" or "devmapper not supported".
        // These are optional plugins and should not fail the engine health check.
        let log = r#"
time="1" level=warning msg="failed to load plugin" error="plugin is not supported on this platform: skip plugin" id=io.containerd.snapshotter.v1.btrfs type=io.containerd.snapshotter.v1
time="2" level=warning msg="failed to load plugin" error="devmapper not supported: skip plugin" id=io.containerd.snapshotter.v1.devmapper type=io.containerd.snapshotter.v1
"#;
        let plugins = parse_containerd_plugins(log);
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].health, Health::Warning);
        assert_eq!(plugins[1].health, Health::Warning);

        let check = check_containerd_plugin_failures(&plugins);
        assert_eq!(check.health, Health::Ok);
        assert!(check.detail.contains("btrfs"));
        assert!(check.detail.contains("devmapper"));
    }

    #[test]
    fn a_real_snapshotter_failure_still_reads_as_broken() {
        // The failure this whole module exists for must not be softened by the
        // rootless exemptions above.
        let log = r#"
time="1" level=warning msg="failed to load plugin" error="mkdir /data/containerd/daemon/io.containerd.snapshotter.v1.overlayfs: file exists" id=io.containerd.snapshotter.v1.overlayfs type=io.containerd.snapshotter.v1
"#;
        let plugins = parse_containerd_plugins(log);
        assert_eq!(plugins[0].health, Health::Error);
        assert_eq!(
            check_containerd_plugin_failures(&plugins).health,
            Health::Error
        );
    }

    #[test]
    fn the_last_line_for_a_plugin_wins() {
        // A log spanning two boots: the earlier failure must not stick once a
        // later boot loaded the plugin cleanly.
        let log = r#"
time="1" level=warning msg="failed to load plugin" error="file exists" id=io.containerd.grpc.v1.leases
time="2" level=info msg="loading plugin" id=io.containerd.grpc.v1.leases
"#;
        let plugins = parse_containerd_plugins(log);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].health, Health::Ok);
    }

    #[test]
    fn non_plugin_lines_are_ignored() {
        let log = r#"
time="1" level=info msg="Daemon has completed initialization"
time="2" level=info msg="API listen on /run/user/1000/hsx2dock/docker.sock"
"#;
        assert!(parse_containerd_plugins(log).is_empty());
    }

    #[test]
    fn logfmt_field_handles_quotes_and_bare_values() {
        assert_eq!(field(r#"a=1 b="two words" c=3"#, "b"), Some("two words"));
        assert_eq!(field("a=1 b=2", "b"), Some("2"));
        assert_eq!(field("a=1", "zz"), None);
        // An unterminated quote must not panic.
        assert_eq!(field(r#"b="unterminated"#, "b"), None);
    }

    #[test]
    fn auto_repair_covers_the_checks_that_must_never_need_a_human() {
        let auto = auto_repairable();
        assert!(auto.contains(&CHECK_SNAPSHOTTER_LINKS));
        assert!(auto.contains(&CHECK_COMPOSE_PLUGIN));
    }

    #[test]
    fn repair_ignores_an_unknown_check_id() {
        let root = std::env::temp_dir().join(format!("dm-doctor-unknown-{}", std::process::id()));
        let paths = test_paths(&root);
        let report = repair(&paths, &["not-a-real-check"]);
        assert!(report.repaired.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "not-a-real-check");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn test_paths(root: &Path) -> EnginePaths {
        EnginePaths {
            data_root: root.join("data"),
            versions_dir: root.join("versions"),
            current_dir: root.join("current"),
            log_file: root.join("engine.log"),
            pid_file: root.join("engine.pid"),
            socket: root.join("docker.sock"),
            runtime_dir: root.join("run"),
            exec_root: root.join("exec"),
            rootless_state: root.join("rootless"),
        }
    }

    #[test]
    fn a_dangling_snapshotter_link_is_an_error_and_is_repairable() {
        let root = std::env::temp_dir().join(format!("dm-doctor-dangling-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = test_paths(&root);
        std::fs::create_dir_all(&paths.current_dir).unwrap();
        std::fs::write(paths.current_dir.join(".complete"), "").unwrap();
        let daemon = paths.data_root.join("containerd").join("daemon");
        std::fs::create_dir_all(&daemon).unwrap();

        let link = daemon.join(SNAPSHOTTER_DTYPE_PLUGINS[0]);
        std::os::unix::fs::symlink(root.join("gone"), &link).unwrap();

        let check = check_snapshotter_links(&paths);
        assert_eq!(check.health, Health::Error);
        assert!(check.repairable);
        assert!(check.auto);
        assert!(check.detail.contains(SNAPSHOTTER_DTYPE_PLUGINS[0]));

        // Repairing it heals the link.
        let report = repair(&paths, &[CHECK_SNAPSHOTTER_LINKS]);
        assert_eq!(report.repaired, vec![CHECK_SNAPSHOTTER_LINKS.to_string()]);
        assert!(link.exists(), "link must resolve after repair");
        assert_eq!(check_snapshotter_links(&paths).health, Health::Ok);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_healthy_root_reports_no_error() {
        let root = std::env::temp_dir().join(format!("dm-doctor-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = test_paths(&root);
        std::fs::create_dir_all(paths.current_dir.join("bin")).unwrap();
        std::fs::write(paths.current_dir.join(".complete"), "").unwrap();
        for name in REQUIRED_BINARIES {
            std::fs::write(paths.current_dir.join("bin").join(name), b"x").unwrap();
        }
        std::fs::create_dir_all(compose_plugin_dir(&paths)).unwrap();
        std::fs::write(compose_plugin_dir(&paths).join("docker-compose"), b"x").unwrap();
        // A healthy root has also booted once, so the plugin check has data.
        std::fs::write(
            &paths.log_file,
            r#"time="1" level=info msg="loading plugin" id=io.containerd.grpc.v1.leases type=io.containerd.grpc.v1
time="2" level=info msg="loading plugin" id=io.containerd.snapshotter.v1.overlayfs type=io.containerd.snapshotter.v1
"#,
        )
        .unwrap();

        let report = diagnose(&paths);
        // A correct install with a *stopped* engine is healthy, not "Ok": the
        // socket and containerd state legitimately read Missing until boot. The
        // contract this test pins is that nothing is broken.
        assert!(report.checks.iter().all(|c| c.health != Health::Error));
        assert!(report.checks.iter().all(|c| c.health != Health::Warning));
        assert_eq!(report.health, Health::Missing, "{:?}", report.checks);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_binary_after_a_complete_marker_is_an_error() {
        let root = std::env::temp_dir().join(format!("dm-doctor-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = test_paths(&root);
        std::fs::create_dir_all(paths.current_dir.join("bin")).unwrap();
        std::fs::write(paths.current_dir.join(".complete"), "").unwrap();
        // Only some binaries present — a partial extract.
        std::fs::write(paths.current_dir.join("bin/dockerd"), b"x").unwrap();

        let check = check_engine_binaries(&paths);
        assert_eq!(check.health, Health::Error);
        assert!(check.detail.contains("containerd"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_legacy_compose_plugin_is_a_warning_that_repair_fixes() {
        let root = std::env::temp_dir().join(format!("dm-doctor-compose-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = test_paths(&root);
        let legacy_dir = paths.current_dir.join("cli-plugins");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("docker-compose"), b"x").unwrap();

        assert_eq!(check_compose_plugin(&paths).health, Health::Warning);

        let report = repair(&paths, &[CHECK_COMPOSE_PLUGIN]);
        assert_eq!(report.repaired, vec![CHECK_COMPOSE_PLUGIN.to_string()]);
        assert!(compose_plugin_dir(&paths).join("docker-compose").is_file());
        assert_eq!(check_compose_plugin(&paths).health, Health::Ok);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plugin_failure_check_summarises_failed_plugins() {
        let plugins = parse_containerd_plugins(LOG);
        let check = check_containerd_plugin_failures(&plugins);
        assert_eq!(check.health, Health::Error);
        assert!(check
            .detail
            .contains("io.containerd.snapshotter.v1.overlayfs"));
        assert!(check.repairable);
        // Never automatic: it needs an engine restart, which the caller owns.
        assert!(!check.auto);
    }
}
