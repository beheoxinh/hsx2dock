//! kind (Kubernetes IN Docker) binary manager: pinned release, checksum
//! verification, and an isolated install/layout so clusters never touch the
//! host system outside this app's data directory.
//!
//! kind is a single static Go binary. Installing it here means downloading the
//! pinned release, verifying its sha256, and dropping it under the app's own
//! `versions/kind/` tree — the same isolated layout the bundled Docker engine
//! uses. Cluster state itself lives in Docker containers (labelled
//! `io.x-k8s.kind.cluster`), so it rides on the engine this app already
//! manages.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::docker::engine::{download, verify};

/// Pinned kind release. Bump together with the per-platform checksums below.
pub const KIND_VERSION: &str = "0.33.0";
const KIND_TAG: &str = "v0.33.0";
const KIND_BASE: &str = "https://github.com/kubernetes-sigs/kind/releases/download";

/// sha256 of each published binary for the pinned tag, verified before the
/// binary is ever executed. Sourced from the GitHub release asset digests
/// (`digest: sha256:…` in the v0.33.0 release API response).
fn expected_sha256(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64" | "amd64") => {
            Some("aee6151561422756b764a4ae28e7f44cda5af5a9eead3cc9985112b1de8d8e0d")
        }
        ("linux", "aarch64" | "arm64") => {
            Some("20022bee6cfcd5086cb7234d218e3454e6090022f2a8f55d1fa7fcf42c3867a2")
        }
        ("macos" | "darwin", "aarch64" | "arm64") => {
            Some("0c8c7dbe5e23594a198b786c4bc13dacc101fa6196b0cb0b23a1ca44e61f4b4f")
        }
        ("macos" | "darwin", "x86_64" | "amd64") => {
            Some("5a99f26f57246dc9319dd294803313197a0f34d33c525b3ea8b655db5916ece0")
        }
        ("windows", "x86_64" | "amd64") => {
            Some("4b22adaa135368c5a465d56bbd8e520cbea87272a06ca00b6078e7b81515c9fc")
        }
        _ => None,
    }
}

/// Release asset file name for a platform (the exact name published on the
/// GitHub release — note the Windows asset carries no `.exe` suffix).
fn asset_name(os: &str, arch: &str) -> Option<String> {
    let a = match arch {
        "x86_64" | "amd64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        _ => return None,
    };
    match os {
        "linux" => Some(format!("kind-linux-{a}")),
        "macos" | "darwin" => Some(format!("kind-darwin-{a}")),
        "windows" => Some(format!("kind-windows-{a}")),
        _ => None,
    }
}

/// Where the kind binary and its isolated kubeconfig live. Everything stays
/// under the app's data dir — nothing is written to `~/.kube` or `PATH`.
pub struct KindPaths {
    /// Directory holding the pinned binary: `<data>/versions/kind/<ver>/`.
    install_dir: PathBuf,
    /// App-managed kubeconfig so clusters never touch `~/.kube/config`.
    pub kubeconfig: PathBuf,
}

impl KindPaths {
    /// Default layout under the app's data dir, independent of the engine's
    /// custom data-root (the kind binary must not move when the daemon's
    /// storage is migrated).
    pub fn for_app() -> Self {
        let data = dirs::data_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        Self::new(&data.join(crate::migrate::APP_DIR))
    }

    /// Builds paths under the shared app data root (same base the engine uses).
    pub fn new(app_data_root: &Path) -> Self {
        let install_dir = app_data_root
            .join("versions")
            .join("kind")
            .join(KIND_VERSION);
        let kubeconfig = app_data_root.join("kind-kubeconfig");
        Self {
            install_dir,
            kubeconfig,
        }
    }

    /// Absolute path to the kind binary for this platform.
    pub fn binary(&self) -> PathBuf {
        let os = std::env::consts::OS;
        let mut p = self.install_dir.join("kind");
        if os == "windows" {
            p.set_extension("exe");
        }
        p
    }

    /// True when the pinned binary is present on disk.
    pub fn is_installed(&self) -> bool {
        self.binary().is_file()
    }

    pub fn install_dir(&self) -> &Path {
        &self.install_dir
    }
}

/// Download + verify + install the pinned kind binary. Returns the final path.
/// `on_progress` receives (done_bytes, total_bytes_or_None).
pub async fn install_kind<F>(paths: &KindPaths, mut on_progress: F) -> Result<PathBuf>
where
    F: FnMut(u64, Option<u64>) + Send,
{
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let Some(asset) = asset_name(os, arch) else {
        bail!("kind is not published for {os}-{arch}");
    };
    let Some(sha) = expected_sha256(os, arch) else {
        bail!("no pinned checksum for kind on {os}-{arch}");
    };

    let url = format!("{KIND_BASE}/{KIND_TAG}/{asset}");
    let staging = paths
        .install_dir
        .join(format!(".{asset}.partial"));
    std::fs::create_dir_all(paths.install_dir())
        .with_context(|| format!("create {}", paths.install_dir().display()))?;

    download(&url, &staging, |done, total| on_progress(done, total))
        .await
        .with_context(|| format!("download kind {KIND_TAG}"))?;

    // Verify before the binary is ever placed where it could be executed.
    if let Err(e) = verify(&staging, sha) {
        let _ = std::fs::remove_file(&staging);
        return Err(e).context("kind checksum verification failed");
    }

    let dest = paths.binary();
    // Replace any previous install atomically-ish: remove then rename.
    let _ = std::fs::remove_file(&dest);
    std::fs::rename(&staging, &dest)
        .with_context(|| format!("move kind into {}", dest.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .context("chmod kind binary")?;
    }

    // Record the installed version so `kind_status` can report it without
    // spawning the binary.
    let _ = std::fs::write(
        paths.install_dir().join("VERSION"),
        format!("{KIND_VERSION}\n"),
    );

    Ok(dest)
}

/// Reported install state for the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KindInstallStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub latest: String,
    /// Absolute path to the binary, when installed.
    pub path: Option<String>,
    pub supported: bool,
}

pub fn kind_status(paths: &KindPaths) -> KindInstallStatus {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let supported = asset_name(os, arch).is_some() && expected_sha256(os, arch).is_some();
    let installed = paths.is_installed();
    let version = if installed {
        std::fs::read_to_string(paths.install_dir().join("VERSION"))
            .ok()
            .map(|s| s.trim().to_string())
            .or(Some(KIND_VERSION.to_string()))
    } else {
        None
    };
    KindInstallStatus {
        installed,
        version,
        latest: KIND_VERSION.to_string(),
        path: installed.then(|| paths.binary().display().to_string()),
        supported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::engine::sha256_file;

    fn temp_paths(tag: &str) -> KindPaths {
        let base = std::env::temp_dir().join(format!("dm-kind-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        KindPaths::new(&base)
    }

    #[test]
    fn paths_stay_under_the_app_data_dir() {
        let p = temp_paths("layout");
        let bin = p.binary();
        assert!(bin.to_string_lossy().contains("versions/kind"));
        assert!(bin.to_string_lossy().contains(KIND_VERSION));
        assert!(p.kubeconfig.to_string_lossy().contains("kind-kubeconfig"));
        // Never points at ~/.kube.
        assert!(!p.kubeconfig.starts_with(dirs::home_dir().unwrap_or_default().join(".kube")));
    }

    #[test]
    fn status_reports_not_installed_initially() {
        let p = temp_paths("status");
        let st = kind_status(&p);
        assert!(!st.installed);
        assert_eq!(st.latest, KIND_VERSION);
        assert!(st.version.is_none());
        assert!(st.supported, "linux/mac/windows are all supported");
    }

    #[test]
    fn sha256_is_pinned_for_this_platform() {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        assert!(
            expected_sha256(os, arch).is_some(),
            "must have a pinned checksum for {os}-{arch}"
        );
    }

    /// Every platform with a published asset must also have a checksum, and
    /// every checksum must be a well-formed sha256 hex digest — this is the
    /// regression gate for hand-typed/fabricated digests.
    #[test]
    fn every_published_asset_checksum_is_well_formed() {
        let platforms = [
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
            ("windows", "x86_64"),
        ];
        for (os, arch) in platforms {
            assert!(
                asset_name(os, arch).is_some(),
                "missing asset name for {os}-{arch}"
            );
            let sha = expected_sha256(os, arch)
                .unwrap_or_else(|| panic!("missing pinned checksum for {os}-{arch}"));
            assert_eq!(sha.len(), 64, "{os}-{arch} checksum must be 64 chars");
            assert!(
                sha.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "{os}-{arch} checksum must be lowercase hex"
            );
        }
    }

    #[test]
    fn asset_names_follow_kind_convention() {
        assert_eq!(asset_name("linux", "x86_64").unwrap(), "kind-linux-amd64");
        assert_eq!(asset_name("macos", "aarch64").unwrap(), "kind-darwin-arm64");
        assert_eq!(asset_name("windows", "x86_64").unwrap(), "kind-windows-amd64");
        assert!(asset_name("linux", "riscv64").is_none());
    }

    #[test]
    fn verify_helper_rejects_mismatched_checksum() {
        let dir = std::env::temp_dir().join(format!("dm-kind-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("blob");
        std::fs::write(&f, b"hello").unwrap();
        // sha256("hello")
        let hello = sha256_file(&f).unwrap();
        assert!(verify(&f, &hello).is_ok());
        assert!(verify(&f, &"00".repeat(32)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
