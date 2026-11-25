export type DaemonState =
  | "Stopped"
  | "Starting"
  | "Stopping"
  | "Running"
  | "NotInstalled"
  | "Installing"
  | { Error: string };

export interface ContainerPort {
  ip?: string;
  private_port: number;
  public_port?: number;
  type: string;
}

/** One host binding as reported by the engine (`ContainerSummary.ports`). */
export interface PublishedPort {
  host_ip: string;
  host_port: number;
  container_port: number;
  protocol: string;
}

export interface ContainerItem {
  id: string;
  short_id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  created: number;
  ports: string;
  /**
   * The same bindings as `ports`, structured. `host_ip` is the address the
   * port is published on; `""` and the wildcard addresses mean every
   * interface — the case that bypasses the host firewall.
   */
  published_ports?: PublishedPort[];
  /** Compose project name from the container's labels; null for plain containers. */
  compose_project?: string | null;
  /** Compose service name from the container's labels. */
  compose_service?: string | null;
  /** Kind cluster this node container belongs to; null for ordinary containers. */
  kind_cluster?: string | null;
  /** Networks this container is connected to. */
  networks?: string[];
  /** Broken network names whose NetworkID does not exist on the daemon. */
  broken_networks?: string[];
}

/** A user-defined container group: bulk start/stop + optional engine-start auto-run. */
export interface ContainerGroup {
  id: string;
  name: string;
  container_ids: string[];
  auto_start: boolean;
}

export interface ContainerStats {
  cpu_percent: number;
  mem_usage: u64_bytes;
  mem_limit: u64_bytes;
  net_rx: u64_bytes;
  net_tx: u64_bytes;
}

export type u64_bytes = number;

export interface ContainerDetails {
  id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  created: string;
  ip_address: string;
  ports: string[];
  mounts: string[];
  env: string[];
  command: string;
  raw_json: string;
  limits: ResourceLimits;
  /** Current restart policy as inspect reports it ("" when unknown). */
  restart_policy?: string;
  /** Structured fields from Rust ContainerDetails */
  short_id?: string;
  command_vec?: string[];
  entrypoint?: string[];
  port_mappings?: PortMapping[];
  env_pairs?: EnvVar[];
  volume_mounts?: VolumeMount[];
  labels?: EnvVar[];
  hostname?: string;
  workdir?: string;
  user?: string;
  network?: string;
  resources?: ResourceLimits;
  restart_max_retries?: string;
  privileged?: boolean;
  tty?: boolean;
  stdin_open?: boolean;
  auto_remove?: boolean;
  read_only_rootfs?: boolean;
}

export interface ImageItem {
  id: string;
  short_id: string;
  tags: string[];
  size: number;
  created: number;
}

export interface ImageHistoryEntry {
  id: string;
  short_id: string;
  created: number;
  created_by: string;
  size: number;
  comment: string;
}

export interface SearchResult {
  name: string;
  description: string;
  star_count: number;
}

export interface PullProgressPayload {
  image: string;
  status: string;
  progress: string | null;
  current: number | null;
  total: number | null;
  done: boolean;
  error: string | null;
}

export interface VolumeItem {
  name: string;
  driver: string;
  mountpoint: string;
  size?: number;
  /** Number of subtrees skipped due to restricted permissions. */
  restricted_dirs?: number;
  /** Managed by kind: deleting or pruning it is refused by the backend. */
  protected?: boolean;
  /** True if currently mounted by at least one container. */
  in_use?: boolean;
}

export interface NetworkItem {
  id: string;
  name: string;
  driver: string;
  subnet: string;
  gateway: string;
  /** kind's network: protected from removal and pruning. */
  protected?: boolean;
}

export interface ComposeService {
  id: string;
  name: string;
  service: string;
  image: string;
  state: string;
  health: string;
  ports: string;
}

export interface ComposePortSpec {
  published: string;
  target: number;
  protocol: string;
}

/** A service as the compose file declares it (`compose config`). */
export interface ComposeServiceSpec {
  name: string;
  image: string;
  build: boolean;
  depends_on: string[];
  ports: ComposePortSpec[];
  profiles: string[];
  healthcheck: boolean;
  replicas: number | null;
  container_name: string;
}

export interface ComposeModel {
  name: string;
  services: ComposeServiceSpec[];
  profiles: string[];
}

export type ComposeProjectSource = "settings" | "pinned" | "labels";

export interface ComposeProjectInfo {
  name: string;
  /** Compose file path; empty when unknown. */
  path: string;
  /** One row per container of the project. */
  services: ComposeService[];
  source?: ComposeProjectSource;
  pinned?: boolean;
  working_dir?: string;
  declared?: ComposeServiceSpec[];
  profiles?: string[];
  /** The project's default `.env`, when it exists. */
  env_file?: string;
  config_error?: string;
}

export interface ComposeUpOptions {
  profiles?: string[];
  env_file?: string | null;
  force_recreate?: boolean;
  remove_orphans?: boolean;
  services?: string[];
}

export interface ComposeDownOptions {
  remove_volumes?: boolean;
  remove_orphans?: boolean;
}

export type ComposeServiceAction = "start" | "stop" | "restart" | "up" | "recreate" | "scale";

export interface ComposeValidation {
  ok: boolean;
  error: string;
  model: ComposeModel | null;
}

export interface ComposeEnvFile {
  path: string;
  exists: boolean;
  content: string;
}

export interface ComposeProgressPayload {
  op_id: string;
  line: string;
}

export interface DfReport {
  images_size: number;
  containers_size: number;
  volumes_size: number;
  build_cache_size: number;
  total_reclaimable: number;
}

export interface EngineInfo {
  version: string;
  api_version?: string;
  kernel?: string;
  root_dir?: string;
  cgroup_driver?: string;
  containers: number;
  containers_running: number;
  containers_paused: number;
  containers_stopped: number;
  images: number;
  driver: string;
  os: string;
  arch: string;
  cpus: number;
  memory_total: number;
}

export interface AppConfig {
  auto_start_daemon: boolean;
  compose_file: string;
  cli_bridge_enabled: boolean;
  cli_bridge_set_default: boolean;
  cli_bridge_prev_context: string;
  data_root?: string;
  theme?: string;
  /** Close the window to the tray instead of quitting; the engine keeps running. */
  close_to_tray?: boolean;
  /** Register an autostart entry so the app comes up at login. */
  auto_start_on_boot?: boolean;
  /** Boot launches stay in the tray. Only meaningful with `auto_start_on_boot`. */
  start_minimized?: boolean;
  /** Which docker CLI to run: `"auto"` (newer wins) | `"system"` | `"bundled"`. */
  cli_mode: string;
  /** Whether `~/.local/bin/docker` was installed to expose docker in a terminal. */
  cli_shim_enabled: boolean;
  schema_version: number;
  /** Shell opened by the container terminal and the Debug action. */
  terminal_shell?: string;
  /** Font size (px) used by terminal panes. */
  terminal_font_size?: number;
  /** Desktop-notification toggles managed by the tray (see `NotificationPrefs`). */
  notifications?: NotificationPrefs;
  /** Default node count pre-filled in the kind "Create cluster" form. */
  kind_default_nodes?: number;
  /** Default node image pre-filled in the kind "Create cluster" form ("" = kind's own default). */
  kind_default_image?: string;
  /** Saved container groups (bulk start/stop + optional auto-start). */
  container_groups?: ContainerGroup[];
  /** Engine memory cap in MiB applied to the engine cgroup (0 = unlimited). */
  engine_memory_limit_mb?: number;
  /** Engine CPU cap in cores as a string, e.g. "2.5" ("" = unlimited). */
  engine_cpu_cores?: string;
  /** Master switch: auto-start kind clusters with the engine. */
  kind_auto_start?: boolean;
  /** Seconds between cluster starts (stagger). */
  kind_auto_start_delay?: number;
  /** Per-cluster prefs keyed by cluster name (camelCase values). */
  kind_cluster_prefs?: Record<string, KindClusterPrefs>;
  /** Id of the login new pushes default to; empty = first saved login. */
  default_registry?: string;
  /** Serve the MCP front-end over loopback HTTP in addition to stdio. */
  mcp_enabled?: boolean;
  /** Loopback port for the MCP HTTP transport. */
  mcp_port?: number;
  /** Automatically prune dangling / untagged images. */
  auto_prune_dangling_images?: boolean;
  /** Automatically prune isolated / unused volumes. */
  auto_prune_isolated_volumes?: boolean;
}

/**
 * What the backend reports about the MCP front-end (`mcp_status`).
 *
 * Deliberately carries no token: it is only ever fetched on demand through
 * `api.mcpToken()`, and lives in `~/.config/hsx2dock/mcp.json` with 0600.
 */
export interface McpStatus {
  enabled: boolean;
  running: boolean;
  url: string;
  port: number;
  hasToken: boolean;
  tokenPath: string;
  stdioPath: string;
  toolCount: number;
  /** Only read tools may be called (and are listed). */
  readOnly: boolean;
  resourceCount: number;
  promptCount: number;
  serverName: string;
  serverVersion: string;
}

/** One tool as the server advertises it (`mcp_catalogue`). */
export interface McpToolInfo {
  name: string;
  title: string;
  description: string;
  /** `"read"` | `"write"` | `"destructive"`. */
  access: string;
  /** Refused while a remote endpoint is active. */
  engineOnly: boolean;
}

/** One resource (`mcp_resources`). */
export interface McpResourceInfo {
  uri: string;
  title: string;
  description: string;
  mimeType: string;
}

/** One prompt argument. */
export interface McpPromptArgInfo {
  name: string;
  description: string;
  required: boolean;
}

/** One prompt (`mcp_prompts`). */
export interface McpPromptInfo {
  name: string;
  title: string;
  description: string;
  arguments: McpPromptArgInfo[];
}

/** Per-feature desktop-notification toggles (tray menu + Settings card). */
export interface NotificationPrefs {
  /** Image build finished (success or failure). */
  build_done: boolean;
  /** Image pull/push finished. */
  transfer_done: boolean;
  /** Engine started / stopped / died unexpectedly. */
  engine_state: boolean;
  /** kind cluster created / deleted. */
  kind_events: boolean;
  /** The window was closed into the tray (app + engine keep running). */
  tray_hidden: boolean;
  /** Kubernetes incident alerts (cluster watcher). */
  k8s_alerts?: K8sAlertPrefs;
}

/** `exec-output` payload streamed while an interactive exec session runs. */
export interface ExecOutputPayload {
  session_id: string;
  /** Base64 chunk of raw terminal output; `null` once the stream is done. */
  data: string | null;
  /** True once the exec stream has closed. */
  eof: boolean;
}

export interface CliBridgeStatus {
  enabled: boolean;
  link_state: boolean | null;
}

/** Result of checking Docker's official channel against the installed engine. */
export interface EngineUpdateInfo {
  /** Version currently installed, `null` when there is no engine yet. */
  installed_version: string | null;
  latest_version: string;
  update_available: boolean;
  /** Version the app ships pinned checksums for. */
  pinned_version: string;
  channel: string;
}

/** Which binary a progress event belongs to. */
export type UpdateTarget = "engine" | "cli" | "compose" | "kind" | "caddy";

/**
 * `engine-install-progress` payload streamed while any bundled binary updates.
 *
 * `plan` is the ordered checklist for the run (step names), `step` indexes it,
 * and `log` carries lines to append to the run console. `done` is set exactly
 * once, with `error` when the run failed.
 */
export interface InstallProgress {
  target: UpdateTarget;
  version: string;
  label: string;
  fraction: number | null;
  plan: string[];
  step: number | null;
  detail?: string | null;
  log?: string | null;
  done?: boolean;
  error?: string | null;
}

/** Kept as an alias: progress is shared by every updater now. */
export type EngineInstallProgress = InstallProgress;

/** Result of checking the bundled `docker` CLI against the static channel. */
export interface CliUpdateInfo {
  installed_version: string | null;
  latest_version: string;
  update_available: boolean;
  /** Absolute path of the bundled CLI an update would replace. */
  path: string;
}

/** Result of checking the kind GitHub release channel. */
export interface KindUpdateInfo {
  installed_version: string | null;
  latest_version: string;
  update_available: boolean;
}

/** Which docker binary the app decided to run. */
export type CliSource = "system" | "bundled";

/**
 * Result of resolving the docker CLI against the host.
 *
 * Both sides are always reported, including the losing one, so the settings
 * card can show what was compared rather than just the winner.
 */
export interface CliResolution {
  source: CliSource;
  path: string;
  version: string;
  system_path: string | null;
  system_version: string | null;
  bundled_path: string;
  bundled_version: string | null;
  /** One sentence naming the winner and why. */
  reason: string;
}

/** Whether the `~/.local/bin/docker` shim will actually win in the terminal. */
export type ShimEffect = "active" | "not_on_path" | "shadowed";

/** State of the opt-in `~/.local/bin/docker` shim. */
export interface CliShimStatus {
  installed: boolean;
  path: string | null;
  /** `null` if no shim installed, else how it will resolve relative to PATH. */
  effect: ShimEffect | null;
  /** A system docker that the shim would override, shown as a warning. */
  shadows: string | null;
  /** The binary the shim currently execs. */
  target: string | null;
  /** True when `target` is no longer what the resolver would pick. */
  stale: boolean;
}

/** One round trip gives the settings card everything it shows. */
export interface CliStatus {
  resolution: CliResolution;
  shim: CliShimStatus;
}

/** Compose plugin is released on its own channel, separate from the engine. */
export interface ComposeUpdateInfo {
  installed_version: string | null;
  latest_version: string;
  update_available: boolean;
  pinned_version: string;
}

/** How one engine check (or one containerd plugin) is doing. */
export type EngineHealth = "ok" | "warning" | "error" | "missing";

/** One diagnosed engine condition. `id` is what `repairEngine` takes back. */
export interface EngineCheck {
  id: string;
  label: string;
  health: EngineHealth;
  /** One actionable line — never a raw error dump. */
  detail: string;
  /** Filesystem path the check looks at, when there is one. */
  path: string | null;
  /** True when a repair exists that does not lose user data. */
  repairable: boolean;
  /** True when the engine applies this repair automatically on every start. */
  auto: boolean;
}

/** One containerd plugin as reported by the engine log. */
export interface ContainerdPlugin {
  id: string;
  health: EngineHealth;
  detail: string;
}

/** Everything the Settings → Engine health card renders. */
export interface EngineDoctorReport {
  /** Worst health across every check — drives the card's colour. */
  health: EngineHealth;
  checks: EngineCheck[];
  /** Empty until the engine has run at least once. */
  plugins: ContainerdPlugin[];
  data_root: string;
  engine_running: boolean;
}

/** What a repair run changed. */
export interface RepairReport {
  /** Check ids that were fixed. */
  repaired: string[];
  /** `[check id, reason]` for checks that could not be fixed. */
  failed: [string, string][];
}

export interface PortMapping {
  host_ip?: string;
  host_port?: string;
  container_port: string;
  protocol?: string;
}

export interface EnvVar {
  key: string;
  value: string;
}

export interface VolumeMount {
  kind?: "bind" | "volume";
  source: string;
  target: string;
  read_only?: boolean;
}

export interface ResourceLimits {
  memory_mb?: string;
  cpus?: string;
  cpuset_cpus?: string;
  cpu_shares?: string;
  pids_limit?: string;
  /** Edit-only override: "" = leave unchanged, else no/always/unless-stopped/on-failure. */
  restart_policy?: string;
  /** Restart attempts for the on-failure policy; "" = engine default. */
  restart_max_retries?: string;
}

export interface Healthcheck {
  test?: string[];
  interval_s?: string;
  timeout_s?: string;
  retries?: string;
  start_period_s?: string;
}

export interface RunConfig {
  image: string;
  name?: string;
  hostname?: string;
  command?: string[];
  entrypoint?: string[];
  workdir?: string;
  user?: string;
  network?: string;
  ports?: PortMapping[];
  env?: EnvVar[];
  volumes?: VolumeMount[];
  labels?: Record<string, string>;
  cap_add?: string[];
  cap_drop?: string[];
  extra_hosts?: string[];
  restart_policy?: string;
  restart_max_retries?: string;
  resources?: ResourceLimits;
  healthcheck?: Healthcheck;
  privileged?: boolean;
  tty?: boolean;
  stdin_open?: boolean;
  auto_remove?: boolean;
  read_only_rootfs?: boolean;
  detach?: boolean;
}

// ---------------------------------------------------------------------------
// Image build
// ---------------------------------------------------------------------------

export interface BuildArg {
  key: string;
  value: string;
}

/** Mirrors the Rust `BuildOptions` request. */
export interface BuildRequest {
  context_dir: string;
  dockerfile: string;
  tag: string;
  target?: string | null;
  no_cache: boolean;
  pull: boolean;
  build_args: BuildArg[];
}

export interface BuildProgressPayload {
  tag: string;
  status: string;
  log: string | null;
  current: number | null;
  total: number | null;
  error: string | null;
  done: boolean;
  cancelled: boolean;
  image_id: string | null;
}

/** What the backend learned about a chosen build context. */
export interface BuildContextInfo {
  exists: boolean;
  is_dir: boolean;
  entries: string[];
  dockerfiles: string[];
  suggested_tag: string;
}

// ---------------------------------------------------------------------------
// Remote engine endpoints
// ---------------------------------------------------------------------------

export type EndpointKind = "local" | "tcp" | "ssh";

export interface EndpointStatus {
  id: string;
  name: string;
  kind: string;
  uri: string;
  is_active: boolean;
  reachable: boolean;
  version: string | null;
  latency_ms: number | null;
  error: string | null;
}

export interface EndpointInput {
  id?: string | null;
  name: string;
  kind: string;
  uri: string;
}

/** A saved registry login (Docker Hub, ghcr, GitLab, Harbor, custom). */
export interface RegistryStatus {
  id: string;
  name: string;
  host: string;
  username: string;
  insecure: boolean;
  /** Plain-HTTP registry: the engine gets `--insecure-registry` for it. */
  allow_http: boolean;
  /** Whether a secret is stored — never the secret itself. */
  has_password: boolean;
  /** True for the login new pushes default to. */
  is_default: boolean;
}

export interface RegistryInput {
  id?: string | null;
  name: string;
  host: string;
  username: string;
  /** Plain password/token; empty on edit keeps the stored secret. */
  password: string;
  insecure: boolean;
  allow_http: boolean;
}

export interface RegistryTestResult {
  ok: boolean;
  message: string;
  latency_ms: number;
}

/** `push-progress` payload streamed while an image pushes. */
export interface PushProgressPayload {
  image: string;
  status: string;
  progress: string | null;
  current: number | null;
  total: number | null;
  done: boolean;
  error: string | null;
}

/** One kind (Kubernetes-in-Docker) cluster, grouped from its node containers. */
export interface KindCluster {
  name: string;
  running: boolean;
  node_count: number;
  running_nodes: number;
  /** Unix seconds of the oldest node container, if known. */
  created: number | null;
  has_kubeconfig: boolean;
  /** Seconds since the oldest running node started. */
  uptime_seconds: number | null;
  /** Unix seconds of the most recent node start, if known. */
  last_started_at: number | null;
  /** Total pods across all namespaces, if the control-plane responded. */
  pod_count: number | null;
  /** Pods in `Running` phase. */
  running_pod_count: number | null;
  /** Whether the API server answers /readyz (null while the cluster is stopped). */
  api_ready: boolean | null;
  /** Cluster CPU usage summed across nodes, as % of the host. */
  cpu_percent: number | null;
  /** Cluster memory usage summed across nodes, in MiB. */
  mem_used_mb: number | null;
  /** Applied per-node CPU cap summed across nodes (millicores), if capped. */
  cpu_quota_m: number | null;
  /** Applied per-node memory cap summed across nodes (MiB), if capped. */
  mem_limit_mb: number | null;
  /** Total host memory in MiB; denominator for the Overview "of host" tile. */
  host_mem_total_mb: number | null;
  /** Total writable layer disk size across all node containers, in bytes. */
  disk_size_bytes?: number | null;
  /** Host port bindings exposed by this cluster's nodes. */
  published_ports?: PublishedPort[];
}

/** Install state of the app-managed (isolated) kind binary. */
export interface KindInstallStatus {
  installed: boolean;
  version: string | null;
  latest: string;
  path: string | null;
  /** False on platforms with no published kind binary. */
  supported: boolean;
}

export interface EmbeddedBinaryInfo {
  name: string;
  category: string;
  version: string | null;
  path: string | null;
  is_active: boolean;
  description: string;
}

export interface CaddyUpdateInfo {
  installed_version: string | null;
  latest_version: string;
  update_available: boolean;
}

export interface CaddyInstallStatus {
  installed: boolean;
  version: string | null;
  latest: string;
  path: string | null;
  running: boolean;
  admin_url: string;
  http_port: number;
  https_port: number;
  trusted: boolean;
  hosts_synced: boolean;
  auto_start: boolean;
}

export interface GatewayRoute {
  id: string;
  domain: string;
  target: string;
  path?: string | null;
  strip_prefix?: boolean;
  container_name?: string | null;
  group_name?: string | null;
  enabled: boolean;
  auto_detected: boolean;
}

export interface HostsSyncStatus {
  managed_domains: string[];
  in_hosts_domains: string[];
  synced: boolean;
  tampered: boolean;
  managed_block_present: boolean;
}

export interface HostsEntryItem {
  id: string;
  ip: string;
  hosts: string[];
  comment?: string | null;
  is_enabled: boolean;
  is_managed_by_app: boolean;
  is_syntax_valid: boolean;
  syntax_error?: string | null;
  raw_line: string;
}

export interface HostsFileModel {
  entries: HostsEntryItem[];
  raw_content: string;
  can_write_direct: boolean;
  managed_block_present: boolean;
  tampered: boolean;
  total_lines: number;
}

/** A streamed line from a kind subprocess (`kind-progress` event). */
export interface KindProgressPayload {
  action?: string;
  name?: string;
  line: string;
}

// ---- Cluster monitoring (kubectl inside the kind control-plane node) ----

export interface K8sNodeInfo {
  name: string;
  ready: boolean;
  status: string;
  roles: string[];
  kubeletVersion: string;
  osImage: string;
  containerRuntime: string;
  internalIp: string;
  cpuCapacityM: number;
  memCapacityBytes: number;
  podsCapacity: number;
  /** `spec.unschedulable` — true after a cordon. */
  unschedulable: boolean;
  created: number | null;
}

export interface K8sPodInfo {
  name: string;
  namespace: string;
  phase: string;
  readyContainers: number;
  totalContainers: number;
  restarts: number;
  node: string;
  images: string[];
  created: number | null;
  reason: string | null;
}

export interface K8sEventInfo {
  type: string;
  reason: string;
  message: string;
  object: string;
  namespace: string;
  count: number;
  lastTimestamp: number | null;
}

export interface K8sWorkloadInfo {
  kind: string;
  name: string;
  namespace: string;
  ready: number;
  desired: number;
  available: number;
}

export interface K8sNodeMetric {
  name: string;
  cpuM: number;
  memBytes: number;
}

export interface K8sPodMetric {
  namespace: string;
  name: string;
  cpuM: number;
  memBytes: number;
}

export interface K8sMetrics {
  nodes: K8sNodeMetric[];
  pods: K8sPodMetric[];
  /** Set when `kubectl top` failed (metrics-server not installed). */
  error: string | null;
}

/** One metrics-server pod, as reported by the cluster. */
export interface MetricsPodStatus {
  name: string;
  phase: string;
  ready: boolean;
  restarts: number;
  reason: string | null;
}

/**
 * Real metrics-server state. `installed` (the deployment exists) and
 * `topWorks` (kubectl top answers) diverge while it warms up — the UI keeps
 * them apart so a started install never reads as "not installed".
 */
export interface MetricsServerStatus {
  installed: boolean;
  desired_replicas: number;
  available_replicas: number;
  ready: boolean;
  kubelet_insecure_tls: boolean;
  pods: MetricsPodStatus[];
  top_works: boolean;
  /** `namespace/name` of the workload backing the metrics API, if found. */
  workload: string | null;
  /** `v1beta1.metrics.k8s.io` is registered — proves the install exists. */
  apiservice_found: boolean;
  /** Available condition of that APIService; null when not registered. */
  apiservice_available: boolean | null;
  message: string | null;
}

export interface K8sClusterSummary {
  gitVersion: string;
  platform: string;
  ready: boolean;
  readyMessage: string | null;
  nodeCount: number;
  readyNodes: number;
  namespaceCount: number;
  podRunning: number;
  podPending: number;
  podFailed: number;
  podSucceeded: number;
  /** Failures of individual sub-queries (pods/nodes/ns) that produced zeros. */
  errors: string[];
}

export interface K8sServiceInfo {
  name: string;
  namespace: string;
  type: string;
  clusterIp: string;
  ports: string;
  /** `spec.selector` as `k=v,k2=v2`; "" when the service has none. */
  selector: string;
  created: number | null;
}

export interface K8sConfigInfo {
  name: string;
  namespace: string;
  /** "ConfigMap" or "Secret". */
  resourceKind: string;
  keys: string[];
  created: number | null;
}

export interface K8sPvcInfo {
  name: string;
  namespace: string;
  status: string;
  capacity: string;
  accessModes: string;
  storageClass: string;
  created: number | null;
}

export interface K8sNamespaceInfo {
  name: string;
  status: string;
  created: number | null;
}

export interface K8sIngressInfo {
  name: string;
  namespace: string;
  className: string;
  /** Comma-separated host rules (`*` when a rule has no host). */
  hosts: string;
  /** Load-balancer address assigned by the controller, if any. */
  address: string;
  tls: boolean;
  created: number | null;
}

export interface K8sPvInfo {
  name: string;
  capacity: string;
  accessModes: string;
  reclaimPolicy: string;
  status: string;
  storageClass: string;
  /** `namespace/name` of the bound claim ("" when unbound). */
  claim: string;
  created: number | null;
}

/** Per-cluster prefs persisted in the app config. */
export interface KindClusterPrefs {
  /** Start together with the engine (staggered, capped). */
  autoStart: boolean;
  /** Per-node CPU cap in millicores (0 = unlimited). */
  nodeCpuM: number;
  /** Per-node memory cap in MiB (0 = unlimited). */
  nodeMemoryMb: number;
  /** Card accent colour `#rrggbb` ("" = default). */
  color?: string;
  /** Free-form labels for filtering the cluster list (max 8). */
  tags?: string[];
}

/** How a kind cluster is reached from the host (`get_cluster_access`). */
export interface ClusterAccess {
  /** `kind-<cluster>`: context, cluster and user name in kubeconfigs. */
  context: string;
  /** Host port of the API server. */
  apiPort: number | null;
  /** A TCP connect from the host to the API port succeeded. */
  apiReachable: boolean;
  /** Port is in the never-forwarded ephemeral range: recreate the cluster. */
  ephemeralPort: boolean;
  /** Path of the user's kubeconfig ($KUBECONFIG or ~/.kube/config). */
  hostKubeconfig: string;
  inHostKubeconfig: boolean;
  /** The user's current context is this cluster. */
  hostCurrent: boolean;
  /** kubectl is on the host PATH. */
  hostKubectl: boolean;
}

/** `kubectl diff` of a manifest against the live cluster. */
export interface ManifestDiff {
  /** Unified diff ("" when nothing would change). */
  diff: string;
  changed: boolean;
}

/** Logs of every pod behind a label selector. */
export interface SelectorLogs {
  /** Selector used (resolved from the workload when one was given). */
  selector: string;
  /** Lines `<RFC3339 ts> [pod/<pod>/<container>] <msg>`, oldest first. */
  lines: string;
}

/** Phase of a container lifecycle operation tracked by the backend. */
export type OpPhase = "queued" | "running" | "done" | "failed";

/** `container-op` event: one container's start/stop/restart progress. */
export interface ContainerOp {
  id: string;
  op: "start" | "stop" | "restart" | string;
  phase: OpPhase;
  group?: string | null;
  error?: string | null;
}

/** `group-op` event: a bulk group start/stop pass. */
export interface GroupOp {
  group: string;
  op: "start" | "stop" | string;
  phase: OpPhase;
  total: number;
  completed: number;
  summary?: string | null;
}

export interface ContainerOpsSnapshot {
  containers: ContainerOp[];
  groups: GroupOp[];
}

// ---------------------------------------------------------- advanced create

export interface PortMappingSpec {
  containerPort: number;
  hostPort: number;
  protocol: "TCP" | "UDP" | "SCTP";
}

export interface HostMountSpec {
  hostPath: string;
  containerPath: string;
  readOnly: boolean;
}

/** Mirrors `kind_spec::ClusterSpec` (serde camelCase). */
export interface ClusterSpec {
  image: string | null;
  controlPlanes: number;
  workers: number;
  ingress: boolean;
  ingressHttpPort: number;
  ingressHttpsPort: number;
  portMappings: PortMappingSpec[];
  mounts: HostMountSpec[];
  podSubnet: string | null;
  serviceSubnet: string | null;
  featureGates: string[];
  localRegistry: boolean;
}

export interface ClusterConfigPreview {
  yaml: string;
  finalYaml: string | null;
  nodes: number;
  errors: string[];
  warnings: string[];
}

export interface NodeImageOption {
  image: string;
  version: string;
  local: boolean;
  default: boolean;
}

export interface LocalRegistryStatus {
  exists: boolean;
  running: boolean;
  host: string;
  image: string;
  connectedClusters: string[];
  repositories: string[];
}

export interface ClusterPreset {
  name: string;
  description: string;
  builtin: boolean;
  spec: ClusterSpec;
}

// ---------------------------------------------------------------- Phase 3–5

export interface K8sToolStatus {
  name: string;
  version: string;
  installed: boolean;
  path: string;
}

export interface PortForwardSpec {
  cluster: string;
  namespace: string;
  /** svc | pod | deployment */
  kind: "svc" | "pod" | "deployment";
  name: string;
  remotePort: number;
  /** 0 = pick a free port. */
  localPort?: number;
  autoStart?: boolean;
}

export interface PortForwardInfo extends Required<PortForwardSpec> {
  id: string;
  /** starting | active | reconnecting | stopped | error */
  status: string;
  message: string | null;
  restarts: number;
  url: string;
}

export interface AddonStatus {
  id: string;
  name: string;
  description: string;
  version: string;
  /** not-installed | installing | uninstalling | progressing | ready | error */
  state: string;
  message: string | null;
  detail: string | null;
}

export interface AddonProgressPayload {
  cluster: string;
  addon: string;
  line: string;
  done: boolean;
  error: string | null;
}

export interface IngressUrl {
  host: string;
  url: string | null;
  exposed: boolean;
  note: string | null;
}

export interface IngressInfo {
  namespace: string;
  name: string;
  class: string | null;
  hosts: string[];
  tls: boolean;
  urls: IngressUrl[];
}

export interface ServiceLinkRequest {
  sourceCluster: string;
  sourceNamespace: string;
  sourceService: string;
  targetCluster: string;
  targetNamespace: string;
  name?: string | null;
}

export interface ServiceLinkPort {
  name: string;
  port: number;
  remotePort: number;
  protocol: string;
}

export interface ServiceLink {
  namespace: string;
  name: string;
  /** cluster/namespace/service */
  source: string;
  address: string;
  ports: ServiceLinkPort[];
  mode: string;
}

export interface ServiceLinkCheck {
  port: number;
  ok: boolean;
  message: string;
}

export interface HelmRelease {
  name: string;
  namespace: string;
  revision: string;
  updated: string;
  status: string;
  chart: string;
  app_version: string;
}

export interface HelmRevision {
  revision: number;
  updated: string;
  status: string;
  chart: string;
  app_version: string;
  description: string;
}

export interface HelmReleaseDetail {
  values: string;
  allValues: string;
  manifest: string;
  notes: string;
  history: HelmRevision[];
}

export interface HelmInstallRequest {
  cluster: string;
  namespace: string;
  name: string;
  chart: string;
  version?: string | null;
  values?: string | null;
  upgrade?: boolean;
  reuseValues?: boolean;
}

export interface HelmRepo {
  name: string;
  url: string;
}

export interface HelmChart {
  name: string;
  version: string;
  app_version: string;
  description: string;
}

export interface ClusterSnapshot {
  id: string;
  cluster: string;
  created: string;
  note: string | null;
  nodeImage: string;
  nodes: string[];
  sizeBytes: number;
}

export interface SnapshotProgressPayload {
  cluster: string;
  line: string;
  done: boolean;
  error: string | null;
}

export type ClusterAlertKind =
  | "crash-loop"
  | "image-pull"
  | "oom"
  | "restart-spike"
  | "node-not-ready"
  | "api-down"
  | "pvc-pending";

export interface ClusterAlert {
  key: string;
  cluster: string;
  kind: ClusterAlertKind;
  severity: "error" | "warning";
  namespace: string | null;
  object: string;
  message: string;
  /** Unix seconds when first seen. */
  since: number;
}

export interface ClusterAlertsEvent {
  cluster: string;
  alerts: ClusterAlert[];
  fresh: ClusterAlert[];
}

export interface K8sAlertPrefs {
  enabled: boolean;
  crash_loop: boolean;
  image_pull: boolean;
  oom: boolean;
  restart_spike: boolean;
  node_not_ready: boolean;
  api_down: boolean;
  pvc_pending: boolean;
  muted_clusters: string[];
}

export type HistoryRange = "1h" | "6h" | "24h" | "7d";

export interface HistorySample {
  /** Unix seconds. */
  t: number;
  cpuPercent: number;
  memMb: number;
  pods: number;
  runningPods: number;
}
