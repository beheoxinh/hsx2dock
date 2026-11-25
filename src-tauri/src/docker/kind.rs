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

use crate::docker::engine::{
    download, normalize_version, step_plan, update_available, verify, InstallProgress,
};

/// Pinned kind release. Bump together with the per-platform checksums below.
pub const KIND_VERSION: &str = "0.33.0";
const KIND_BASE: &str = "https://github.com/kubernetes-sigs/kind/releases/download";

/// Steps every kind install/update follows; the UI renders this checklist.
pub const KIND_UPDATE_STEPS: &[&str] = &[
    "Download kind",
    "Verify checksum",
    "Replace binary",
    "Verify version",
];

/// Docker label kind stamps on every node container.
pub const CLUSTER_LABEL: &str = "io.x-k8s.kind.cluster";

/// The Docker network kind creates for its node containers.
pub const NETWORK_NAME: &str = "kind";

/// True when a tag names kind's node image (`kindest/node:…`), optionally
/// prefixed with an explicit registry (`docker.io/kindest/node:…`).
pub fn is_kind_image_tag(tag: &str) -> bool {
    let tag = tag.trim().trim_start_matches("docker.io/");
    tag.starts_with("kindest/") || tag.contains("/kindest/")
}

/// True when any tag belongs to kind. `configured` is the user's
/// `kind_default_image`: if they run clusters from a custom node image, that
/// image is just as load-bearing as the official one.
pub fn is_kind_image(tags: &[String], configured: Option<&str>) -> bool {
    if tags.iter().any(|tag| is_kind_image_tag(tag)) {
        return true;
    }
    let Some(configured) = configured.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let configured = configured.trim_start_matches("docker.io/");
    tags.iter()
        .any(|tag| tag.trim().trim_start_matches("docker.io/") == configured)
}

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
    /// Root of every kind release: `<data>/versions/kind/`.
    root: PathBuf,
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
        Self {
            root: app_data_root.join("versions").join("kind"),
            kubeconfig: app_data_root.join("kind-kubeconfig"),
        }
    }

    /// Version selected by the `current` pointer. Trees installed before kind
    /// updates existed have no pointer and fall back to the pinned release.
    pub fn active_version(&self) -> String {
        std::fs::read_to_string(self.root.join("current"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| KIND_VERSION.to_string())
    }

    /// Directory holding one release: `<data>/versions/kind/<version>/`.
    pub fn version_dir(&self, version: &str) -> PathBuf {
        self.root.join(version)
    }

    /// Directory of the release currently in use.
    pub fn install_dir(&self) -> PathBuf {
        self.version_dir(&self.active_version())
    }

    /// Absolute path to the active kind binary for this platform.
    pub fn binary(&self) -> PathBuf {
        binary_in(&self.install_dir())
    }

    /// Absolute path to a specific release's binary.
    pub fn binary_at(&self, version: &str) -> PathBuf {
        binary_in(&self.version_dir(version))
    }

    /// True when the active binary is present on disk.
    pub fn is_installed(&self) -> bool {
        self.binary().is_file()
    }

    /// Point the `current` marker at `version`. A separate file (rather than a
    /// directory symlink) keeps the layout readable and works on every
    /// filesystem the app data dir may live on.
    pub fn set_active(&self, version: &str) -> Result<()> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("create {}", self.root.display()))?;
        let tmp = self.root.join(".current.tmp");
        std::fs::write(&tmp, format!("{version}\n"))?;
        std::fs::rename(&tmp, self.root.join("current"))?;
        Ok(())
    }
}

fn binary_in(dir: &Path) -> PathBuf {
    let mut p = dir.join("kind");
    if std::env::consts::OS == "windows" {
        p.set_extension("exe");
    }
    p
}

/// GitHub's own sha256 of a release asset (`"digest": "sha256:<hex>"` in the
/// releases API), or `None` for releases published before GitHub recorded it.
async fn github_asset_digest(version: &str, asset: &str) -> Result<Option<String>> {
    let url = format!(
        "https://api.github.com/repos/kubernetes-sigs/kind/releases/tags/v{version}"
    );
    let body = crate::docker::engine::http_get(&url).await?.text().await?;
    let json: serde_json::Value = serde_json::from_str(&body).context("decode GitHub release JSON")?;
    Ok(asset_digest_from_release(&json, asset))
}

fn asset_digest_from_release(release: &serde_json::Value, asset: &str) -> Option<String> {
    release["assets"]
        .as_array()?
        .iter()
        .find(|a| a["name"].as_str() == Some(asset))?["digest"]
        .as_str()?
        .strip_prefix("sha256:")
        .map(str::to_string)
}

/// Download + verify + install the pinned kind binary.
pub async fn install_kind<F>(paths: &KindPaths, on_progress: F) -> Result<PathBuf>
where
    F: FnMut(InstallProgress) + Send,
{
    install_kind_version(paths, KIND_VERSION, on_progress).await
}

/// Download + verify + install a specific kind release, then point `current`
/// at it. The previous release stays on disk, so a failed update never leaves
/// the app without a usable kind.
pub async fn install_kind_version<F>(
    paths: &KindPaths,
    version: &str,
    mut on_progress: F,
) -> Result<PathBuf>
where
    F: FnMut(InstallProgress) + Send,
{
    let version = normalize_version(version).to_string();
    if version.is_empty() {
        bail!("no kind version given");
    }
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let Some(asset) = asset_name(os, arch) else {
        bail!("kind is not published for {os}-{arch}");
    };

    let plan = step_plan(KIND_UPDATE_STEPS);
    let version_dir = paths.version_dir(&version);
    std::fs::create_dir_all(&version_dir)
        .with_context(|| format!("create {}", version_dir.display()))?;
    let staging = version_dir.join(format!(".{asset}.partial"));
    let _ = std::fs::remove_file(&staging);
    let url = format!("{KIND_BASE}/v{version}/{asset}");

    on_progress(InstallProgress::stage(
        "kind",
        &version,
        &plan,
        0,
        Some(0.05),
    ));
    let mut last = 0u64;
    download(&url, &staging, |done, total| {
        if done.saturating_sub(last) < 256 * 1024 && Some(done) != total {
            return;
        }
        last = done;
        on_progress(InstallProgress::tick(
            "kind",
            &version,
            &plan,
            0,
            Some(0.05 + 0.5 * crate::docker::engine::fraction_of(done, total)),
            match total {
                Some(t) if t > 0 => format!(
                    "{} / {}",
                    crate::docker::engine::human_size(done),
                    crate::docker::engine::human_size(t)
                ),
                _ => crate::docker::engine::human_size(done),
            },
        ));
    })
    .await
    .with_context(|| format!("download kind v{version}"))?;

    on_progress(InstallProgress::stage(
        "kind",
        &version,
        &plan,
        1,
        Some(0.6),
    ));
    // The pinned release carries a maintainer-checked digest; every other tag
    // publishes `<asset>.sha256sum` next to the binary.
    let expected = match expected_sha256(os, arch) {
        Some(sha) if version == KIND_VERSION => sha.to_string(),
        _ => {
            let published = fetch_kind_checksum(&version, &asset).await?;
            // The `.sha256sum` file sits next to the binary, so on its own it
            // only proves the two came from the same place. GitHub records its
            // own digest of every uploaded asset; a disagreement means one of
            // them was swapped and the install stops. An API that cannot be
            // reached (rate limit, offline mirror) is not fatal.
            match github_asset_digest(&version, &asset).await {
                Ok(Some(api)) if !api.eq_ignore_ascii_case(&published) => bail!(
                    "kind v{version}: the published checksum ({published}) does not match \
                     GitHub's digest of the asset ({api}); not installing"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!("could not cross-check kind v{version} with GitHub: {e:#}"),
            }
            published
        }
    };
    if let Err(e) = verify(&staging, &expected) {
        let _ = std::fs::remove_file(&staging);
        return Err(e).context("kind checksum verification failed");
    }
    on_progress(InstallProgress::note(
        "kind",
        &version,
        &plan,
        1,
        Some(0.65),
        format!("sha256 {expected}"),
    ));

    on_progress(InstallProgress::stage(
        "kind",
        &version,
        &plan,
        2,
        Some(0.75),
    ));
    let dest = paths.binary_at(&version);
    let _ = std::fs::remove_file(&dest);
    std::fs::rename(&staging, &dest)
        .with_context(|| format!("move kind into {}", dest.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .context("chmod kind binary")?;
    }

    on_progress(InstallProgress::stage(
        "kind",
        &version,
        &plan,
        3,
        Some(0.9),
    ));
    // A binary that cannot report its own version never becomes current.
    let reported = probe_kind_version(&dest)
        .ok_or_else(|| anyhow::anyhow!("staged kind {version} failed `kind version`"))?;
    if reported != version {
        bail!("staged kind reported {reported}, but {version} was expected");
    }

    let _ = std::fs::write(version_dir.join("VERSION"), format!("{version}\n"));
    paths.set_active(&version)?;

    on_progress(InstallProgress::finished("kind", &version, &plan));
    Ok(dest)
}

/// Read `<asset>.sha256sum` from the release and return the digest token.
async fn fetch_kind_checksum(version: &str, asset: &str) -> Result<String> {
    let url = format!("{KIND_BASE}/v{version}/{asset}.sha256sum");
    let text = crate::docker::engine::http_get(&url)
        .await?
        .text()
        .await
        .context("read kind checksum file")?;
    parse_sha256sum(&text).ok_or_else(|| anyhow::anyhow!("no usable checksum in {url}"))
}

/// A `sha256sum` file is `<64 hex chars>  <file name>`; keep only the digest.
fn parse_sha256sum(text: &str) -> Option<String> {
    text.split_whitespace()
        .next()
        .filter(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|hash| hash.to_string())
}

/// `kind version` prints `kind v0.33.0 go1.24.6 linux/amd64` — note it has no
/// `version` keyword, so the docker parser cannot be reused.
fn probe_kind_version(bin: &Path) -> Option<String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("version");
    let out = crate::procutil::output_with_timeout(cmd, crate::procutil::PROBE, "kind version").ok()?;
    if !out.status.success() {
        return None;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let token = text.split_whitespace().nth(1)?;
    let version = normalize_version(token);
    (!version.is_empty()).then(|| version.to_string())
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

/// What the GitHub release channel reports versus the installed binary.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KindUpdateInfo {
    pub installed_version: Option<String>,
    pub latest_version: String,
    pub update_available: bool,
}

pub async fn check_kind_update(paths: &KindPaths) -> Result<KindUpdateInfo> {
    let installed_version = paths.is_installed().then(|| paths.active_version());
    let latest_version =
        crate::docker::engine::fetch_github_latest_cached("kubernetes-sigs/kind").await?;
    Ok(KindUpdateInfo {
        update_available: update_available(&latest_version, installed_version.as_deref()),
        installed_version,
        latest_version,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn github_asset_digest_is_read_for_the_named_asset_only() {
        let release = serde_json::json!({ "assets": [
            { "name": "kind-linux-arm64", "digest": "sha256:aaa" },
            { "name": "kind-linux-amd64", "digest": "sha256:bbb" },
            { "name": "kind-darwin-amd64" }
        ]});
        assert_eq!(
            super::asset_digest_from_release(&release, "kind-linux-amd64").as_deref(),
            Some("bbb")
        );
        assert_eq!(super::asset_digest_from_release(&release, "kind-darwin-amd64"), None);
        assert_eq!(super::asset_digest_from_release(&release, "missing"), None);
    }

    use super::*;
    use crate::docker::engine::sha256_file;

    #[test]
    fn kind_image_detection_covers_registry_prefixes_and_custom_defaults() {
        assert!(is_kind_image_tag("kindest/node:v1.33.0"));
        assert!(is_kind_image_tag("docker.io/kindest/node:v1.33.0"));
        assert!(!is_kind_image_tag("nginx:latest"));
        assert!(!is_kind_image_tag("kindest-fake/node:v1"));

        // Official node image seen through any tag.
        assert!(is_kind_image(&["kindest/node:v1.33.0".into()], None));
        // A custom node image configured in settings is equally load-bearing.
        assert!(is_kind_image(
            &["myreg/node:v1".into()],
            Some("myreg/node:v1")
        ));
        // Same repository, different tag: not the configured image.
        assert!(!is_kind_image(
            &["myreg/node:v2".into()],
            Some("myreg/node:v1")
        ));
        // Blank configured value must not match anything.
        assert!(!is_kind_image(&["nginx:latest".into()], Some("  ")));
        assert!(!is_kind_image(&[], None));
    }

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
        assert!(!p
            .kubeconfig
            .starts_with(dirs::home_dir().unwrap_or_default().join(".kube")));
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
    fn sha256sum_parser_takes_the_digest_token() {
        let hash = "a".repeat(64);
        let text = format!("{hash}  kind-linux-amd64\n");
        assert_eq!(parse_sha256sum(&text).as_deref(), Some(hash.as_str()));
        assert!(parse_sha256sum("not-a-hash  kind-linux-amd64").is_none());
        assert!(parse_sha256sum("").is_none());
    }

    #[test]
    fn active_version_pointer_wins_over_the_pinned_default() {
        let p = temp_paths("active");
        assert_eq!(p.active_version(), KIND_VERSION);
        std::fs::create_dir_all(p.version_dir("0.99.0")).unwrap();
        p.set_active("0.99.0").unwrap();
        assert_eq!(p.active_version(), "0.99.0");
        assert_eq!(p.install_dir(), p.version_dir("0.99.0"));
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
                sha.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "{os}-{arch} checksum must be lowercase hex"
            );
        }
    }

    #[test]
    fn asset_names_follow_kind_convention() {
        assert_eq!(asset_name("linux", "x86_64").unwrap(), "kind-linux-amd64");
        assert_eq!(asset_name("macos", "aarch64").unwrap(), "kind-darwin-arm64");
        assert_eq!(
            asset_name("windows", "x86_64").unwrap(),
            "kind-windows-amd64"
        );
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
