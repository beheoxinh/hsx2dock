use std::path::PathBuf;
use std::sync::Arc;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;

pub const CADDY_VERSION: &str = "2.11.7";
pub const CADDY_BASE: &str = "https://github.com/caddyserver/caddy/releases/download";

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
}

pub struct CaddySupervisor {
    pub child: Arc<Mutex<Option<tokio::process::Child>>>,
    pub paths: CaddyPaths,
}

impl CaddySupervisor {
    pub fn new() -> Self {
        Self {
            child: Arc::new(Mutex::new(None)),
            paths: CaddyPaths::for_app(),
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
        // Also probe admin API on port 2019 in case process was started externally
        is_admin_api_live().await
    }

    pub async fn start(
        &self,
        http_port: u16,
        https_port: u16,
        routes: &[GatewayRoute],
    ) -> Result<()> {
        if self.is_running().await {
            // Apply updated configuration via API
            return self.apply_routes(http_port, https_port, routes).await;
        }

        let bin = self.paths.binary();
        if !bin.is_file() {
            bail!("Caddy binary not installed yet at {}", bin.display());
        }

        let initial_json = generate_caddy_json(http_port, https_port, routes)?;

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
            if is_admin_api_live().await {
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

        let mut guard = self.child.lock().await;
        *guard = Some(child);
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let mut guard = self.child.lock().await;
        if let Some(mut child) = guard.take() {
            let _ = child.kill().await;
        }

        // Try graceful stop via admin API
        let client = reqwest::Client::new();
        let _ = client.post("http://127.0.0.1:2019/stop").send().await;

        Ok(())
    }

    pub async fn apply_routes(
        &self,
        http_port: u16,
        https_port: u16,
        routes: &[GatewayRoute],
    ) -> Result<()> {
        let json_body = generate_caddy_json(http_port, https_port, routes)?;
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
        let output = Command::new(&bin)
            .arg("trust")
            .output()
            .await
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

pub fn is_port_available(port: u16) -> bool {
    use std::net::TcpListener;
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Builds dynamic Caddy JSON configuration
pub fn generate_caddy_json(http_port: u16, https_port: u16, routes: &[GatewayRoute]) -> Result<String> {
    let mut server_routes = Vec::new();
    let mut tls_subjects = Vec::new();

    for r in routes {
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
            match_rules.as_object_mut().unwrap().insert(
                "path".to_string(),
                serde_json::json!([format!("{p}*")]),
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
    let can_bind_http = is_port_available(http_port);
    if can_bind_http {
        listen_addrs.push(format!(":{http_port}"));
    }
    listen_addrs.push(format!(":{https_port}"));

    let mut gateway_server = serde_json::json!({
        "listen": listen_addrs,
        "routes": server_routes
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

    if std::fs::write("/etc/hosts", new_content).is_ok() {
        return Ok(());
    }

    let tmp_file = std::env::temp_dir().join(format!("hsx2dock-raw-hosts-{}", std::process::id()));
    std::fs::write(&tmp_file, new_content).context("Failed to write temporary hosts file")?;

    let status = std::process::Command::new("pkexec")
        .args(["cp", tmp_file.to_str().unwrap(), "/etc/hosts"])
        .status();

    let _ = std::fs::remove_file(&tmp_file);

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => bail!("Failed to save /etc/hosts (pkexec exit code {:?})", s.code()),
        Err(e) => bail!("Failed to invoke pkexec: {e}"),
    }
}

pub fn clear_app_hosts_entries() -> Result<()> {
    let original = std::fs::read_to_string("/etc/hosts").context("Cannot read /etc/hosts")?;
    let mut lines = Vec::new();
    let mut inside = false;

    for line in original.lines() {
        if line.trim() == HOSTS_MARKER_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == HOSTS_MARKER_END {
            inside = false;
            continue;
        }
        if !inside {
            lines.push(line.to_string());
        }
    }

    let cleaned = lines.join("\n") + "\n";
    save_raw_hosts_content(&cleaned)
}

pub fn update_hosts_file(routes: &[GatewayRoute]) -> Result<()> {
    let status = inspect_hosts_sync(routes);
    let original = std::fs::read_to_string("/etc/hosts").context("Cannot read /etc/hosts")?;

    let mut lines: Vec<String> = Vec::new();
    let mut inside_block = false;

    for line in original.lines() {
        if line.trim() == HOSTS_MARKER_BEGIN {
            inside_block = true;
            continue;
        }
        if line.trim() == HOSTS_MARKER_END {
            inside_block = false;
            continue;
        }
        if !inside_block {
            lines.push(line.to_string());
        }
    }

    if !status.managed_domains.is_empty() {
        lines.push(HOSTS_MARKER_BEGIN.to_string());
        let domains_str = status.managed_domains.join(" ");
        lines.push(format!("127.0.0.1\t{domains_str}"));
        lines.push(format!("::1\t{domains_str}"));
        lines.push(HOSTS_MARKER_END.to_string());
    }

    let new_content = lines.join("\n") + "\n";

    // Try direct write first (if running as root / permissive permissions)
    if std::fs::write("/etc/hosts", &new_content).is_ok() {
        return Ok(());
    }

    // Write to a temporary file, then use pkexec or tee
    let tmp_file = std::env::temp_dir().join(format!("hsx2dock-hosts-{}", std::process::id()));
    std::fs::write(&tmp_file, &new_content).context("Failed to write temporary hosts file")?;

    let status = std::process::Command::new("pkexec")
        .args(["cp", tmp_file.to_str().unwrap(), "/etc/hosts"])
        .status();

    let _ = std::fs::remove_file(&tmp_file);

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => bail!("Failed to update /etc/hosts with pkexec (exit code {:?})", s.code()),
        Err(e) => bail!("Failed to invoke pkexec: {e}"),
    }
}
