//! Filesystem helpers that must behave correctly under restrictive
//! permissions. Volumes frequently contain root-owned subtrees the app cannot
//! read; those must degrade to "partial size" instead of erroring out or
//! silently reporting zero.

use std::path::{Path, PathBuf};

/// Result of walking a directory tree and summing regular-file sizes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirSize {
    /// Sum of `len()` over every regular file reached.
    pub bytes: u64,
    /// Number of regular files reached.
    pub files: u64,
    /// Directories skipped because listing them was denied. When non-zero,
    /// `bytes` is a lower bound, not the true total.
    pub restricted_dirs: u32,
}

impl DirSize {
    /// True when at least one subtree could not be read, so the reported size
    /// is incomplete.
    pub fn is_partial(&self) -> bool {
        self.restricted_dirs > 0
    }
}

/// Sums the size of every regular file under `root`, without following
/// symlinks and without aborting on permission errors.
///
/// * Symlinks are counted as zero bytes and never traversed — this both
///   prevents infinite loops through bind-mounted cycles and avoids charging
///   the volume for host files it merely points at.
/// * A directory that cannot be listed (EACCES/EPERM) increments
///   [`DirSize::restricted_dirs`] and the walk continues elsewhere.
/// * The top-level `root` itself being unreadable returns `Err` with the
///   original I/O error so the caller can tell "no access at all" apart from
///   "partial access".
pub fn dir_size(root: &Path) -> std::io::Result<DirSize> {
    let mut out = DirSize::default();

    // Probe the root first: an unreadable mountpoint is a hard failure, not a
    // partial result.
    let root_meta = std::fs::symlink_metadata(root)?;
    if !root_meta.is_dir() {
        // Not a directory (e.g. a plain file bind-mount source): size it directly.
        if root_meta.is_file() {
            out.bytes = root_meta.len();
            out.files = 1;
        }
        return Ok(out);
    }

    // Iterative DFS so a deep tree can't blow the call stack.
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                out.restricted_dirs += 1;
                continue;
            }
            Err(_) => {
                // Vanished or I/O error mid-walk: treat like a restricted dir
                // so one flaky entry doesn't poison the whole count.
                out.restricted_dirs += 1;
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => {
                    out.restricted_dirs += 1;
                    continue;
                }
            };
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            let path = entry.path();

            if file_type.is_symlink() {
                // Never follow: count as nothing, skip.
                continue;
            }
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if file_type.is_file() {
                // Prefer the size from the dirent metadata we already have;
                // fall back to a stat when it's missing.
                let len = match entry.metadata() {
                    Ok(m) => m.len(),
                    Err(_) => match std::fs::symlink_metadata(&path) {
                        Ok(m) => m.len(),
                        Err(_) => continue,
                    },
                };
                out.bytes += len;
                out.files += 1;
            }
            // Sockets, FIFOs, devices: no meaningful size, ignore.
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("dm-fsutil-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn sums_nested_regular_files() {
        let root = temp_dir("sum");
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("top.bin"), vec![7u8; 100]).unwrap();
        fs::write(root.join("a/mid.bin"), vec![7u8; 250]).unwrap();
        fs::write(root.join("a/b/deep.bin"), vec![7u8; 400]).unwrap();

        let got = dir_size(&root).unwrap();
        assert_eq!(got.bytes, 100 + 250 + 400);
        assert_eq!(got.files, 3);
        assert_eq!(got.restricted_dirs, 0);
        assert!(!got.is_partial());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_directory_is_zero_not_error() {
        let root = temp_dir("empty");
        let got = dir_size(&root).unwrap();
        assert_eq!(got.bytes, 0);
        assert_eq!(got.files, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn symlink_is_not_followed() {
        let root = temp_dir("symlink");
        fs::write(root.join("real.bin"), vec![9u8; 64]).unwrap();
        // Point a symlink at a large file outside the tree.
        let outside = temp_dir("symlink-outside");
        fs::write(outside.join("big.bin"), vec![9u8; 4096]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("big.bin"), root.join("link.bin")).unwrap();

        let got = dir_size(&root).unwrap();
        assert_eq!(got.bytes, 64, "symlink target must not be counted");
        assert_eq!(got.files, 1);

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn single_file_root_is_sized() {
        let root = temp_dir("file");
        let file = root.join("only.bin");
        fs::write(&file, vec![1u8; 77]).unwrap();
        let got = dir_size(&file).unwrap();
        assert_eq!(got.bytes, 77);
        assert_eq!(got.files, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_root_errors() {
        let err = dir_size(Path::new("/definitely/not/here/dm-test"));
        assert!(err.is_err());
    }

    /// Permission handling only asserts when we are NOT root — root bypasses
    /// directory permissions entirely, so the chmod would be a no-op.
    #[cfg(unix)]
    #[test]
    fn restricted_subdir_is_skipped_and_reported() {
        use std::os::unix::fs::PermissionsExt;
        if unsafe { libc::geteuid() } == 0 {
            // Running as root: cannot exercise EACCES. Skip.
            return;
        }
        let root = temp_dir("perm");
        fs::write(root.join("open.bin"), vec![3u8; 10]).unwrap();
        let locked = root.join("locked");
        fs::create_dir_all(&locked).unwrap();
        fs::write(locked.join("secret.bin"), vec![3u8; 9999]).unwrap();
        let prev = fs::metadata(&locked).unwrap().permissions();
        let mut deny = prev.clone();
        deny.set_mode(0o000);
        fs::set_permissions(&locked, deny).unwrap();

        let got = dir_size(&root).unwrap();
        // Only the open file is counted; the locked subtree is reported.
        assert_eq!(got.bytes, 10);
        assert_eq!(got.files, 1);
        assert_eq!(got.restricted_dirs, 1);
        assert!(got.is_partial());

        // Restore so cleanup can remove it.
        fs::set_permissions(&locked, prev).unwrap();
        let _ = fs::remove_dir_all(&root);
    }
}
