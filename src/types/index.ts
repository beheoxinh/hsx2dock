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
  schema_version: number;
}

export interface CliBridgeStatus {
  enabled: boolean;
  link_state: boolean | null;
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
