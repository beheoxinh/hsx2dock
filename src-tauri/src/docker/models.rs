use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum DaemonState {
    #[default]
    Stopped,
    Starting,
    /// A stop was requested and the socket is still winding down.
    Stopping,
    Running,
    NotInstalled,
    /// A bundled-engine download/install is in flight.
    Installing,
    /// Engine probe failed with a human-readable reason (e.g. ping error).
    Error(String),
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContainerItem {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub created: i64,
    pub running: bool,
    pub ports: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ImageItem {
    pub id: String,
    pub short_id: String,
    pub tags: Vec<String>,
    pub size: i64,
    pub created: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VolumeItem {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    pub created: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NetworkItem {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub subnet: String,
    pub gateway: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContainerStats {
    pub cpu_percent: f64,
    pub mem_usage: u64,
    pub mem_limit: u64,
}

impl ContainerStats {
    pub fn mem_percent(&self) -> f64 {
        if self.mem_limit == 0 {
            0.0
        } else {
            (self.mem_usage as f64 / self.mem_limit as f64) * 100.0
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct EngineInfo {
    pub version: String,
    pub api_version: String,
    pub os: String,
    pub kernel: String,
    pub arch: String,
    pub containers_running: i64,
    pub containers_stopped: i64,
    pub containers_paused: i64,
    pub images: i64,
    pub storage_driver: String,
    pub root_dir: String,
    pub cgroup_driver: String,
    #[serde(rename = "cpus")]
    pub n_cpu: i64,
    pub mem_total: i64,
}

/// Full detail snapshot for one container, built from `inspect_container`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContainerDetails {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub created: String,
    pub command: String,
    pub env: Vec<String>,
    pub mounts: Vec<String>,
    pub networks: Vec<String>,
    pub ports: String,
    pub restart_policy: String,
    /// Structured view of the same data, used to re-create the container.
    pub command_vec: Vec<String>,
    pub entrypoint: Vec<String>,
    pub port_mappings: Vec<PortMapping>,
    pub env_pairs: Vec<EnvVar>,
    pub volume_mounts: Vec<VolumeMount>,
    pub labels: Vec<EnvVar>,
    pub hostname: String,
    pub workdir: String,
    pub user: String,
    pub network: String,
    pub resources: ResourceLimits,
    pub restart_max_retries: String,
    pub healthcheck: Healthcheck,
    pub privileged: bool,
    pub tty: bool,
    pub stdin_open: bool,
    pub auto_remove: bool,
    pub read_only_rootfs: bool,
}

/// Transport protocol for a published port.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PortProtocol {
    #[default]
    Tcp,
    Udp,
    Sctp,
}

impl PortProtocol {
    pub const ALL: [PortProtocol; 3] = [Self::Tcp, Self::Udp, Self::Sctp];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Sctp => "sctp",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "tcp" | "" => Some(Self::Tcp),
            "udp" => Some(Self::Udp),
            "sctp" => Some(Self::Sctp),
            _ => None,
        }
    }
}

/// One published port, Docker Desktop style: host address + host port +
/// container port + protocol.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PortMapping {
    /// Bind address on the host; empty means every interface.
    pub host_ip: String,
    /// Host port or range (`8080`, `8000-8010`); empty lets Docker pick.
    pub host_port: String,
    /// Container port or range; required.
    pub container_port: String,
    pub protocol: PortProtocol,
}

impl PortMapping {
    pub fn new(container_port: u16, host_port: u16) -> Self {
        Self {
            host_ip: String::new(),
            host_port: host_port.to_string(),
            container_port: container_port.to_string(),
            protocol: PortProtocol::Tcp,
        }
    }

    /// Parse `[HOST_IP:][HOST_PORT:]CONTAINER_PORT[/PROTO]`.
    ///
    /// `8080:80`, `127.0.0.1:8080:80`, `80`, `5000-5010:5000-5010/udp`.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let raw = spec.trim();
        if raw.is_empty() {
            return Err("Port mapping is empty".to_string());
        }
        let (body, protocol) = match raw.split_once('/') {
            Some((b, p)) => match PortProtocol::parse(p) {
                Some(proto) => (b, proto),
                None => {
                    return Err(format!(
                        "Invalid protocol '{p}' in port mapping '{raw}' (expected tcp, udp or sctp)"
                    ));
                }
            },
            None => (raw, PortProtocol::Tcp),
        };

        let parts: Vec<&str> = body.split(':').collect();
        let (host_ip, host_port, container_port) = match parts.len() {
            1 => (String::new(), String::new(), parts[0].to_string()),
            2 => (String::new(), parts[0].to_string(), parts[1].to_string()),
            3 => (
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
            ),
            _ => {
                return Err(format!(
                    "Invalid port mapping '{raw}' (expected [HOST_IP:][HOST_PORT:]CONTAINER_PORT[/PROTO])"
                ));
            }
        };

        let m = Self {
            host_ip: host_ip.trim().to_string(),
            host_port: host_port.trim().to_string(),
            container_port: container_port.trim().to_string(),
            protocol,
        };
        m.validate()?;
        Ok(m)
    }

    /// Render back to the docker CLI spec; round-trips through [`parse`](Self::parse).
    pub fn to_spec(&self) -> String {
        let mut out = String::new();
        if !self.host_ip.trim().is_empty() {
            out.push_str(self.host_ip.trim());
            out.push(':');
        }
        if !self.host_port.trim().is_empty() {
            out.push_str(self.host_port.trim());
            out.push(':');
        }
        out.push_str(self.container_port.trim());
        if self.protocol != PortProtocol::Tcp {
            out.push('/');
            out.push_str(self.protocol.as_str());
        }
        out
    }

    /// Engine API key for this mapping: `"80/tcp"`.
    pub fn container_key(&self) -> String {
        format!("{}/{}", self.container_port.trim(), self.protocol.as_str())
    }

    /// `HostIp` for the binding, or `None` to bind every interface.
    pub fn host_ip_opt(&self) -> Option<String> {
        let ip = self.host_ip.trim();
        if ip.is_empty() {
            None
        } else {
            Some(ip.to_string())
        }
    }

    /// `HostPort` for the binding, or `None` to let Docker pick a free port.
    pub fn host_port_opt(&self) -> Option<String> {
        let p = self.host_port.trim();
        if p.is_empty() {
            None
        } else {
            Some(p.to_string())
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let container = self.container_port.trim();
        if container.is_empty() {
            return Err("Container port is required".to_string());
        }
        let (c_lo, c_hi) = parse_port_range(container).ok_or_else(|| {
            format!("Invalid container port '{container}' (expected 1-65535 or A-B)")
        })?;
        let host = self.host_port.trim();
        let host_span = if host.is_empty() {
            None
        } else {
            let (h_lo, h_hi) = parse_port_range(host)
                .ok_or_else(|| format!("Invalid host port '{host}' (expected 1-65535 or A-B)"))?;
            Some(h_hi - h_lo)
        };
        if let Some(span) = host_span {
            if span != c_hi - c_lo {
                return Err(format!(
                    "Host port range '{host}' and container port range '{container}' must be the same size"
                ));
            }
        }
        if !self.host_ip.trim().is_empty()
            && self.host_ip.trim().parse::<std::net::IpAddr>().is_err()
        {
            return Err(format!(
                "Invalid host IP '{}' (expected an IPv4 or IPv6 address)",
                self.host_ip.trim()
            ));
        }
        Ok(())
    }
}

/// Parse `8080` or `8000-8010` into an inclusive range.
fn parse_port_range(s: &str) -> Option<(u16, u16)> {
    match s.split_once('-') {
        Some((a, b)) => {
            let lo: u16 = a.trim().parse().ok()?;
            let hi: u16 = b.trim().parse().ok()?;
            if lo == 0 || hi < lo {
                return None;
            }
            Some((lo, hi))
        }
        None => {
            let p: u16 = s.trim().parse().ok()?;
            if p == 0 {
                return None;
            }
            Some((p, p))
        }
    }
}

/// One `KEY=VALUE` environment entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

impl EnvVar {
    pub fn new(key: &str, value: &str) -> Self {
        Self {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    pub fn parse(line: &str) -> Result<Self, String> {
        let raw = line.trim();
        match raw.split_once('=') {
            Some((k, v)) if !k.trim().is_empty() && !k.contains(char::is_whitespace) => Ok(Self {
                key: k.trim().to_string(),
                value: v.to_string(),
            }),
            _ => Err(format!("Invalid entry '{raw}' (expected KEY=VALUE)")),
        }
    }

    pub fn to_line(&self) -> String {
        format!("{}={}", self.key, self.value)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.key.trim().is_empty() {
            return Err("Key must not be empty".to_string());
        }
        if self.key.contains(char::is_whitespace) {
            return Err(format!("Key '{}' must not contain spaces", self.key));
        }
        Ok(())
    }
}

/// Where a mount's source comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MountKind {
    /// A host path, e.g. `/srv/data`.
    #[default]
    Bind,
    /// A named volume managed by Docker, e.g. `pgdata`.
    Volume,
}

impl MountKind {
    pub const ALL: [MountKind; 2] = [Self::Bind, Self::Volume];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bind => "bind",
            Self::Volume => "volume",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bind => "Bind (host path)",
            Self::Volume => "Volume (named)",
        }
    }
}

/// One bind mount or named volume.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VolumeMount {
    pub kind: MountKind,
    /// Host path (bind) or volume name (volume).
    pub source: String,
    pub target: String,
    pub read_only: bool,
}

impl VolumeMount {
    pub fn new(source: &str, target: &str) -> Self {
        Self {
            kind: MountKind::Bind,
            source: source.to_string(),
            target: target.to_string(),
            read_only: false,
        }
    }

    /// Parse `SOURCE:TARGET[:ro]`.
    ///
    /// A source that is not an absolute/relative path is treated as a named
    /// volume, matching the `docker run -v` shorthand.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let raw = spec.trim();
        let parts: Vec<&str> = raw.split(':').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return Err(format!(
                "Invalid volume mapping '{raw}' (expected SOURCE:TARGET[:ro])"
            ));
        }
        let source = parts[0].trim();
        let target = parts[1].trim();
        let mode = parts.get(2).map(|m| m.trim()).unwrap_or("");
        if !matches!(mode, "" | "ro" | "rw") {
            return Err(format!(
                "Invalid volume mode '{mode}' in '{raw}' (expected ro or rw)"
            ));
        }
        let m = Self {
            kind: if is_path_like(source) {
                MountKind::Bind
            } else {
                MountKind::Volume
            },
            source: source.to_string(),
            target: target.to_string(),
            read_only: mode == "ro",
        };
        m.validate()?;
        Ok(m)
    }

    /// Render back to the `SOURCE:TARGET[:ro]` shorthand.
    pub fn to_spec(&self) -> String {
        let mut out = format!("{}:{}", self.source.trim(), self.target.trim());
        if self.read_only {
            out.push_str(":ro");
        }
        out
    }

    pub fn validate(&self) -> Result<(), String> {
        let source = self.source.trim();
        let target = self.target.trim();
        if source.is_empty() {
            return Err("Mount source is required".to_string());
        }
        if target.is_empty() {
            return Err("Mount target is required".to_string());
        }
        if !target.starts_with('/') {
            return Err(format!(
                "Mount target '{target}' must be an absolute path inside the container"
            ));
        }
        if self.kind == MountKind::Bind && !is_path_like(source) {
            return Err(format!("Bind source '{source}' must be a host path"));
        }
        Ok(())
    }
}

/// Heuristic used by the `-v` shorthand: absolute, `~`, `.` or `..` prefixes
/// are host paths; anything else is a named volume.
fn is_path_like(source: &str) -> bool {
    source.starts_with('/')
        || source.starts_with("~/")
        || source.starts_with("./")
        || source.starts_with("../")
        || source == "."
        || source == ".."
        || source == "~"
}

/// Container restart policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RestartPolicy {
    #[default]
    No,
    Always,
    UnlessStopped,
    OnFailure,
}

impl RestartPolicy {
    pub const ALL: [RestartPolicy; 4] =
        [Self::No, Self::Always, Self::UnlessStopped, Self::OnFailure];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Always => "always",
            Self::UnlessStopped => "unless-stopped",
            Self::OnFailure => "on-failure",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "always" => Self::Always,
            "unless-stopped" => Self::UnlessStopped,
            "on-failure" => Self::OnFailure,
            _ => Self::No,
        }
    }
}

/// Memory / CPU constraints for the container.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceLimits {
    /// Memory limit in MiB; empty means unlimited.
    pub memory_mb: String,
    /// CPU limit in cores, e.g. `1.5`; empty means unlimited.
    pub cpus: String,
    /// Allowed CPUs, e.g. `0-2,4`; empty means all.
    pub cpuset_cpus: String,
    /// Relative CPU weight, 2..=262144; empty means the daemon default.
    pub cpu_shares: String,
    /// Max process count; empty means unlimited.
    pub pids_limit: String,
}

impl ResourceLimits {
    pub fn is_empty(&self) -> bool {
        self.memory_mb.trim().is_empty()
            && self.cpus.trim().is_empty()
            && self.cpuset_cpus.trim().is_empty()
            && self.cpu_shares.trim().is_empty()
            && self.pids_limit.trim().is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        self.memory_bytes()?;
        self.nano_cpus()?;
        self.cpu_shares_value()?;
        self.pids_limit_value()?;
        if !self.cpuset_cpus.trim().is_empty() && !is_valid_cpuset(self.cpuset_cpus.trim()) {
            return Err(format!(
                "Invalid CPU set '{}' (expected e.g. 0-2,4)",
                self.cpuset_cpus.trim()
            ));
        }
        Ok(())
    }

    /// Memory limit in bytes, `None` when unset.
    pub fn memory_bytes(&self) -> Result<Option<i64>, String> {
        let raw = self.memory_mb.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        let mb: i64 = raw.parse().map_err(|_| {
            format!("Invalid memory limit '{raw}' (expected MiB as a whole number)")
        })?;
        if mb < 6 {
            return Err(format!(
                "Memory limit '{raw}' MiB is below the 6 MiB minimum"
            ));
        }
        mb.checked_mul(1024 * 1024)
            .map(Some)
            .ok_or_else(|| format!("Memory limit '{raw}' MiB is too large"))
    }

    /// CPU limit in nano-CPUs, `None` when unset.
    pub fn nano_cpus(&self) -> Result<Option<i64>, String> {
        let raw = self.cpus.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        let cores: f64 = raw
            .parse()
            .map_err(|_| format!("Invalid CPU limit '{raw}' (expected e.g. 1.5)"))?;
        if !cores.is_finite() || cores <= 0.0 {
            return Err(format!("CPU limit '{raw}' must be greater than zero"));
        }
        let nano = (cores * 1_000_000_000.0).round();
        if nano > i64::MAX as f64 {
            return Err(format!("CPU limit '{raw}' is too large"));
        }
        Ok(Some(nano as i64))
    }

    /// CPU shares, `None` when unset.
    pub fn cpu_shares_value(&self) -> Result<Option<i64>, String> {
        let raw = self.cpu_shares.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        let v: i64 = raw
            .parse()
            .map_err(|_| format!("Invalid CPU shares '{raw}' (expected a whole number)"))?;
        if !(2..=262_144).contains(&v) {
            return Err(format!("CPU shares '{raw}' must be between 2 and 262144"));
        }
        Ok(Some(v))
    }

    /// PIDs limit, `None` when unset.
    pub fn pids_limit_value(&self) -> Result<Option<i64>, String> {
        let raw = self.pids_limit.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        let v: i64 = raw
            .parse()
            .map_err(|_| format!("Invalid PIDs limit '{raw}' (expected a whole number)"))?;
        if v < 1 {
            return Err(format!("PIDs limit '{raw}' must be at least 1"));
        }
        Ok(Some(v))
    }
}

/// `0-2,4` style CPU set check.
fn is_valid_cpuset(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.split(',').all(|part| {
        let part = part.trim();
        if part.is_empty() {
            return false;
        }
        match part.split_once('-') {
            Some((a, b)) => match (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                (Ok(a), Ok(b)) => a <= b,
                _ => false,
            },
            None => part.parse::<u32>().is_ok(),
        }
    })
}

/// Container health check. An empty `test` disables it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Healthcheck {
    /// `["CMD", "curl", "-f", "http://localhost"]` style argv.
    pub test: Vec<String>,
    pub interval_s: String,
    pub timeout_s: String,
    pub retries: String,
    pub start_period_s: String,
}

impl Healthcheck {
    pub fn is_enabled(&self) -> bool {
        !self.test.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.is_enabled() {
            return Ok(());
        }
        for (label, raw) in [
            ("interval", &self.interval_s),
            ("timeout", &self.timeout_s),
            ("start period", &self.start_period_s),
        ] {
            if raw.trim().is_empty() {
                continue;
            }
            parse_duration_ns(raw.trim()).ok_or_else(|| {
                format!("Invalid health check {label} '{raw}' (expected e.g. 30s, 1m)")
            })?;
        }
        let retries = self.retries.trim();
        if !retries.is_empty() {
            let v: u32 = retries.parse().map_err(|_| {
                format!("Invalid health check retries '{retries}' (expected a whole number)")
            })?;
            if v == 0 {
                return Err("Health check retries must be at least 1".to_string());
            }
        }
        Ok(())
    }

    pub fn interval_ns(&self) -> Option<i64> {
        parse_duration_ns(self.interval_s.trim())
    }

    pub fn timeout_ns(&self) -> Option<i64> {
        parse_duration_ns(self.timeout_s.trim())
    }

    pub fn start_period_ns(&self) -> Option<i64> {
        parse_duration_ns(self.start_period_s.trim())
    }

    pub fn retries_value(&self) -> Option<i64> {
        self.retries.trim().parse().ok()
    }
}

/// Parse `500ms`, `30s`, `1m30s`, `2h` into nanoseconds.
///
/// Matches the Go duration syntax the Engine uses for health-check timings:
/// `ns`, `us`/`µs`, `ms`, `s`, `m`, `h`, and combinations like `1m30s`.
fn parse_duration_ns(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    const NS: i64 = 1;
    const US: i64 = 1_000;
    const MS: i64 = 1_000_000;
    const S: i64 = 1_000_000_000;
    const M: i64 = 60 * S;
    const H: i64 = 60 * M;

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut total: i64 = 0;
    let mut saw_unit = false;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            return None;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let n: i64 = chars[start..i].iter().collect::<String>().parse().ok()?;
        if i >= chars.len() {
            return None; // trailing digits with no unit
        }
        // Two-character units are consumed before their one-character prefix.
        let rest = &chars[i..];
        let (mult, len) = if rest.starts_with(&['m', 's']) {
            (MS, 2)
        } else if rest.starts_with(&['u', 's']) {
            (US, 2)
        } else if rest.starts_with(&['n', 's']) {
            (NS, 2)
        } else if rest.starts_with(&['µ', 's']) {
            (US, 2)
        } else {
            match chars[i] {
                's' => (S, 1),
                'm' => (M, 1),
                'h' => (H, 1),
                _ => return None,
            }
        };
        total = total.checked_add(n.checked_mul(mult)?)?;
        i += len;
        saw_unit = true;
    }
    if !saw_unit {
        return None;
    }
    Some(total)
}

/// Parameters for creating + starting a container (Run wizard).
///
/// Mirrors the Docker Desktop "Run a new container" form: every field the
/// Engine accepts on create is representable here.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RunConfig {
    pub image: String,
    pub name: String,
    /// Overrides the image `CMD`.
    pub command: Vec<String>,
    /// Overrides the image `ENTRYPOINT`.
    pub entrypoint: Vec<String>,
    pub ports: Vec<PortMapping>,
    pub env: Vec<EnvVar>,
    pub volumes: Vec<VolumeMount>,
    pub restart_policy: RestartPolicy,
    /// Only meaningful for [`RestartPolicy::OnFailure`].
    pub restart_max_retries: String,
    pub detach: bool,
    pub hostname: String,
    pub workdir: String,
    pub user: String,
    /// Docker network to attach to; empty means the daemon default.
    pub network: String,
    pub labels: Vec<EnvVar>,
    pub resources: ResourceLimits,
    pub healthcheck: Healthcheck,
    pub cap_add: Vec<String>,
    pub cap_drop: Vec<String>,
    /// `/etc/hosts` entries in `host:ip` form.
    pub extra_hosts: Vec<String>,
    pub privileged: bool,
    /// Keep STDIN open.
    pub stdin_open: bool,
    /// Allocate a pseudo-TTY.
    pub tty: bool,
    /// Remove the container automatically when it exits.
    pub auto_remove: bool,
    /// Mount the container root filesystem read-only.
    pub read_only_rootfs: bool,
}

impl RunConfig {
    /// Validate user input; returns a human-readable error.
    pub fn validate(&self) -> Result<(), String> {
        if self.image.trim().is_empty() {
            return Err("Image is required".to_string());
        }
        if !self.name.trim().is_empty() && !is_valid_container_name(self.name.trim()) {
            return Err(format!(
                "Invalid container name '{}' (letters, digits, . _ - only)",
                self.name.trim()
            ));
        }
        for (i, p) in self.ports.iter().enumerate() {
            p.validate()
                .map_err(|e| format!("Port mapping #{}: {e}", i + 1))?;
        }
        for (i, v) in self.volumes.iter().enumerate() {
            v.validate().map_err(|e| format!("Mount #{}: {e}", i + 1))?;
        }
        for (i, e) in self.env.iter().enumerate() {
            e.validate()
                .map_err(|e| format!("Environment variable #{}: {e}", i + 1))?;
        }
        for (i, l) in self.labels.iter().enumerate() {
            l.validate().map_err(|e| format!("Label #{}: {e}", i + 1))?;
        }
        self.resources.validate()?;
        self.healthcheck.validate()?;
        let retries = self.restart_max_retries.trim();
        if !retries.is_empty() {
            if self.restart_policy != RestartPolicy::OnFailure {
                return Err("Restart retries only apply to the 'on-failure' policy".to_string());
            }
            let v: u32 = retries.parse().map_err(|_| {
                format!("Invalid restart retries '{retries}' (expected a whole number)")
            })?;
            if v == 0 {
                return Err("Restart retries must be at least 1".to_string());
            }
        }
        if !self.workdir.trim().is_empty() && !self.workdir.trim().starts_with('/') {
            return Err(format!(
                "Working directory '{}' must be an absolute path",
                self.workdir.trim()
            ));
        }
        Ok(())
    }

    /// `HostPort`/`HostIp` bindings keyed by container port, as the Engine
    /// expects them for `HostConfig.PortBindings`.
    ///
    /// Several mappings may publish the same container port on different host
    /// ports, so the values are accumulated rather than overwritten.
    pub fn port_bindings(
        &self,
    ) -> std::collections::HashMap<String, Option<Vec<bollard::models::PortBinding>>> {
        let mut out: std::collections::HashMap<String, Option<Vec<bollard::models::PortBinding>>> =
            std::collections::HashMap::new();
        for p in &self.ports {
            let binding = bollard::models::PortBinding {
                host_ip: p.host_ip_opt(),
                host_port: p.host_port_opt(),
            };
            out.entry(p.container_key())
                .or_insert_with(|| Some(Vec::new()))
                .get_or_insert_with(Vec::new)
                .push(binding);
        }
        out
    }

    /// `ExposedPorts` keys (`"80/tcp"`), derived from the published ports.
    pub fn exposed_ports(&self) -> Vec<String> {
        let mut out: Vec<String> = self.ports.iter().map(|p| p.container_key()).collect();
        out.sort();
        out.dedup();
        out
    }

    /// `KEY=VALUE` list for `ContainerConfig.Env`.
    pub fn env_list(&self) -> Vec<String> {
        self.env.iter().map(|e| e.to_line()).collect()
    }

    /// `KEY=VALUE` list for `ContainerConfig.Labels`.
    pub fn label_list(&self) -> Vec<String> {
        self.labels.iter().map(|l| l.to_line()).collect()
    }

    /// `RestartPolicy` for `HostConfig`.
    pub fn restart_policy_body(&self) -> bollard::models::RestartPolicy {
        bollard::models::RestartPolicy {
            name: Some(match self.restart_policy {
                RestartPolicy::No => bollard::models::RestartPolicyNameEnum::NO,
                RestartPolicy::Always => bollard::models::RestartPolicyNameEnum::ALWAYS,
                RestartPolicy::UnlessStopped => {
                    bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED
                }
                RestartPolicy::OnFailure => bollard::models::RestartPolicyNameEnum::ON_FAILURE,
            }),
            maximum_retry_count: self.restart_retries_value(),
        }
    }

    /// Parse a multiline text area into trimmed non-empty lines.
    pub fn parse_lines(text: &str) -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Parse one docker-spec entry per line: port, mount or `KEY=VALUE`.
    pub fn parse_ports(text: &str) -> Result<Vec<PortMapping>, String> {
        Self::parse_lines(text)
            .iter()
            .map(|l| PortMapping::parse(l))
            .collect()
    }

    pub fn parse_volumes(text: &str) -> Result<Vec<VolumeMount>, String> {
        Self::parse_lines(text)
            .iter()
            .map(|l| VolumeMount::parse(l))
            .collect()
    }

    pub fn parse_env(text: &str) -> Result<Vec<EnvVar>, String> {
        Self::parse_lines(text)
            .iter()
            .map(|l| EnvVar::parse(l))
            .collect()
    }

    /// `HealthConfig` for the Engine, or `None` when health checking is off.
    pub fn healthcheck_body(&self) -> Option<bollard::models::HealthConfig> {
        if !self.healthcheck.is_enabled() {
            return None;
        }
        Some(bollard::models::HealthConfig {
            test: Some(self.healthcheck.test.clone()),
            interval: self.healthcheck.interval_ns(),
            timeout: self.healthcheck.timeout_ns(),
            retries: self.healthcheck.retries_value(),
            start_period: self.healthcheck.start_period_ns(),
            ..Default::default()
        })
    }

    /// Rebuild a run configuration from an existing container's details.
    ///
    /// Used by the duplicate / recreate flows: the result feeds the Run form
    /// so the user can adjust anything before the new container is created.
    /// The Engine cannot change ports, environment or mounts in place, so
    /// re-creating is the only way to edit them.
    pub fn from_details(d: &ContainerDetails) -> Self {
        Self {
            image: d.image.clone(),
            name: d.name.clone(),
            command: d.command_vec.clone(),
            entrypoint: d.entrypoint.clone(),
            ports: d.port_mappings.clone(),
            env: d.env_pairs.clone(),
            volumes: d.volume_mounts.clone(),
            restart_policy: RestartPolicy::parse(&d.restart_policy),
            restart_max_retries: d.restart_max_retries.clone(),
            detach: true,
            hostname: d.hostname.clone(),
            workdir: d.workdir.clone(),
            user: d.user.clone(),
            network: d.network.clone(),
            labels: d.labels.clone(),
            resources: d.resources.clone(),
            healthcheck: d.healthcheck.clone(),
            cap_add: Vec::new(),
            cap_drop: Vec::new(),
            extra_hosts: Vec::new(),
            privileged: d.privileged,
            stdin_open: d.stdin_open,
            tty: d.tty,
            auto_remove: d.auto_remove,
            read_only_rootfs: d.read_only_rootfs,
        }
    }

    /// Restart retry count, `None` unless the policy is `on-failure`.
    pub fn restart_retries_value(&self) -> Option<i64> {
        if self.restart_policy != RestartPolicy::OnFailure {
            return None;
        }
        self.restart_max_retries.trim().parse().ok()
    }
}

/// Docker container names: `[a-zA-Z0-9][a-zA-Z0-9_.-]*`.
fn is_valid_container_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// One entry of `docker history`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ImageHistoryEntry {
    pub id: String,
    pub short_id: String,
    pub created: i64,
    pub created_by: String,
    pub tags: Vec<String>,
    pub size: i64,
}

/// Aggregated `docker system df` numbers for the dashboard.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct DfReport {
    pub images_count: usize,
    pub images_size: u64,
    pub containers_count: usize,
    pub containers_size: u64,
    pub volumes_count: usize,
    pub volumes_size: u64,
    pub build_cache_size: u64,
    pub total_reclaimable: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(image: &str, ports: &[&str], env: &[&str]) -> RunConfig {
        RunConfig {
            image: image.to_string(),
            ports: ports
                .iter()
                .map(|s| PortMapping::parse(s).unwrap_or_default())
                .collect(),
            env: env
                .iter()
                .map(|s| EnvVar::parse(s).unwrap_or_default())
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn validate_rejects_empty_image() {
        assert!(cfg("", &[], &[]).validate().is_err());
        assert!(cfg("  ", &[], &[]).validate().is_err());
    }

    #[test]
    fn validate_accepts_host_port_mapping() {
        assert!(cfg("nginx:latest", &["8080:80"], &["K=V"])
            .validate()
            .is_ok());
    }

    #[test]
    fn validate_rejects_bad_ports_and_env() {
        assert!(cfg("img", &["no-colon"], &[]).validate().is_err());
        assert!(cfg("img", &["a:b:c"], &[]).validate().is_err());
        assert!(cfg("img", &[], &["NOEQUALS"]).validate().is_err());
    }

    #[test]
    fn parse_lines_trims_and_skips_empty() {
        assert_eq!(
            RunConfig::parse_lines("  a\n\n  b  \n"),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(RunConfig::parse_lines("  \n ").is_empty());
    }

    #[test]
    fn port_mapping_parse_roundtrip() {
        let m = PortMapping::parse("127.0.0.1:8080:80/udp").unwrap();
        assert_eq!(m.host_ip, "127.0.0.1");
        assert_eq!(m.host_port, "8080");
        assert_eq!(m.container_port, "80");
        assert_eq!(m.protocol, PortProtocol::Udp);
        assert_eq!(m.to_spec(), "127.0.0.1:8080:80/udp");

        let m2 = PortMapping::parse("8080:80").unwrap();
        assert_eq!(m2.host_ip, "");
        assert_eq!(m2.host_port, "8080");
        assert_eq!(m2.container_port, "80");
        assert_eq!(m2.protocol, PortProtocol::Tcp);
        assert_eq!(m2.to_spec(), "8080:80");

        let m3 = PortMapping::parse("80").unwrap();
        assert_eq!(m3.host_port, "");
        assert_eq!(m3.container_port, "80");
        assert_eq!(m3.to_spec(), "80");
    }

    #[test]
    fn port_mapping_range_validation() {
        let m = PortMapping::parse("8000-8010:5000-5010").unwrap();
        assert_eq!(m.host_port, "8000-8010");
        assert_eq!(m.container_port, "5000-5010");
        // Range sizes must match: 11 host ports vs 6 container ports.
        assert!(PortMapping::parse("8000-8010:5000-5005").is_err());
        // Inverted ranges and port 0 are rejected.
        assert!(PortMapping::parse("8010-8000:5000").is_err());
        assert!(PortMapping::parse("0:80").is_err());
        assert!(PortMapping::parse("70000:80").is_err());
    }

    #[test]
    fn port_mapping_empty_host_port_means_auto() {
        let m = PortMapping::parse("80").unwrap();
        assert_eq!(m.host_port_opt(), None);
        assert_eq!(m.host_ip_opt(), None);
        assert_eq!(m.container_key(), "80/tcp");
        assert_eq!(PortMapping::new(80, 8080).to_spec(), "8080:80");
    }

    #[test]
    fn port_mapping_rejects_bad_protocol_and_ip() {
        assert!(PortMapping::parse("80/icmp").is_err());
        assert!(PortMapping::parse("not-an-ip:8080:80").is_err());
        assert!(PortMapping::parse("::1:8080:80").is_err()); // too many colons
        assert!(PortMapping::parse("").is_err());
    }

    #[test]
    fn volume_mount_parse_roundtrip() {
        let v = VolumeMount::parse("/srv/data:/data:ro").unwrap();
        assert_eq!(v.kind, MountKind::Bind);
        assert_eq!(v.source, "/srv/data");
        assert_eq!(v.target, "/data");
        assert!(v.read_only);
        assert_eq!(v.to_spec(), "/srv/data:/data:ro");

        let v2 = VolumeMount::parse("pgdata:/var/lib/postgresql/data").unwrap();
        assert_eq!(v2.kind, MountKind::Volume);
        assert_eq!(v2.source, "pgdata");
        assert_eq!(v2.target, "/var/lib/postgresql/data");
    }

    #[test]
    fn env_var_parse() {
        let e = EnvVar::parse("FOO=BAR").unwrap();
        assert_eq!(e.key, "FOO");
        assert_eq!(e.value, "BAR");
        let e2 = EnvVar::parse("FOO=").unwrap();
        assert_eq!(e2.value, "");
        assert!(EnvVar::parse("=BAR").is_err());
        assert!(EnvVar::parse("FOO BAR").is_err());
    }

    #[test]
    fn resource_limits_convert_to_engine_units() {
        let r = ResourceLimits {
            memory_mb: "512".to_string(),
            cpus: "1.5".to_string(),
            cpuset_cpus: "0-2,4".to_string(),
            cpu_shares: "1024".to_string(),
            pids_limit: "200".to_string(),
        };
        assert!(r.validate().is_ok());
        assert_eq!(r.memory_bytes().unwrap(), Some(512 * 1024 * 1024));
        assert_eq!(r.nano_cpus().unwrap(), Some(1_500_000_000));
        assert_eq!(r.cpu_shares_value().unwrap(), Some(1024));
        assert_eq!(r.pids_limit_value().unwrap(), Some(200));
        assert!(!r.is_empty());
    }

    #[test]
    fn resource_limits_empty_means_unlimited() {
        let r = ResourceLimits::default();
        assert!(r.validate().is_ok());
        assert_eq!(r.memory_bytes().unwrap(), None);
        assert_eq!(r.nano_cpus().unwrap(), None);
        assert_eq!(r.cpu_shares_value().unwrap(), None);
        assert_eq!(r.pids_limit_value().unwrap(), None);
        assert!(r.is_empty());
    }

    #[test]
    fn resource_limits_reject_out_of_range_values() {
        // Below Docker's 6 MiB floor.
        assert!(ResourceLimits {
            memory_mb: "5".to_string(),
            ..Default::default()
        }
        .validate()
        .is_err());
        // Non-numeric.
        assert!(ResourceLimits {
            memory_mb: "1g".to_string(),
            ..Default::default()
        }
        .validate()
        .is_err());
        // Zero / negative CPU.
        assert!(ResourceLimits {
            cpus: "0".to_string(),
            ..Default::default()
        }
        .validate()
        .is_err());
        // CPU shares below 2.
        assert!(ResourceLimits {
            cpu_shares: "1".to_string(),
            ..Default::default()
        }
        .validate()
        .is_err());
        // Malformed CPU set.
        assert!(ResourceLimits {
            cpuset_cpus: "3-1".to_string(),
            ..Default::default()
        }
        .validate()
        .is_err());
    }

    #[test]
    fn healthcheck_durations_parse_to_nanoseconds() {
        assert_eq!(parse_duration_ns("30s"), Some(30_000_000_000));
        assert_eq!(parse_duration_ns("500ms"), Some(500_000_000));
        assert_eq!(parse_duration_ns("1m30s"), Some(90_000_000_000));
        assert_eq!(parse_duration_ns("2h"), Some(7_200_000_000_000));
        // Rejected forms.
        assert_eq!(parse_duration_ns("30"), None);
        assert_eq!(parse_duration_ns("s"), None);
        assert_eq!(parse_duration_ns("30x"), None);
        assert_eq!(parse_duration_ns(""), None);
    }

    #[test]
    fn healthcheck_enabled_only_with_a_test() {
        let mut h = Healthcheck::default();
        assert!(!h.is_enabled());
        assert!(h.validate().is_ok());
        h.test = vec!["CMD-SHELL".to_string(), "curl -f localhost".to_string()];
        h.interval_s = "30s".to_string();
        h.retries = "3".to_string();
        assert!(h.is_enabled());
        assert!(h.validate().is_ok());
        assert_eq!(h.interval_ns(), Some(30_000_000_000));
        assert_eq!(h.retries_value(), Some(3));
    }

    #[test]
    fn healthcheck_rejects_bad_timings_and_zero_retries() {
        let base = Healthcheck {
            test: vec!["CMD".to_string(), "true".to_string()],
            ..Default::default()
        };
        assert!(Healthcheck {
            interval_s: "soon".to_string(),
            ..base.clone()
        }
        .validate()
        .is_err());
        assert!(Healthcheck {
            retries: "0".to_string(),
            ..base.clone()
        }
        .validate()
        .is_err());
        assert!(Healthcheck {
            timeout_s: "5".to_string(),
            ..base
        }
        .validate()
        .is_err());
    }

    #[test]
    fn run_config_validates_name_and_workdir_and_retries() {
        let mut cfg = RunConfig {
            image: "nginx".to_string(),
            ..Default::default()
        };
        cfg.name = "bad name!".to_string();
        assert!(cfg.validate().is_err());

        cfg.name = "web-1.prod_ok".to_string();
        assert!(cfg.validate().is_ok());

        cfg.workdir = "relative/path".to_string();
        assert!(cfg.validate().is_err());
        cfg.workdir = "/srv/app".to_string();
        assert!(cfg.validate().is_ok());

        // Retries only apply to on-failure.
        cfg.restart_max_retries = "5".to_string();
        cfg.restart_policy = RestartPolicy::Always;
        assert!(cfg.validate().is_err());
        cfg.restart_policy = RestartPolicy::OnFailure;
        assert!(cfg.validate().is_ok());
        assert_eq!(cfg.restart_retries_value(), Some(5));
    }

    #[test]
    fn restart_policy_parses_and_maps_to_engine_enum() {
        assert_eq!(RestartPolicy::parse("always"), RestartPolicy::Always);
        assert_eq!(
            RestartPolicy::parse("unless-stopped"),
            RestartPolicy::UnlessStopped
        );
        assert_eq!(RestartPolicy::parse("on-failure"), RestartPolicy::OnFailure);
        assert_eq!(RestartPolicy::parse("nonsense"), RestartPolicy::No);
        assert_eq!(RestartPolicy::No.as_str(), "no");
        assert_eq!(
            RestartPolicy::Always.as_str(),
            bollard::models::RestartPolicyNameEnum::ALWAYS
                .to_string()
                .to_lowercase()
        );
    }

    #[test]
    fn port_bindings_group_multiple_host_ports_per_container_port() {
        let cfg = RunConfig {
            image: "nginx".to_string(),
            ports: vec![
                PortMapping::parse("8080:80").unwrap(),
                PortMapping::parse("8081:80").unwrap(),
                PortMapping::parse("443:443/udp").unwrap(),
            ],
            ..Default::default()
        };
        let b = cfg.port_bindings();
        assert_eq!(b.len(), 2, "80/tcp and 443/udp");
        let v80 = b.get("80/tcp").unwrap().as_ref().unwrap();
        assert_eq!(v80.len(), 2, "both host ports kept");
        let v443 = b.get("443/udp").unwrap().as_ref().unwrap();
        assert_eq!(v443.len(), 1);
        assert_eq!(v443[0].host_port.as_deref(), Some("443"));
        assert_eq!(cfg.exposed_ports(), vec!["443/udp", "80/tcp"]);
    }

    #[test]
    fn run_config_from_details_round_trips_editable_fields() {
        let d = ContainerDetails {
            name: "web".to_string(),
            image: "nginx:latest".to_string(),
            command_vec: vec!["nginx".to_string(), "-g".to_string()],
            entrypoint: vec!["/docker-entrypoint.sh".to_string()],
            port_mappings: vec![PortMapping::parse("8080:80").unwrap()],
            env_pairs: vec![EnvVar::new("A", "1")],
            volume_mounts: vec![VolumeMount::parse("/srv:/data:ro").unwrap()],
            restart_policy: "unless-stopped".to_string(),
            restart_max_retries: String::new(),
            hostname: "webhost".to_string(),
            workdir: "/srv".to_string(),
            user: "1000:1000".to_string(),
            network: "bridge".to_string(),
            labels: vec![EnvVar::new("app", "web")],
            resources: ResourceLimits {
                memory_mb: "512".to_string(),
                cpus: "2".to_string(),
                ..Default::default()
            },
            privileged: true,
            tty: true,
            stdin_open: true,
            auto_remove: true,
            read_only_rootfs: true,
            ..Default::default()
        };
        let cfg = RunConfig::from_details(&d);
        assert_eq!(cfg.image, "nginx:latest");
        assert_eq!(cfg.command, vec!["nginx", "-g"]);
        assert_eq!(cfg.entrypoint, vec!["/docker-entrypoint.sh"]);
        assert_eq!(cfg.ports.len(), 1);
        assert_eq!(cfg.ports[0].to_spec(), "8080:80");
        assert_eq!(cfg.env, vec![EnvVar::new("A", "1")]);
        assert_eq!(cfg.volumes[0].to_spec(), "/srv:/data:ro");
        assert_eq!(cfg.restart_policy, RestartPolicy::UnlessStopped);
        assert_eq!(cfg.hostname, "webhost");
        assert_eq!(cfg.workdir, "/srv");
        assert_eq!(cfg.user, "1000:1000");
        assert_eq!(cfg.labels, vec![EnvVar::new("app", "web")]);
        assert_eq!(
            cfg.resources.memory_bytes().unwrap(),
            Some(512 * 1024 * 1024)
        );
        assert!(cfg.privileged && cfg.tty && cfg.stdin_open && cfg.auto_remove);
        assert!(cfg.read_only_rootfs);
        // The rebuilt config must still be valid.
        assert!(cfg.validate().is_ok(), "{:?}", cfg.validate());
    }

    #[test]
    fn run_config_lists_env_and_labels_for_engine() {
        let cfg = RunConfig {
            image: "img".to_string(),
            env: vec![EnvVar::new("A", "1"), EnvVar::new("B", "2")],
            labels: vec![EnvVar::new("app", "web")],
            ..Default::default()
        };
        assert_eq!(cfg.env_list(), vec!["A=1", "B=2"]);
        assert_eq!(cfg.label_list(), vec!["app=web"]);
        assert_eq!(cfg.healthcheck_body(), None);
    }
}
