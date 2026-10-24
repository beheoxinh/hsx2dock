//! One-time migration of the on-disk identity from `docker-manager` to
//! `hsx2dock`.
//!
//! The rebrand is not only cosmetic: the config dir, the data dir (engine
//! versions, images, containers, kind clusters), the runtime dir (engine socket,
//! pid, exec sockets) and the `/tmp` snapshotter compat dir all carried
//! `docker-manager` in their path. Some of those absolute paths are baked into
//! state we do not own — kind node containers bind-mount the compat dir, and an
//! engine process was started against its socket path — so a plain rename would
//! orphan every existing cluster and engine install.
//!
//! Each legacy directory is therefore *renamed* to the new name and then
//! re-created as a symlink pointing at it. New code only ever writes the new
//! paths; old absolute paths keep resolving through the link.
//!
//! This runs once at startup, before `AppState` reads the config or builds any
//! path. It is idempotent: once the legacy entry is a symlink every step is a
//! no-op, so running on every launch is safe.

use std::path::{Path, PathBuf};

/// Directory name used by every Hsx2Dock release.
pub const APP_DIR: &str = "hsx2dock";
/// Directory name used by releases before the rebrand.
pub const LEGACY_APP_DIR: &str = "docker-manager";

/// `/tmp` fallback housing the d_type-capable containerd snapshotter state.
pub const DTYPE_COMPAT_DIR: &str = "hsx2dock-dtype-compat";
pub const LEGACY_DTYPE_COMPAT_DIR: &str = "docker-manager-dtype-compat";

/// What happened to one legacy directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOutcome {
    /// Nothing at the legacy path — fresh install or already cleaned up.
    Missing,
    /// Renamed to the new name and linked back.
    Migrated,
    /// The legacy entry is already a symlink (a previous pass ran).
    AlreadyMigrated,
    /// Both a real legacy dir and a real new dir exist. Left untouched: merging
    /// two live stores could lose data, and the new one is already canonical.
    BothExist,
    /// Rename failed (permissions, cross-device); nothing was changed.
    Failed,
}

/// The directory pairs to migrate, in the order they must be handled.
///
/// The data dir goes first: it contains the on-disk `dtype-compat` subtree, so
/// moving it covers that nested state too. The `/tmp` compat dir is separate
/// because it lives on another filesystem.
pub fn legacy_dir_pairs() -> Vec<(PathBuf, PathBuf)> {
    let mut pairs = Vec::new();
    if let Some(cfg) = dirs::config_dir() {
        pairs.push((cfg.join(LEGACY_APP_DIR), cfg.join(APP_DIR)));
    }
    if let Some(data) = dirs::data_dir() {
        pairs.push((data.join(LEGACY_APP_DIR), data.join(APP_DIR)));
    }
    if let Some(rt) = dirs::runtime_dir() {
        pairs.push((rt.join(LEGACY_APP_DIR), rt.join(APP_DIR)));
    }
    // Hard-coded, not `std::env::temp_dir()`: the previous releases wrote (and
    // kind nodes bind-mount) exactly `/tmp/docker-manager-dtype-compat`.
    pairs.push((
        PathBuf::from("/tmp").join(LEGACY_DTYPE_COMPAT_DIR),
        PathBuf::from("/tmp").join(DTYPE_COMPAT_DIR),
    ));
    pairs
}

/// Rename `old` to `new`, then link `old` back so absolute paths recorded
/// elsewhere keep resolving.
pub fn migrate_dir(old: &Path, new: &Path) -> MoveOutcome {
    let Ok(meta) = std::fs::symlink_metadata(old) else {
        return MoveOutcome::Missing;
    };
    if meta.file_type().is_symlink() {
        return MoveOutcome::AlreadyMigrated;
    }
    if std::fs::symlink_metadata(new).is_ok() {
        return MoveOutcome::BothExist;
    }
    if let Some(parent) = new.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return MoveOutcome::Failed;
        }
    }
    if std::fs::rename(old, new).is_err() {
        return MoveOutcome::Failed;
    }
    // Best effort: the data has already moved, and a missing link only affects
    // paths stored by older releases, so a failure here must not undo the move.
    let _ = std::os::unix::fs::symlink(new, old);
    MoveOutcome::Migrated
}

/// Run every migration step and drop the pre-rebrand docker context.
///
/// Safe to call on every launch; each step becomes a no-op after the first
/// successful pass.
pub fn migrate_legacy_identity() {
    for (old, new) in legacy_dir_pairs() {
        match migrate_dir(&old, &new) {
            MoveOutcome::Migrated => {
                tracing::info!("migrated {} -> {}", old.display(), new.display());
            }
            MoveOutcome::BothExist => {
                tracing::warn!(
                    "{} and {} both exist; leaving the legacy tree untouched",
                    old.display(),
                    new.display()
                );
            }
            MoveOutcome::Failed => {
                tracing::warn!(
                    "could not migrate {} to {}; continuing with the new path",
                    old.display(),
                    new.display()
                );
            }
            MoveOutcome::Missing | MoveOutcome::AlreadyMigrated => {}
        }
    }
    crate::docker::cli_bridge::migrate_legacy_context();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_case(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("hsx2dock-migrate-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp root");
        root
    }

    #[test]
    fn moves_the_tree_and_keeps_the_old_path_resolving() {
        let root = temp_case("move");
        let old = root.join("docker-manager");
        let new = root.join("hsx2dock");
        std::fs::create_dir_all(old.join("versions")).expect("seed");
        std::fs::write(old.join("versions/marker"), "state").expect("seed file");

        assert_eq!(migrate_dir(&old, &new), MoveOutcome::Migrated);

        assert!(new.join("versions/marker").exists());
        assert!(std::fs::symlink_metadata(&old)
            .expect("legacy link")
            .file_type()
            .is_symlink());
        // Old absolute paths recorded by the engine/kind must still resolve.
        assert_eq!(
            std::fs::read_to_string(old.join("versions/marker")).expect("through link"),
            "state"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn is_idempotent() {
        let root = temp_case("idempotent");
        let old = root.join("docker-manager");
        let new = root.join("hsx2dock");
        std::fs::create_dir_all(&old).expect("seed");

        assert_eq!(migrate_dir(&old, &new), MoveOutcome::Migrated);
        assert_eq!(migrate_dir(&old, &new), MoveOutcome::AlreadyMigrated);
        assert!(new.is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn keeps_both_trees_when_the_new_dir_already_exists() {
        let root = temp_case("both");
        let old = root.join("docker-manager");
        let new = root.join("hsx2dock");
        std::fs::create_dir_all(&old).expect("seed old");
        std::fs::create_dir_all(&new).expect("seed new");
        std::fs::write(old.join("legacy"), "keep").expect("seed file");

        assert_eq!(migrate_dir(&old, &new), MoveOutcome::BothExist);

        // Nothing was moved or linked: the legacy tree is still a real dir.
        assert!(old.join("legacy").exists());
        assert!(!std::fs::symlink_metadata(&old)
            .expect("legacy")
            .file_type()
            .is_symlink());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_legacy_dir_is_a_noop() {
        let root = temp_case("missing");
        let old = root.join("docker-manager");
        let new = root.join("hsx2dock");
        assert_eq!(migrate_dir(&old, &new), MoveOutcome::Missing);
        assert!(!new.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
