//! Bridge the bundled engine's private unix socket to the system's Docker CLI
//! so `docker ...` in a terminal talks to *this* app's engine.
//!
//! Two complementary, optional mechanisms, both re-applied on a timer:
//!
//! 1. **Symlink** `$XDG_RUNTIME_DIR/docker.sock -> <engine socket>` — where
//!    rootless/XDG-convention tools look. Never touches `/var/run/docker.sock`
//!    (root-owned; stealing it would break a system docker daemon).
//! 2. **Docker context** `docker-manager-engine` pointing at the engine socket.
//!    `docker context use` makes the system CLI default to it. We create/repair
//!    only our own context and never modify any other.
//!
//! The context (selected as default) is what makes a bare `docker ps` reach our
//! engine; the XDG symlink alone does not, since the CLI ignores it unless
//! `DOCKER_HOST` points there. On take-over we remember the context the user
//! was on and restore it when the bridge is torn down, so we never clobber it.
//!
//! State introspection is file-level (no daemon ping), so the 10s timer is
//! cheap and safe on the main loop.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Context name this app owns. Namespaced, so it never collides by accident.
pub const CONTEXT_NAME: &str = "docker-manager-engine";

/// `$XDG_RUNTIME_DIR/docker.sock` — the conventional rootless socket path.
pub fn xdg_socket() -> PathBuf {
    let runtime = dirs::runtime_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".local/state")
    });
    runtime.join("docker.sock")
}

/// `$DOCKER_CONFIG` if set, else `~/.docker`.
pub fn docker_config_dir() -> PathBuf {
    if let Some(c) = std::env::var_os("DOCKER_CONFIG") {
        return PathBuf::from(c);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".docker")
}

// ---------------------------------------------------------------------------
// Symlink
// ---------------------------------------------------------------------------

/// Symlink state at `link` relative to `engine_socket`:
/// `Some(true)` ours and correct, `Some(false)` a symlink pointing elsewhere,
/// `None` absent **or** a real (non-symlink) file we must never touch.
fn symlink_state_at(link: &Path, engine_socket: &Path) -> Option<bool> {
    let md = std::fs::symlink_metadata(link).ok()?;
    if !md.file_type().is_symlink() {
        return None;
    }
    let target = std::fs::read_link(link).ok()?;
    Some(target == engine_socket)
}

pub fn xdg_symlink_state(engine_socket: &Path) -> Option<bool> {
    symlink_state_at(&xdg_socket(), engine_socket)
}

/// Create the symlink at `link` if absent. Never overwrites an existing entry
/// (ours or foreign). `Ok(true)` = created now, `Ok(false)` = already handled.
fn ensure_symlink_at(link: &Path, engine_socket: &Path) -> anyhow::Result<bool> {
    if std::fs::symlink_metadata(link).is_ok() {
        return Ok(false); // present in any form: leave it (fix only if stale-ours)
    }
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::os::unix::fs::symlink(engine_socket, link)?;
    Ok(true)
}

/// Ensure `$XDG_RUNTIME_DIR/docker.sock` points at our engine.
///
/// Repairs the case where it is *our* symlink but the target moved (engine
/// reinstall / runtime dir rotation). Never touches a foreign file.
pub fn ensure_xdg_symlink(engine_socket: &Path) -> anyhow::Result<bool> {
    let link = xdg_socket();
    match symlink_state_at(&link, engine_socket) {
        Some(true) => return Ok(false),
        Some(false) => {
            // Ours-shaped symlink (a symlink) but wrong target. If it points at
            // a path under our own runtime dir, it is a stale link of ours —
            // repoint it. Anything else is foreign: leave alone.
            if let Ok(target) = std::fs::read_link(&link) {
                let ours = dirs::runtime_dir()
                    .map(|r| target.starts_with(r.join("docker-manager")))
                    .unwrap_or(false);
                if ours {
                    let _ = std::fs::remove_file(&link);
                } else {
                    return Ok(false);
                }
            }
        }
        None => {}
    }
    ensure_symlink_at(&link, engine_socket)
}

// ---------------------------------------------------------------------------
// Docker context
// ---------------------------------------------------------------------------

fn contexts_meta_dir() -> PathBuf {
    docker_config_dir().join("contexts").join("meta")
}

/// The app context's endpoint host from its `meta.json`, matched by Name
/// (robust to the sha256 dir-naming scheme and to re-creation).
pub fn our_context_host() -> Option<String> {
    for entry in std::fs::read_dir(contexts_meta_dir()).ok()?.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path().join("meta.json")) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if json.get("Name").and_then(|v| v.as_str()) != Some(CONTEXT_NAME) {
            continue;
        }
        return json
            .pointer("/Endpoints/docker/Host")
            .and_then(|v| v.as_str())
            .map(str::to_string);
    }
    None
}

fn engine_host(engine_socket: &Path) -> String {
    format!("unix://{}", engine_socket.display())
}

pub fn has_our_context(engine_socket: &Path) -> bool {
    our_context_host().as_deref() == Some(engine_host(engine_socket).as_str())
}

/// Create the app context when missing/stale. Does not select it as default.
pub fn ensure_context(engine_socket: &Path) -> anyhow::Result<()> {
    if has_our_context(engine_socket) {
        return Ok(());
    }
    if our_context_host().is_some() {
        let _ = Command::new("docker")
            .args(["context", "rm", "-f", CONTEXT_NAME])
            .status();
    }
    let ok = Command::new("docker")
        .args(["context", "create", CONTEXT_NAME, "--docker"])
        .arg(format!("host={}", engine_host(engine_socket)))
        .status()?
        .success();
    anyhow::ensure!(ok, "docker context create {CONTEXT_NAME} failed");
    Ok(())
}

/// Currently active context name, or `None` for the implicit default
/// (`/var/run/docker.sock` / `DOCKER_HOST`).
///
/// Read straight from `config.json` — no `docker` process spawn, so the 10s
/// UI timer stays pure file I/O on the healthy path.
pub fn current_context_name() -> Option<String> {
    let text = std::fs::read_to_string(docker_config_dir().join("config.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    match json.get("currentContext").and_then(|v| v.as_str()) {
        None | Some("") | Some("default") => None,
        Some(n) => Some(n.to_string()),
    }
}

/// Select our context as the CLI default. Only called when the user asked for
/// it; the active context is otherwise never changed.
pub fn set_context_default() -> anyhow::Result<()> {
    set_context_default_named(CONTEXT_NAME)
}

/// `docker context use <name>`. Centralises the only place the app spawns
/// `context use`, so both the take-over and the restore go through one path.
pub fn set_context_default_named(name: &str) -> anyhow::Result<()> {
    let ok = Command::new("docker")
        .args(["context", "use", name])
        .status()?
        .success();
    anyhow::ensure!(ok, "docker context use {name} failed");
    Ok(())
}
/// Names of the contexts the CLI currently knows about, from `config.json`'s
/// sibling `contexts/meta/*/meta.json` files. No `docker` spawn.
pub fn available_context_names() -> Vec<String> {
    let meta = contexts_meta_dir();
    let Ok(entries) = std::fs::read_dir(&meta) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path().join("meta.json")) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(name) = json.get("Name").and_then(|v| v.as_str()) {
            names.push(name.to_string());
        }
    }
    names
}

/// Pick the context to restore after the bridge releases the default.
///
/// Returns `prev` when it still exists on disk, otherwise `"default"` — so we
/// never leave the CLI pointing at a name we removed or that never existed.
pub fn resolve_restore_target(prev: Option<&str>, available: &[String]) -> String {
    match prev {
        Some(p) if !p.is_empty() && p != CONTEXT_NAME && available.iter().any(|a| a == p) => {
            p.to_string()
        }
        _ => "default".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Aggregate state for the UI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliLink {
    /// Context exists and is the active default → plain `docker` works.
    Connected,
    /// Mapped somewhere (symlink and/or context) but not the active default →
    /// `docker --context docker-manager-engine ...` or `-H` needed.
    Partial,
    /// Nothing maps the CLI to our engine.
    Disconnected,
}

/// Pure decision function over the two booleans — unit-tested.
pub fn classify(symlink_ok: bool, context_ok: bool, is_default: bool) -> CliLink {
    match (symlink_ok || context_ok, context_ok && is_default) {
        (false, _) => CliLink::Disconnected,
        (true, true) => CliLink::Connected,
        (true, false) => CliLink::Partial,
    }
}

pub fn link_state(engine_socket: &Path) -> CliLink {
    let symlink_ok = matches!(xdg_symlink_state(engine_socket), Some(true));
    let context_ok = has_our_context(engine_socket);
    let is_default = current_context_name().as_deref() == Some(CONTEXT_NAME);
    classify(symlink_ok, context_ok, is_default)
}

/// Full snapshot rendered by the Engine page's "Docker CLI" panel: what is
/// mapped, and whether the CLI would actually reach our engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliStatus {
    pub link: CliLink,
    pub symlink_ok: bool,
    pub context_ok: bool,
    pub context_is_default: bool,
    /// The socket the CLI would hit with a bare `docker ...` today, when that
    /// is not our engine (so the UI can show what it points at instead).
    pub active_context: Option<String>,
    pub xdg_path: PathBuf,
    /// Carried over from the accompanying [`reconcile`] pass: the context to
    /// persist as the restore target. `None` from a bare [`status`] read.
    pub prev_context_saved: Option<String>,
}

pub fn status(engine_socket: &Path) -> CliStatus {
    let symlink_ok = matches!(xdg_symlink_state(engine_socket), Some(true));
    let context_ok = has_our_context(engine_socket);
    let active = current_context_name();
    let context_is_default = active.as_deref() == Some(CONTEXT_NAME);
    CliStatus {
        link: classify(symlink_ok, context_ok, context_is_default),
        symlink_ok,
        context_ok,
        context_is_default,
        active_context: active,
        xdg_path: xdg_socket(),
        prev_context_saved: None,
    }
}

/// Outcome of one reconcile pass, for a toast / log line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Applied {
    pub symlink_created: bool,
    pub context_created: bool,
    pub default_set: bool,
    pub default_cleared: bool,
    /// True when a previously-created mapping was removed (engine stopped or
    /// the bridge was switched off).
    pub torn_down: bool,
    /// Set on the pass that takes over the default: the context the CLI was on
    /// beforehand, to be persisted and restored on teardown. `None` when there
    /// was nothing worth remembering (already `default` / already ours).
    pub prev_context_saved: Option<String>,
}

/// Idempotently map the CLI to `engine_socket` according to `enabled` /
/// `set_default`, and the engine's own liveness.
///
/// Safe to call every 10s: each step is a no-op when already correct.
///
/// Ownership rule — the app removes *only* what it created, and only while
/// it is still needed:
/// - engine not live, or `enabled` false → tear the mapping down, since a
///   symlink/context pointing at a dead socket is misleading garbage
///   (`docker` would report a cryptic connect error).
/// - same socket, engine live, enabled → ensure symlink + context exist.
///
/// The boolean is passed in rather than probed here so the caller (which
/// already tracks daemon state) stays the single source of truth.
pub fn reconcile(
    engine_socket: &Path,
    enabled: bool,
    set_default: bool,
    engine_live: bool,
    prev_context: &str,
) -> anyhow::Result<Applied> {
    let mut out = Applied::default();
    if !enabled || !engine_live {
        out.torn_down = teardown(engine_socket, prev_context)?;
        return Ok(out);
    }
    out.symlink_created = ensure_xdg_symlink(engine_socket)?;
    ensure_context(engine_socket)?;
    out.context_created = has_our_context(engine_socket);
    if set_default {
        let active = current_context_name();
        if active.as_deref() != Some(CONTEXT_NAME) {
            // First take-over: remember what the user was on so teardown can
            // put it back. Only capture once (`prev_context` empty).
            if prev_context.is_empty() {
                out.prev_context_saved = active.filter(|n| n != "default");
            }
            set_context_default()?;
            out.default_set = true;
        }
    } else if current_context_name().as_deref() == Some(CONTEXT_NAME) {
        let target = resolve_restore_target(Some(prev_context), &available_context_names());
        set_context_default_named(&target)?;
        out.default_cleared = true;
    }
    Ok(out)
}

/// Remove the symlink and context this app created (explicit "disconnect").
/// Leaves anything foreign untouched.
pub fn disconnect(engine_socket: &Path) -> anyhow::Result<()> {
    // Explicit disconnect has no stored restore target: fall back to `default`.
    let _ = teardown_at(&xdg_socket(), engine_socket, "default");
    Ok(())
}

/// Tear down anything the bridge created for this engine. Returns `true` if
/// either the symlink or the context was present and removed (our mapping).
///
/// The check uses "our mapping" tests: only removes a symlink that points at
/// `engine_socket` (or a stale link under our runtime dir), and only removes
/// the context named `CONTEXT_NAME` whose endpoint is `engine_socket`.
fn teardown(engine_socket: &Path, restore_to: &str) -> anyhow::Result<bool> {
    teardown_at(&xdg_socket(), engine_socket, restore_to)
}

fn teardown_at(link: &Path, engine_socket: &Path, restore_to: &str) -> anyhow::Result<bool> {
    let mut torn = false;
    if symlink_state_at(link, engine_socket) == Some(true) {
        let _ = std::fs::remove_file(link);
        torn = true;
    }
    if current_context_name().as_deref() == Some(CONTEXT_NAME) {
        // Hand the default back to whatever the user had before us, not a
        // blind `default` (which would clobber e.g. `desktop-linux`).
        let target = resolve_restore_target(Some(restore_to), &available_context_names());
        let _ = set_context_default_named(&target);
        torn = true;
    }
    // Remove our context if it exists under our name (whether it currently
    // points at this engine socket or is stale from a previous install).
    if our_context_host().is_some() {
        let _ = Command::new("docker")
            .args(["context", "rm", "-f", CONTEXT_NAME])
            .status();
        torn = true;
    }
    Ok(torn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("clibridge-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Serialises the tests that read `docker_config_dir()`.
    ///
    /// `DOCKER_CONFIG` is process-global, so tests that point it at their own
    /// sandbox must not run concurrently with each other.
    static CONFIG_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Point `DOCKER_CONFIG` at `dir` for the duration of a test and return a
    /// guard holding the env lock.
    ///
    /// `current_context_name()` reads `<DOCKER_CONFIG>/config.json`. Without
    /// this, a test would read the developer's real `~/.docker/config.json`,
    /// so its result depended on whatever context the machine happened to have
    /// selected — `teardown_at_ignores_foreign_symlink` failed whenever the
    /// live context was `docker-manager-engine`.
    fn isolate_docker_config(dir: &Path) -> std::sync::MutexGuard<'static, ()> {
        let guard = CONFIG_ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("DOCKER_CONFIG", dir);
        // A config with no `currentContext` = the CLI default, which is the
        // state these tests mean by "not ours".
        let _ = std::fs::write(dir.join("config.json"), "{}");
        guard
    }

    #[test]
    fn symlink_state_detects_ours_and_foreign() {
        let dir = tmpdir("sl");
        let engine = dir.join("engine.sock");
        std::fs::write(&engine, b"").unwrap();
        let link = dir.join("docker.sock");

        assert_eq!(symlink_state_at(&link, &engine), None, "absent");

        symlink(&engine, &link).unwrap();
        assert_eq!(symlink_state_at(&link, &engine), Some(true), "ours");

        let other = dir.join("other.sock");
        std::fs::write(&other, b"").unwrap();
        std::fs::remove_file(&link).unwrap();
        symlink(&other, &link).unwrap();
        assert_eq!(symlink_state_at(&link, &engine), Some(false), "foreign");
    }

    #[test]
    fn symlink_state_ignores_real_file() {
        let dir = tmpdir("real");
        let engine = dir.join("engine.sock");
        std::fs::write(&engine, b"").unwrap();
        let link = dir.join("docker.sock");
        std::fs::write(&link, b"not a symlink").unwrap();
        // A real socket/file must never be touched or overwritten.
        assert_eq!(symlink_state_at(&link, &engine), None);
        assert!(!ensure_symlink_at(&link, &engine).unwrap());
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "not a symlink");
    }

    #[test]
    fn ensure_symlink_creates_only_when_absent() {
        let dir = tmpdir("ensure");
        let engine = dir.join("engine.sock");
        std::fs::write(&engine, b"").unwrap();
        let link = dir.join("sub/docker.sock");

        assert!(ensure_symlink_at(&link, &engine).unwrap());
        assert_eq!(symlink_state_at(&link, &engine), Some(true));
        // Second call is a no-op.
        assert!(!ensure_symlink_at(&link, &engine).unwrap());
    }

    #[test]
    fn classify_covers_all_states() {
        assert_eq!(classify(false, false, false), CliLink::Disconnected);
        assert_eq!(classify(true, false, false), CliLink::Partial);
        assert_eq!(classify(false, true, false), CliLink::Partial);
        assert_eq!(classify(true, true, true), CliLink::Connected);
        // Context exists but a *foreign* context is the default → Partial.
        assert_eq!(classify(true, true, false), CliLink::Partial);
    }

    #[test]
    fn teardown_at_removes_our_symlink() {
        let dir = tmpdir("teardown-sym");
        let _cfg = isolate_docker_config(&dir);
        let engine = dir.join("engine.sock");
        let _ = std::fs::write(&engine, b"");
        let link = dir.join("docker.sock");
        symlink(&engine, &link).expect("symlink");
        assert!(teardown_at(&link, &engine, "default").unwrap());
        assert!(!link.exists());
    }

    #[test]
    fn teardown_at_ignores_foreign_symlink() {
        let dir = tmpdir("teardown-ign");
        let _cfg = isolate_docker_config(&dir);
        let engine = dir.join("engine.sock");
        let foreign = dir.join("other.sock");
        let _ = std::fs::write(&engine, b"");
        let _ = std::fs::write(&foreign, b"");
        let link = dir.join("docker.sock");
        symlink(&foreign, &link).expect("symlink foreign");
        assert!(!teardown_at(&link, &engine, "default").unwrap());
        assert!(link.exists());
    }

    #[test]
    fn restore_target_prefers_prev_when_still_present() {
        let avail = vec!["default".to_string(), "desktop-linux".to_string()];
        assert_eq!(
            resolve_restore_target(Some("desktop-linux"), &avail),
            "desktop-linux"
        );
    }

    #[test]
    fn restore_target_falls_back_to_default_when_prev_gone() {
        let avail = vec!["default".to_string()];
        // The remembered context was since deleted → do not point at a ghost.
        assert_eq!(
            resolve_restore_target(Some("desktop-linux"), &avail),
            "default"
        );
        assert_eq!(resolve_restore_target(None, &avail), "default");
        assert_eq!(resolve_restore_target(Some(""), &avail), "default");
    }

    #[test]
    fn restore_target_never_restores_our_own_context() {
        // Guard against a stale config that saved our own name as "previous".
        let avail = vec![CONTEXT_NAME.to_string()];
        assert_eq!(
            resolve_restore_target(Some(CONTEXT_NAME), &avail),
            "default"
        );
    }
}
