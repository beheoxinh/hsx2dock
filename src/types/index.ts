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

/** `engine-install-progress` payload streamed while an engine installs. */
export interface EngineInstallProgress {
  label: string;
  fraction: number | null;
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

/** Download progress for the kind binary (`kind-install-progress` event). */
export interface KindInstallProgressPayload {
  done?: number;
  total?: number;
  finished?: boolean;
  error?: string;
}
