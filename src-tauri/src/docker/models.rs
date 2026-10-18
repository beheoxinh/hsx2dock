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
    /// Compose project name from the `com.docker.compose.project` label —
    /// `None` for containers that were not created by docker compose.
    #[serde(default)]
    pub compose_project: Option<String>,
    /// Compose service name from the `com.docker.compose.service` label.
    #[serde(default)]
    pub compose_service: Option<String>,
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
    /// Total bytes of regular files under the mountpoint. `None` when the
    /// mountpoint is unreadable or the driver exposes no host path.
    pub size: Option<u64>,
    /// Number of subtrees that could not be read (permissions). When > 0,
    /// `size` is a lower bound.
    pub restricted_dirs: u32,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NetworkItem {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub subnet: String,
    pub gateway: String,
}

/// One `--build-arg KEY=VALUE` pair collected by the build form.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BuildArg {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

/// Everything the build form collects before a build starts.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BuildOptions {
    /// Host directory that becomes the build context archive.
    pub context_dir: String,
    /// Dockerfile path, relative to the context (absolute paths are accepted
    /// only when they live inside the context).
    #[serde(default)]
    pub dockerfile: String,
    pub tag: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub no_cache: bool,
    #[serde(default)]
    pub pull: bool,
    #[serde(default)]
    pub build_args: Vec<BuildArg>,
}

/// One parsed event from the `/build` stream, handed to the progress sink.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildStep {
    pub status: String,
    pub log: Option<String>,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub is_error: bool,
}

/// The form payload for creating/updating a remote endpoint.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EndpointInput {
    /// Present when editing an existing endpoint; `None` creates a new one.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    /// `"tcp"` or `"ssh"`.
    pub kind: String,
    pub uri: String,
}

/// A saved remote endpoint, or the implicit local bundled engine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EndpointStatus {
    /// `"local"` for the built-in engine; otherwise the saved id.
    pub id: String,
    pub name: String,
    /// `"local"`, `"tcp"` or `"ssh"`.
    pub kind: String,
    /// `unix:///…/docker.sock` for the local engine, else the saved URI.
    pub uri: String,
    pub is_active: bool,
    pub reachable: bool,
    pub version: Option<String>,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

impl EndpointStatus {
    /// The row the UI always shows for the engine this app bundles.
    pub fn local(socket: &str) -> Self {
        Self {
            id: "local".to_string(),
            name: "Bundled engine".to_string(),
            kind: "local".to_string(),
            uri: format!("unix://{socket}"),
            is_active: true,
            reachable: false,
            version: None,
            latency_ms: None,
            error: None,
        }
    }

    /// The implicit local engine can never be renamed or removed.
    pub fn is_local(&self) -> bool {
        self.kind == "local"
    }

    /// Engine lifecycle controls address the daemon this app owns, so they are
    /// only meaningful while the local endpoint is the active one.
    pub fn engine_controls_available(&self) -> bool {
        self.is_local()
    }
}

/// `host:port` for a tcp endpoint, `user@host` for ssh — what the card shows
/// and what a `DOCKER_HOST` value is built from.
pub fn endpoint_host_port(kind: &str, uri: &str) -> String {
    let trimmed = uri.trim();
    let without_scheme = match trimmed.split_once("://") {
        Some((_, rest)) => rest,
        None => trimmed,
    };
    match kind {
        "tcp" => {
            if without_scheme.contains(':') {
                without_scheme.to_string()
            } else {
                format!("{without_scheme}:2375")
            }
        }
        _ => without_scheme.to_string(),
    }
}

/// `DOCKER_HOST` for the CLI shim: the same daemon the UI talks to.
pub fn endpoint_docker_host(kind: &str, uri: &str) -> Option<String> {
    let host = endpoint_host_port(kind, uri);
    if host.is_empty() {
        return None;
    }
    match kind {
        "tcp" => Some(format!("tcp://{host}")),
        "ssh" => Some(format!("ssh://{host}")),
        _ => None,
    }
}

/// A stable, filesystem- and URL-safe id derived from a display name.
pub fn endpoint_id_from_name(name: &str) -> String {
    let mut id: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while id.contains("--") {
        id = id.replace("--", "-");
    }
    let id = id.trim_matches('-').to_string();
    if id.is_empty() {
        "endpoint".to_string()
    } else {
        id
    }
}

/// Extracts the registry host from an image reference. Docker Hub uses
/// `docker.io` as the implicit registry.
pub fn registry_host_for_image(image: &str) -> String {
    let trimmed = image.trim();
    // Local image names (no slash, or "localhost/...") have no external registry.
    if !trimmed.contains('/') {
        return "docker.io".to_string();
    }
    let first = trimmed.split('/').next().unwrap_or("");
    if first.is_empty() {
        return "docker.io".to_string();
    }
    // Handle official images like `library/nginx`? Docker Hub accepts both
    // forms; when no registry is explicit, the engine treats it as docker.io.
    // Names without a `.` or `:` in the first component are usually Docker Hub
    // namespaces (e.g. "library", "username"), not registry hosts.
    if !first.contains('.') && !first.contains(':') {
        return "docker.io".to_string();
    }
    first.to_string()
}

/// Cheap structural check used before the reachability probe.
pub fn validate_endpoint(kind: &str, uri: &str) -> Result<(), String> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return Err("Address is required".to_string());
    }
    match kind {
        "tcp" => {
            if trimmed.starts_with("ssh://") || trimmed.starts_with("unix://") {
                return Err("A TCP endpoint cannot use that scheme".to_string());
            }
            Ok(())
        }
        "ssh" => {
            if trimmed.starts_with("tcp://")
                || trimmed.starts_with("http://")
                || trimmed.starts_with("unix://")
            {
                return Err("An SSH endpoint cannot use that scheme".to_string());
            }
            Ok(())
        }
        other => Err(format!("Unsupported endpoint kind: {other}")),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct BuildContextInfo {
    pub exists: bool,
    pub is_dir: bool,
    /// Top-level file names, capped by the command, for the quick-pick chips.
    pub entries: Vec<String>,
    /// Names of the Dockerfiles the command found in the context.
    pub dockerfiles: Vec<String>,
    pub suggested_tag: String,
}

/// Quote a value the way a POSIX shell would, so the command preview the UI
/// shows is something the user could paste into a terminal and run.
pub fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let plain = value.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | ':' | '=' | '-' | '@')
    });
    if plain {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

impl BuildArg {
    /// `KEY=VALUE` for `--build-arg`; empty keys are dropped.
    pub fn as_pair(&self) -> Option<(String, String)> {
        let key = self.key.trim();
        if key.is_empty() {
            return None;
        }
        Some((key.to_string(), self.value.trim().to_string()))
    }
}

impl BuildOptions {
    /// The dockerfile value the daemon expects: a path relative to the
    /// context root, never the host-absolute path.
    pub fn dockerfile_arg(&self, context: &std::path::Path) -> String {
        let raw = self.dockerfile.trim();
        if raw.is_empty() {
            return "Dockerfile".to_string();
        }
        let path = std::path::Path::new(raw);
        if path.is_absolute() {
            if let Ok(relative) = path.strip_prefix(context) {
                return relative.to_string_lossy().into_owned();
            }
        }
        raw.to_string()
    }

    /// Validates the request and returns `(context, dockerfile relative arg)`.
    ///
    /// Both paths are canonicalised, so a `..` escape or a Dockerfile outside
    /// the context is rejected with a message the UI can show verbatim.
    pub fn validate(&self) -> Result<(std::path::PathBuf, String), String> {
        let raw_context = self.context_dir.trim();
        if raw_context.is_empty() {
            return Err("Build context is required".to_string());
        }
        if self.tag.trim().is_empty() {
            return Err("Tag is required".to_string());
        }
        let context = std::path::PathBuf::from(raw_context);
        if !context.exists() {
            return Err(format!("Build context does not exist: {}", context.display()));
        }
        if !context.is_dir() {
            return Err(format!("Build context is not a directory: {}", context.display()));
        }
        let dockerfile = self.dockerfile_arg(&context);
        let context_real = context
            .canonicalize()
            .map_err(|_| format!("Build context does not exist: {}", context.display()))?;
        let dockerfile_path = context.join(&dockerfile);
        let dockerfile_real = dockerfile_path
            .canonicalize()
            .map_err(|_| format!("Dockerfile not found: {}", dockerfile_path.display()))?;
        if !dockerfile_real.starts_with(&context_real) {
            return Err("Dockerfile must live inside the build context".to_string());
        }
        Ok((context, dockerfile))
    }

    /// Build-arg pairs for `--build-arg`, in the order the user typed them.
    pub fn build_arg_pairs(&self) -> Vec<(String, String)> {
        self.build_args.iter().filter_map(BuildArg::as_pair).collect()
    }

    /// The `docker build …` line that mirrors this request, for the UI preview.
    pub fn command_preview(&self) -> String {
        let mut parts = vec!["docker build".to_string()];
        parts.push(format!("-f {}", shell_quote(&self.dockerfile.trim().replace('\\', "/"))));
        if !self.tag.trim().is_empty() {
            parts.push(format!("-t {}", shell_quote(self.tag.trim())));
        }
        if let Some(target) = self.target.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            parts.push(format!("--target {}", shell_quote(target)));
        }
        if self.no_cache {
            parts.push("--no-cache".to_string());
        }
        if self.pull {
            parts.push("--pull".to_string());
        }
        for (key, value) in self.build_arg_pairs() {
            parts.push(format!("--build-arg {}", shell_quote(&format!("{key}={value}"))));
        }
        parts.push(".".to_string());
        parts.join(" ")
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ContainerStats {
    pub cpu_percent: f64,
    pub mem_usage: u64,
    pub mem_limit: u64,
    /// Cumulative bytes across the container's network endpoints. The
    /// dashboard diffs consecutive samples to turn these into rates.
    pub net_rx: u64,
    pub net_tx: u64,
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
    /// Current resource limits, prefilled from the engine for editing.
    pub limits: ResourceLimits,
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

    /// Canonical string for a policy name coming back from inspect — Debug
    /// formatting would produce `unless_stopped`, which the update endpoint
    /// rejects (it wants `unless-stopped`).
    pub fn from_bollard_name(name: &bollard::models::RestartPolicyNameEnum) -> &'static str {
        use bollard::models::RestartPolicyNameEnum as B;
        match name {
            B::ALWAYS => Self::Always.as_str(),
            B::UNLESS_STOPPED => Self::UnlessStopped.as_str(),
            B::ON_FAILURE => Self::OnFailure.as_str(),
            B::NO | B::EMPTY => Self::No.as_str(),
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
    /// Restart policy override for edits: empty = leave unchanged,
    /// otherwise one of [`RestartPolicy::as_str`].
    #[serde(default)]
    pub restart_policy: String,
    /// Restart attempts for the `on-failure` policy; empty = engine default.
    #[serde(default)]
    pub restart_max_retries: String,
}

impl ResourceLimits {
    pub fn is_empty(&self) -> bool {
        self.memory_mb.trim().is_empty()
            && self.cpus.trim().is_empty()
            && self.cpuset_cpus.trim().is_empty()
            && self.cpu_shares.trim().is_empty()
            && self.pids_limit.trim().is_empty()
            && self.restart_policy.trim().is_empty()
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

/// Derives the editable limits snapshot from the engine's `HostConfig`.
/// `None` (engine sent no HostConfig) means every limit is unset.
pub fn limits_from_host_config(
    host: Option<&bollard::models::HostConfig>,
) -> ResourceLimits {
    let Some(host) = host else {
        return ResourceLimits::default();
    };
    let memory_mb = host
        .memory
        .filter(|&b| b > 0)
        .map(|b| (b / (1024 * 1024)).to_string())
        .unwrap_or_default();
    let cpus = host
        .nano_cpus
        .filter(|&n| n > 0)
        .map(|n| ((n as f64) / 1e9).to_string())
        .or_else(|| {
            match (host.cpu_quota, host.cpu_period) {
                (Some(q), Some(p)) if q > 0 && p > 0 => {
                    Some(((q as f64) / (p as f64)).to_string())
                }
                _ => None,
            }
        })
        .unwrap_or_default();
    let cpuset_cpus = host
        .cpuset_cpus
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string();
    let cpu_shares = host
        .cpu_shares
        .filter(|&v| v > 0)
        .map(|v| v.to_string())
        .unwrap_or_default();
    let pids_limit = host
        .pids_limit
        .filter(|&v| v > 0)
        .map(|v| v.to_string())
        .unwrap_or_default();
    ResourceLimits {
        memory_mb,
        cpus,
        cpuset_cpus,
        cpu_shares,
        pids_limit,
        restart_policy: host
            .restart_policy
            .as_ref()
            .and_then(|r| r.name.as_ref())
            .map(|n| RestartPolicy::from_bollard_name(n).to_string())
            .unwrap_or_default(),
        restart_max_retries: host
            .restart_policy
            .as_ref()
            .and_then(|r| r.maximum_retry_count)
            .filter(|v| *v > 0)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    }
}

/// Builds the update body for `POST /containers/{id}/update`.
///
/// The engine treats an absent field (or a zero value) as "leave unchanged",
/// so unset fields are simply omitted here: the update endpoint can change a
/// limit but cannot remove one.
pub fn update_body_from_limits(
    limits: &ResourceLimits,
) -> Result<bollard::models::ContainerUpdateBody, String> {
    limits.validate()?;
    if limits.is_empty() {
        return Err("No resource limits were provided".to_string());
    }
    let mut body = bollard::models::ContainerUpdateBody::default();
    body.memory = limits.memory_bytes()?;
    body.nano_cpus = limits.nano_cpus()?;
    body.cpu_shares = limits.cpu_shares_value()?;
    body.pids_limit = limits.pids_limit_value()?;
    let cpuset = limits.cpuset_cpus.trim();
    if !cpuset.is_empty() {
        body.cpuset_cpus = Some(cpuset.to_string());
    }
    // Restart policy is editable through the same endpoint: an empty string
    // means "leave unchanged", mirroring how the limit fields behave.
    let rp = limits.restart_policy.trim();
    if !rp.is_empty() {
        if !RestartPolicy::ALL.iter().any(|p| p.as_str() == rp) {
            return Err(format!(
                "Invalid restart policy '{rp}' (expected no / always / unless-stopped / on-failure)"
            ));
        }
        let policy = RestartPolicy::parse(rp);
        let retries = if !limits.restart_max_retries.trim().is_empty() {
            if policy != RestartPolicy::OnFailure {
                return Err(
                    "Restart retries only apply to the 'on-failure' policy".to_string(),
                );
            }
            let raw = limits.restart_max_retries.trim();
            let v: i64 = raw.parse().map_err(|_| {
                format!("Invalid restart retries '{raw}' (expected a whole number)")
            })?;
            if v < 1 {
                return Err("Restart retries must be at least 1".to_string());
            }
            Some(v)
        } else {
            None
        };
        body.restart_policy = Some(bollard::models::RestartPolicy {
            name: Some(match policy {
                RestartPolicy::No => bollard::models::RestartPolicyNameEnum::NO,
                RestartPolicy::Always => bollard::models::RestartPolicyNameEnum::ALWAYS,
                RestartPolicy::UnlessStopped => {
                    bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED
                }
                RestartPolicy::OnFailure => bollard::models::RestartPolicyNameEnum::ON_FAILURE,
            }),
            maximum_retry_count: retries,
        });
    }
    Ok(body)
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

    #[test]
    fn registry_host_is_docker_io_for_bare_and_hub_names() {
        assert_eq!(registry_host_for_image("nginx"), "docker.io");
        assert_eq!(registry_host_for_image("library/nginx:1.25"), "docker.io");
        assert_eq!(registry_host_for_image("myuser/myapp"), "docker.io");
    }

    #[test]
    fn registry_host_is_taken_from_qualified_references() {
        assert_eq!(registry_host_for_image("ghcr.io/owner/img"), "ghcr.io");
        assert_eq!(
            registry_host_for_image("registry.gitlab.com/group/proj:tag"),
            "registry.gitlab.com"
        );
        assert_eq!(
            registry_host_for_image("harbor.example.com:8443/team/app@sha256:abc"),
            "harbor.example.com:8443"
        );
        assert_eq!(registry_host_for_image("localhost:5000/dev/x"), "localhost:5000");
    }

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
            ..Default::default()
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

    /// Unique directory per test process — std only, matching the pattern the
    /// other modules already use (no tempfile crate).
    fn temp_context(test: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("dm-models-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn build_opts(dir: &std::path::Path) -> BuildOptions {
        BuildOptions {
            context_dir: dir.to_string_lossy().into_owned(),
            tag: "app:dev".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn build_arg_pairs_collects_trimmed_pairs_and_drops_empty_keys() {
        let opts = BuildOptions {
            build_args: vec![
                BuildArg {
                    key: " NODE_ENV ".into(),
                    value: " production ".into(),
                },
                BuildArg {
                    key: String::new(),
                    value: "ignored".into(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            opts.build_arg_pairs(),
            vec![("NODE_ENV".to_string(), "production".to_string())]
        );
    }

    #[test]
    fn command_preview_includes_tag_target_flags_and_build_args() {
        let opts = BuildOptions {
            context_dir: "/tmp".into(),
            dockerfile: "Dockerfile".into(),
            tag: "app:1".into(),
            target: Some("prod".into()),
            no_cache: true,
            pull: true,
            build_args: vec![BuildArg {
                key: "FOO".into(),
                value: "bar".into(),
            }],
            ..Default::default()
        };
        let preview = opts.command_preview();
        assert!(preview.contains("-f Dockerfile"), "{preview}");
        assert!(preview.contains("-t app:1"), "{preview}");
        assert!(preview.contains("--target prod"), "{preview}");
        assert!(preview.contains("--no-cache"), "{preview}");
        assert!(preview.contains("--pull"), "{preview}");
        assert!(preview.contains("--build-arg FOO=bar"), "{preview}");
        assert!(preview.ends_with(" ."), "{preview}");
    }

    #[test]
    fn command_preview_quotes_values_the_shell_would_split() {
        let opts = BuildOptions {
            context_dir: "/tmp".into(),
            dockerfile: "docker/app file".into(),
            tag: "app:1".into(),
            build_args: vec![BuildArg {
                key: "GREETING".into(),
                value: "hello world".into(),
            }],
            ..Default::default()
        };
        let preview = opts.command_preview();
        assert!(preview.contains("-f 'docker/app file'"), "{preview}");
        assert!(
            preview.contains("--build-arg 'GREETING=hello world'"),
            "{preview}"
        );
    }

    #[test]
    fn validate_requires_a_tag() {
        let dir = temp_context("validate-requires-tag");
        let mut opts = build_opts(&dir);
        opts.tag = "  ".into();
        assert_eq!(opts.validate().unwrap_err(), "Tag is required");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_reports_a_context_that_does_not_exist() {
        let mut opts = build_opts(std::path::Path::new("/definitely/not/here"));
        opts.tag = "app:dev".into();
        let err = opts.validate().unwrap_err();
        assert!(err.starts_with("Build context does not exist"), "{err}");
    }

    #[test]
    fn validate_reports_a_dockerfile_that_does_not_exist() {
        let dir = temp_context("validate-df-missing");
        let err = build_opts(&dir).validate().unwrap_err();
        assert!(err.starts_with("Dockerfile not found"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_rejects_a_dockerfile_outside_the_context() {
        // The context is a subdirectory; the Dockerfile sits beside it, so
        // `../Dockerfile` escapes and must be refused.
        let root = temp_context("validate-outside");
        let context = root.join("ctx");
        std::fs::create_dir_all(&context).unwrap();
        std::fs::write(root.join("Dockerfile"), "FROM scratch\n").unwrap();
        let opts = BuildOptions {
            context_dir: context.to_string_lossy().into_owned(),
            dockerfile: "../Dockerfile".into(),
            tag: "app:dev".into(),
            ..Default::default()
        };
        assert_eq!(
            opts.validate().unwrap_err(),
            "Dockerfile must live inside the build context"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn validate_accepts_a_real_context_and_dockerfile() {
        let dir = temp_context("validate-ok");
        std::fs::write(dir.join("Dockerfile"), "FROM scratch\n").unwrap();
        let (context, dockerfile) = build_opts(&dir).validate().unwrap();
        assert!(context.is_dir());
        assert_eq!(dockerfile, "Dockerfile");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dockerfile_arg_rewrites_a_host_absolute_path_inside_the_context() {
        let context = std::path::Path::new("/tmp/ctx");
        let opts = BuildOptions {
            context_dir: "/tmp/ctx".into(),
            dockerfile: "/tmp/ctx/build/Dockerfile".into(),
            ..Default::default()
        };
        assert_eq!(opts.dockerfile_arg(context), "build/Dockerfile");
        assert_eq!(
            BuildOptions {
                dockerfile: String::new(),
                ..opts
            }
            .dockerfile_arg(context),
            "Dockerfile"
        );
    }

    #[test]
    fn endpoint_host_port_fills_the_default_tcp_port() {
        assert_eq!(endpoint_host_port("tcp", "host:2376"), "host:2376");
        assert_eq!(endpoint_host_port("tcp", "docker.example.com"), "docker.example.com:2375");
        assert_eq!(endpoint_host_port("tcp", "tcp://host:2375"), "host:2375");
        assert_eq!(endpoint_host_port("ssh", "user@host"), "user@host");
        assert_eq!(endpoint_host_port("ssh", "ssh://user@host"), "user@host");
    }

    #[test]
    fn endpoint_docker_host_only_exists_for_remotes() {
        assert_eq!(
            endpoint_docker_host("tcp", "10.0.0.5:2375"),
            Some("tcp://10.0.0.5:2375".to_string())
        );
        assert_eq!(
            endpoint_docker_host("ssh", "user@host"),
            Some("ssh://user@host".to_string())
        );
        // The local endpoint has no DOCKER_HOST override.
        assert_eq!(endpoint_docker_host("local", "unix:///run/docker.sock"), None);
    }

    #[test]
    fn endpoint_id_from_name_is_slugged_and_never_empty() {
        assert_eq!(endpoint_id_from_name("Prod Swarm"), "prod-swarm");
        assert_eq!(endpoint_id_from_name("  Build  Box  "), "build-box");
        assert_eq!(endpoint_id_from_name("!!!"), "endpoint");
    }

    #[test]
    fn validate_endpoint_rejects_a_scheme_from_the_other_kind() {
        assert!(validate_endpoint("tcp", "host:2375").is_ok());
        assert!(validate_endpoint("ssh", "user@host").is_ok());
        assert!(validate_endpoint("tcp", "ssh://user@host").is_err());
        assert!(validate_endpoint("ssh", "tcp://host:2375").is_err());
        assert!(validate_endpoint("tcp", "   ").is_err());
        assert!(validate_endpoint("carrier-pigeon", "host").is_err());
    }

    #[test]
    fn only_the_local_endpoint_offers_engine_controls() {
        let local = EndpointStatus::local("/run/user/1000/docker.sock");
        assert!(local.is_local());
        assert!(local.engine_controls_available());
        assert!(local.uri.ends_with("docker.sock"));

        let remote = EndpointStatus {
            is_active: true,
            ..local.clone()
        };
        let remote = EndpointStatus {
            id: "prod".into(),
            kind: "tcp".into(),
            ..remote
        };
        assert!(!remote.is_local());
        assert!(!remote.engine_controls_available());
    }

    #[test]
    fn shell_quote_only_quotes_what_the_shell_would_split() {
        assert_eq!(shell_quote("nginx:latest"), "nginx:latest");
        assert_eq!(shell_quote("build/app.Dockerfile"), "build/app.Dockerfile");
        assert_eq!(shell_quote("hello world"), "'hello world'");
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn limits_from_host_config_maps_engine_units_to_form_strings() {
        let host = bollard::models::HostConfig {
            memory: Some(536_870_912),
            nano_cpus: Some(1_500_000_000),
            cpuset_cpus: Some("0-2,4".to_string()),
            cpu_shares: Some(512),
            pids_limit: Some(128),
            ..Default::default()
        };
        let limits = limits_from_host_config(Some(&host));
        assert_eq!(limits.memory_mb, "512");
        assert_eq!(limits.cpus, "1.5");
        assert_eq!(limits.cpuset_cpus, "0-2,4");
        assert_eq!(limits.cpu_shares, "512");
        assert_eq!(limits.pids_limit, "128");
    }

    #[test]
    fn limits_from_host_config_unmapped_values_read_as_empty() {
        let host = bollard::models::HostConfig {
            memory: Some(0),
            nano_cpus: Some(0),
            cpu_quota: Some(-1),
            cpu_period: Some(100_000),
            cpu_shares: Some(0),
            pids_limit: Some(-1),
            cpuset_cpus: Some("  ".to_string()),
            ..Default::default()
        };
        let limits = limits_from_host_config(Some(&host));
        assert!(limits.is_empty(), "{limits:?}");
        assert_eq!(limits, ResourceLimits::default());
        assert!(limits_from_host_config(None).is_empty());
    }

    #[test]
    fn limits_from_host_config_falls_back_to_quota_period() {
        let host = bollard::models::HostConfig {
            cpu_quota: Some(150_000),
            cpu_period: Some(100_000),
            ..Default::default()
        };
        let limits = limits_from_host_config(Some(&host));
        assert_eq!(limits.cpus, "1.5");
    }

    #[test]
    fn update_body_maps_set_limits() {
        let limits = ResourceLimits {
            memory_mb: "512".to_string(),
            cpus: "1.5".to_string(),
            cpuset_cpus: "0-1".to_string(),
            cpu_shares: "1024".to_string(),
            pids_limit: "256".to_string(),
            ..Default::default()
        };
        let body = update_body_from_limits(&limits).unwrap();
        assert_eq!(body.memory, Some(536_870_912));
        assert_eq!(body.nano_cpus, Some(1_500_000_000));
        assert_eq!(body.cpu_shares, Some(1024));
        assert_eq!(body.cpuset_cpus.as_deref(), Some("0-1"));
        assert_eq!(body.pids_limit, Some(256));
    }

    #[test]
    fn update_body_omits_unset_fields_and_rejects_nothing_to_do() {
        let limits = ResourceLimits {
            cpus: "2".to_string(),
            ..Default::default()
        };
        let body = update_body_from_limits(&limits).unwrap();
        assert_eq!(body.memory, None);
        assert_eq!(body.cpu_shares, None);
        assert_eq!(body.cpuset_cpus, None);
        assert_eq!(body.nano_cpus, Some(2_000_000_000));

        // An all-empty payload is refused instead of hitting the engine.
        let err = update_body_from_limits(&ResourceLimits::default()).unwrap_err();
        assert!(err.contains("No resource limits"), "{err}");
    }

    #[test]
    fn update_body_rejects_invalid_limits() {
        // Below the 6 MiB floor enforced by ResourceLimits::validate.
        let err = update_body_from_limits(&ResourceLimits {
            memory_mb: "2".to_string(),
            ..Default::default()
        })
        .unwrap_err();
        assert!(err.contains("Memory limit"), "{err}");
    }

    #[test]
    fn update_body_maps_restart_policy() {
        let body = update_body_from_limits(&ResourceLimits {
            restart_policy: "unless-stopped".to_string(),
            ..Default::default()
        })
        .expect("policy-only update is valid");
        let rp = body.restart_policy.expect("policy set");
        assert_eq!(
            rp.name,
            Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED)
        );
        assert_eq!(rp.maximum_retry_count, None);

        let body = update_body_from_limits(&ResourceLimits {
            restart_policy: "on-failure".to_string(),
            restart_max_retries: "5".to_string(),
            ..Default::default()
        })
        .expect("on-failure with retries");
        let rp = body.restart_policy.expect("policy set");
        assert_eq!(
            rp.name,
            Some(bollard::models::RestartPolicyNameEnum::ON_FAILURE)
        );
        assert_eq!(rp.maximum_retry_count, Some(5));

        // Unknown policy strings are rejected rather than silently becoming "no".
        assert!(update_body_from_limits(&ResourceLimits {
            restart_policy: "sometimes".to_string(),
            ..Default::default()
        })
        .is_err());
        // Retries only make sense with on-failure.
        assert!(update_body_from_limits(&ResourceLimits {
            restart_policy: "always".to_string(),
            restart_max_retries: "3".to_string(),
            ..Default::default()
        })
        .is_err());
        // Unset policy leaves the body's policy absent ("leave unchanged").
        let body = update_body_from_limits(&ResourceLimits {
            cpus: "1".to_string(),
            ..Default::default()
        })
        .expect("limits update");
        assert!(body.restart_policy.is_none());
    }

    #[test]
    fn restart_policy_only_update_is_not_empty() {
        let limits = ResourceLimits {
            restart_policy: "no".to_string(),
            ..Default::default()
        };
        assert!(!limits.is_empty());
        assert!(update_body_from_limits(&limits).is_ok());
    }

    #[test]
    fn inspect_restart_policy_names_round_trip() {
        use bollard::models::RestartPolicyNameEnum as B;
        assert_eq!(RestartPolicy::from_bollard_name(&B::ALWAYS), "always");
        assert_eq!(
            RestartPolicy::from_bollard_name(&B::UNLESS_STOPPED),
            "unless-stopped"
        );
        assert_eq!(
            RestartPolicy::from_bollard_name(&B::ON_FAILURE),
            "on-failure"
        );
        assert_eq!(RestartPolicy::from_bollard_name(&B::NO), "no");
        assert_eq!(RestartPolicy::from_bollard_name(&B::EMPTY), "no");
        // And every canonical string parses back through update.
        for s in ["no", "always", "unless-stopped", "on-failure"] {
            assert!(update_body_from_limits(&ResourceLimits {
                restart_policy: s.to_string(),
                ..Default::default()
            })
            .is_ok());
        }
    }
}