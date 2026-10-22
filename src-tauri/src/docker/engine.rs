//! Bundled rootless Docker Engine: paths, pinned release manifest, download +
//! verify + extract, and the child-process lifecycle.
//!
//! The app owns its engine. Nothing here reads or writes the host's Docker
//! install: the engine lives entirely under the app's own data dir and talks
//! over a private unix socket. `dockerd` runs rootless inside `rootlesskit`
//! and is a *child of this app* — it dies with the app.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
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

/// How long a `docker --version` sniff may take before we give up on that
/// candidate. Generous for a real binary, short enough that one broken entry
/// on PATH cannot stall an interactive action.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

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
    /// `XDG_RUNTIME_DIR/hsx2dock` — passed to the child as its runtime dir.
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
        let base = data.join(crate::migrate::APP_DIR);
        let rt = runtime.join(crate::migrate::APP_DIR);
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

    /// Home of the compose plugin Docker will actually discover.
    ///
    /// Docker searches `$DOCKER_CONFIG/cli-plugins`, and every invocation we
    /// run sets `DOCKER_CONFIG` to `current/docker-config`. Trees installed
    /// before that became true kept the plugin at `current/cli-plugins`, so
    /// prefer the canonical path and fall back to the legacy one until
    /// [`ensure_compose_layout`] has moved it.
    pub fn compose_plugin(&self) -> PathBuf {
        let canonical = compose_plugin_dir(self).join("docker-compose");
        if canonical.is_file() {
            return canonical;
        }
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

    /// Locks the child-process state, recovering from a poisoned mutex
    /// instead of panicking: a panic anywhere else while the lock was held
    /// must not cascade into killing the whole app on the next call.
    fn inner_lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Pid of the supervising engine process (rootlesskit) — everything the
    /// engine runs (dockerd, containerd, containers) descends from it, so
    /// caps applied to it cover the whole daemon.
    pub fn child_pid(&self) -> Option<u32> {
        self.inner_lock().child.as_ref().map(|c| c.id())
    }

    /// Applies the configured cgroup caps to the running engine. Both limits
    /// at zero is a no-op ("limits disabled").
    pub fn apply_resource_limits(
        &self,
        memory_mb: u64,
        cpu_cores: f64,
    ) -> Result<String, String> {
        let pid = self
            .child_pid()
            .ok_or_else(|| "the engine is not running".to_string())?;
        apply_cgroup_limits(pid, memory_mb, cpu_cores)
    }

    /// Same recovery for the paths lock (see [`Self::inner_lock`]).
    fn paths_read(&self) -> std::sync::RwLockReadGuard<'_, EnginePaths> {
        self.paths.read().unwrap_or_else(|e| e.into_inner())
    }

    /// See [`Self::paths_read`].
    fn paths_write(&self) -> std::sync::RwLockWriteGuard<'_, EnginePaths> {
        self.paths.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn paths(&self) -> EnginePaths {
        self.paths_read().clone()
    }

    pub fn set_data_root(&self, custom_data_root: Option<&str>) {
        let mut p = self.paths_write();
        *p = EnginePaths::with_custom_data_root(custom_data_root);
    }

    pub fn socket(&self) -> PathBuf {
        self.paths_read().socket.clone()
    }

    /// True once a release has been fully downloaded, verified and extracted.
    pub fn is_installed(&self) -> bool {
        let p = self.paths();
        p.current_dir.join(".complete").is_file()
    }

    pub fn installed_version(&self) -> Option<String> {
        installed_engine_version(&self.paths())
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

        // Retarget containerd's snapshotter dirs when the data root sits on a
        // filesystem without d_type, *before* spawning: dockerd would
        // otherwise boot and then fail every pull with an opaque lease error.
        ensure_snapshotter_compat(&paths.data_root)?;
        // Compose must sit under the `DOCKER_CONFIG` we hand every docker
        // invocation, otherwise the app silently uses the host's copy (or
        // none). Idempotent, and a no-op on trees that are already correct.
        let _ = ensure_compose_layout(&paths);

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

        // Reachable host DNS servers, installed inside the netns by
        // RESOLV_WRAPPER before dockerd starts (empty = leave the network
        // driver's resolv.conf alone).
        let (dns_servers, dns_search) = host_dns_upstreams();

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
            .arg("sh")
            .arg("-c")
            .arg(RESOLV_WRAPPER)
            .arg("dockerd-wrapper")
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
            .env("DM_DNS_SERVERS", dns_servers.join(" "))
            .env("DM_DNS_SEARCH", dns_search.join(" "))
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
        self.inner_lock().child = Some(child);

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
                let mut guard = self.inner_lock();
                match guard.child.as_mut() {
                    Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                    None => true,
                }
            };
            if exited {
                let mut guard = self.inner_lock();
                guard.child = None;
                let _ = fs::remove_file(&paths.pid_file);
                bail!(
                    "engine exited during startup; see {} for details",
                    paths.log_file.display()
                );
            }
            if Instant::now() >= deadline {
                // Still alive but never opened the socket — kill and report.
                if let Some(c) = self.inner_lock().child.take() {
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
        let child = self.inner_lock().child.take();
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
        let child = self.inner_lock().child.take();
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
    // Release CDNs occasionally cut a connection short; a short file would
    // then fail its checksum and look like a bad release. Retry the transfer
    // (from scratch) a couple of times before giving up.
    const ATTEMPTS: u64 = 3;
    let mut last_error = None;
    for attempt in 1..=ATTEMPTS {
        match download_once(url, dest, &mut on_progress).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                let _ = fs::remove_file(dest);
                last_error = Some(e);
                if attempt < ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(400 * attempt)).await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("download failed")))
        .with_context(|| format!("download {url} after {ATTEMPTS} attempts"))
}

/// One transfer attempt. Fails when fewer bytes arrive than the server
/// advertised (a truncated body must never reach checksum verification).
async fn download_once<F>(url: &str, dest: &Path, on_progress: &mut F) -> Result<()>
where
    F: FnMut(u64, Option<u64>),
{
    // `http_get` already turned any non-2xx into an error.
    let resp = http_get(url)
        .await
        .with_context(|| format!("GET {url}"))?;
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
    if let Some(expected) = total {
        if done != expected {
            bail!("incomplete download: got {done} of {expected} bytes");
        }
    }
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

/// Extract a single named member of a docker tarball to `dest`.
///
/// The engine tarball also carries `dockerd`, `containerd`, `runc` and friends;
/// a CLI-only update must not overwrite any of them.
fn extract_member(tgz_path: &Path, member: &str, dest: &Path) -> Result<()> {
    let file = File::open(tgz_path).with_context(|| format!("open {}", tgz_path.display()))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !matches!(entry.header().entry_type(), tar::EntryType::Regular) {
            continue;
        }
        if entry.path()?.file_name().and_then(|n| n.to_str()) != Some(member) {
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = File::create(dest).with_context(|| format!("create {}", dest.display()))?;
        std::io::copy(&mut entry, &mut out)?;
        out.flush()?;
        drop(out);
        set_executable(dest)?;
        return Ok(());
    }
    bail!("{member} not found in {}", tgz_path.display())
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

const SNAPSHOTTER_DTYPE_PLUGINS: [&str; 2] = [
    "io.containerd.snapshotter.v1.overlayfs",
    "io.containerd.snapshotter.v1.erofs",
];

/// containerd's snapshotter plugins refuse to start on a filesystem that
/// reports `DT_UNKNOWN` for every dirent, which cascades into `leases` failing
/// to load and every `docker pull` dying with "unknown service
/// containerd.services.leases.v1.Leases".
///
/// The offending dirs (`overlayfs`, `erofs`) only carry containerd-side
/// bookkeeping — with `containerd-snapshotter=false` our image layers live in
/// the graphdriver under the data root — so retargeting them to a
/// d_type-capable filesystem fixes the engine without moving any real storage.
/// Idempotent: an already-linked dir is left alone, a non-empty real dir is
/// never deleted.
pub fn ensure_snapshotter_compat(dir: &Path) -> Result<()> {
    if dir_supports_d_type(dir) != Some(false) {
        return Ok(());
    }

    let Some(compat) = dtype_compat_root() else {
        bail!(
            "the filesystem holding {} does not report directory entry types \
             (d_type) and no d_type-capable directory is available to host \
             containerd's snapshotter state. Pick a data root on btrfs, xfs \
             with ftype=1, or tmpfs.",
            dir.display()
        );
    };

    let daemon = dir.join("containerd").join("daemon");
    fs::create_dir_all(&daemon)
        .with_context(|| format!("create {}", daemon.display()))?;

    for plugin in SNAPSHOTTER_DTYPE_PLUGINS {
        let target = compat.join(plugin);
        fs::create_dir_all(&target)
            .with_context(|| format!("create {}", target.display()))?;

        let link = daemon.join(plugin);
        match fs::symlink_metadata(&link) {
            // Already retargeted by a previous boot — nothing to do.
            Ok(meta) if meta.file_type().is_symlink() => {}
            // An empty real dir is safe to replace; a populated one is not.
            Ok(meta) if meta.is_dir() => {
                let mut entries = fs::read_dir(&link)
                    .with_context(|| format!("read {}", link.display()))?;
                if entries.next().is_some() {
                    bail!(
                        "{} already contains containerd data on a filesystem \
                         without d_type. If you're migrating from another root, \
                         this is likely dormant snapshotter state (with \
                         containerd-snapshotter=false, image layers live in the \
                         graphdriver). Consider starting from a clean target \
                         data root, or move this data to btrfs/xfs with ftype=1 \
                         first.",
                        link.display()
                    );
                }
                fs::remove_dir(&link)?;
                std::os::unix::fs::symlink(&target, &link)?;
            }
            Ok(_) => bail!("{} exists and is not a directory", link.display()),
            Err(_) => std::os::unix::fs::symlink(&target, &link)?,
        }
    }
    Ok(())
}

/// A directory that does report real `d_type` values, to host the retargeted
/// containerd snapshotter state. Prefers the app's own data dir; falls back to
/// `/tmp` (tmpfs, always d_type-capable) when that lives on a bad filesystem.
pub fn dtype_compat_root() -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let candidates = [
        home.join(".local/share")
            .join(crate::migrate::APP_DIR)
            .join("dtype-compat"),
        PathBuf::from("/tmp").join(crate::migrate::DTYPE_COMPAT_DIR),
    ];

    candidates.into_iter().find(|candidate| {
        // The dir may not exist yet — probe the nearest ancestor that does,
        // exactly like `check_data_root_filesystem` does for the data root.
        let mut probe: &Path = candidate;
        while !probe.exists() {
            match probe.parent() {
                Some(p) if p != probe => probe = p,
                _ => break,
            }
        }
        dir_supports_d_type(probe) == Some(true)
    })
}

/// Would copying `source` onto the d_type-less `target` leave containerd
/// snapshotter state that [`ensure_snapshotter_compat`] must refuse to delete?
///
/// Checked before the copy so a doomed migration fails up front — no engine
/// stop, no gigabytes moved, no half-finished target — instead of erroring
/// *after* `cp -a` has already run.
pub fn migrate_dtype_conflict(source: &Path, target: &Path) -> Option<String> {
    if dir_supports_d_type(target) != Some(false) {
        return None; // target fs is fine, whatever the source holds is fine too
    }
    if source == target {
        return None;
    }

    for plugin in SNAPSHOTTER_DTYPE_PLUGINS {
        let from = source.join("containerd").join("daemon").join(plugin);
        let Ok(meta) = fs::symlink_metadata(&from) else {
            continue; // absent — cp -a won't copy it
        };
        if meta.file_type().is_symlink() {
            continue; // already retargeted — cp -a preserves the link
        }
        if !meta.is_dir() {
            return Some(format!("{} exists and is not a directory", from.display()));
        }
        let populated = fs::read_dir(&from)
            .ok()
            .map(|mut it| it.next().is_some())
            .unwrap_or(false);
        if populated {
            return Some(format!(
                "{} already contains containerd data, which cannot be moved onto \
                 a filesystem without d_type. If migrating from another root, \
                 this is likely dormant snapshotter state (with \
                 containerd-snapshotter=false image layers live in the \
                 graphdriver) — start from a clean target data root, or move to \
                 btrfs/xfs with ftype=1 first.",
                from.display()
            ));
        }
    }
    None
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

/// Host DNS servers and search domains that are safe to install inside the
/// engine's netns. Loopback entries are skipped: inside the netns they would
/// point at the engine's own (empty) loopback, and even host-side they are
/// often a local proxy (AdGuard, systemd-stub) that may be down.
fn parse_resolv_conf(text: &str) -> (Vec<String>, Vec<String>) {
    let mut servers = Vec::new();
    let mut search = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("nameserver") => {
                if servers.len() >= 3 {
                    continue;
                }
                let addr = it.next().unwrap_or("");
                let loopback = addr.starts_with("127.") || addr == "::1";
                if addr.is_empty() || loopback || servers.iter().any(|s| s == addr) {
                    continue;
                }
                servers.push(addr.to_string());
            }
            Some("search") | Some("domain") => {
                for d in it {
                    if !search.iter().any(|s| s == d) {
                        search.push(d.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    (servers, search)
}

fn host_dns_upstreams() -> (Vec<String>, Vec<String>) {
    std::fs::read_to_string("/etc/resolv.conf")
        .map(|t| parse_resolv_conf(&t))
        .unwrap_or_default()
}

/// Runs inside the rootlesskit netns (as the command before dockerd): rewrites
/// `/etc/resolv.conf` with the reachable host nameservers the parent parsed,
/// so dockerd, every container, and kind nodes resolve through real DNS even
/// when the network driver's relay (10.0.2.3) or a local DNS proxy is dead.
/// A no-op when `DM_DNS_SERVERS` is unset.
const RESOLV_WRAPPER: &str = r#"if [ -n "$DM_DNS_SERVERS" ]; then
    if [ -L /etc/resolv.conf ]; then rm -f /etc/resolv.conf; fi
    {
        printf 'nameserver %s\n' $DM_DNS_SERVERS
        if [ -n "$DM_DNS_SEARCH" ]; then printf 'search %s\n' "$DM_DNS_SEARCH"; fi
    } > /etc/resolv.conf || true
fi
exec "$@""#;

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

/// Places the engine process into a dedicated cgroup v2 and applies the
/// memory/CPU caps there, so a runaway daemon cannot starve the host.
///
/// Best-effort by design: the cgroup is created as a child of *our own*
/// cgroup (which systemd already delegated to the user session), and any
/// failure — non-cgroup-v2 kernel, read-only hierarchy, missing controllers —
/// returns an error the caller logs and ignores rather than failing the
/// engine start. Both limits at zero is a no-op.
#[cfg(target_os = "linux")]
pub fn apply_cgroup_limits(pid: u32, memory_mb: u64, cpu_cores: f64) -> Result<String, String> {
    if memory_mb == 0 && cpu_cores <= 0.0 {
        return Ok("engine resource limits are disabled".to_string());
    }

    // Our own cgroup path from the unified (`0::`) v2 hierarchy; creating the
    // child cgroup under it inherits the session's delegation.
    let self_cgroup = std::fs::read_to_string("/proc/self/cgroup")
        .map_err(|e| format!("cannot read /proc/self/cgroup: {e}"))?;
    let cg_rel = self_cgroup
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "/")
        .ok_or_else(|| "the process is not running under a writable cgroup v2 scope".to_string())?;
    let dir = std::path::PathBuf::from("/sys/fs/cgroup")
        .join(cg_rel.trim_start_matches('/'))
        .join("hsx2dock-engine");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("cannot create cgroup {}: {e}", dir.display()))?;

    // Move the engine (and, by inheritance, its whole process tree) in.
    std::fs::write(dir.join("cgroup.procs"), pid.to_string())
        .map_err(|e| format!("cannot move engine pid {pid} into {}: {e}", dir.display()))?;

    // Always (re)write both controllers so dropping a limit back to
    // "unlimited" clears a cap left over from a previous start.
    let mem_value = if memory_mb > 0 {
        memory_mb.saturating_mul(1024 * 1024).to_string()
    } else {
        "max".to_string()
    };
    std::fs::write(dir.join("memory.max"), &mem_value)
        .map_err(|e| format!("cannot set memory.max to {mem_value}: {e}"))?;
    // cgroup v2 cpu.max: quota (µs) per 100 ms period; cores * 100000.
    let cpu_value = if cpu_cores > 0.0 {
        format!("{} 100000", (cpu_cores * 100_000.0).round() as u64)
    } else {
        "max 100000".to_string()
    };
    std::fs::write(dir.join("cpu.max"), &cpu_value)
        .map_err(|e| format!("cannot set cpu.max to '{cpu_value}': {e}"))?;

    Ok(format!(
        "engine capped at {} / {}",
        if memory_mb > 0 {
            format!("{memory_mb} MiB memory")
        } else {
            "unlimited memory".to_string()
        },
        if cpu_cores > 0.0 {
            format!("{cpu_cores} CPU cores")
        } else {
            "unlimited CPU".to_string()
        }
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn apply_cgroup_limits(_pid: u32, _memory_mb: u64, _cpu_cores: f64) -> Result<String, String> {
    Err("engine resource limits require cgroup v2 (Linux)".to_string())
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

/// A `Command` for a specific docker binary, pointed at our engine.
///
/// `DOCKER_HOST` always targets *our* socket — both the system and the
/// bundled binary talk to the engine this app manages, never the host
/// daemon. `DOCKER_CONFIG` always points at our config dir so plugin
/// resolution (compose) stays inside the app. The two sources differ only in
/// which binary runs and whether we prepend our own `bin` to `PATH`: for the
/// bundled CLI that is required so it finds our helpers, while a system CLI
/// runs in the user's own environment untouched.
pub fn docker_command_for(paths: &EnginePaths, bin: &Path, source: CliSource) -> Command {
    let mut cmd = Command::new(bin);
    cmd.env("DOCKER_HOST", format!("unix://{}", paths.socket.display()))
        .env("DOCKER_CONFIG", paths.current_dir.join("docker-config"));
    if source == CliSource::Bundled {
        cmd.env("PATH", prefixed_path(&paths.bin_dir()));
    }
    cmd
}

/// A `Command` for the **bundled** `docker` CLI, pointed at our engine and our
/// config dir. This is the single choke point: nothing else in the app may
/// spawn a `docker` binary from PATH.
///
/// Callers that should honour the user's system CLI use
/// [`docker_command_resolved`] instead.
pub fn docker_command(paths: &EnginePaths) -> Command {
    let mut cmd = Command::new(paths.bin_dir().join("docker"));
    cmd.env("DOCKER_HOST", format!("unix://{}", paths.socket.display()))
        .env("DOCKER_CONFIG", paths.current_dir.join("docker-config"))
        .env("PATH", prefixed_path(&paths.bin_dir()));
    cmd
}

/// The CLI preference the app is running with.
///
/// Held process-wide rather than threaded through every call site: the
/// resolver runs inside `docker_command_resolved`, which is reached from
/// compose helpers that have no `AppState`. `save_app_config` keeps this in
/// step with the persisted config.
static CLI_MODE: std::sync::RwLock<Option<String>> = std::sync::RwLock::new(None);

/// Record the user's CLI preference (`"auto"`, `"system"`, `"bundled"`).
pub fn set_cli_mode(mode: &str) {
    let mode = match mode {
        "system" | "bundled" => mode,
        _ => "auto",
    };
    if let Ok(mut guard) = CLI_MODE.write() {
        *guard = Some(mode.to_string());
    }
}

/// The current CLI preference; `"auto"` until something sets it.
pub fn cli_mode() -> String {
    CLI_MODE
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| "auto".to_string())
}

/// A `Command` for whichever docker CLI the resolver picked.
///
/// Falls back to the bundled binary when nothing resolves, so callers never
/// have to handle a missing CLI just to build a command.
pub fn docker_command_resolved(paths: &EnginePaths) -> Command {
    let mode = cli_mode();
    match resolve_cli(paths, &mode, &[]) {
        Ok(r) => docker_command_for(paths, &r.path, r.source),
        Err(_) => docker_command(paths),
    }
}

// ---------------------------------------------------------------------------
// Install / update
// ---------------------------------------------------------------------------

/// Ordered checklist for a bundled-engine update. Stop/start are driven by the
/// command (they need the supervisor), the middle steps by the installer; both
/// emit indices into this same list so the UI can show every step up front.
pub const ENGINE_UPDATE_STEPS: &[&str] = &[
    "Stop Docker Engine",
    "Download Docker Engine",
    "Verify checksums",
    "Replace binaries",
    "Verify new engine",
    "Start Docker Engine",
];

/// Checklist for updating the bundled `docker` CLI on its own.
pub const CLI_UPDATE_STEPS: &[&str] = &[
    "Download Docker CLI",
    "Verify checksum",
    "Replace CLI",
    "Verify version",
];

/// Checklist for updating the docker-compose plugin.
pub const COMPOSE_UPDATE_STEPS: &[&str] = &[
    "Download Docker Compose",
    "Verify checksum",
    "Verify binary",
    "Install plugin",
];

/// Turn a static checklist into the owned form carried by every event.
pub fn step_plan(steps: &[&str]) -> Vec<String> {
    steps.iter().map(|s| s.to_string()).collect()
}

/// Progress for one binary update run.
///
/// `plan` is the complete ordered checklist, repeated on every event so a
/// listener that attaches late still renders the whole stepper. `step` indexes
/// into it. Only step boundaries and notable events set `log`; byte-level
/// download ticks only set `detail`, otherwise a 70 MB download would flood
/// the run log.
#[derive(Clone, Debug, serde::Serialize)]
pub struct InstallProgress {
    /// `engine`, `cli`, `compose` or `kind`.
    pub target: &'static str,
    /// Version being installed.
    pub version: String,
    /// Human label for the current step.
    pub label: String,
    /// 0.0–1.0 across the whole run, when known.
    pub fraction: Option<f64>,
    pub plan: Vec<String>,
    pub step: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl InstallProgress {
    fn build(
        target: &'static str,
        version: &str,
        plan: &[String],
        step: usize,
        fraction: Option<f64>,
        detail: Option<String>,
        log: Option<String>,
        done: bool,
        error: Option<String>,
    ) -> Self {
        Self {
            target,
            version: version.to_string(),
            label: plan.get(step).cloned().unwrap_or_default(),
            fraction,
            plan: plan.to_vec(),
            step: Some(step),
            detail,
            log,
            done,
            error,
        }
    }

    /// Enter a step; the step name is also the log line.
    pub fn stage(
        target: &'static str,
        version: &str,
        plan: &[String],
        step: usize,
        fraction: Option<f64>,
    ) -> Self {
        let label = plan.get(step).cloned();
        Self::build(
            target,
            version,
            plan,
            step,
            fraction,
            None,
            label,
            false,
            None,
        )
    }

    /// High-frequency update inside a step: detail only, no log line.
    pub fn tick(
        target: &'static str,
        version: &str,
        plan: &[String],
        step: usize,
        fraction: Option<f64>,
        detail: impl Into<String>,
    ) -> Self {
        Self::build(
            target,
            version,
            plan,
            step,
            fraction,
            Some(detail.into()),
            None,
            false,
            None,
        )
    }

    /// An extra log line inside the current step (size, digest, skip reason).
    pub fn note(
        target: &'static str,
        version: &str,
        plan: &[String],
        step: usize,
        fraction: Option<f64>,
        line: impl Into<String>,
    ) -> Self {
        Self::build(
            target,
            version,
            plan,
            step,
            fraction,
            None,
            Some(line.into()),
            false,
            None,
        )
    }

    /// Terminal success event.
    pub fn finished(target: &'static str, version: &str, plan: &[String]) -> Self {
        let step = plan.len().saturating_sub(1);
        Self::build(
            target,
            version,
            plan,
            step,
            Some(1.0),
            None,
            Some("Update complete".to_string()),
            true,
            None,
        )
    }

    /// Terminal failure event: keeps the step that failed and the message.
    pub fn failed(
        target: &'static str,
        version: &str,
        plan: &[String],
        step: usize,
        error: impl Into<String>,
    ) -> Self {
        Self::build(
            target,
            version,
            plan,
            step,
            None,
            None,
            None,
            true,
            Some(error.into()),
        )
    }
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
    let plan = step_plan(ENGINE_UPDATE_STEPS);
    let version = release.version;
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
        on_progress(InstallProgress::stage(
            "engine", version, &plan, 1, Some(0.05),
        ));
        download(url, &dest, |done, total| {
            if done.saturating_sub(last) < 256 * 1024 && Some(done) != total {
                return;
            }
            last = done;
            let within = fraction_of(done, total);
            on_progress(InstallProgress::tick(
                "engine",
                version,
                &plan,
                1,
                Some(0.05 + 0.45 * ((base + within) / artifacts.len() as f64)),
                match total {
                    Some(t) if t > 0 => {
                        format!("{name}: {} / {}", human_size(done), human_size(t))
                    }
                    _ => format!("{name}: {}", human_size(done)),
                },
            ));
        })
        .await?;
        on_progress(InstallProgress::note(
            "engine",
            version,
            &plan,
            2,
            Some(0.55),
            format!("Verifying {name} checksum"),
        ));
        verify(&dest, expected).with_context(|| format!("verifying {name}"))?;
    }

    // 2. Extract. Binaries keep only their file name; compose goes to the
    //    CLI-plugin path the bundled `docker` will look under DOCKER_CONFIG.
    on_progress(InstallProgress::stage(
        "engine", version, &plan, 3, Some(0.65),
    ));
    extract_binaries(&tmp.join("docker"), &bin_dir)?;
    extract_binaries(&tmp.join("rootless-extras"), &bin_dir)?;
    // Docker finds plugins under `$DOCKER_CONFIG/cli-plugins`, and every
    // invocation we run sets `DOCKER_CONFIG` to `<current>/docker-config`, so
    // stage the plugin on that path. The tree becomes `current` by symlink.
    let compose_src = tmp.join("compose");
    let compose_dst = staging
        .join("docker-config")
        .join("cli-plugins")
        .join("docker-compose");
    fs::create_dir_all(
        compose_dst
            .parent()
            .ok_or_else(|| anyhow::anyhow!("compose destination has no parent"))?,
    )?;
    fs::copy(&compose_src, &compose_dst)?;
    set_executable(&compose_dst)?;
    fs::write(staging.join("VERSION"), release.version)?;

    // 3. Self-check the staged CLI: a tree that cannot report its own version
    //    must never become `current`.
    on_progress(InstallProgress::stage(
        "engine", version, &plan, 4, Some(0.85),
    ));
    let probe = Command::new(bin_dir.join("docker"))
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("run the staged docker --version")?;
    let probe_out = format!(
        "{}{}",
        String::from_utf8_lossy(&probe.stdout),
        String::from_utf8_lossy(&probe.stderr)
    );
    let reported = reported_version(&probe_out);
    if !probe.status.success() || normalize_version(reported) != version {
        bail!(
            "staged engine reported {}, but {version} was expected",
            if reported.is_empty() { "nothing" } else { reported }
        );
    }
    on_progress(InstallProgress::note(
        "engine",
        version,
        &plan,
        4,
        Some(0.9),
        format!("Staged CLI reports Docker {reported}"),
    ));

    // 4. Mark complete, move staging into place, then swap the symlink. The
    //    marker is written *before* the move so a half-moved tree never looks
    //    installed; `is_installed()` is checked via the `current` symlink only.
    fs::write(staging.join(".complete"), "")?;
    let _ = fs::remove_dir_all(&tmp);
    let final_dir = paths.versions_dir.join(release.version);
    let _ = fs::remove_dir_all(&final_dir);
    fs::rename(&staging, &final_dir)
        .with_context(|| format!("move staged engine to {}", final_dir.display()))?;
    swap_current(paths, release.version)?;

    on_progress(InstallProgress::note(
        "engine",
        version,
        &plan,
        4,
        Some(0.95),
        format!("Docker Engine {version} activated"),
    ));
    Ok(())
}

pub fn fraction_of(done: u64, total: Option<u64>) -> f64 {
    match total {
        Some(t) if t > 0 => (done as f64 / t as f64).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// Human byte count for live progress details (e.g. `67.2 MB`).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
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

// ---------------------------------------------------------------------------
// Runtime engine update: discover a newer release, then install it
// ---------------------------------------------------------------------------

/// Directory listing of Docker's official static channel — where both the
/// engine tarballs and the version list we check against come from.
const STATIC_LISTING_URL: &str = "https://download.docker.com/linux/static/stable/x86_64/";

/// What a "check for engine updates" pass found, for the Settings UI.
#[derive(Clone, Debug, serde::Serialize)]
pub struct EngineUpdateInfo {
    /// Version the `current` symlink points at, `None` when nothing is installed.
    pub installed_version: Option<String>,
    /// Newest stable version Docker publishes on the official static channel.
    pub latest_version: String,
    /// True when `latest_version` beats what we have — or when we have nothing
    /// installed at all, in which case the same button performs a first install.
    pub update_available: bool,
    /// Version we ship pinned checksums for, so installing *that* one is
    /// verified end to end rather than only over TLS.
    pub pinned_version: String,
    /// Channel the check read; surfaced so the UI can label the row.
    pub channel: String,
}

/// Compare dotted versions by their numeric X.Y.Z prefix, missing parts reading
/// as zero. Anything unparseable sorts lowest instead of erroring — this is a
/// UI hint, not a security decision.
fn version_triple(v: &str) -> (u64, u64, u64) {
    let mut it = v.trim().trim_start_matches('v').split('.');
    let mut next = || it.next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    (next(), next(), next())
}

/// True when `a` is a strictly newer dotted version than `b`.
pub fn version_gt(a: &str, b: &str) -> bool {
    version_triple(a) > version_triple(b)
}

/// Whether a freshly discovered `latest` is worth offering an update for.
/// Nothing installed counts as an update, since the button performs the first
/// install in that case.
pub fn update_available(latest: &str, installed: Option<&str>) -> bool {
    installed.map(|installed| version_gt(latest, installed)).unwrap_or(true)
}

/// The version token out of `docker --version` output —
/// `Docker version 29.9.1, build f415da8` → `29.9.1`.
///
/// The token arrives comma-punctuated in practice, so punctuation is trimmed
/// while letters, digits, `.` and `-` are kept. An unrecognised format yields
/// an empty string, which the install's self-check treats as a failure rather
/// than as a pass.
pub fn reported_version(output: &str) -> &str {
    output
        .split_whitespace()
        .skip_while(|word| *word != "version")
        .nth(1)
        .unwrap_or_default()
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-')
}

/// A version token without its leading `v`, so `v2.39.4` and `2.39.4` compare
/// equal. Compose and kind both print the `v` prefix; docker does not.
pub fn normalize_version(version: &str) -> &str {
    version.trim().trim_start_matches('v')
}

/// Highest `docker-<major.minor.patch>.tgz` version in a static-channel
/// directory listing.
///
/// Rootless extras share the `docker-` prefix (`docker-rootless-extras-…`) but
/// their first segment is not a number, so they are dropped along with any
/// entry that has a fourth version segment or an unparseable tail — only plain
/// stable X.Y.Z builds can win.
pub fn parse_latest_static_version(listing: &str) -> Option<String> {
    let mut best: Option<((u64, u64, u64), String)> = None;
    for (idx, _) in listing.match_indices("docker-") {
        let after = &listing[idx + "docker-".len()..];
        let Some(end) = after.find(".tgz") else { continue };
        let version = &after[..end];
        let mut parts = version.split('.');
        let (Some(major), Some(minor), Some(patch)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if parts.next().is_some() {
            continue;
        }
        let (Ok(major), Ok(minor), Ok(patch)) =
            (major.parse::<u64>(), minor.parse::<u64>(), patch.parse::<u64>())
        else {
            continue;
        };
        let key = (major, minor, patch);
        if best.as_ref().is_none_or(|(k, _)| *k < key) {
            best = Some((key, version.to_string()));
        }
    }
    best.map(|(_, version)| version)
}

/// Newest stable engine version published on Docker's static channel.
pub async fn fetch_latest_engine_version() -> Result<String> {
    let listing = http_get(STATIC_LISTING_URL)
        .await
        .context("connect to Docker's static binary channel")?
        .text()
        .await
        .context("read the channel listing body")?;
    parse_latest_static_version(&listing)
        .context("no stable engine version found in the channel listing")
}

/// The engine version `current` points at, `None` when nothing is installed.
pub fn installed_engine_version(paths: &EnginePaths) -> Option<String> {
    fs::read_to_string(paths.current_dir.join("VERSION"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Check Docker's official channel against what is installed right now.
///
/// A listing fetch failure is an error rather than an empty result: the UI
/// needs to say "couldn't check" instead of quietly reporting "up to date".
pub async fn check_engine_update(paths: &EnginePaths) -> Result<EngineUpdateInfo> {
    let installed_version = installed_engine_version(paths);
    let latest_version = fetch_latest_engine_version().await?;
    let update_available = update_available(&latest_version, installed_version.as_deref());
    Ok(EngineUpdateInfo {
        installed_version,
        latest_version,
        update_available,
        pinned_version: DEFAULT_RELEASE.version.to_string(),
        channel: "stable".to_string(),
    })
}

/// Download `version` from Docker's official channel and point `current` at it.
///
/// The installed engine stays untouched until the new one is fully staged,
/// self-checked and renamed into place, so an interrupted or failed update
/// leaves the previous engine working — `swap_current` is the only mutation
/// and it is a single atomic rename.
///
/// `version == DEFAULT_RELEASE.version` routes through [`install_release`] and
/// is checked against the checksums shipped with the app. Docker publishes no
/// checksums for any other build, so those are fetched over TLS from
/// `download.docker.com`, their sha256 is recorded into the new version's
/// `CHECKSUMS`, and the staged CLI must answer `docker --version` with its own
/// version before anything becomes current.
pub async fn install_engine_update<F>(paths: &EnginePaths, version: &str, mut on_progress: F) -> Result<()>
where
    F: FnMut(InstallProgress),
{
    if version == DEFAULT_RELEASE.version {
        return install_release(paths, &DEFAULT_RELEASE, on_progress).await;
    }

    let plan = step_plan(ENGINE_UPDATE_STEPS);
    let staging = paths.versions_dir.join(format!(".staging-{version}"));
    let _ = fs::remove_dir_all(&staging);
    let bin_dir = staging.join("bin");
    fs::create_dir_all(&bin_dir)?;
    let tmp = staging.join("download");
    fs::create_dir_all(&tmp)?;

    // 1. Fetch both tarballs, recording what we actually received.
    let artifacts = [(docker_url(version), "docker"), (rootless_url(version), "rootless-extras")];
    let mut checksums = String::new();
    for (i, (url, name)) in artifacts.iter().enumerate() {
        let dest = tmp.join(name);
        let base = i as f64;
        let mut last = 0u64;
        on_progress(InstallProgress::stage(
            "engine", version, &plan, 1, Some(0.05),
        ));
        download(url, &dest, |done, total| {
            if done.saturating_sub(last) < 256 * 1024 && Some(done) != total {
                return;
            }
            last = done;
            let within = fraction_of(done, total);
            on_progress(InstallProgress::tick(
                "engine",
                version,
                &plan,
                1,
                Some(0.05 + 0.45 * ((base + within) / artifacts.len() as f64)),
                match total {
                    Some(t) if t > 0 => {
                        format!("{name}: {} / {}", human_size(done), human_size(t))
                    }
                    _ => format!("{name}: {}", human_size(done)),
                },
            ));
        })
        .await?;
        on_progress(InstallProgress::note(
            "engine",
            version,
            &plan,
            2,
            Some(0.55),
            format!("Verifying {name} checksum"),
        ));
        checksums.push_str(&format!("{}  {}\n", sha256_file(&dest)?, name));
    }
    fs::write(staging.join("CHECKSUMS"), &checksums)?;

    // 2. Extract. Binaries keep only their file name, as in the pinned path.
    on_progress(InstallProgress::stage(
        "engine", version, &plan, 3, Some(0.65),
    ));
    extract_binaries(&tmp.join("docker"), &bin_dir)?;
    extract_binaries(&tmp.join("rootless-extras"), &bin_dir)?;

    // 3. Keep the compose plugin the current engine already has; a first ever
    //    install falls back to the pinned release so `docker compose` works.
    // Stage the plugin on the path Docker will search once this tree becomes
    // `current`: `$DOCKER_CONFIG/cli-plugins`, and our invocations always set
    // `DOCKER_CONFIG` to `<current>/docker-config`.
    let plugins = staging.join("docker-config").join("cli-plugins");
    fs::create_dir_all(&plugins)?;
    let existing = paths.compose_plugin();
    if existing.is_file() {
        fs::copy(&existing, plugins.join("docker-compose"))?;
        set_executable(&plugins.join("docker-compose"))?;
    } else {
        let dest = tmp.join("compose");
        download(
            &compose_url(DEFAULT_RELEASE.compose_version),
            &dest,
            |_, _| {},
        )
        .await?;
        verify(&dest, DEFAULT_RELEASE.compose_sha256).context("verifying compose")?;
        fs::copy(&dest, plugins.join("docker-compose"))?;
        set_executable(&plugins.join("docker-compose"))?;
    }

    // 4. Self-check before anything is swapped: a CLI that cannot report its
    //    own version must never become `current`.
    on_progress(InstallProgress::stage(
        "engine", version, &plan, 4, Some(0.85),
    ));
    let probe = Command::new(bin_dir.join("docker"))
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("run the staged docker --version")?;
    let probe_out = format!(
        "{}{}",
        String::from_utf8_lossy(&probe.stdout),
        String::from_utf8_lossy(&probe.stderr)
    );
    // `docker --version` prints `Docker version <ver>, build <hash>`; match the
    // version token exactly so `29.9.1` can never pass as `29.9.10`.
    let reported = reported_version(&probe_out);
    if !probe.status.success() || normalize_version(reported) != version {
        bail!(
            "staged engine reported {}, but {version} was expected: {}",
            if reported.is_empty() {
                "no readable version"
            } else {
                reported
            },
            probe_out.trim()
        );
    }
    on_progress(InstallProgress::note(
        "engine",
        version,
        &plan,
        4,
        Some(0.9),
        format!("Staged CLI reports Docker {reported}"),
    ));

    // 5. Mark complete, move staging into place, then swap the symlink. The
    //    marker is written *before* the move so a half-moved tree never looks
    //    installed.
    fs::write(staging.join("VERSION"), version)?;
    let _ = fs::remove_dir_all(&tmp);
    fs::write(staging.join(".complete"), "")?;
    let final_dir = paths.versions_dir.join(version);
    let _ = fs::remove_dir_all(&final_dir);
    fs::rename(&staging, &final_dir)
        .with_context(|| format!("move staged engine to {}", final_dir.display()))?;
    swap_current(paths, version)?;

    on_progress(InstallProgress::note(
        "engine",
        version,
        &plan,
        4,
        Some(0.95),
        format!("Docker Engine {version} activated"),
    ));
    Ok(())
}

// ---------------------------------------------------------------------------
// Docker CLI resolution
// ---------------------------------------------------------------------------
//
// The app must work whether or not the host has Docker installed. Case 1: a
// system `docker` exists — we link with it and let the user's own binary run
// against our engine. Case 2: there is no system `docker`, or it is older
// than the binary we shipped — we run our bundled copy. Either way we always
// point `DOCKER_HOST` at our socket; the choice of *binary* is what differs.

/// Which docker binary the app decided to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliSource {
    System,
    Bundled,
}

/// What the resolver found, plus the human sentence explaining the choice.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CliResolution {
    /// The binary that will actually be executed.
    pub source: CliSource,
    /// Absolute path of the chosen binary.
    pub path: PathBuf,
    /// `docker --version` of the chosen binary.
    pub version: String,
    pub system_path: Option<PathBuf>,
    /// `None` when there is no system `docker` at all.
    pub system_version: Option<String>,
    pub bundled_path: PathBuf,
    /// `None` when no engine has been installed yet.
    pub bundled_version: Option<String>,
    /// One sentence naming the winner and why. Always populated.
    pub reason: String,
}

pub fn probe_cli_version(bin: &Path) -> Option<String> {
    // Bounded: a `docker` on PATH that hangs (a broken wrapper, a slow
    // network mount) must not freeze compose or the settings page. The
    // version probe is only ever a sniff, so a timeout just means "unknown".
    let mut child = std::process::Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(_) => return None,
        }
    }

    let mut out = Vec::new();
    child.stdout.as_mut()?.read_to_end(&mut out).ok()?;
    let stdout = String::from_utf8_lossy(&out);
    let version = reported_version(&stdout);
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

/// Find a `docker` binary on `PATH`, never one that belongs to us.
///
/// `skip` lets the caller exclude paths we installed ourselves (the PATH
/// shim) so the resolver never mistakes our own shim for the user's CLI.
pub fn find_system_docker(paths: &EnginePaths, skip: &[PathBuf]) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let ours = paths.bin_dir().canonicalize().ok();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("docker");
        if !candidate.is_file() {
            continue;
        }
        if skip.iter().any(|s| s == &candidate) {
            continue;
        }
        // A shim we installed earlier is not the user's CLI: it execs one of
        // our binaries, so counting it would make the resolver compare our
        // bundled docker against itself.
        if is_our_shim(&candidate) {
            continue;
        }
        if let (Some(ours), Ok(real)) = (ours.as_ref(), candidate.canonicalize()) {
            if &real == ours {
                continue;
            }
        }
        if probe_cli_version(&candidate).is_some() {
            return Some(candidate);
        }
    }
    None
}

/// The pure half of [`resolve_cli`]: given already-probed versions, decide
/// which CLI wins. Kept separate from the probing so the precedence rules are
/// testable without a filesystem or a PATH lookup.
pub fn decide_cli(
    bundled_path: &Path,
    bundled_version: Option<&str>,
    system_path: Option<&Path>,
    system_version: Option<&str>,
    mode: &str,
) -> Result<CliResolution, String> {
    let bundled_path = bundled_path.to_path_buf();
    let system_path = system_path.map(Path::to_path_buf);
    let bundled_version = bundled_version.map(str::to_string);
    let system_version = system_version.map(str::to_string);

    let build = |source: CliSource, path: PathBuf, version: String, reason: String| CliResolution {
        source,
        path,
        version,
        system_path: system_path.clone(),
        system_version: system_version.clone(),
        bundled_path: bundled_path.clone(),
        bundled_version: bundled_version.clone(),
        reason,
    };

    match mode {
        "system" => match (system_path.as_deref(), system_version.as_deref()) {
            (Some(path), Some(version)) => Ok(build(
                CliSource::System,
                path.to_path_buf(),
                version.to_string(),
                format!(
                    "Prefer system docker set in Settings — using {version} at {}.",
                    path.display()
                ),
            )),
            _ => Err(
                "Prefer system docker is set, but no working system `docker` was found on PATH."
                    .into(),
            ),
        },
        "bundled" => match bundled_version.as_deref() {
            Some(version) => Ok(build(
                CliSource::Bundled,
                bundled_path.clone(),
                version.to_string(),
                format!(
                    "Prefer bundled docker set in Settings — using {version} at {}.",
                    bundled_path.display()
                ),
            )),
            None => Err("Prefer bundled docker is set, but no engine has been installed yet.".into()),
        },
        _ => match (
            system_path.as_deref(),
            system_version.as_deref(),
            bundled_version.as_deref(),
        ) {
            (None, _, None) => Err(
                "No docker CLI found — install Docker on this machine, or install an engine in Settings."
                    .into(),
            ),
            (Some(path), Some(sv), None) => Ok(build(
                CliSource::System,
                path.to_path_buf(),
                sv.to_string(),
                format!("No bundled engine installed — using system docker {sv}."),
            )),
            (None, _, Some(bv)) => Ok(build(
                CliSource::Bundled,
                bundled_path.clone(),
                bv.to_string(),
                format!("No system docker on PATH — using bundled docker {bv}."),
            )),
            (Some(path), Some(sv), Some(bv)) => {
                // Bundled only wins when it is strictly newer; a tie goes to
                // the system binary so we never shadow the user's own
                // toolchain with an identical copy.
                if version_gt(bv, sv) {
                    Ok(build(
                        CliSource::Bundled,
                        bundled_path.clone(),
                        bv.to_string(),
                        format!(
                            "Bundled docker {bv} is newer than the system {sv} — using the bundled docker."
                        ),
                    ))
                } else {
                    Ok(build(
                        CliSource::System,
                        path.to_path_buf(),
                        sv.to_string(),
                        format!(
                            "System docker {sv} is at least as new as the bundled {bv} — using your docker."
                        ),
                    ))
                }
            }
            // Unreachable: a resolved path always carries a probed version.
            _ => Err("No docker CLI found.".into()),
        },
    }
}

pub fn resolve_cli(
    paths: &EnginePaths,
    mode: &str,
    skip: &[PathBuf],
) -> Result<CliResolution, String> {
    let bundled_path = paths.bin_dir().join("docker");
    let bundled_version = if bundled_path.is_file() {
        probe_cli_version(&bundled_path).or_else(|| installed_engine_version(paths))
    } else {
        None
    };
    let system_path = find_system_docker(paths, skip);
    let system_version = system_path.as_deref().and_then(probe_cli_version);

    decide_cli(
        &bundled_path,
        bundled_version.as_deref(),
        system_path.as_deref(),
        system_version.as_deref(),
        mode,
    )
}

// ---------------------------------------------------------------------------
// Opt-in PATH shim
// ---------------------------------------------------------------------------
//
// A machine with no Docker installed gets a `docker` command in the user's
// terminal, without touching the system. The shim is a small script that
// execs the resolved binary with our engine's socket and config — so it works
// for a bundled CLI that no other PATH entry would expose. It is only ever
// written on request, and only ever over itself.

/// Where the shim goes. `~/.local/bin` precedes `/usr/local/bin` on a typical
/// interactive PATH, so this wins without reordering anything.
pub fn shim_path() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(home.join(".local/bin/docker"))
}

/// Marker so we never delete a shim we did not write, and can tell an old
/// shim from a real binary.
const SHIM_MARKER: &str = "# hsx2dock shim";

/// Marker written by builds before the rename to Hsx2Dock. Still recognised as
/// ours so the shim can be regenerated in place instead of being refused.
const LEGACY_SHIM_MARKER: &str = "# docker-manager shim";

/// True when `path` is the script we generate, not a real binary the user
/// installed (nor someone else's file that merely mentions us).
fn is_our_shim(path: &Path) -> bool {
    // The marker sits on the second line, after the `/bin/sh` shebang.
    fs::read_to_string(path)
        .map(|c| {
            c.lines()
                .take(3)
                .any(|l| l.trim() == SHIM_MARKER || l.trim() == LEGACY_SHIM_MARKER)
        })
        .unwrap_or(false)
}

pub fn shim_installed() -> bool {
    shim_path()
        .filter(|d| is_our_shim(d))
        .filter(|d| d.exists())
        .is_some()
}

/// The binary the installed shim would exec. `None` when not installed.
pub fn shim_target() -> Option<PathBuf> {
    let dest = shim_path()?;
    if !is_our_shim(&dest) || !dest.exists() {
        return None;
    }
    let body = std::fs::read_to_string(&dest).ok()?;
    parse_shim_exec(&body)
}

/// Pull the target out of a generated shim's `exec …` line.
///
/// Split out from [`shim_target`] so it can be tested without a `$HOME`, the
/// same way [`is_our_shim`] is.
fn parse_shim_exec(body: &str) -> Option<PathBuf> {
    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("exec ") else {
            continue;
        };
        let rest = rest.trim();
        // Generated shims write the path unquoted (`exec /p/docker "$@"`), but
        // accept a quoted form too so a hand-edited shim still parses.
        if let Some(stripped) = rest.strip_prefix('"') {
            if let Some(end) = stripped.find('"') {
                return Some(PathBuf::from(&stripped[..end]));
            }
        }
        if let Some(part) = rest.split_whitespace().next() {
            return Some(PathBuf::from(part));
        }
    }
    None
}

/// Whether the shim actually takes effect in a terminal.
///
/// Writing the file is not enough: it only wins if its directory is on PATH
/// *and* no other `docker` sits ahead of it. Reporting "installed" without
/// this check would promise something the shell may never deliver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShimEffect {
    /// The shim's directory is on PATH and nothing shadows it.
    Active,
    /// The shim's directory is not on PATH at all.
    NotOnPath,
    /// Another `docker` earlier on PATH wins over the shim.
    Shadowed,
}

/// How the installed shim would resolve, or `None` when none is installed.
pub fn shim_effect() -> Option<ShimEffect> {
    let dest = shim_path()?;
    if !is_our_shim(&dest) {
        return None;
    }
    let shim_dir = dest.parent()?.to_path_buf();
    let path_var = std::env::var_os("PATH")?;

    let mut shim_at: Option<usize> = None;
    for (idx, dir) in std::env::split_paths(&path_var).enumerate() {
        if dir == shim_dir && shim_at.is_none() {
            shim_at = Some(idx);
        }
        // A real docker ahead of the shim directory takes precedence.
        let candidate = dir.join("docker");
        if candidate.is_file() && !is_our_shim(&candidate) {
            match shim_at {
                // Already passed the shim dir → the shim wins.
                Some(at) if at < idx => break,
                // This docker comes first → the shim never runs.
                _ => return Some(ShimEffect::Shadowed),
            }
        }
    }

    Some(match shim_at {
        Some(_) => ShimEffect::Active,
        None => ShimEffect::NotOnPath,
    })
}

/// Write the shim, refusing to overwrite anything that is not already ours.
pub fn ensure_shim(paths: &EnginePaths, mode: &str) -> Result<PathBuf> {
    ensure_shim_for(paths, mode, None)
}

/// Writes the shim with `DOCKER_HOST` pointing at `host_override` when given
/// (a remote endpoint), and at our own socket otherwise.
pub fn ensure_shim_for(
    paths: &EnginePaths,
    mode: &str,
    host_override: Option<&str>,
) -> Result<PathBuf> {
    let dest = shim_path().ok_or_else(|| anyhow::anyhow!("no home directory for the shim"))?;
    if dest.exists() && !shim_installed() {
        bail!(
            "{} already exists and is not our shim — refusing to overwrite it",
            dest.display()
        );
    }
    let res = resolve_cli(paths, mode, &[]).map_err(anyhow::Error::msg)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let script = format!(
        "#!/bin/sh\n\
         {SHIM_MARKER}\n\
         # Generated by Hsx2Dock; regenerate from Settings if it looks stale.\n\
         export DOCKER_HOST={host}\n\
         export DOCKER_CONFIG={config}\n\
         exec {bin} \"$@\"\n",
        config = paths.current_dir.join("docker-config").display(),
        bin = res.path.display(),
        host = host_override
            .map(|h| h.to_string())
            .unwrap_or_else(|| format!("unix://{}", paths.socket.display())),
    );
    fs::write(&dest, script).with_context(|| format!("write {}", dest.display()))?;
    set_executable(&dest)?;
    Ok(dest)
}

/// Remove the shim, but only if it is ours.
pub fn remove_shim() -> Result<bool> {
    let Some(dest) = shim_path() else {
        return Ok(false);
    };
    if !shim_installed() {
        return Ok(false);
    }
    fs::remove_file(&dest).with_context(|| format!("remove {}", dest.display()))?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Compose plugin layout
// ---------------------------------------------------------------------------
//
// Docker discovers plugins under `$DOCKER_CONFIG/cli-plugins`, and
// `docker_command` always sets `DOCKER_CONFIG` to `current/docker-config`.
// Older trees dropped the plugin at `current/cli-plugins`, where Docker will
// never look, so the app silently fell back to whatever compose the host had
// (or none at all). `ensure_compose_layout` moves the plugin into place.

/// Canonical home of the bundled compose plugin, under our `DOCKER_CONFIG`.
pub fn compose_plugin_dir(paths: &EnginePaths) -> PathBuf {
    paths.current_dir
        .join("docker-config")
        .join("cli-plugins")
}

/// Move a plugin into the directory Docker actually searches.
fn place_compose_plugin(src: &Path, dest: &Path) -> Result<()> {
    if src == dest {
        return Ok(());
    }
    fs::create_dir_all(
        dest.parent()
            .ok_or_else(|| anyhow::anyhow!("plugin path has no parent"))?,
    )?;
    // Copy rather than rename: the source may sit on another filesystem, and
    // a leftover legacy file is harmless next to a working canonical one.
    fs::copy(src, dest)
        .with_context(|| format!("copy {} to {}", src.display(), dest.display()))?;
    set_executable(dest)?;
    Ok(())
}

/// Normalise an installed tree so compose lives where Docker will find it.
///
/// Idempotent: a missing plugin is simply reported, never invented. Called
/// before every engine start and by the CLI status command.
pub fn ensure_compose_layout(paths: &EnginePaths) -> Result<Option<PathBuf>> {
    let canonical = compose_plugin_dir(paths).join("docker-compose");
    if canonical.is_file() {
        return Ok(Some(canonical));
    }
    let legacy = paths.current_dir.join("cli-plugins").join("docker-compose");
    if legacy.is_file() {
        place_compose_plugin(&legacy, &canonical)?;
        return Ok(Some(canonical));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Compose plugin updates (independent release axis)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Serialize)]
pub struct ComposeUpdateInfo {
    pub installed_version: Option<String>,
    pub latest_version: String,
    pub update_available: bool,
    pub pinned_version: String,
}

/// Compose ships `checksums.txt` alongside its release assets, so unlike the
/// engine its digest is upstream's rather than ours.
fn compose_checksums_url(version: &str) -> String {
    format!("{COMPOSE_BASE}/v{version}/checksums.txt")
}

fn parse_checksums(text: &str, file_name: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let digest = parts.next()?;
        let name = parts.next()?;
        if name.trim_start_matches('*') == file_name {
            return Some(digest.to_string());
        }
    }
    None
}

/// Version token of `docker-compose version`, or `None` when it will not run.
pub fn compose_plugin_version(bin: &Path) -> Option<String> {
    let out = std::process::Command::new(bin)
        .arg("version")
        .arg("--short")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() {
        return None;
    }
    Some(raw.trim_start_matches('v').to_string())
}

/// `GET` with a User-Agent.
///
/// The GitHub API rejects requests that omit one with `403 Forbidden`
/// ("Request forbidden by administrative rules"), and a bare `reqwest::get`
/// sends none. Release-asset downloads are served from a CDN where the header
/// is optional, but sending it everywhere keeps one code path.
pub async fn http_get(url: &str) -> Result<reqwest::Response> {
    let resp = reqwest::Client::builder()
        .user_agent(concat!("hsx2dock/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("build HTTP client")?
        .get(url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    resp.error_for_status()
        .with_context(|| format!("GET {url}"))
}

/// Latest release tag for `repo` (e.g. `docker/compose`), sans leading `v`.
pub async fn fetch_github_latest_version(repo: &str) -> Result<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let body = http_get(&url)
        .await?
        .text()
        .await
        .context("read GitHub releases response")?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("decode GitHub releases JSON")?;
    let tag = json["tag_name"].as_str().unwrap_or_default();
    let version = tag.trim().trim_start_matches('v');
    if version.is_empty() {
        bail!("could not read a release tag from GitHub ({repo})");
    }
    Ok(version.to_string())
}

/// GitHub's unauthenticated API allows ~60 requests/hour per IP. The Settings
/// page can be opened repeatedly, so every "latest release" answer is cached
/// per repo: successes for six hours, failures for two minutes (a flaky link
/// must not turn into a rate-limit loop, but an explicit retry still goes out).
const GITHUB_OK_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const GITHUB_FAIL_TTL: Duration = Duration::from_secs(2 * 60);

type GithubCache = HashMap<String, (Instant, std::result::Result<String, String>)>;
static GITHUB_LATEST: OnceLock<tokio::sync::Mutex<GithubCache>> = OnceLock::new();

/// Cached, single-flight wrapper around [`fetch_github_latest_version`]:
/// concurrent page mounts serialize on the mutex, so a burst costs one request.
pub async fn fetch_github_latest_cached(repo: &str) -> Result<String> {
    let cache = GITHUB_LATEST.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()));
    let mut guard = cache.lock().await;
    if let Some((at, cached)) = guard.get(repo) {
        let ttl = if cached.is_ok() {
            GITHUB_OK_TTL
        } else {
            GITHUB_FAIL_TTL
        };
        if at.elapsed() < ttl {
            return cached.clone().map_err(|e| anyhow::anyhow!(e));
        }
    }
    let fresh = fetch_github_latest_version(repo)
        .await
        .map_err(|e| format!("{e:#}"));
    guard.insert(repo.to_string(), (Instant::now(), fresh.clone()));
    fresh.map_err(|e| anyhow::anyhow!(e))
}

pub async fn check_compose_update(paths: &EnginePaths) -> Result<ComposeUpdateInfo> {
    let plugin = ensure_compose_layout(paths)?
        .unwrap_or_else(|| compose_plugin_dir(paths).join("docker-compose"));
    let installed = compose_plugin_version(&plugin);
    let latest = fetch_github_latest_cached("docker/compose").await?;
    let update = update_available(&latest, installed.as_deref());
    Ok(ComposeUpdateInfo {
        installed_version: installed,
        latest_version: latest,
        update_available: update,
        pinned_version: DEFAULT_RELEASE.compose_version.to_string(),
    })
}

/// Download a newer compose plugin and drop it in front of Docker.
///
/// No daemon stop: the plugin is a file we load per-invocation, so replacing
/// it while compose is idle cannot disturb a running engine.
pub async fn install_compose_update<F>(
    paths: &EnginePaths,
    version: &str,
    mut on_progress: F,
) -> Result<()>
where
    F: FnMut(InstallProgress),
{
    let version = version.trim().trim_start_matches('v').to_string();
    if version.is_empty() {
        bail!("no compose version given");
    }

    let dir = compose_plugin_dir(paths);
    fs::create_dir_all(&dir)?;
    let dest = dir.join("docker-compose");
    let staging = dir.join(format!(".compose-{version}.staging"));
    if staging.exists() {
        fs::remove_file(&staging).ok();
    }

    let plan = step_plan(COMPOSE_UPDATE_STEPS);
    on_progress(InstallProgress::stage(
        "compose", &version, &plan, 0, Some(0.05),
    ));
    let file_name = "docker-compose-linux-x86_64";
    let mut last = 0u64;
    download(&compose_url(&version), &staging, |done, total| {
        if done.saturating_sub(last) < 256 * 1024 && Some(done) != total {
            return;
        }
        last = done;
        on_progress(InstallProgress::tick(
            "compose",
            &version,
            &plan,
            0,
            Some(0.05 + 0.45 * fraction_of(done, total)),
            match total {
                Some(t) if t > 0 => format!("{} / {}", human_size(done), human_size(t)),
                _ => human_size(done),
            },
        ));
    })
    .await?;
    // The downloaded file is created 0644; the self-check below executes it,
    // so make it runnable *before* probing (this is what used to fail every
    // compose update at "Verify binary").
    set_executable(&staging).context("make staged compose executable")?;

    on_progress(InstallProgress::stage(
        "compose", &version, &plan, 1, Some(0.55),
    ));
    let sums = http_get(&compose_checksums_url(&version))
        .await?
        .text()
        .await
        .context("read compose checksums.txt")?;
    let expected = parse_checksums(&sums, file_name)
        .ok_or_else(|| anyhow::anyhow!("no {file_name} entry in compose {version} checksums.txt"))?;
    verify(&staging, &expected).context("verifying compose")?;

    on_progress(InstallProgress::stage(
        "compose", &version, &plan, 2, Some(0.7),
    ));
    // A compose that cannot report its own version is not fit to install.
    let reported = probe_cli_version(&staging).ok_or_else(|| {
        anyhow::anyhow!("staged compose {version} failed `--version` — refusing to install")
    })?;
    if normalize_version(&reported) != version {
        bail!("staged compose reported {reported}, but {version} was expected");
    }

    on_progress(InstallProgress::note(
        "compose",
        &version,
        &plan,
        2,
        Some(0.8),
        format!("Staged compose reports {}", normalize_version(&reported)),
    ));

    on_progress(InstallProgress::stage(
        "compose", &version, &plan, 3, Some(0.9),
    ));
    fs::rename(&staging, &dest)
        .with_context(|| format!("install compose {version} to {}", dest.display()))?;

    on_progress(InstallProgress::finished("compose", &version, &plan));
    Ok(())
}

// ---------------------------------------------------------------------------
// Bundled Docker CLI updates (independent of the daemon lifecycle)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Serialize)]
pub struct CliUpdateInfo {
    /// Version the bundled `docker` reports, `None` when nothing is installed.
    pub installed_version: Option<String>,
    /// Newest stable version Docker publishes on the static channel.
    pub latest_version: String,
    pub update_available: bool,
    /// Absolute path of the bundled CLI an update would replace.
    pub path: PathBuf,
}

/// Version the bundled `docker` reports. Probed live so a manual replacement
/// is reflected immediately instead of reading a stale `VERSION` file.
pub fn bundled_cli_version(paths: &EnginePaths) -> Option<String> {
    probe_cli_version(&paths.bin_dir().join("docker"))
}

pub async fn check_cli_update(paths: &EnginePaths) -> Result<CliUpdateInfo> {
    let installed_version = bundled_cli_version(paths);
    let latest_version = fetch_latest_engine_version().await?;
    Ok(CliUpdateInfo {
        update_available: update_available(&latest_version, installed_version.as_deref()),
        installed_version,
        latest_version,
        path: paths.bin_dir().join("docker"),
    })
}

/// Download just the `docker` CLI from Docker's static channel and swap it in.
///
/// `dockerd` already holds its own inode, so replacing the CLI under a running
/// engine is safe: the next `docker` invocation simply execs the new file.
/// Nothing else in the engine tree is touched.
pub async fn install_cli_update<F>(
    paths: &EnginePaths,
    version: &str,
    mut on_progress: F,
) -> Result<()>
where
    F: FnMut(InstallProgress),
{
    let version = version.trim().trim_start_matches('v').to_string();
    if version.is_empty() {
        bail!("no CLI version given");
    }
    let bin_dir = paths.bin_dir();
    if !bin_dir.is_dir() {
        bail!("no bundled engine installed yet — install the engine first");
    }

    let plan = step_plan(CLI_UPDATE_STEPS);
    let staging = paths.current_dir.join(format!(".cli-{version}.staging"));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    let tarball = staging.join("docker.tgz");

    on_progress(InstallProgress::stage(
        "cli", &version, &plan, 0, Some(0.05),
    ));
    let mut last = 0u64;
    download(&docker_url(&version), &tarball, |done, total| {
        if done.saturating_sub(last) < 256 * 1024 && Some(done) != total {
            return;
        }
        last = done;
        on_progress(InstallProgress::tick(
            "cli",
            &version,
            &plan,
            0,
            Some(0.05 + 0.45 * fraction_of(done, total)),
            match total {
                Some(t) if t > 0 => format!("{} / {}", human_size(done), human_size(t)),
                _ => human_size(done),
            },
        ));
    })
    .await?;

    on_progress(InstallProgress::stage(
        "cli", &version, &plan, 1, Some(0.55),
    ));
    // The pinned release has a maintainer-checked digest; any other version is
    // fetched over TLS, and we record the digest we actually received.
    let expected = if version == DEFAULT_RELEASE.version {
        DEFAULT_RELEASE.docker_sha256.to_string()
    } else {
        sha256_file(&tarball)?
    };
    verify(&tarball, &expected).context("verifying docker CLI tarball")?;
    on_progress(InstallProgress::note(
        "cli",
        &version,
        &plan,
        1,
        Some(0.6),
        format!("sha256 {expected}"),
    ));

    on_progress(InstallProgress::stage(
        "cli", &version, &plan, 2, Some(0.7),
    ));
    let staged = staging.join("docker");
    extract_member(&tarball, "docker", &staged)?;

    on_progress(InstallProgress::stage(
        "cli", &version, &plan, 3, Some(0.85),
    ));
    let reported = probe_cli_version(&staged).ok_or_else(|| {
        anyhow::anyhow!("staged docker {version} failed `--version` — refusing to install")
    })?;
    if normalize_version(&reported) != version {
        bail!("staged docker reported {reported}, but {version} was expected");
    }
    fs::rename(&staged, bin_dir.join("docker"))
        .with_context(|| "replace bundled docker CLI".to_string())?;
    if version != DEFAULT_RELEASE.version {
        let _ = fs::write(
            paths.current_dir.join("CLI-CHECKSUM"),
            format!("{expected}  docker {version}\n"),
        );
    }
    let _ = fs::remove_dir_all(&staging);

    on_progress(InstallProgress::finished("cli", &version, &plan));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_prefix_is_ignored_when_comparing() {
        assert_eq!(normalize_version("v2.39.4"), "2.39.4");
        assert_eq!(normalize_version("2.39.4"), "2.39.4");
        assert_eq!(
            reported_version("Docker Compose version v2.39.4"),
            "v2.39.4"
        );
        assert_eq!(
            normalize_version(reported_version("Docker version 28.4.0, build abc")),
            "28.4.0"
        );
    }

    #[test]
    fn engine_plan_names_every_step_in_order() {
        let plan = step_plan(ENGINE_UPDATE_STEPS);
        assert_eq!(plan.len(), 6);
        assert_eq!(plan.first().map(String::as_str), Some("Stop Docker Engine"));
        assert_eq!(plan.last().map(String::as_str), Some("Start Docker Engine"));
    }

    #[test]
    fn failed_progress_marks_the_failing_step() {
        let plan = step_plan(COMPOSE_UPDATE_STEPS);
        let progress = InstallProgress::failed("compose", "2.40.0", &plan, 2, "boom");
        assert!(progress.done);
        assert_eq!(progress.step, Some(2));
        assert_eq!(progress.error.as_deref(), Some("boom"));
        assert_eq!(progress.plan.len(), COMPOSE_UPDATE_STEPS.len());
        assert_eq!(progress.target, "compose");
    }

    #[test]
    fn human_size_uses_binary_units() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(70 * 1024 * 1024), "70.0 MB");
    }

    /// The netns resolv.conf we install must keep only reachable (non-loopback)
    /// host servers, in order, capped at three, with search domains merged.
    #[test]
    fn parse_resolv_conf_filters_loops_and_limits_count() {
        let txt = "nameserver 127.0.0.1\nnameserver ::1\nnameserver 10.15.16.70\nnameserver 10.15.16.68\nnameserver 10.15.16.69\nsearch fis.vn internal.example";
        let (s, d) = parse_resolv_conf(txt);
        assert_eq!(s.len(), 3);
        assert!(!s.iter().any(|x| x.starts_with("127.")));
        assert!(!s.iter().any(|x| x == "::1"));
        assert_eq!(&s[0], "10.15.16.70");
        assert_eq!(&s[1], "10.15.16.68");
        assert_eq!(&s[2], "10.15.16.69");
        assert!(d.contains(&"fis.vn".to_string()));
        assert!(d.contains(&"internal.example".to_string()));
    }

    /// A panic while a lock is held poisons it; the accessors must recover
    /// the guard instead of cascading the panic into the whole app.
    #[test]
    fn poisoned_locks_degrade_instead_of_panicking() {
        let mgr = EngineManager::new(EnginePaths::with_custom_data_root(None));
        let quiet = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let inner_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = mgr.inner.lock().unwrap();
            panic!("poison the child lock");
        }))
        .is_err();
        let paths_panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = mgr.paths.write().unwrap();
            panic!("poison the paths lock");
        }))
        .is_err();
        std::panic::set_hook(quiet);
        assert!(inner_panicked && paths_panicked, "poisoning steps must panic");

        // Every accessor recovers from the poisoned locks instead of panicking.
        let g1 = mgr.inner_lock();
        let g2 = mgr.paths_read();
        drop(g1);
        drop(g2);
        let g3 = mgr.paths_write();
        drop(g3);
        let _paths = mgr.paths();
        let _socket = mgr.socket();
    }

    #[test]
    fn version_gt_compares_numeric_triples() {
        assert!(version_gt("29.9.1", "29.9.0"));
        assert!(version_gt("29.10.0", "29.9.9"));
        assert!(version_gt("30.0.0", "29.99.99"));
        assert!(!version_gt("29.9.0", "29.9.1"));
        assert!(!version_gt("29.9.1", "29.9.1"));
        // A leading `v` and a missing trailing part are tolerated.
        assert!(version_gt("v29.9.1", "29.9.0"));
        assert!(version_gt("29.10", "29.9.9"));
        // Unparseable input sorts lowest rather than panicking.
        assert!(version_gt("29.9.1", "nightly"));
        assert!(!version_gt("nightly", "29.9.1"));
    }

    #[test]
    fn reported_version_reads_the_token_and_tolerates_punctuation() {
        assert_eq!(
            reported_version("Docker version 29.9.1, build f415da8"),
            "29.9.1"
        );
        assert_eq!(reported_version("Docker version 29.9.0\n"), "29.9.0");
        // Unrecognised output must read as "no version", never as a match.
        assert_eq!(reported_version("docker: command not found"), "");
        assert_eq!(reported_version(""), "");
    }

    #[test]
    fn update_is_offered_only_for_a_newer_version_or_no_engine() {
        assert!(update_available("29.9.1", Some("29.9.0")));
        assert!(update_available("29.9.1", None));
        assert!(!update_available("29.9.1", Some("29.9.1")));
        assert!(!update_available("29.9.1", Some("29.10.0")));
    }

    #[test]
    fn parse_latest_static_version_picks_the_highest_stable_build() {
        let listing = r#"
            <a href="../">../</a>
            <a href="docker-rootless-extras-29.7.0.tgz">docker-rootless-extras-29.7.0.tgz</a>
            <a href="docker-29.9.0.tgz">docker-29.9.0.tgz</a>
            <a href="docker-29.9.1.tgz">docker-29.9.1.tgz</a>
            <a href="docker-rootless-extras-29.9.1.tgz">docker-rootless-extras-29.9.1.tgz</a>
            <a href="docker-29.10.0.tgz">docker-29.10.0.tgz</a>
            <a href="sbx/">sbx/</a>
        "#;
        assert_eq!(
            parse_latest_static_version(listing).as_deref(),
            Some("29.10.0")
        );
    }

    #[test]
    fn parse_latest_static_version_ignores_listings_without_an_engine() {
        // Rootless extras are the only `docker-` entries here and are not engines.
        assert_eq!(
            parse_latest_static_version(
                "<a href=\"docker-rootless-extras-29.9.1.tgz\">docker-rootless-extras-29.9.1.tgz</a>"
            )
            .as_deref(),
            None
        );
        assert_eq!(parse_latest_static_version(""), None);
    }

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

    /// On a healthy filesystem the shim must be a pure no-op: no directories
    /// created, no symlinks written. This is the case every normal user hits.
    #[test]
    fn snapshotter_compat_is_noop_on_d_type_filesystem() {
        let root = std::env::temp_dir().join(format!("dm-compat-ok-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        assert_eq!(dir_supports_d_type(&root), Some(true));
        ensure_snapshotter_compat(&root).unwrap();

        assert!(
            !root.join("containerd").exists(),
            "no-op shim must not create containerd/ under a healthy data root"
        );

        fs::remove_dir_all(&root).ok();
    }

    /// `migrate_data_root` calls the shim on a target that may not exist yet.
    /// It must judge the filesystem the path *would* land on and stay a no-op
    /// there, rather than erroring or creating directories early.
    #[test]
    fn snapshotter_compat_handles_missing_dir() {
        let root = std::env::temp_dir().join(format!("dm-compat-missing-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let absent = root.join("not/created/yet");

        assert!(!absent.exists());
        ensure_snapshotter_compat(&absent).unwrap();

        assert!(
            !absent.exists(),
            "shim must not create the data root itself"
        );
        assert!(
            !root.join("not").exists(),
            "shim must not create intermediate directories for a healthy fs"
        );

        fs::remove_dir_all(&root).ok();
    }

    /// A doomed migration must be caught *before* any data moves: source holds a
    /// populated snapshotter dir, target is on a d_type-less fs.
    #[test]
    fn migrate_conflict_is_detected_before_copy() {
        let root = std::env::temp_dir().join(format!("dm-mig-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();

        // Populated overlayfs snapshotter dir in the source, like the default
        // data root's 42M of dormant state.
        let plugin = source
            .join("containerd/daemon")
            .join(SNAPSHOTTER_DTYPE_PLUGINS[0]);
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("metadata.db"), b"x").unwrap();

        // Target on a healthy fs → no conflict, whatever source holds.
        assert_eq!(migrate_dtype_conflict(&source, &target), None);
        assert!(ensure_snapshotter_compat(&target).is_ok());

        fs::remove_dir_all(&root).ok();
    }

    /// The real conflict path needs a d_type-less *target*, so it lives in the
    /// env-gated test below. What is assertable everywhere: a populated
    /// snapshotter dir in the source never blocks a migration onto a healthy fs,
    /// because the target needs no shim at all.
    #[test]
    fn migrate_conflict_never_blocks_healthy_target() {
        let root = std::env::temp_dir().join(format!("dm-mig-ok-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();

        let plugin = source
            .join("containerd/daemon")
            .join(SNAPSHOTTER_DTYPE_PLUGINS[0]);
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("metadata.db"), b"x").unwrap();

        // Healthy target: no conflict regardless of source contents.
        if dir_supports_d_type(&target) == Some(true) {
            assert_eq!(migrate_dtype_conflict(&source, &target), None);
            assert!(
                ensure_snapshotter_compat(&target).is_ok(),
                "shim must stay a no-op on a healthy target"
            );
            assert!(
                !target.join("containerd").exists(),
                "no-op must not create dirs on a healthy target"
            );
        }

        fs::remove_dir_all(&root).ok();
    }

    /// The path that actually matters: source with populated snapshotter state,
    /// target on a filesystem without d_type. Must be reported *before* any copy.
    ///
    /// `DM_DTYPE_PATH=/mnt/... cargo test --lib migrate_conflict_on_dtype_less -- --ignored`
    #[test]
    #[ignore = "requires a machine-specific path in DM_DTYPE_PATH"]
    fn migrate_conflict_on_dtype_less() {
        let Ok(pat) = std::env::var("DM_DTYPE_PATH") else {
            return;
        };
        let target = std::path::PathBuf::from(&pat);
        assert_eq!(dir_supports_d_type(&target), Some(false));

        let root = std::env::temp_dir().join(format!("dm-mig-src-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let source = root.join("source");
        fs::create_dir_all(&source).unwrap();

        let plugin = source
            .join("containerd/daemon")
            .join(SNAPSHOTTER_DTYPE_PLUGINS[0]);
        fs::create_dir_all(&plugin).unwrap();
        fs::write(plugin.join("metadata.db"), b"x").unwrap();

        let conflict = migrate_dtype_conflict(&source, &target);
        assert!(
            conflict.is_some(),
            "populated snapshotter dir moving to a d_type-less target must conflict"
        );
        assert!(
            conflict.unwrap().contains("containerd data"),
            "conflict message must name the offending data"
        );

        // Absent and empty source dirs must NOT conflict — those are what the
        // shim can safely replace with a link after the copy.
        let clean = root.join("clean");
        fs::create_dir_all(&clean).unwrap();
        assert_eq!(migrate_dtype_conflict(&clean, &target), None);

        let empty = root.join("empty");
        let ep = empty
            .join("containerd/daemon")
            .join(SNAPSHOTTER_DTYPE_PLUGINS[1]);
        fs::create_dir_all(&ep).unwrap();
        assert_eq!(migrate_dtype_conflict(&empty, &target), None);

        fs::remove_dir_all(&root).ok();
    }

    /// The fallback directory must itself resolve onto a d_type-capable
    /// filesystem — retargeting onto another DT_UNKNOWN fs would fix nothing.
    #[test]
    fn dtype_compat_root_reports_d_type() {
        let Some(compat) = dtype_compat_root() else {
            // No d_type-capable location at all: not something a unit test can
            // arrange, and the real-path test below covers the shim itself.
            return;
        };

        let mut probe: &Path = &compat;
        while !probe.exists() {
            match probe.parent() {
                Some(p) if p != probe => probe = p,
                _ => break,
            }
        }
        assert_eq!(
            dir_supports_d_type(probe),
            Some(true),
            "compat root {} lands on a filesystem without d_type",
            compat.display()
        );
    }

    /// End-to-end against a real d_type-less volume: both snapshotter dirs come
    /// back symlinked onto a d_type-capable filesystem, and a re-run is a no-op.
    ///
    /// Run with:
    /// `DM_DTYPE_PATH=/mnt/... cargo test --lib snapshotter_compat_shims -- --ignored`
    #[test]
    #[ignore = "requires a machine-specific path in DM_DTYPE_PATH"]
    fn snapshotter_compat_shims_real_d_type_path() {
        let Ok(pat) = std::env::var("DM_DTYPE_PATH") else {
            return;
        };
        let data_root = std::path::PathBuf::from(pat);
        assert_eq!(
            dir_supports_d_type(&data_root),
            Some(false),
            "{} should be on a filesystem without d_type",
            data_root.display()
        );

        ensure_snapshotter_compat(&data_root).unwrap();
        ensure_snapshotter_compat(&data_root).unwrap(); // idempotent

        let daemon = data_root.join("containerd").join("daemon");
        for plugin in [
            "io.containerd.snapshotter.v1.overlayfs",
            "io.containerd.snapshotter.v1.erofs",
        ] {
            let link = daemon.join(plugin);
            let meta = fs::symlink_metadata(&link)
                .unwrap_or_else(|e| panic!("{}: {e}", link.display()));
            assert!(
                meta.file_type().is_symlink(),
                "{} must be a symlink, got {meta:?}",
                link.display()
            );

            let target = fs::read_link(&link).unwrap();
            let mut probe: &Path = &target;
            while !probe.exists() {
                match probe.parent() {
                    Some(p) if p != probe => probe = p,
                    _ => break,
                }
            }
            assert_eq!(
                dir_supports_d_type(probe),
                Some(true),
                "{} points at a filesystem without d_type",
                link.display()
            );
        }
    }

    #[test]
    fn paths_use_xdg_and_keep_socket_short() {
        let p = EnginePaths::new();
        assert!(
            p.socket.as_os_str().len() < 100,
            "unix socket path too long"
        );
        assert!(p.data_root.to_string_lossy().contains("hsx2dock"));
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

    #[test]
    fn resolver_prefers_the_newer_binary() {
        let d = PathBuf::from("/tmp/bundled");
        let s = PathBuf::from("/tmp/system");
        // System newer
        let r = decide_cli(&d, Some("29.8.2"), Some(&s), Some("29.9.0"), "auto").unwrap();
        assert_eq!(r.source, CliSource::System);
        assert!(r.reason.contains("29.9.0"));
        // Bundled newer
        let r = decide_cli(&d, Some("29.9.1"), Some(&s), Some("29.9.0"), "auto").unwrap();
        assert_eq!(r.source, CliSource::Bundled);
        // Tie → System (we never shadow the user's identical toolchain)
        let r = decide_cli(&d, Some("29.9.0"), Some(&s), Some("29.9.0"), "auto").unwrap();
        assert_eq!(r.source, CliSource::System);
    }

    #[test]
    fn resolver_reports_why_it_chose() {
        let d = PathBuf::from("/tmp/bundled");
        let s = PathBuf::from("/tmp/system");
        let r = decide_cli(&d, Some("29.8.2"), Some(&s), Some("29.8.3"), "auto").unwrap();
        assert!(r.reason.to_lowercase().contains("system"));
        assert!(r.reason.contains("29.8.3"));
        assert!(r.reason.contains("29.8.2"));
    }

    #[test]
    fn resolver_errors_when_nothing_is_installed() {
        let d = PathBuf::from("/tmp/bundled");
        assert!(decide_cli(&d, None, None, None, "auto").is_err());
    }

    #[test]
    fn ensure_compose_layout_moves_the_legacy_plugin_into_docker_config() {
        let tmp = std::env::temp_dir().join(format!("dm_compose_test_{}", std::process::id()));
        let current = tmp.join("current");
        let legacy_dir = current.join("cli-plugins");
        std::fs::create_dir_all(&legacy_dir).expect("create legacy dir");
        std::fs::write(legacy_dir.join("docker-compose"), "# dummy").expect("write legacy compose");
        let canonical_dir = current.join("docker-config").join("cli-plugins");
        let canonical = canonical_dir.join("docker-compose");
        let _ = place_compose_plugin(&legacy_dir.join("docker-compose"), &canonical);
        assert!(canonical.is_file());
        // idempotent
        let _ = place_compose_plugin(&legacy_dir.join("docker-compose"), &canonical);
        assert!(canonical.is_file());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn compose_plugin_prefers_the_canonical_path_and_falls_back() {
        let tmp = std::env::temp_dir().join(format!("dm_cp_{}", std::process::id()));
        let p = EnginePaths {
            data_root: tmp.join("data"),
            versions_dir: tmp.join("versions"),
            current_dir: tmp.join("current"),
            log_file: tmp.join("dockerd.log"),
            pid_file: tmp.join("dockerd.pid"),
            socket: tmp.join("docker.sock"),
            runtime_dir: tmp.join("run"),
            exec_root: tmp.join("exec"),
            rootless_state: tmp.join("rootless"),
        };
        std::fs::create_dir_all(p.current_dir.join("cli-plugins")).expect("legacy dir");
        std::fs::write(p.current_dir.join("cli-plugins").join("docker-compose"), "# legacy")
            .expect("legacy compose");
        // Legacy only → fallback points at the legacy file.
        assert_eq!(p.compose_plugin(), p.current_dir.join("cli-plugins").join("docker-compose"));
        // Canonical exists → that one wins.
        let canonical = compose_plugin_dir(&p).join("docker-compose");
        std::fs::create_dir_all(canonical.parent().unwrap()).expect("canonical dir");
        std::fs::write(&canonical, "# canonical").expect("canonical compose");
        assert_eq!(p.compose_plugin(), canonical);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_compose_layout_is_a_noop_without_any_plugin() {
        let tmp = std::env::temp_dir().join(format!("dm_cl_{}", std::process::id()));
        let p = EnginePaths {
            data_root: tmp.join("data"),
            versions_dir: tmp.join("versions"),
            current_dir: tmp.join("current"),
            log_file: tmp.join("dockerd.log"),
            pid_file: tmp.join("dockerd.pid"),
            socket: tmp.join("docker.sock"),
            runtime_dir: tmp.join("run"),
            exec_root: tmp.join("exec"),
            rootless_state: tmp.join("rootless"),
        };
        std::fs::create_dir_all(&p.current_dir).expect("current dir");
        assert!(ensure_compose_layout(&p).expect("layout").is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_checksums_reads_the_matching_file() {
        let sums = "abcdef0123456789  docker-compose-linux-x86_64\nfedcba9876543210  checksums.txt\n";
        assert_eq!(
            parse_checksums(sums, "docker-compose-linux-x86_64").as_deref(),
            Some("abcdef0123456789")
        );
        assert!(parse_checksums(sums, "docker-buildx").is_none());
    }

    #[test]
    fn report_records_the_shim_marker() {
        // The generated script keeps `/bin/sh` first and puts the marker on
        // the second line, so detection must look at the first few lines
        // rather than at byte 0.
        let ours = format!("#!/bin/sh\n{SHIM_MARKER}\nexec /bin/true \"$@\"\n");
        let tmp = std::env::temp_dir().join(format!("dm_shim_a_{}", std::process::id()));
        std::fs::write(&tmp, &ours).expect("write shim");
        assert!(is_our_shim(&tmp));

        // A script that does not carry our marker is left alone.
        let theirs = std::env::temp_dir().join(format!("dm_shim_b_{}", std::process::id()));
        std::fs::write(&theirs, "#!/bin/sh\nexec docker \"$@\"\n").expect("write other");
        assert!(!is_our_shim(&theirs));

        // A mention buried past the marker window must not count as ours —
        // otherwise we would refuse to overwrite a file we never wrote.
        let buried = std::env::temp_dir().join(format!("dm_shim_c_{}", std::process::id()));
        std::fs::write(&buried, format!("#!/bin/sh\n# other\n# {SHIM_MARKER}\n")).expect("write");
        assert!(!is_our_shim(&buried));

        // A real ELF binary is not UTF-8 text; reading it must not match.
        let binary = std::env::temp_dir().join(format!("dm_shim_d_{}", std::process::id()));
        std::fs::write(&binary, [0u8, 159, 146, 150]).expect("write bytes");
        assert!(!is_our_shim(&binary));

        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(&theirs);
        let _ = std::fs::remove_file(&buried);
        let _ = std::fs::remove_file(&binary);
    }

    #[test]
    fn pre_rebrand_shim_is_still_ours() {
        // The rebrand changed the marker; an installed shim from an older
        // build must still be recognised so startup can rewrite it instead of
        // refusing to overwrite it as a foreign file.
        let tmp = std::env::temp_dir().join(format!("dm_shim_legacy_{}", std::process::id()));
        std::fs::write(&tmp, "#!/bin/sh\n# docker-manager shim\nexec /bin/true \"$@\"\n")
            .expect("write legacy shim");
        assert!(is_our_shim(&tmp));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn parse_shim_exec_reads_the_target() {
        let body = "#!/bin/sh\n# hsx2dock shim\nexec /usr/bin/docker \"$@\"\n";
        assert_eq!(parse_shim_exec(body), Some(PathBuf::from("/usr/bin/docker")));
        let quoted = "#!/bin/sh\n# hsx2dock shim\nexec \"/usr/bin/docker\" \"$@\"\n";
        assert_eq!(parse_shim_exec(quoted), Some(PathBuf::from("/usr/bin/docker")));
        let args = "#!/bin/sh\n# hsx2dock shim\nexec /usr/local/bin/docker $*\n";
        assert_eq!(
            parse_shim_exec(args),
            Some(PathBuf::from("/usr/local/bin/docker"))
        );
        // No exec line → nothing to report, so the caller cannot claim staleness.
        assert!(parse_shim_exec("#!/bin/sh\n# hsx2dock shim\n").is_none());
    }
}


