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

export interface ContainerItem {
  id: string;
  short_id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  created: number;
  ports: string;
  /** Compose project name from the container's labels; null for plain containers. */
  compose_project?: string | null;
  /** Compose service name from the container's labels. */
  compose_service?: string | null;
  /** Kind cluster this node container belongs to; null for ordinary containers. */
  kind_cluster?: string | null;
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
}

export interface NetworkItem {
  id: string;
  name: string;
  driver: string;
  subnet: string;
  gateway: string;
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

export interface ComposeProjectInfo {
  name: string;
  path: string;
  services: ComposeService[];
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
export type UpdateTarget = "engine" | "cli" | "compose" | "kind";

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
  /** Whether a secret is stored — never the secret itself. */
  has_password: boolean;
}

export interface RegistryInput {
  id?: string | null;
  name: string;
  host: string;
  username: string;
  /** Plain password/token; empty on edit keeps the stored secret. */
  password: string;
  insecure: boolean;
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
}
