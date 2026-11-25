use std::path::{Path, PathBuf};
use std::sync::Arc;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::docker::engine::{
    download, normalize_version, step_plan, update_available, verify, InstallProgress,
};

pub const CADDY_VERSION: &str = "2.11.7";
pub const CADDY_BASE: &str = "https://github.com/caddyserver/caddy/releases/download";

/// Steps every Caddy install/update follows; the UI renders this checklist.
pub const CADDY_UPDATE_STEPS: &[&str] = &[
    "Download Caddy archive",
    "Verify checksum",
    "Extract binary",
    "Verify version",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaddyInstallStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub latest: String,
    pub path: Option<String>,
    pub running: bool,
    pub admin_url: String,
    pub http_port: u16,
    pub https_port: u16,
    pub trusted: bool,
    pub hosts_synced: bool,
    pub auto_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaddyUpdateInfo {
    pub installed_version: Option<String>,
    pub latest_version: String,
    pub update_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayRoute {
    pub id: String,
    pub domain: String,
    pub target: String, // e.g. "127.0.0.1:8081"
    pub path: Option<String>, // e.g. "/api" or "/web"
    pub strip_prefix: Option<bool>,
    pub container_name: Option<String>,
    pub group_name: Option<String>,
    pub enabled: bool,
    pub auto_detected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostsSyncStatus {
    pub managed_domains: Vec<String>,
    pub in_hosts_domains: Vec<String>,
    pub synced: bool,
    pub tampered: bool,
    pub managed_block_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostsEntryItem {
    pub id: String,
    pub ip: String,
    pub hosts: Vec<String>,
    pub comment: Option<String>,
    pub is_enabled: bool,
    pub is_managed_by_app: bool,
    pub is_syntax_valid: bool,
    pub syntax_error: Option<String>,
    pub raw_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostsFileModel {
    pub entries: Vec<HostsEntryItem>,
    pub raw_content: String,
    pub can_write_direct: bool,
    pub managed_block_present: bool,
    pub tampered: bool,
    pub total_lines: usize,
}

pub struct CaddyPaths {
    pub root: PathBuf,
}

impl CaddyPaths {
    pub fn for_app() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let data = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
        let base = data.join(crate::migrate::APP_DIR);
        Self {
            root: base.join("versions").join("caddy"),
        }
    }

    pub fn active_version(&self) -> String {
        std::fs::read_to_string(self.root.join("current"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| CADDY_VERSION.to_string())
    }

    pub fn version_dir(&self, version: &str) -> PathBuf {
        self.root.join(version)
    }

    pub fn binary(&self) -> PathBuf {
        self.version_dir(&self.active_version()).join(if cfg!(windows) { "caddy.exe" } else { "caddy" })
    }

    pub fn is_installed(&self) -> bool {
        self.binary().is_file()
    }

    pub fn set_active(&self, version: &str) -> Result<()> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("create {}", self.root.display()))?;
        let tmp = self.root.join(".current.tmp");
        std::fs::write(&tmp, format!("{version}\n"))?;
        std::fs::rename(&tmp, self.root.join("current"))?;
        Ok(())
    }
}

/// Release asset file name for Caddy on this platform
pub fn caddy_asset_name(version: &str, os: &str, arch: &str) -> Option<String> {
    let a = match arch {
        "x86_64" | "amd64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        "arm" => "armv7",
        _ => return None,
    };
    match os {
        "linux" => Some(format!("caddy_{version}_linux_{a}.tar.gz")),
        "macos" | "darwin" => Some(format!("caddy_{version}_mac_{a}.tar.gz")),
        "windows" => Some(format!("caddy_{version}_windows_{a}.zip")),
        _ => None,
    }
}

pub fn read_caddy_status(paths: &CaddyPaths) -> (bool, Option<String>) {
    let installed = paths.is_installed();
    let version = if installed {
        std::fs::read_to_string(paths.version_dir(&paths.active_version()).join("VERSION"))
            .ok()
            .map(|s| s.trim().to_string())
            .or_else(|| probe_caddy_version(&paths.binary()))
            .or_else(|| Some(paths.active_version()))
    } else {
        None
    };
    (installed, version)
}

pub fn probe_caddy_version(bin: &Path) -> Option<String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("version");
    let out = crate::procutil::output_with_timeout(cmd, crate::procutil::PROBE, "caddy version").ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // e.g. "v2.11.7 h1:..." -> "2.11.7"
    let token = stdout.split_whitespace().next()?;
    Some(token.trim_start_matches('v').to_string())
}

/// Check available Caddy updates against the GitHub releases API.
pub async fn check_caddy_update(paths: &CaddyPaths) -> Result<CaddyUpdateInfo> {
    let (_, installed) = read_caddy_status(paths);
    let latest = fetch_latest_caddy_release().await.unwrap_or_else(|_| CADDY_VERSION.to_string());
    let update_available = update_available(&latest, installed.as_deref());
    Ok(CaddyUpdateInfo {
        installed_version: installed,
        latest_version: latest,
        update_available,
    })
}

async fn fetch_latest_caddy_release() -> Result<String> {
    let client = reqwest::Client::builder()
        .user_agent("hsx2dock-caddy-updater")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let resp = client
        .get("https://api.github.com/repos/caddyserver/caddy/releases/latest")
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!("GitHub API returned {}", resp.status());
    }
    let text = resp.text().await?;
    let body: serde_json::Value = serde_json::from_str(&text)?;
    let tag = body["tag_name"]
        .as_str()
        .context("tag_name not in release json")?;
    Ok(normalize_version(tag).to_string())
}

/// Download, verify checksum, extract and install Caddy version.
pub async fn install_caddy_version<F>(
    paths: &CaddyPaths,
    version: &str,
    mut on_progress: F,
) -> Result<PathBuf>
where
    F: FnMut(InstallProgress) + Send,
{
    let version = normalize_version(version).to_string();
    if version.is_empty() {
        bail!("no caddy version specified");
    }
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let Some(asset) = caddy_asset_name(&version, os, arch) else {
        bail!("caddy is not published for {os}-{arch}");
    };

    let plan = step_plan(CADDY_UPDATE_STEPS);
    let version_dir = paths.version_dir(&version);
    std::fs::create_dir_all(&version_dir)
        .with_context(|| format!("create {}", version_dir.display()))?;
    let staging_archive = version_dir.join(format!(".{asset}.partial"));
    let _ = std::fs::remove_file(&staging_archive);
    let url = format!("{CADDY_BASE}/v{version}/{asset}");

    on_progress(InstallProgress::stage(
        "caddy",
        &version,
        &plan,
        0,
        Some(0.05),
    ));

    let mut last = 0u64;
    download(&url, &staging_archive, |done, total| {
        if done.saturating_sub(last) < 256 * 1024 && Some(done) != total {
            return;
        }
        last = done;
        on_progress(InstallProgress::tick(
            "caddy",
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
    .with_context(|| format!("download caddy v{version}"))?;

    on_progress(InstallProgress::stage(
        "caddy",
        &version,
        &plan,
        1,
        Some(0.6),
    ));

    let expected = fetch_caddy_checksum(&version, &asset).await?;
    if let Err(e) = verify(&staging_archive, &expected) {
        let _ = std::fs::remove_file(&staging_archive);
        return Err(e).context("caddy checksum verification failed");
    }
    on_progress(InstallProgress::note(
        "caddy",
        &version,
        &plan,
        1,
        Some(0.65),
        format!("sha256 {expected}"),
    ));

    on_progress(InstallProgress::stage(
        "caddy",
        &version,
        &plan,
        2,
        Some(0.75),
    ));

    let dest = version_dir.join(if cfg!(windows) { "caddy.exe" } else { "caddy" });
    // Extract next to the destination and rename into place only once the
    // binary checks out. Deleting `dest` first meant a failed reinstall of the
    // active version left the gateway with no binary at all.
    let staged = version_dir.join(if cfg!(windows) { ".caddy.exe.partial" } else { ".caddy.partial" });
    let extract = {
        let archive_path = staging_archive.clone();
        let staged = staged.clone();
        tokio::task::spawn_blocking(move || extract_caddy_binary(&archive_path, &staged))
    };
    let extracted = extract
        .await
        .map_err(|e| anyhow::anyhow!("extracting caddy failed: {e}"))
        .and_then(|r| r);
    let _ = std::fs::remove_file(&staging_archive);
    if let Err(e) = extracted {
        let _ = std::fs::remove_file(&staged);
        return Err(e);
    }

    on_progress(InstallProgress::stage(
        "caddy",
        &version,
        &plan,
        3,
        Some(0.9),
    ));

    match probe_caddy_version(&staged) {
        Some(r) if r.starts_with(&version) => {}
        Some(r) => {
            let _ = std::fs::remove_file(&staged);
            bail!("staged caddy reported {r}, but {version} was expected");
        }
        None => {
            let _ = std::fs::remove_file(&staged);
            bail!("staged caddy {version} failed `caddy version`");
        }
    }
    std::fs::rename(&staged, &dest)
        .with_context(|| format!("move caddy into {}", dest.display()))?;

    let _ = std::fs::write(version_dir.join("VERSION"), format!("{version}\n"));
    paths.set_active(&version)?;

    on_progress(InstallProgress::finished("caddy", &version, &plan));
    Ok(dest)
}

/// Pulls the `caddy` binary out of a release tarball into `dest` (0755).
fn extract_caddy_binary(archive_path: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    let _ = std::fs::remove_file(dest);
    let file = std::fs::File::open(archive_path)
        .with_context(|| format!("open {}", archive_path.display()))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.into_owned();
        let fname = name.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if fname == "caddy" || fname == "caddy.exe" {
            let mut out = std::fs::File::create(dest)
                .with_context(|| format!("create {}", dest.display()))?;
            std::io::copy(&mut entry, &mut out)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
                    .context("chmod caddy binary")?;
            }
            return Ok(());
        }
    }
    bail!("caddy binary not found inside downloaded archive")
}

async fn fetch_caddy_checksum(version: &str, asset: &str) -> Result<String> {
    let url = format!("{CADDY_BASE}/v{version}/caddy_{version}_checksums.txt");
    let text = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1].trim_start_matches('*').ends_with(asset) {
            return Ok(parts[0].trim().to_lowercase());
        }
    }
    bail!("checksum for {asset} not found in {url}")
}

pub struct CaddySupervisor {
    pub child: Arc<Mutex<Option<tokio::process::Child>>>,
    pub paths: CaddyPaths,
    /// Whether the plain-HTTP listener is served, decided once per HTTP port.
    /// It cannot be re-probed on reload: the running Caddy holds the port, so
    /// the probe failed and every reload silently dropped `:80`.
    http_listener: std::sync::Mutex<Option<(u16, bool)>>,
    /// Serialises start/stop/reload. Two quick route saves could post their
    /// `/load`s out of order (the older config winning), and a reload racing
    /// a start could hit the admin API of a Caddy that was being replaced.
    ops: Mutex<()>,
}

/// `@id` stamped on the gateway server so the app only adopts a Caddy it
/// configured itself, never a system Caddy that happens to use :2019.
const GATEWAY_ID: &str = "hsx2dock_gateway";

impl CaddySupervisor {
    pub fn new() -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            paths: CaddyPaths::for_app(),
            http_listener: std::sync::Mutex::new(None),
            ops: Mutex::new(()),
        }
    }

    /// Serve `http_port` only when Caddy can bind it. Probed when the port is
    /// first used (before our Caddy binds it) and remembered for later reloads.
    fn bind_http(&self, bin: &Path, http_port: u16, owns_port: bool) -> bool {
        let mut guard = self.http_listener.lock().unwrap_or_else(|e| e.into_inner());
        match *guard {
            Some((port, bind)) if port == http_port && owns_port => bind,
            _ => {
                let bind = http_port_bindable(bin, http_port);
                *guard = Some((http_port, bind));
                bind
            }
        }
    }

    pub async fn is_running(&self) -> bool {
        let mut guard = self.child.lock().await;
        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(None) => return true,
                _ => {
                    *guard = None;
                }
            }
        }
        // A Caddy this app configured in an earlier session (e.g. after a
        // crash) is adopted; any other Caddy on :2019 is not ours.
        is_gateway_live().await
    }

    pub async fn start(
        &self,
        http_port: u16,
        https_port: u16,
        routes: &[GatewayRoute],
    ) -> Result<()> {
        let _op = self.ops.lock().await;
        if self.is_running().await {
            // Apply updated configuration via API
            return self.load_config(http_port, https_port, routes).await;
        }

        let bin = self.paths.binary();
        if !bin.is_file() {
            bail!("Caddy binary not installed yet at {}", bin.display());
        }

        // The default ports (80/443) are privileged: a rootless caddy cannot
        // bind them without CAP_NET_BIND_SERVICE and dies with "bind:
        // permission denied" before the admin API ever comes up. Grant the
        // capability to the binary once (one pkexec prompt); a declined grant
        // surfaces as caddy's own bind error below.
        if (http_port < 1024 || https_port < 1024) && !has_net_bind_service(&bin) {
            grant_net_bind_service(&bin).with_context(|| {
                format!(
                    "ports {http_port}/{https_port} are privileged (<1024) — accept the \
                     capability prompt, or pick ports above 1023 in the Gateway settings"
                )
            })?;
        }

        if is_admin_api_live().await {
            bail!("Another program is already using the Caddy admin port 127.0.0.1:2019; stop it before starting the gateway");
        }

        let bind_http = self.bind_http(&bin, http_port, false);
        let initial_json = generate_caddy_json(http_port, https_port, routes, bind_http)?;

        let mut cmd = Command::new(&bin);
        cmd.args(["run", "--config", "-", "--adapter", ""]);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        let mut child = cmd.spawn().with_context(|| format!("Failed to spawn Caddy from {}", bin.display()))?;

        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(initial_json.as_bytes()).await;
            let _ = stdin.flush().await;
        }

        // Wait up to 3s for admin API to become ready
        let mut ready = false;
        for _ in 0..30 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if is_gateway_live().await {
                ready = true;
                break;
            }
        }

        if !ready {
            let _ = child.kill().await;
            let output = child.wait_with_output().await;
            let stderr_msg = output.map(|o| String::from_utf8_lossy(&o.stderr).to_string()).unwrap_or_default();
            bail!("Caddy process failed to start or open admin API on 127.0.0.1:2019. Details: {stderr_msg}");
        }

        // stderr stays piped for the failure message above, but nothing read
        // it afterwards: once Caddy's TLS/upstream logging filled the ~64KB
        // pipe buffer, its next write blocked and the gateway stopped serving.
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "caddy", "{line}");
                }
            });
        }

        let mut guard = self.child.lock().await;
        *guard = Some(child);
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let _op = self.ops.lock().await;
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.kill().await;
        }

        // Try graceful stop via admin API
        if let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
        {
            let _ = client.post("http://127.0.0.1:2019/stop").send().await;
        }

        Ok(())
    }

    pub async fn apply_routes(
        &self,
        http_port: u16,
        https_port: u16,
        routes: &[GatewayRoute],
    ) -> Result<()> {
        let _op = self.ops.lock().await;
        self.load_config(http_port, https_port, routes).await
    }

    async fn load_config(
        &self,
        http_port: u16,
        https_port: u16,
        routes: &[GatewayRoute],
    ) -> Result<()> {
        let bin = self.paths.binary();
        let bind_http = self.bind_http(&bin, http_port, true);
        let json_body = generate_caddy_json(http_port, https_port, routes, bind_http)?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?;

        let resp = client
            .post("http://127.0.0.1:2019/load")
            .header("Content-Type", "application/json")
            .body(json_body)
            .send()
            .await
            .context("Failed to communicate with Caddy Admin API at 127.0.0.1:2019")?;

        if !resp.status().is_success() {
            let err_txt = resp.text().await.unwrap_or_default();
            bail!("Caddy rejected config load: {err_txt}");
        }

        Ok(())
    }

    pub async fn trust_root_ca(&self) -> Result<()> {
        let bin = self.paths.binary();
        if !bin.is_file() {
            bail!("Caddy is not installed");
        }
        // `caddy trust` escalates through sudo to write the system store. With
        // no terminal, sudo waited on a password prompt nobody could answer
        // and the call never returned. stdin is closed so sudo fails fast
        // (pkexec/polkit dialogs still work), and the whole run is bounded.
        let mut cmd = Command::new(&bin);
        cmd.arg("trust")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(120), cmd.output())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "caddy trust did not finish within 2 minutes (it may be waiting for an \
                     administrator password); run `caddy trust` in a terminal instead"
                )
            })?
            .context("Failed to run 'caddy trust'")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("caddy trust failed: {stderr}");
        }
        Ok(())
    }
}

pub async fn is_admin_api_live() -> bool {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build();
    if let Ok(c) = client {
        c.get("http://127.0.0.1:2019/config/").send().await.map_or(false, |r| r.status().is_success())
    } else {
        false
    }
}

/// True when the Caddy on :2019 carries this app's gateway config.
pub async fn is_gateway_live() -> bool {
    let Ok(c) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
    else {
        return false;
    };
    c.get(format!("http://127.0.0.1:2019/id/{GATEWAY_ID}"))
        .send()
        .await
        .map_or(false, |r| r.status().is_success())
}

pub fn is_port_available(port: u16) -> bool {
    use std::net::TcpListener;
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Whether the binary may bind privileged ports (<1024).
fn has_net_bind_service(bin: &Path) -> bool {
    std::process::Command::new("getcap")
        .arg(bin)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("cap_net_bind_service"))
        .unwrap_or(false)
}

/// Grant `cap_net_bind_service` to the binary via `pkexec` (polkit prompts for
/// the password). Idempotent — a re-grant on an already-capable binary is a
/// no-op, and a caddy update replaces the binary, dropping the capability,
/// which the next start re-grants.
fn grant_net_bind_service(bin: &Path) -> Result<()> {
    let status = std::process::Command::new("pkexec")
        .args(["setcap", "cap_net_bind_service=+ep"])
        .arg(bin)
        .status()
        .context("invoke pkexec setcap")?;
    if !status.success() {
        bail!("pkexec setcap exited with {:?}", status.code());
    }
    Ok(())
}

/// Whether caddy can actually bind the HTTP port.
///
/// A plain `TcpListener::bind` probe from *this* process cannot answer for
/// privileged ports (<1024): the app itself has no `CAP_NET_BIND_SERVICE`, so
/// the probe is refused even when the caddy binary — which carries the
/// capability — binds the port fine. Only that refusal is excused by the
/// capability: a port that is already taken stays unbindable. Treating every
/// failure on a privileged port as "permission" made Caddy try `:80` while
/// another listener held it, and the whole gateway failed to start.
fn http_port_bindable(bin: &Path, port: u16) -> bool {
    // The kernel checks the privilege before the address, so an unprivileged
    // probe gets "permission denied" for a privileged port whether or not it
    // is taken. Ask the socket table instead in that case.
    bind_probe_allows(probe_bind(port), port < 1024 && has_net_bind_service(bin))
        && !port_has_listener(port)
}

/// True when some socket is listening on TCP `port` (any address), read from
/// `/proc/net/tcp{,6}`. Elsewhere, or when the tables are unreadable, a
/// connect to loopback stands in.
fn port_has_listener(port: u16) -> bool {
    let mut read_any = false;
    let mut found = false;
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(table) {
            read_any = true;
            found |= proc_net_tcp_listens_on(&text, port);
        }
    }
    if read_any {
        return found;
    }
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(300),
    )
    .is_ok()
}

/// Parses one `/proc/net/tcp`-style table: `local_address` is `HEXIP:HEXPORT`
/// and state `0A` is LISTEN.
fn proc_net_tcp_listens_on(text: &str, port: u16) -> bool {
    text.lines().skip(1).any(|line| {
        let mut cols = line.split_whitespace();
        let local = cols.nth(1);
        let state = cols.nth(1);
        state == Some("0A")
            && local
                .and_then(|l| l.rsplit_once(':'))
                .and_then(|(_, p)| u16::from_str_radix(p, 16).ok())
                == Some(port)
    })
}

/// Bind the wildcard address, as Caddy's `:<port>` listener does.
fn probe_bind(port: u16) -> std::io::Result<()> {
    std::net::TcpListener::bind(("0.0.0.0", port)).map(drop)
}

fn bind_probe_allows(probe: std::io::Result<()>, caddy_has_capability: bool) -> bool {
    match probe {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => caddy_has_capability,
        Err(_) => false,
    }
}

/// Builds dynamic Caddy JSON configuration
pub fn generate_caddy_json(
    http_port: u16,
    https_port: u16,
    routes: &[GatewayRoute],
    can_bind_http: bool,
) -> Result<String> {
    let mut server_routes = Vec::new();
    let mut tls_subjects = Vec::new();

    // Routes are terminal, so the first match wins: the most specific path of
    // a host must come first, or `app.test` (no path) would swallow
    // `app.test/api` whenever it was listed earlier.
    let mut ordered: Vec<&GatewayRoute> = routes.iter().collect();
    ordered.sort_by(|a, b| {
        let (ha, pa) = route_host_path(a);
        let (hb, pb) = route_host_path(b);
        ha.cmp(&hb)
            .then_with(|| pb.as_deref().map_or(0, str::len).cmp(&pa.as_deref().map_or(0, str::len)))
    });

    for r in ordered {
        if !r.enabled {
            continue;
        }
        let clean_domain = r.domain.trim().trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/');
        let (host, domain_path) = match clean_domain.split_once('/') {
            Some((h, p)) => (h.trim(), Some(format!("/{}", p.trim()))),
            None => (clean_domain, None),
        };

        if host.is_empty() {
            continue;
        }

        tls_subjects.push(host.to_string());

        let effective_path = match (&r.path, domain_path) {
            (Some(p), _) if !p.trim().is_empty() => {
                let trimmed = p.trim().trim_end_matches('/');
                let with_leading = if trimmed.starts_with('/') { trimmed.to_string() } else { format!("/{trimmed}") };
                Some(with_leading)
            }
            (_, Some(dp)) => Some(dp),
            _ => None,
        };

        let mut match_rules = serde_json::json!({
            "host": [host]
        });

        if let Some(ref p) = effective_path {
            // `/api*` would also match `/apiv2`; match the segment exactly.
            match_rules.as_object_mut().unwrap().insert(
                "path".to_string(),
                serde_json::json!([p, format!("{p}/*")]),
            );
        }

        let clean_target = r.target.trim().trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/');

        let mut subroute_handlers = Vec::new();
        if r.strip_prefix.unwrap_or(false) {
            if let Some(ref p) = effective_path {
                subroute_handlers.push(serde_json::json!({
                    "handler": "rewrite",
                    "strip_path_prefix": p
                }));
            }
        }

        subroute_handlers.push(serde_json::json!({
            "handler": "reverse_proxy",
            "upstreams": [
                {
                    "dial": clean_target
                }
            ]
        }));

        server_routes.push(serde_json::json!({
            "match": [match_rules],
            "handle": [
                {
                    "handler": "subroute",
                    "routes": [
                        {
                            "handle": subroute_handlers
                        }
                    ]
                }
            ],
            "terminal": true
        }));
    }

    tls_subjects.sort();
    tls_subjects.dedup();

    let tls_policy = if tls_subjects.is_empty() {
        serde_json::json!([])
    } else {
        serde_json::json!([
            {
                "subjects": tls_subjects,
                "issuers": [
                    {
                        "module": "internal"
                    }
                ]
            }
        ])
    };

    let mut listen_addrs = Vec::new();
    if can_bind_http {
        listen_addrs.push(format!(":{http_port}"));
    }
    listen_addrs.push(format!(":{https_port}"));

    let mut gateway_server = serde_json::json!({
        "@id": GATEWAY_ID,
        "listen": listen_addrs,
        "routes": server_routes,
        // A stopped container used to answer with an empty 502 that looked
        // like a gateway fault. Say which host failed and what to check.
        "errors": {
            "routes": [{
                "handle": [{
                    "handler": "static_response",
                    "status_code": "{http.error.status_code}",
                    "headers": { "Content-Type": ["text/html; charset=utf-8"] },
                    "body": GATEWAY_ERROR_PAGE
                }]
            }]
        }
    });

    if !can_bind_http {
        gateway_server.as_object_mut().unwrap().insert(
            "automatic_https".to_string(),
            serde_json::json!({
                "disable_redirects": true
            }),
        );
    }

    let config = serde_json::json!({
        "admin": {
            "listen": "127.0.0.1:2019"
        },
        "apps": {
            "http": {
                "http_port": http_port,
                "https_port": https_port,
                "servers": {
                    "gateway": gateway_server
                }
            },
            "tls": {
                "automation": {
                    "policies": tls_policy
                }
            }
        }
    });

    Ok(serde_json::to_string(&config)?)
}

/// Served by the gateway when a route's upstream fails (most often: the
/// container is stopped or no longer publishes the target port).
const GATEWAY_ERROR_PAGE: &str = "<!doctype html><meta charset=utf-8><title>{http.error.status_code} \u{2014} {http.request.host}</title>\
<body style=\"font:15px system-ui;background:#18181b;color:#e4e4e7;display:grid;place-items:center;height:100vh;margin:0\">\
<div style=\"max-width:560px\"><h2 style=\"margin:0 0 8px\">{http.error.status_code} \u{00b7} {http.request.host}</h2>\
<p>The HSX2Dock gateway is running, but the upstream for this route did not answer.</p>\
<p style=\"color:#a1a1aa\">Check that the container is running and still publishes the target port \
(Gateway page \u{2192} Upstream Target).</p></div></body>";

/// One route's upstream reachability, for the Gateway page.
#[derive(Debug, Clone, Serialize)]
pub struct UpstreamProbe {
    pub id: String,
    pub reachable: bool,
}

/// TCP-connects every enabled route's target concurrently (short timeout), so
/// the UI can flag a route that would answer 502 before anyone opens it.
pub async fn probe_upstreams(routes: &[GatewayRoute]) -> Vec<UpstreamProbe> {
    let probes = routes.iter().filter(|r| r.enabled).map(|r| async move {
        let target = r
            .target
            .trim()
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/')
            .to_string();
        let reachable = matches!(
            tokio::time::timeout(
                std::time::Duration::from_millis(600),
                tokio::net::TcpStream::connect(target.as_str()),
            )
            .await,
            Ok(Ok(_))
        );
        UpstreamProbe { id: r.id.clone(), reachable }
    });
    futures_util::future::join_all(probes).await
}

/// Host and normalised path of a route, as the matcher sees them.
fn route_host_path(r: &GatewayRoute) -> (String, Option<String>) {
    let clean = r
        .domain
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let (host, domain_path) = match clean.split_once('/') {
        Some((h, p)) => (h.trim().to_string(), Some(format!("/{}", p.trim()))),
        None => (clean.to_string(), None),
    };
    let path = match (&r.path, domain_path) {
        (Some(p), _) if !p.trim().is_empty() => {
            let t = p.trim().trim_end_matches('/');
            Some(if t.starts_with('/') { t.to_string() } else { format!("/{t}") })
        }
        (_, dp) => dp,
    };
    (host.to_lowercase(), path.filter(|p| p != "/"))
}

fn valid_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

/// Rejects routes Caddy or the hosts file cannot represent, before they are
/// saved: a host with spaces became two hostnames in /etc/hosts, a target
/// without a port never dialled, and duplicates shadowed each other.
pub fn validate_routes(routes: &[GatewayRoute]) -> Result<()> {
    let mut ids = std::collections::HashSet::new();
    let mut keys = std::collections::HashSet::new();
    for r in routes {
        if r.id.trim().is_empty() || !ids.insert(r.id.trim().to_string()) {
            bail!("Duplicate or empty route id '{}'", r.id);
        }
        let (host, path) = route_host_path(r);
        if !valid_hostname(&host) {
            bail!("'{}' is not a valid domain (letters, digits, '-' and '.' only)", r.domain.trim());
        }
        if let Some(p) = &path {
            if p.chars().any(|c| c.is_whitespace() || c == '*' || c == '?' || c == '#') {
                bail!("Path '{p}' of {host} contains characters a path prefix cannot have");
            }
        }
        let target = r
            .target
            .trim()
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .trim_end_matches('/');
        let port_ok = target
            .rsplit_once(':')
            .map(|(h, p)| !h.is_empty() && p.parse::<u16>().map_or(false, |n| n > 0))
            .unwrap_or(false);
        if !port_ok || target.contains('/') || target.chars().any(char::is_whitespace) {
            bail!("Target '{}' of {host} must be host:port, e.g. 127.0.0.1:8080", r.target.trim());
        }
        if r.enabled && !keys.insert((host.clone(), path.clone())) {
            bail!(
                "Two enabled routes use {host}{}; only the first would ever match",
                path.as_deref().unwrap_or("")
            );
        }
    }
    Ok(())
}

// ---------------- /etc/hosts helpers ----------------

const HOSTS_MARKER_BEGIN: &str = "# --- BEGIN HSX2DOCK GATEWAY MANAGED ---";
const HOSTS_MARKER_END: &str = "# --- END HSX2DOCK GATEWAY MANAGED ---";

pub fn inspect_hosts_sync(routes: &[GatewayRoute]) -> HostsSyncStatus {
    let mut custom_domains: Vec<String> = routes
        .iter()
        .filter(|r| r.enabled)
        .map(|r| {
            let d = r.domain.trim().trim_start_matches("http://").trim_start_matches("https://");
            d.split('/').next().unwrap_or("").to_string()
        })
        .filter(|d| !d.is_empty() && !d.ends_with(".localhost") && d != "localhost")
        .collect();

    custom_domains.sort();
    custom_domains.dedup();

    let hosts_content = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
    let mut in_hosts = Vec::new();
    let mut managed_block_present = false;
    let mut block_domains = Vec::new();
    let mut inside_block = false;

    for line in hosts_content.lines() {
        let trimmed = line.trim();
        if trimmed == HOSTS_MARKER_BEGIN {
            managed_block_present = true;
            inside_block = true;
            continue;
        }
        if trimmed == HOSTS_MARKER_END {
            inside_block = false;
            continue;
        }
        if inside_block && !trimmed.starts_with('#') && !trimmed.is_empty() {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() > 1 {
                for h in &parts[1..] {
                    block_domains.push(h.to_string());
                }
            }
        }
    }
    block_domains.sort();
    block_domains.dedup();

    for d in &custom_domains {
        let pattern = format!("{d}");
        if hosts_content.lines().any(|line| {
            let l = line.trim();
            !l.starts_with('#') && l.split_whitespace().skip(1).any(|host| host == pattern)
        }) {
            in_hosts.push(d.clone());
        }
    }

    let synced = custom_domains.iter().all(|d| in_hosts.contains(d));
    let tampered = if !custom_domains.is_empty() {
        !managed_block_present || custom_domains != block_domains
    } else {
        managed_block_present // should be empty but block remains
    };

    HostsSyncStatus {
        managed_domains: custom_domains,
        in_hosts_domains: in_hosts,
        synced,
        tampered,
        managed_block_present,
    }
}

pub fn parse_hosts_file(routes: &[GatewayRoute]) -> HostsFileModel {
    let raw_content = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
    let sync_status = inspect_hosts_sync(routes);

    let mut entries = Vec::new();
    let mut inside_block = false;
    let mut line_num = 0;

    for line in raw_content.lines() {
        line_num += 1;
        let trimmed = line.trim();

        if trimmed == HOSTS_MARKER_BEGIN {
            inside_block = true;
            continue;
        }
        if trimmed == HOSTS_MARKER_END {
            inside_block = false;
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        // Bỏ qua dòng comment thuần túy (không phải dòng cấu hình IP hostname hợp lệ)
        if trimmed.starts_with('#') {
            // Kiểm tra xem đây có phải là một dòng cấu hình bị comment-out (vd: # 127.0.0.1 example.com) hay không
            let unmarshalled = trimmed.trim_start_matches('#').trim();
            let (addr_part, _) = match unmarshalled.split_once('#') {
                Some((ah, _)) => (ah.trim(), ()),
                None => (unmarshalled, ()),
            };
            let parts: Vec<&str> = addr_part.split_whitespace().collect();
            // Nếu không có ít nhất 2 phần tử (IP và ít nhất 1 hostname) hoặc phần tử đầu không phải IP,
            // thì đây là dòng ghi chú thuần túy (như # Standard host addresses, # This host address, etc.) -> BỎ QUA HOÀN TOÀN
            if parts.len() < 2 || parts[0].parse::<std::net::IpAddr>().is_err() {
                continue;
            }
        }

        let is_comment = trimmed.starts_with('#');
        let effective = if is_comment {
            trimmed.trim_start_matches('#').trim()
        } else {
            trimmed
        };

        let (addr_and_hosts, inline_comment) = match effective.split_once('#') {
            Some((ah, c)) => (ah.trim(), Some(c.trim().to_string())),
            None => (effective, None),
        };

        let parts: Vec<&str> = addr_and_hosts.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        let ip_candidate = parts[0];
        let hosts: Vec<String> = parts.iter().skip(1).map(|s| s.to_string()).collect();

        let mut is_syntax_valid = true;
        let mut syntax_error = None;

        if ip_candidate.parse::<std::net::IpAddr>().is_err() {
            is_syntax_valid = false;
            syntax_error = Some(format!("Invalid IP address format: '{ip_candidate}'"));
        } else if hosts.is_empty() {
            is_syntax_valid = false;
            syntax_error = Some("Missing hostname for IP address".to_string());
        }

        entries.push(HostsEntryItem {
            id: format!("line-{line_num}"),
            ip: ip_candidate.to_string(),
            hosts,
            comment: inline_comment,
            is_enabled: !is_comment,
            is_managed_by_app: inside_block,
            is_syntax_valid,
            syntax_error,
            raw_line: line.to_string(),
        });
    }

    let can_write_direct = std::fs::OpenOptions::new().write(true).open("/etc/hosts").is_ok();

    HostsFileModel {
        entries,
        raw_content: raw_content.clone(),
        can_write_direct,
        managed_block_present: sync_status.managed_block_present,
        tampered: sync_status.tampered,
        total_lines: raw_content.lines().count(),
    }
}

pub fn save_raw_hosts_content(new_content: &str) -> Result<()> {
    // Validate syntax line-by-line first to prevent bricking /etc/hosts
    for (idx, line) in new_content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let code_part = trimmed.split('#').next().unwrap_or("").trim();
        let parts: Vec<&str> = code_part.split_whitespace().collect();
        if !parts.is_empty() {
            let ip = parts[0];
            if ip.parse::<std::net::IpAddr>().is_err() {
                bail!("Line {}: invalid IP address '{}'", idx + 1, ip);
            }
            if parts.len() < 2 {
                bail!("Line {}: IP '{}' has no hostname defined", idx + 1, ip);
            }
        }
    }

    let original = std::fs::read_to_string("/etc/hosts").unwrap_or_default();
    write_hosts(new_content, &original)
}

/// The hosts file without the app's managed block.
///
/// Only a complete BEGIN…END pair is removed. A BEGIN whose END the user
/// deleted used to swallow every following line — the user's own entries —
/// on the next save; now only the stray marker line goes.
fn without_managed_block(original: &str) -> Vec<String> {
    let lines: Vec<&str> = original.lines().collect();
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == HOSTS_MARKER_BEGIN {
            if let Some(end) = lines[i + 1..]
                .iter()
                .position(|l| l.trim() == HOSTS_MARKER_END)
            {
                i += end + 2;
                continue;
            }
            i += 1; // stray BEGIN: drop the marker, keep what follows
            continue;
        }
        if lines[i].trim() == HOSTS_MARKER_END {
            i += 1; // stray END
            continue;
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    out
}

/// Writes /etc/hosts, skipping the write (and the password prompt) when the
/// content is unchanged. Elevation pipes the content to `pkexec tee` instead
/// of copying a predictably named temp file another user could swap.
fn write_hosts(new_content: &str, original: &str) -> Result<()> {
    if new_content == original {
        return Ok(());
    }
    if std::fs::write("/etc/hosts", new_content).is_ok() {
        return Ok(());
    }
    use std::io::Write;
    let mut child = std::process::Command::new("pkexec")
        .args(["tee", "/etc/hosts"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to invoke pkexec")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(new_content.as_bytes())
            .context("Failed to pass the hosts content to pkexec")?;
    }
    let out = child.wait_with_output().context("pkexec did not finish")?;
    match out.status.code() {
        Some(0) => Ok(()),
        // 126: the user dismissed the authentication dialog.
        Some(126) => bail!("Updating /etc/hosts was cancelled at the password prompt"),
        code => bail!(
            "Failed to update /etc/hosts (pkexec exit code {code:?}): {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

pub fn clear_app_hosts_entries() -> Result<()> {
    let original = std::fs::read_to_string("/etc/hosts").context("Cannot read /etc/hosts")?;
    let cleaned = without_managed_block(&original).join("\n") + "\n";
    write_hosts(&cleaned, &original)
}

pub fn update_hosts_file(routes: &[GatewayRoute]) -> Result<()> {
    let status = inspect_hosts_sync(routes);
    let original = std::fs::read_to_string("/etc/hosts").context("Cannot read /etc/hosts")?;
    let new_content = hosts_with_managed_block(&original, &status.managed_domains);
    write_hosts(&new_content, &original)
}

/// `original` with the managed block replaced by one for `domains`.
fn hosts_with_managed_block(original: &str, domains: &[String]) -> String {
    let mut lines = without_managed_block(original);
    if !domains.is_empty() {
        lines.push(HOSTS_MARKER_BEGIN.to_string());
        for domain in domains {
            lines.push(format!("127.0.0.1\t{domain}"));
        }
        lines.push(HOSTS_MARKER_END.to_string());
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(id: &str, domain: &str, path: Option<&str>, target: &str) -> GatewayRoute {
        GatewayRoute {
            id: id.into(),
            domain: domain.into(),
            target: target.into(),
            path: path.map(Into::into),
            strip_prefix: None,
            container_name: None,
            group_name: None,
            enabled: true,
            auto_detected: false,
        }
    }

    #[test]
    fn a_missing_end_marker_keeps_the_users_lines() {
        let hosts = format!("127.0.0.1 localhost\n{HOSTS_MARKER_BEGIN}\n127.0.0.1\told.test\n10.0.0.5 nas.lan\n");
        let out = without_managed_block(&hosts);
        assert!(out.iter().any(|l| l == "10.0.0.5 nas.lan"));
        assert!(!out.iter().any(|l| l.trim() == HOSTS_MARKER_BEGIN));
    }

    #[test]
    fn a_complete_block_is_replaced_and_unchanged_content_is_stable() {
        let domains = vec!["app.test".to_string()];
        let first = hosts_with_managed_block("127.0.0.1 localhost\n", &domains);
        assert_eq!(hosts_with_managed_block(&first, &domains), first);
        assert!(first.contains("127.0.0.1\tapp.test"));
    }

    #[test]
    fn specific_paths_come_first_and_match_whole_segments() {
        let routes = vec![
            route("1", "app.test", None, "127.0.0.1:3000"),
            route("2", "app.test", Some("/api"), "127.0.0.1:4000"),
        ];
        let json: serde_json::Value =
            serde_json::from_str(&generate_caddy_json(80, 443, &routes, true).unwrap()).unwrap();
        let first = &json["apps"]["http"]["servers"]["gateway"]["routes"][0]["match"][0];
        assert_eq!(first["path"], serde_json::json!(["/api", "/api/*"]));
        assert_eq!(json["apps"]["http"]["servers"]["gateway"]["listen"][0], ":80");
    }

    #[test]
    fn invalid_routes_are_rejected() {
        assert!(validate_routes(&[route("1", "my app.test", None, "127.0.0.1:80")]).is_err());
        assert!(validate_routes(&[route("1", "app.test", None, "127.0.0.1")]).is_err());
        assert!(validate_routes(&[
            route("1", "app.test", Some("/x"), "127.0.0.1:80"),
            route("2", "app.test", Some("/x/"), "127.0.0.1:81"),
        ])
        .is_err());
        assert!(validate_routes(&[route("1", "app.test", Some("/api"), "localhost:8080")]).is_ok());
    }

    #[test]
    fn dead_upstreams_get_an_explanatory_error_page() {
        let json = generate_caddy_json(80, 443, &[route("a", "app.local", None, "127.0.0.1:1")], false)
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let handler = &v["apps"]["http"]["servers"]["gateway"]["errors"]["routes"][0]["handle"][0];
        assert_eq!(handler["handler"], "static_response");
        assert_eq!(handler["status_code"], "{http.error.status_code}");
        assert!(handler["body"].as_str().unwrap().contains("{http.request.host}"));
    }

    #[tokio::test]
    async fn upstream_probe_reports_listening_and_dead_targets() {
        let live = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let live_port = live.local_addr().unwrap().port();
        let dead_port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let mut off = route("off", "off.local", None, "127.0.0.1:1");
        off.enabled = false;
        let probes = probe_upstreams(&[
            route("up", "up.local", None, &format!("127.0.0.1:{live_port}")),
            route("down", "down.local", None, &format!("127.0.0.1:{dead_port}")),
            off,
        ])
        .await;
        let by_id: std::collections::HashMap<_, _> =
            probes.iter().map(|p| (p.id.as_str(), p.reachable)).collect();
        assert_eq!(by_id.get("up"), Some(&true));
        assert_eq!(by_id.get("down"), Some(&false));
        assert!(!by_id.contains_key("off"), "disabled routes are not probed");
    }

    #[test]
    fn a_taken_port_is_never_bindable_even_with_the_capability() {
        use std::io::{Error, ErrorKind};
        assert!(bind_probe_allows(Ok(()), false));
        assert!(bind_probe_allows(Err(Error::from(ErrorKind::PermissionDenied)), true));
        assert!(!bind_probe_allows(Err(Error::from(ErrorKind::PermissionDenied)), false));
        assert!(!bind_probe_allows(Err(Error::from(ErrorKind::AddrInUse)), true));
    }

    #[test]
    fn proc_net_tcp_listen_rows_are_found_by_port() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue\n\
   0: 00000000:0050 00000000:0000 0A 00000000:00000000 00:00000000 00000000\n\
   1: 0100007F:07E3 0100007F:9C40 01 00000000:00000000 00:00000000 00000000\n";
        assert!(proc_net_tcp_listens_on(table, 80));
        assert!(!proc_net_tcp_listens_on(table, 2019), "an established socket is not a listener");
        assert!(!proc_net_tcp_listens_on(table, 443));
    }

    #[test]
    fn net_bind_service_probe_is_false_without_the_capability() {
        // A missing binary has no file capabilities; getcap exits non-zero
        // (or prints nothing) and the probe must read that as "not granted".
        assert!(!has_net_bind_service(Path::new("/nonexistent/hsx2dock-caddy")));
    }
}
