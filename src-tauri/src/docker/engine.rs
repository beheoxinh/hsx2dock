//! Bundled rootless Docker Engine: paths, pinned release manifest, download +
//! verify + extract, and the child-process lifecycle.
//!
//! The app owns its engine. Nothing here reads or writes the host's Docker
//! install: the engine lives entirely under the app's own data dir and talks
//! over a private unix socket. `dockerd` runs rootless inside `rootlesskit`
//! and is a *child of this app* — it dies with the app.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

/// Base URL for Docker's official static binaries (Apache-2.0).
const STATIC_BASE: &str = "https://download.docker.com/linux/static/stable/x86_64";
/// Base URL for the compose v2 release binaries (Apache-2.0).
const COMPOSE_BASE: &str = "https://github.com/docker/compose/releases/download";

/// Where users can read about the engine upstream.
pub const DOCKER_INSTALL_DOCS_URL: &str = "https://docs.docker.com/engine/install/";

/// How long `start()` waits for the daemon's API socket to answer before
/// declaring the start a failure. dockerd's first boot on a cold data-root can
/// take a few seconds (unpacking layers, initializing storage driver).
const HEALTH_TIMEOUT_SECS: u64 = 30;

/// Pinned default release.
///
/// `download.docker.com` publishes **no** checksum manifest for the static
/// tarballs, so their digests are pinned here by the maintainer at release
/// time. Compose *does* publish `checksums.txt`, so its digest is upstream's.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: &'static str,
    pub docker_sha256: &'static str,
    pub rootless_sha256: &'static str,
    pub compose_version: &'static str,
    pub compose_sha256: &'static str,
}

pub const DEFAULT_RELEASE: Release = Release {
    version: "29.9.0",
    docker_sha256: "33e1ab8b63d14bca449f7a3d30d7f6aa669daa544e60a86b17fec87f61afe2b4",
    rootless_sha256: "6b4a2e085a1e4ade5ba3de99fd770a0a9b53c719e6bf6b1632242bf4b068adb6",
    compose_version: "5.6.0",
    compose_sha256: "40343e21ca777173e69cff5dbafeb37c6f81f3b0d57d9e597f036e95eb63e76a",
};

/// Every path the bundled engine uses. Construction is pure (no filesystem
/// access) so it can be built on the GTK main loop without blocking.
#[derive(Clone, Debug)]
pub struct EnginePaths {
    /// `dockerd --data-root` — containers, images, volumes live here.
    pub data_root: PathBuf,
    pub versions_dir: PathBuf,
    /// Symlink to the active `versions/<ver>`.
    pub current_dir: PathBuf,
    pub log_file: PathBuf,
    pub pid_file: PathBuf,
    pub socket: PathBuf,
    /// `XDG_RUNTIME_DIR/docker-manager` — passed to the child as its runtime dir.
    pub runtime_dir: PathBuf,
    pub exec_root: PathBuf,
    pub rootless_state: PathBuf,
}

impl Default for EnginePaths {
    fn default() -> Self {
        Self::new()
    }
}

impl EnginePaths {
    pub fn new() -> Self {
        Self::with_custom_data_root(None)
    }

    pub fn with_custom_data_root(custom_data_root: Option<&str>) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let data = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
        let runtime = dirs::runtime_dir().unwrap_or_else(|| home.join(".local/state"));
        let base = data.join("docker-manager");
        let rt = runtime.join("docker-manager");
        let data_root = match custom_data_root {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
            _ => base.join("data"),
        };
        Self {
            data_root,
            versions_dir: base.join("versions"),
            current_dir: base.join("current"),
            log_file: base.join("log/engine.log"),
            pid_file: base.join("engine.pid"),
            socket: rt.join("docker.sock"),
            runtime_dir: rt.clone(),
            exec_root: rt.join("exec"),
            rootless_state: rt.join("rootless"),
        }
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.current_dir.join("bin")
    }

    pub fn compose_plugin(&self) -> PathBuf {
        self.current_dir.join("cli-plugins/docker-compose")
    }

    fn dockerd_pid_file(&self) -> PathBuf {
        self.current_dir.join("dockerd.pid")
    }
}

/// Shared, cloneable engine handle. The child lives behind a mutex so both the
/// GTK main loop (state queries) and `spawn_blocking` workers can touch it.
#[derive(Clone)]
pub struct EngineManager {
    inner: Arc<Mutex<Inner>>,
    paths: Arc<std::sync::RwLock<EnginePaths>>,
}

struct Inner {
    child: Option<Child>,
}

impl EngineManager {
    pub fn new(paths: EnginePaths) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner { child: None })),
            paths: Arc::new(std::sync::RwLock::new(paths)),
        }
    }

    pub fn paths(&self) -> EnginePaths {
        self.paths.read().unwrap().clone()
    }

    pub fn set_data_root(&self, custom_data_root: Option<&str>) {
        let mut p = self.paths.write().unwrap();
        *p = EnginePaths::with_custom_data_root(custom_data_root);
    }

    pub fn socket(&self) -> PathBuf {
        self.paths.read().unwrap().socket.clone()
    }

    /// True once a release has been fully downloaded, verified and extracted.
    pub fn is_installed(&self) -> bool {
        let p = self.paths();
        p.current_dir.join(".complete").is_file()
    }

    pub fn installed_version(&self) -> Option<String> {
        let p = self.paths();
        fs::read_to_string(p.current_dir.join("VERSION"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Socket the engine listens on, if the socket exists and accepts a
    /// connection. Mirrors the "usable socket" rule used elsewhere.
    pub fn socket_live(&self) -> bool {
        let p = self.paths();
        is_usable_socket(&p.socket)
    }

    /// True only when the engine process is genuinely alive AND the API socket
    /// answers. A spawned-but-immediately-crashed child (e.g. dockerd exiting
    /// during buildkit init) must not report as running: we reap it here with
    /// `try_wait` so the stale handle is cleared and the UI flips to Stopped.
    pub fn is_running(&self) -> bool {
        // `try_lock`: the engine thread may be inside `stop()` (which holds the
        // lock for up to 15s while waiting for SIGTERM). Blocking the caller —
        // including the 2s background monitor — on that would stall the runtime;
        // when contended we simply report "not running", which is what a stop
        // in progress means anyway.
        let mut guard = match self.inner.try_lock() {
            Ok(g) => g,
            Err(_) => return false,
        };

        let Some(child) = guard.child.as_mut() else {
            return false;
        };

        // Reap: `Ok(Some(status))` means the child already exited.
        match child.try_wait() {
            Ok(Some(status)) => {
                tracing::warn!("engine process exited unexpectedly: {status}");
                guard.child = None;
                false
            }
            Ok(None) => {
                // Process still alive; confirm the API socket is actually up.
                let socket = self.paths.read().map(|p| p.socket.clone()).ok();
                match socket {
                    Some(s) => is_usable_socket(&s),
                    None => false,
                }
            }
            Err(e) => {
                tracing::warn!("engine try_wait failed: {e}");
                false
            }
        }
    }

    /// Spawn `rootlesskit` running `dockerd` rootless, as our child.
    ///
    /// We deliberately do **not** use upstream's `dockerd-rootless.sh`: it
    /// re-execs itself through rootlesskit, so the PID we would hold is not the
    /// engine and signal forwarding is murky. Spawning rootlesskit directly
    /// keeps the process tree shallow and predictable.
    pub fn start(&self) -> Result<()> {
        if self.is_running() {
            return Ok(());
        }
        if !self.is_installed() {
            bail!("engine is not installed yet");
        }
        // Reap anything left over from a hard-killed previous run.
        self.kill_stale()?;

        let paths = self.paths();

        // Refuse a data root on a filesystem without d_type *before* spawning:
        // dockerd would boot, then fail every pull with an opaque lease error.
        check_data_root_filesystem(&paths.data_root)?;

        for d in [
            &paths.data_root,
            &paths.runtime_dir,
            &paths.exec_root,
            &paths.rootless_state,
        ] {
            fs::create_dir_all(d).with_context(|| format!("create {}", d.display()))?;
        }
        if let Some(parent) = paths.log_file.parent() {
            fs::create_dir_all(parent)?;
        }

        let bin = paths.bin_dir();
        let net = detect_net_driver(&bin);
        let log = File::create(&paths.log_file)
            .with_context(|| format!("open {}", paths.log_file.display()))?;
        let log_err = log.try_clone()?;

        let mut cmd = Command::new(bin.join("rootlesskit"));
        cmd.arg("--state-dir")
            .arg(&paths.rootless_state)
            .arg(format!("--net={net}"))
            .arg("--mtu=65520");
        for flag in net_flags(&net) {
            cmd.arg(flag);
        }
        cmd.arg("--copy-up=/etc")
            .arg("--copy-up=/run")
            .arg("--")
            .arg(bin.join("dockerd"))
            .arg(format!("--data-root={}", paths.data_root.display()))
            .arg(format!("--exec-root={}", paths.exec_root.display()))
            .arg(format!("--host=unix://{}", paths.socket.display()))
            .arg("--pidfile")
            .arg(paths.dockerd_pid_file())
            // Required for rootless: the userland proxy can't work without
            // its helper binary (and pasta needs no proxy — port forwarding
            // is handled by the implicit port driver), and bridge netfilter
            // sysctls don't exist inside the user namespace.
            .arg("--userland-proxy=false")
            .arg("--feature")
            .arg("containerd-snapshotter=false")
            .env("DOCKER_IGNORE_BR_NETFILTER_ERROR", "1")
            .env("XDG_RUNTIME_DIR", &paths.runtime_dir)
            .env("PATH", prefixed_path(&bin))
            .stdin(Stdio::null())
            // A `piped` stdout that nobody drains would fill the 64 KB pipe
            // buffer and block dockerd forever — always log to a file.
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err));

        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                // Ask the kernel to SIGTERM us if the app dies. Belt-and-braces:
                // the pidfile guard below covers SIGKILL, which skips this.
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }

        let child = cmd.spawn().context("failed to spawn rootlesskit")?;
        let _ = fs::write(&paths.pid_file, child.id().to_string());
        let pid = child.id();
        self.inner.lock().unwrap().child = Some(child);

        // Verify the daemon actually reaches a live API socket. dockerd can
        // spawn fine and then die during init (e.g. buildkit/containerd
        // failure) — without this probe the UI would show "Running" for a
        // process that is already gone.
        let deadline = Instant::now() + Duration::from_secs(HEALTH_TIMEOUT_SECS);
        loop {
            if is_usable_socket(&paths.socket) {
                tracing::info!("engine socket is live after start (pid {pid})");
                return Ok(());
            }
            let exited = {
                let mut guard = self.inner.lock().unwrap();
                match guard.child.as_mut() {
                    Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                    None => true,
                }
            };
            if exited {
                let mut guard = self.inner.lock().unwrap();
                guard.child = None;
                let _ = fs::remove_file(&paths.pid_file);
                bail!(
                    "engine exited during startup; see {} for details",
                    paths.log_file.display()
                );
            }
            if Instant::now() >= deadline {
                // Still alive but never opened the socket — kill and report.
                if let Some(c) = self.inner.lock().unwrap().child.take() {
                    signal(c.id(), libc::SIGKILL);
                }
                let _ = fs::remove_file(&paths.pid_file);
                bail!(
                    "engine did not open its API socket within {HEALTH_TIMEOUT_SECS}s; see {}",
                    paths.log_file.display()
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// SIGTERM → wait up to 15s → SIGKILL → reap. Never leaves a zombie.
    pub fn stop(&self) -> Result<()> {
        let child = self.inner.lock().unwrap().child.take();
        if let Some(mut child) = child {
            signal(child.id(), libc::SIGTERM);
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(200))
                    }
                    Ok(None) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
        let _ = fs::remove_file(&self.paths().pid_file);
        Ok(())
    }

    /// Kill an engine left behind by a previous (hard-killed) app run, but only
    /// if the pidfile's process is really ours — never signal a recycled PID.
    fn kill_stale(&self) -> Result<()> {
        let paths = self.paths();
        let Ok(raw) = fs::read_to_string(&paths.pid_file) else {
            return Ok(());
        };
        let Ok(pid) = raw.trim().parse::<i32>() else {
            let _ = fs::remove_file(&paths.pid_file);
            return Ok(());
        };
        if matches!(
            proc_comm(pid).as_deref(),
            Some("rootlesskit") | Some("dockerd")
        ) {
            signal(pid as u32, libc::SIGTERM);
            let deadline = Instant::now() + Duration::from_secs(5);
            while pid_alive(pid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        let _ = fs::remove_file(&paths.pid_file);
        Ok(())
    }
}

impl Drop for EngineManager {
    fn drop(&mut self) {
        // Only the last clone to go away should stop the engine.
        if Arc::strong_count(&self.inner) != 1 {
            return;
        }
        let child = self.inner.lock().unwrap().child.take();
        if let Some(mut child) = child {
            signal(child.id(), libc::SIGTERM);
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(100))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Download / verify / extract
// ---------------------------------------------------------------------------

fn docker_url(version: &str) -> String {
    format!("{STATIC_BASE}/docker-{version}.tgz")
}

fn rootless_url(version: &str) -> String {
    format!("{STATIC_BASE}/docker-rootless-extras-{version}.tgz")
}

fn compose_url(version: &str) -> String {
    // Compose tags are `vX.Y.Z`; the asset name carries no version.
    format!("{COMPOSE_BASE}/v{version}/docker-compose-linux-x86_64")
}

/// Stream `url` to `dest`, reporting (downloaded, total) as it goes.
pub async fn download<F>(url: &str, dest: &Path, mut on_progress: F) -> Result<()>
where
    F: FnMut(u64, Option<u64>),
{
    let resp = reqwest::get(url)
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        bail!("GET {url} -> HTTP {}", resp.status());
    }
    let total = resp.content_length();
    let mut file = File::create(dest).with_context(|| format!("create {}", dest.display()))?;
    let mut resp = resp;
    let mut done: u64 = 0;
    while let Some(chunk) = resp.chunk().await.context("read body chunk")? {
        file.write_all(&chunk)?;
        done += chunk.len() as u64;
        on_progress(done, total);
    }
    file.flush()?;
    Ok(())
}

/// Lowercase hex sha256 of a file, read in chunks (never loads it whole).
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Verify a downloaded artifact **before** extracting it. A mismatch is fatal:
/// we never extract unverified binaries.
pub fn verify(path: &Path, expected_hex: &str) -> Result<()> {
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected_hex) {
        bail!(
            "checksum mismatch for {}: expected {expected_hex}, got {actual}",
            path.display()
        );
    }
    Ok(())
}

/// Extract the executable members of a docker tarball into `bin_dir`.
///
/// Members are laid out as `<prefix>/<name>` (e.g. `docker/dockerd`,
/// `docker-rootless-extras/rootlesskit`); only the file name is kept.
fn extract_binaries(tgz_path: &Path, bin_dir: &Path) -> Result<()> {
    fs::create_dir_all(bin_dir)?;
    let file = File::open(tgz_path).with_context(|| format!("open {}", tgz_path.display()))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !matches!(entry.header().entry_type(), tar::EntryType::Regular) {
            continue;
        }
        let path = entry.path()?.into_owned();
        let Some(name) = path.file_name() else {
            continue;
        };
        let dest = bin_dir.join(name);
        let mut out = File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        std::io::copy(&mut entry, &mut out)?;
        set_executable(&dest)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Data-root filesystem compatibility
// ---------------------------------------------------------------------------

/// Whether `dir` reports real `d_type` values from `readdir`.
///
/// containerd's `overlayfs` and `erofs` snapshotters **refuse to load** on a
/// filesystem where `readdir` returns `DT_UNKNOWN`. When they fail, the
/// `leases` gRPC service they register goes down with them, and every image
/// pull dies with:
///
/// ```text
/// error creating temporary lease: Unimplemented:
///   unknown service containerd.services.leases.v1.Leases
/// ```
///
/// This is a *filesystem* property, not a data-root one: an ext4 volume
/// formatted **without** the `filetype` feature fails, while btrfs, xfs
/// (`ftype=1`), tmpfs and ext4-with-`filetype` all work.
///
/// We probe with a raw `getdents64` because Rust's `read_dir` hides the
/// distinction — `DirEntry::file_type()` falls back to an `lstat` when the
/// kernel reports `DT_UNKNOWN`, so such a filesystem still looks fine.
pub fn dir_supports_d_type(dir: &Path) -> Option<bool> {
    /// The kernel's `DT_UNKNOWN`, returned when the fs cannot supply d_type.
    const DT_UNKNOWN: u8 = 0;

    // An empty directory tells us nothing — probe its parent instead.
    let mut saw_entry = false;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if name != "." && name != ".." {
                    saw_entry = true;
                    break;
                }
            }
        }
    }
    if !saw_entry {
        return dir.parent().and_then(dir_supports_d_type);
    }

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;

        let file = File::open(dir).ok()?;
        // 32 KiB comfortably holds a typical directory's dirents.
        let mut buf = vec![0u8; 32 * 1024];
        let nread = unsafe {
            libc::syscall(
                libc::SYS_getdents64,
                file.as_raw_fd() as libc::c_long,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len() as libc::c_long,
            )
        };
        if nread <= 0 {
            return None;
        }
        let end = nread as usize;
        let mut off = 0usize;
        let mut saw_real_type = false;
        // struct linux_dirent64: u64 d_ino, s64 d_off, u16 d_reclen,
        //                         u8 d_type, char d_name[]
        while off + 19 <= end {
            let d_reclen = u16::from_ne_bytes([buf[off + 16], buf[off + 17]]) as usize;
            if d_reclen == 0 || off + d_reclen > end {
                break;
            }
            let d_type = buf[off + 18];
            let name = buf[off + 19..off + d_reclen]
                .split(|b| *b == 0)
                .next()
                .unwrap_or(b"");
            if name != b"." && name != b".." {
                if d_type == DT_UNKNOWN {
                    // One unknown entry means the fs cannot answer at all.
                    return Some(false);
                }
                saw_real_type = true;
            }
            off += d_reclen;
        }
        return Some(saw_real_type);
    }

    #[cfg(not(target_os = "linux"))]
    {
        Some(true)
    }
}

/// Reject a data-root whose filesystem cannot support the bundled engine's
/// containerd snapshotters, with an explanation the user can act on.
///
/// Called before spawning `dockerd` and by the Settings wizard, so a bad path
/// is refused up front instead of surfacing later as an opaque
/// "unknown service containerd.services.leases.v1.Leases" on the first pull.
pub fn check_data_root_filesystem(dir: &Path) -> Result<()> {
    // The directory may not exist yet — its nearest existing ancestor is on
    // the same filesystem, so probe that.
    let mut probe = dir;
    while !probe.exists() {
        match probe.parent() {
            Some(p) if p != probe => probe = p,
            _ => return Ok(()), // nothing to probe; let dockerd report it
        }
    }
    if dir_supports_d_type(probe) == Some(false) {
        bail!(
            "the filesystem holding {} does not report directory entry types \
             (d_type), which Docker's containerd snapshotters require. Image \
             pulls would fail with \"unknown service \
             containerd.services.leases.v1.Leases\". Choose a data root on a \
             filesystem that supports d_type — btrfs, xfs with ftype=1, ext4 \
             with the filetype feature, or tmpfs.",
            probe.display()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Lifecycle helpers
// ---------------------------------------------------------------------------

/// Is this a live unix socket we can actually connect to (not a stale file)?
pub fn is_usable_socket(path: &Path) -> bool {
    use std::os::unix::net::UnixStream;
    path.exists() && UnixStream::connect(path).is_ok()
}

fn prefixed_path(bin: &Path) -> String {
    let old = std::env::var("PATH").unwrap_or_default();
    format!("{}:{old}", bin.display())
}

/// Extra rootlesskit flags for a given `--net` driver.
///
/// The port driver is the one that matters: left unset, rootlesskit defaults to
/// `none` and every published port (`docker run -p`) silently never reaches the
/// host. `pasta` forwards ports itself (`implicit`); `slirp4netns` needs
/// rootlesskit's own forwarder (`builtin`). `host` needs neither.
fn net_flags(net: &str) -> Vec<String> {
    match net {
        "pasta" => vec![
            "--port-driver=implicit".to_string(),
            "--disable-host-loopback".to_string(),
        ],
        "slirp4netns" => vec!["--port-driver=builtin".to_string()],
        _ => Vec::new(),
    }
}

/// Pick a rootless network driver: host `pasta`, else host `slirp4netns`, else
/// the copy we ship alongside the engine. `pasta` is preferred — it is built
/// into modern kernels' userland and needs no sandbox helper.
fn detect_net_driver(bin: &Path) -> String {
    if which("pasta").is_some() {
        "pasta".to_string()
    } else if which("slirp4netns").is_some() || bin.join("slirp4netns").is_file() {
        "slirp4netns".to_string()
    } else {
        // Last resort: hope pasta appears in PATH at spawn time; rootlesskit
        // will report a clear error if neither is present.
        "pasta".to_string()
    }
}

/// True when neither `pasta` nor `slirp4netns` is reachable: rootless
/// networking will likely fail to start. Checks the engine's own bin dir
/// first, then `PATH`.
pub fn missing_net_helper(paths: &EnginePaths) -> bool {
    !net_helper_found(&paths.bin_dir(), std::env::var_os("PATH").as_deref())
}

/// Pure lookup kept separate so the preflight is unit-testable.
fn net_helper_found(bin: &Path, path_var: Option<&std::ffi::OsStr>) -> bool {
    for name in ["pasta", "slirp4netns"] {
        if bin.join(name).is_file() {
            return true;
        }
    }
    let Some(path) = path_var else {
        return false;
    };
    std::env::split_paths(path)
        .any(|d| ["pasta", "slirp4netns"].iter().any(|n| d.join(n).is_file()))
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
}

#[cfg(target_os = "linux")]
fn signal(pid: u32, sig: i32) {
    unsafe {
        libc::kill(pid as i32, sig);
    }
}

#[cfg(not(target_os = "linux"))]
fn signal(_pid: u32, _sig: i32) {}

#[cfg(target_os = "linux")]
fn pid_alive(pid: i32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(not(target_os = "linux"))]
fn pid_alive(_pid: i32) -> bool {
    false
}

/// `comm` of a live pid, used to make sure a pidfile's PID is really ours
/// before signalling it (PIDs get recycled).
#[cfg(target_os = "linux")]
fn proc_comm(pid: i32) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_string())
}

#[cfg(not(target_os = "linux"))]
fn proc_comm(_pid: i32) -> Option<String> {
    None
}

/// A `Command` for the **bundled** `docker` CLI, pointed at our engine and our
/// config dir. This is the single choke point: nothing else in the app may
/// spawn a `docker` binary from PATH.
pub fn docker_command(paths: &EnginePaths) -> Command {
    let mut cmd = Command::new(paths.bin_dir().join("docker"));
    cmd.env("DOCKER_HOST", format!("unix://{}", paths.socket.display()))
        .env("DOCKER_CONFIG", paths.current_dir.join("docker-config"))
        .env("PATH", prefixed_path(&paths.bin_dir()));
    cmd
}

// ---------------------------------------------------------------------------
// Install / update
// ---------------------------------------------------------------------------

/// Progress for the download phase of an install, as a human-readable label
/// plus a 0.0–1.0 fraction when the total size is known.
pub struct InstallProgress {
    pub label: String,
    pub fraction: Option<f64>,
}

/// Download, verify and stage `release`, then atomically point `current` at it.
///
/// Nothing is extracted until every artifact has passed its checksum, and the
/// live `current` symlink is only swapped once the new version is complete — so
/// a failed or interrupted update leaves the previous engine intact.
pub async fn install_release<F>(
    paths: &EnginePaths,
    release: &Release,
    mut on_progress: F,
) -> Result<()>
where
    F: FnMut(InstallProgress),
{
    let staging = paths
        .versions_dir
        .join(format!(".staging-{}", release.version));
    let _ = fs::remove_dir_all(&staging);
    let bin_dir = staging.join("bin");
    fs::create_dir_all(&bin_dir)?;

    let tmp = staging.join("download");
    fs::create_dir_all(&tmp)?;

    // 1. Fetch the two Docker tarballs + the compose plugin, verifying each
    //    before anything is unpacked.
    let artifacts = [
        (docker_url(release.version), "docker", release.docker_sha256),
        (
            rootless_url(release.version),
            "rootless-extras",
            release.rootless_sha256,
        ),
        (
            compose_url(release.compose_version),
            "compose",
            release.compose_sha256,
        ),
    ];
    for (i, (url, name, expected)) in artifacts.iter().enumerate() {
        let dest = tmp.join(name);
        let base = i as f64;
        let mut last = 0u64;
        download(url, &dest, |done, total| {
            if done.saturating_sub(last) < 256 * 1024 && Some(done) != total {
                return;
            }
            last = done;
            on_progress(InstallProgress {
                label: format!("Downloading {name}…"),
                fraction: Some((base + fraction_of(done, total)) / artifacts.len() as f64),
            });
        })
        .await?;
        verify(&dest, expected).with_context(|| format!("verifying {name}"))?;
    }

    // 2. Extract. Binaries keep only their file name; compose goes to the
    //    CLI-plugin path the bundled `docker` will look under DOCKER_CONFIG.
    on_progress(InstallProgress {
        label: "Extracting…".into(),
        fraction: Some(0.9),
    });
    extract_binaries(&tmp.join("docker"), &bin_dir)?;
    extract_binaries(&tmp.join("rootless-extras"), &bin_dir)?;
    let plugins = staging.join("cli-plugins");
    fs::create_dir_all(&plugins)?;
    fs::copy(tmp.join("compose"), plugins.join("docker-compose"))?;
    set_executable(&plugins.join("docker-compose"))?;
    fs::write(staging.join("VERSION"), release.version)?;

    // 3. Mark complete, move staging into place, then swap the symlink. The
    //    marker is written *before* the move so a half-moved tree never looks
    //    installed; `is_installed()` is checked via the `current` symlink only.
    fs::write(staging.join(".complete"), "")?;
    let _ = fs::remove_dir_all(&tmp);
    let final_dir = paths.versions_dir.join(release.version);
    let _ = fs::remove_dir_all(&final_dir);
    fs::rename(&staging, &final_dir)
        .with_context(|| format!("move staged engine to {}", final_dir.display()))?;
    swap_current(paths, release.version)?;

    on_progress(InstallProgress {
        label: "Engine ready".into(),
        fraction: Some(1.0),
    });
    Ok(())
}

fn fraction_of(done: u64, total: Option<u64>) -> f64 {
    match total {
        Some(t) if t > 0 => (done as f64 / t as f64).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// Atomically repoint `current` → `versions/<version>`. A plain rename over the
/// symlink is atomic on the same filesystem, so a reader never sees it missing.
pub fn swap_current(paths: &EnginePaths, version: &str) -> Result<()> {
    let target = paths.versions_dir.join(version);
    let tmp_link = paths.versions_dir.join(".current-tmp");
    let _ = fs::remove_file(&tmp_link);
    std::os::unix::fs::symlink(&target, &tmp_link)?;
    fs::rename(&tmp_link, &paths.current_dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The detector must accept a filesystem that answers with real d_types.
    /// We can't conjure a d_type-less fs in a unit test, so this pins the
    /// contract on the tmpfs/ext4 the tests run on plus the empty-directory
    /// path, which must fall through to the parent rather than report `false`.
    #[test]
    fn d_type_probe_accepts_populated_dir() {
        let dir = std::env::temp_dir().join(format!("dm-dtype-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("entry"), b"x").unwrap();

        assert_eq!(dir_supports_d_type(&dir), Some(true));
        assert!(check_data_root_filesystem(&dir).is_ok());

        fs::remove_dir_all(&dir).ok();
    }

    /// An empty directory carries no dirents to inspect, so the probe must walk
    /// up to a parent that does — never claim `false` out of silence.
    #[test]
    fn d_type_probe_walks_up_from_empty_dir() {
        let root = std::env::temp_dir().join(format!("dm-dtype-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let empty = root.join("empty");
        fs::create_dir_all(&empty).unwrap();

        assert_eq!(dir_supports_d_type(&empty), Some(true));

        fs::remove_dir_all(&root).ok();
    }

    /// A path that does not exist yet must still be judged by the filesystem it
    /// would land on — that is exactly the Settings-wizard case.
    #[test]
    fn filesystem_check_probes_nearest_existing_ancestor() {
        let root = std::env::temp_dir().join(format!("dm-dtype-new-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let not_yet = root.join("a/b/c");

        assert!(!not_yet.exists());
        assert!(check_data_root_filesystem(&not_yet).is_ok());

        fs::remove_dir_all(&root).ok();
    }

    /// Opt-in probe against a real path, to check the detector on the actual
    /// volume rather than tmpfs. The fs backing `/mnt` on the machine that
    /// reported this bug is ext4 formatted **without** the `filetype` feature,
    /// where containerd's snapshotters refuse to load and every pull fails with
    /// the lease error. Run with:
    /// `DM_DTYPE_PATH=/mnt/... cargo test --lib d_type_against_real -- --ignored`
    #[test]
    #[ignore = "requires a machine-specific path in DM_DTYPE_PATH"]
    fn d_type_against_real_path_matches_gate() {
        let Ok(path) = std::env::var("DM_DTYPE_PATH") else {
            return;
        };
        let probe = std::path::PathBuf::from(path);
        // The two functions must always agree: a detection of `false` is
        // exactly what makes the pre-flight check refuse the path.
        assert_eq!(
            dir_supports_d_type(&probe) == Some(false),
            check_data_root_filesystem(&probe).is_err(),
            "detector and pre-flight gate disagree for {}",
            probe.display()
        );
    }

    #[test]
    fn paths_use_xdg_and_keep_socket_short() {
        let p = EnginePaths::new();
        assert!(
            p.socket.as_os_str().len() < 100,
            "unix socket path too long"
        );
        assert!(p.data_root.to_string_lossy().contains("docker-manager"));
        assert!(p.bin_dir().ends_with("bin"));
        assert!(p.compose_plugin().ends_with("docker-compose"));
    }

    #[test]
    fn is_installed_false_when_marker_absent() {
        let p = EnginePaths {
            current_dir: std::env::temp_dir().join("dm-test-none"),
            ..EnginePaths::new()
        };
        let m = EngineManager::new(p);
        assert!(!m.is_installed());
        assert_eq!(m.installed_version(), None);
    }

    #[test]
    fn sha256_file_matches_manual_computation() {
        let dir = std::env::temp_dir().join(format!("dm-sha-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("a.bin");
        std::fs::write(&f, b"hello docker manager").unwrap();
        assert_eq!(
            sha256_file(&f).unwrap(),
            "8ef32b2241975943e4e7247d32a2f780d8791e5b687d199243bdc2825bd002fd"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verify_rejects_bad_checksum_without_exception() {
        let dir = std::env::temp_dir().join(format!("dm-verify-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("x.tgz");
        std::fs::write(&f, b"nope").unwrap();
        assert!(verify(
            &f,
            "0000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extract_binaries_flattens_prefix_and_chmods() {
        let dir = std::env::temp_dir().join(format!("dm-extract-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let tgz = dir.join("pkg.tgz");
        let bin_dir = dir.join("out");

        // Build a tar.gz { docker/foo } fixture.
        let file = std::fs::File::create(&tgz).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "docker/rootlesskit", b"xyz".as_slice())
            .unwrap();
        let gz = tar.into_inner().unwrap();
        gz.finish().unwrap();

        extract_binaries(&tgz, &bin_dir).unwrap();
        let dest = bin_dir.join("rootlesskit");
        assert!(dest.is_file());
        use std::os::unix::fs::PermissionsExt;
        assert!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o111 != 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn docker_command_sets_host_config_and_path() {
        let p = EnginePaths::new();
        let cmd = docker_command(&p);
        let prog = cmd.get_program().to_str().unwrap().to_string();
        assert!(prog.ends_with("docker"));
        let envs: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(k, v)| {
                v.map(|val| {
                    (
                        k.to_string_lossy().into_owned(),
                        val.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect();
        let host = envs
            .iter()
            .find(|(k, _)| k == "DOCKER_HOST")
            .expect("DOCKER_HOST set");
        assert!(host.1.starts_with("unix://"));
        let cfg = envs
            .iter()
            .find(|(k, _)| k == "DOCKER_CONFIG")
            .expect("DOCKER_CONFIG set");
        assert!(cfg.1.ends_with("docker-config"));
        let path = envs.iter().find(|(k, _)| k == "PATH").expect("PATH set");
        assert!(path.1.starts_with(&p.bin_dir().display().to_string()));
    }

    #[test]
    fn swap_current_repoints_symlink_atomically() {
        let dir = std::env::temp_dir().join(format!("dm-swap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let versions = dir.join("versions");
        let v1 = versions.join("v1");
        std::fs::create_dir_all(&v1).unwrap();
        let current = dir.join("current");
        let paths = EnginePaths {
            versions_dir: versions.clone(),
            current_dir: current.clone(),
            ..EnginePaths::new()
        };
        swap_current(&paths, "v1").unwrap();
        assert!(current.is_symlink());
        assert_eq!(
            std::fs::canonicalize(&current).unwrap(),
            versions.join("v1").canonicalize().unwrap()
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn net_helper_found_tracks_bin_dir_and_path() {
        let dir = std::env::temp_dir().join(format!("dm-nethlp-{}", std::process::id()));
        let bin = dir.join("bin");
        let pathdir = dir.join("pathdir");
        let _ = std::fs::create_dir_all(&bin);
        let _ = std::fs::create_dir_all(&pathdir);
        // Neither in bin dir nor PATH -> missing.
        assert!(!net_helper_found(&bin, Some(std::ffi::OsStr::new(""))));
        // Present in the engine's own bin dir -> found regardless of PATH.
        std::fs::write(bin.join("pasta"), b"x").unwrap();
        assert!(net_helper_found(&bin, Some(std::ffi::OsStr::new(""))));
        // Only on PATH (bin dir empty) -> found.
        std::fs::remove_file(bin.join("pasta")).unwrap();
        std::fs::write(pathdir.join("slirp4netns"), b"x").unwrap();
        let path = std::env::join_paths([&pathdir]).unwrap();
        assert!(net_helper_found(&bin, Some(path.as_os_str())));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn net_flags_pasta_sets_implicit_port_driver() {
        let f = net_flags("pasta");
        assert!(f.contains(&"--port-driver=implicit".to_string()));
        assert!(f.contains(&"--disable-host-loopback".to_string()));
    }

    #[test]
    fn net_flags_slirp_sets_builtin_port_driver() {
        let f = net_flags("slirp4netns");
        assert!(f.contains(&"--port-driver=builtin".to_string()));
        assert!(!f.iter().any(|x| x.starts_with("--disable-host-loopback")));
    }

    #[test]
    fn net_flags_unknown_returns_empty() {
        assert!(net_flags("host").is_empty());
        assert!(net_flags("none").is_empty());
    }
}
