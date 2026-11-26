/**
 * Compose file keys for the editor's autocompletion, following the Compose
 * specification (docs.docker.com/reference/compose-file). Hand-kept and
 * deliberately compact: it suggests keys and a few enumerated values, it does
 * not validate. `docker compose config` stays the validator.
 */

export interface SchemaNode {
  /** Short description shown next to the completion. */
  desc: string;
  /** Child keys. The "*" key matches any user-chosen name (service, network...). */
  keys?: Record<string, SchemaNode>;
  /** Enumerated scalar values offered after `key: `, with a description each. */
  values?: Record<string, string>;
}

export interface ComposeSuggestion {
  label: string;
  /** Text that replaces the typed fragment; keys get a trailing ": ". */
  insert: string;
  kind: "key" | "value";
  description: string;
}

export interface ComposeSuggestions {
  /** Document offset where the replaced fragment starts. */
  from: number;
  options: ComposeSuggestion[];
}

/** Marks a sequence item in a key path (`volumes` > `[]` > `source`). */
export const LIST_ITEM = "[]";

const node = (desc: string, keys?: Record<string, SchemaNode>, values?: Record<string, string>): SchemaNode => ({
  desc,
  keys,
  values,
});
const S = (desc: string) => node(desc);
const BOOL = { true: "Enabled", false: "Disabled" };
const RESTART = {
  no: "Never restart",
  always: "Restart always",
  "on-failure": "Restart when the process exits non-zero",
  "unless-stopped": "Restart unless stopped manually",
};
const PULL = {
  always: "Always pull the image",
  missing: "Pull only when the image is missing",
  never: "Never pull",
  build: "Build the image instead of pulling",
  daily: "Pull at most once a day",
  weekly: "Pull at most once a week",
};
const DURATION = "Duration, e.g. 10s or 1m30s";

const healthcheck = node("Container health probe", {
  test: S('Probe command: ["CMD-SHELL", "..."] or "NONE"'),
  interval: S(`Time between checks (${DURATION})`),
  timeout: S("Time allowed for one check"),
  retries: S("Consecutive failures before unhealthy"),
  start_period: S("Grace period before failures count"),
  start_interval: S("Check interval during start_period"),
  disable: node("Disable a healthcheck inherited from the image", undefined, BOOL),
});

const updateConfig = node("Rolling update settings", {
  parallelism: S("Tasks updated at once"),
  delay: S("Delay between updates"),
  failure_action: node("Action on failed update", undefined, {
    continue: "Keep going",
    pause: "Pause the update",
    rollback: "Roll back",
  }),
  monitor: S("Time to watch each update for failure"),
  max_failure_ratio: S("Tolerated failure ratio"),
  order: node("Start/stop ordering", undefined, { "start-first": "Start new before stopping old", "stop-first": "Stop old before starting new" }),
});

const deploy = node("Deployment and resource settings", {
  mode: node("Replicated or global service", undefined, { replicated: "N replicas", global: "One per node" }),
  replicas: S("Number of replicas"),
  resources: node("Resource limits and reservations", {
    limits: node("Hard limits", {
      cpus: S('CPU limit, e.g. "0.5"'),
      memory: S("Memory limit, e.g. 512M"),
      pids: S("Maximum number of processes"),
    }),
    reservations: node("Guaranteed resources", {
      cpus: S("CPU reservation"),
      memory: S("Memory reservation"),
      devices: S("Device reservations (e.g. GPUs)"),
      generic_resources: S("Generic resource reservations"),
    }),
  }),
  restart_policy: node("Restart policy for deployed tasks", {
    condition: node("When to restart", undefined, { none: "Never", "on-failure": "On failure", any: "Always" }),
    delay: S("Delay before a restart"),
    max_attempts: S("Maximum restart attempts"),
    window: S("Window to evaluate restarts"),
  }),
  placement: node("Node placement", {
    constraints: S("Placement constraints"),
    preferences: S("Placement preferences"),
    max_replicas_per_node: S("Maximum replicas per node"),
  }),
  labels: S("Labels for the deployed service"),
  update_config: updateConfig,
  rollback_config: updateConfig,
  endpoint_mode: node("How the service is exposed", undefined, { vip: "Virtual IP", dnsrr: "DNS round-robin" }),
});

const build = node("Build the image from a Dockerfile", {
  context: S("Build context path or URL"),
  dockerfile: S("Dockerfile path inside the context"),
  dockerfile_inline: S("Dockerfile content given inline"),
  args: S("Build arguments"),
  target: S("Multi-stage build target"),
  cache_from: S("Cache sources"),
  cache_to: S("Cache exports"),
  additional_contexts: S("Named extra build contexts"),
  labels: S("Image labels"),
  network: node("Network for RUN steps", undefined, { host: "Host network", none: "No network" }),
  platforms: S("Target platforms, e.g. linux/amd64"),
  secrets: S("Build secrets"),
  ssh: S("SSH agent sockets or keys"),
  tags: S("Extra image tags"),
  no_cache: node("Ignore the build cache", undefined, BOOL),
  pull: node("Always pull base images", undefined, BOOL),
  shm_size: S("Size of /dev/shm during build"),
  extra_hosts: S("Extra host entries during build"),
  privileged: node("Build with extended privileges", undefined, BOOL),
});

const refItem = (desc: string) =>
  node(desc, {
    source: S("Name of the top-level definition"),
    target: S("Path inside the container"),
    uid: S("Owner user id"),
    gid: S("Owner group id"),
    mode: S("File permissions, e.g. 0440"),
  });

const service: Record<string, SchemaNode> = {
  image: S("Image to run, e.g. nginx:1.27"),
  build,
  command: S("Override the image CMD"),
  entrypoint: S("Override the image ENTRYPOINT"),
  environment: S("Environment variables (map or list)"),
  env_file: S("File(s) with environment variables"),
  ports: node("Published ports", {
    target: S("Container port"),
    published: S("Host port"),
    protocol: node("Protocol", undefined, { tcp: "TCP", udp: "UDP", sctp: "SCTP" }),
    host_ip: S("Host interface address"),
    mode: node("Publishing mode", undefined, { host: "Publish on each node", ingress: "Swarm routing mesh" }),
  }),
  expose: S("Ports reachable from linked services only"),
  volumes: node("Volume mounts", {
    type: node("Mount type", undefined, { volume: "Named volume", bind: "Host path", tmpfs: "In-memory", npipe: "Named pipe", image: "Image content" }),
    source: S("Volume name or host path"),
    target: S("Path inside the container"),
    read_only: node("Mount read-only", undefined, BOOL),
    bind: node("Bind mount options", { propagation: S("Mount propagation, e.g. rprivate") }),
    volume: node("Volume options", { nocopy: node("Skip copying image data", undefined, BOOL), subpath: S("Subdirectory of the volume") }),
    tmpfs: node("tmpfs options", { size: S("Size limit in bytes") }),
    consistency: S("Consistency mode (macOS)"),
  }),
  depends_on: node("Services this one depends on", {
    "*": node("Dependency options (long form)", {
      condition: node("Wait condition", undefined, {
        service_started: "Dependency has started",
        service_healthy: "Dependency passes its healthcheck",
        service_completed_successfully: "Dependency exited with 0",
      }),
      restart: node("Restart when the dependency restarts", undefined, RESTART),
      required: node("Fail if the dependency is missing", undefined, BOOL),
    }),
  }),
  restart: node("Restart policy", undefined, RESTART),
  healthcheck,
  deploy,
  profiles: S("Start only with these profiles"),
  networks: node("Networks this service joins", {
    "*": node("Per-network options", {
      aliases: S("Extra DNS names on this network"),
      ipv4_address: S("Static IPv4 address"),
      ipv6_address: S("Static IPv6 address"),
      priority: S("Connection priority"),
      mac_address: S("MAC address"),
      link_local_ips: S("Link-local addresses"),
      driver_opts: S("Driver options"),
    }),
  }),
  labels: S("Container labels"),
  container_name: S("Fixed container name"),
  hostname: S("Container hostname"),
  domainname: S("Container domain name"),
  working_dir: S("Working directory in the container"),
  user: S("User as name, uid or uid:gid"),
  cap_add: S("Linux capabilities to add"),
  cap_drop: S("Linux capabilities to drop"),
  devices: S("Host devices to map"),
  dns: S("Custom DNS servers"),
  dns_search: S("Custom DNS search domains"),
  dns_opt: S("Custom DNS options"),
  extra_hosts: S("Extra host:ip entries"),
  logging: node("Log driver and options", {
    driver: S("Log driver, e.g. json-file or journald"),
    options: node("Driver options", { "*": S("Option value") }),
  }),
  ulimits: node("Resource limits (ulimit)", {
    "*": node("One ulimit", { soft: S("Soft limit"), hard: S("Hard limit") }),
  }),
  tmpfs: S("tmpfs mounts"),
  stop_grace_period: S(`Time before SIGKILL (${DURATION})`),
  stop_signal: S("Signal sent on stop, e.g. SIGTERM"),
  init: node("Run an init process as PID 1", undefined, BOOL),
  privileged: node("Extended privileges", undefined, BOOL),
  read_only: node("Read-only root filesystem", undefined, BOOL),
  tty: node("Allocate a pseudo-TTY", undefined, BOOL),
  stdin_open: node("Keep STDIN open", undefined, BOOL),
  platform: S("Platform, e.g. linux/arm64"),
  pull_policy: node("When to pull the image", undefined, PULL),
  shm_size: S("Size of /dev/shm"),
  network_mode: node("Network mode", undefined, { bridge: "Default bridge", host: "Host network", none: "No networking" }),
  ipc: S("IPC namespace mode"),
  pid: S("PID namespace mode"),
  sysctls: S("Kernel parameters"),
  security_opt: S("Security options"),
  mem_limit: S("Memory limit, e.g. 512m"),
  mem_reservation: S("Memory soft limit"),
  memswap_limit: S("Memory plus swap limit"),
  cpus: S("CPU quota, e.g. 1.5"),
  cpu_shares: S("Relative CPU weight"),
  cpuset: S("CPUs allowed, e.g. 0,1"),
  pids_limit: S("Maximum number of processes"),
  oom_kill_disable: node("Disable the OOM killer", undefined, BOOL),
  group_add: S("Extra groups for the user"),
  runtime: S("OCI runtime name"),
  secrets: node("Secrets granted to the service (list or map)", { "*": refItem("Secret options") }),
  configs: node("Configs granted to the service (list or map)", { "*": refItem("Config options") }),
  develop: node("Dev-time file watch and rebuild", {
    watch: node("Watch rules (list)", {
      action: node("What to do on change", undefined, {
        sync: "Copy changed files",
        rebuild: "Rebuild the image",
        "sync+restart": "Copy files and restart",
        "sync+exec": "Copy files and run a command",
      }),
      path: S("Path to watch"),
      target: S("Target path in the container"),
      ignore: S("Patterns to ignore"),
      include: S("Patterns to include"),
      exclude: S("Patterns to exclude"),
      exec: S("Command for sync+exec"),
    }),
  }),
  extends: node("Inherit from another service", {
    service: S("Service to extend"),
    file: S("Compose file that defines it"),
  }),
  attach: node("Include in docker compose logs", undefined, BOOL),
  scale: S("Number of containers"),
  "x-": S("Extension field: any name starting with x-"),
};

const fileDef = (desc: string, extra: Record<string, SchemaNode> = {}) =>
  node(desc, {
    external: node("Defined outside this project", undefined, BOOL),
    name: S("Name of the resource on the host"),
    labels: S("Labels"),
    ...extra,
  });

const COMPOSE: SchemaNode = node("Compose file", {
  name: S("Project name"),
  services: node("Service definitions", { "*": node("Service", service) }),
  networks: node(
    "Network definitions",
    {
      "*": fileDef("Network", {
        driver: S("Network driver, e.g. bridge or overlay"),
        driver_opts: S("Driver options"),
        attachable: node("Allow standalone containers", undefined, BOOL),
        enable_ipv6: node("Enable IPv6", undefined, BOOL),
        internal: node("No external connectivity", undefined, BOOL),
        ipam: node("IP address management", {
          driver: S("IPAM driver"),
          config: node("Subnet settings (list)", {
            subnet: S("Subnet, e.g. 172.28.0.0/16"),
            gateway: S("Gateway address"),
            ip_range: S("Range for container addresses"),
          }),
        }),
      }),
    }
  ),
  volumes: node("Named volume definitions", {
    "*": fileDef("Volume", {
      driver: S("Volume driver"),
      driver_opts: S("Driver options"),
    }),
  }),
  configs: node("Config definitions", { "*": fileDef("Config", { file: S("Host file to read"), environment: S("Environment variable to read"), content: S("Inline content") }) }),
  secrets: node("Secret definitions", { "*": fileDef("Secret", { file: S("Host file to read"), environment: S("Environment variable to read") }) }),
  include: S("Other Compose files to include (list)"),
  "x-": S("Extension field: any name starting with x-"),
});

/** The node for a key path such as ["services","web","healthcheck"]. */
export function lookupNode(path: readonly string[]): SchemaNode | null {
  let current: SchemaNode = COMPOSE;
  for (const seg of path) {
    if (seg === LIST_ITEM) continue;
    const next = current.keys?.[seg] ?? current.keys?.["*"];
    if (!next) return null;
    current = next;
  }
  return current;
}

interface LineInfo {
  /** Column of the first "-" of a sequence item, or -1. */
  dash: number;
  /** Column where the mapping key (or the item's content) starts. */
  col: number;
  /** Mapping key on this line, if any. */
  key: string | null;
}

/** Parses one line; null for blank and comment-only lines. */
function parseLine(text: string): LineInfo | null {
  const m = /^(\s*)((?:-\s+)*)([\s\S]*)$/.exec(text);
  if (!m) return null;
  const rest = m[3];
  const dash = m[2] ? m[1].length : -1;
  if (dash < 0 && (rest.trim() === "" || rest.trimStart().startsWith("#"))) return null;
  const km = /^(["']?)([^"'#\s:][^"':]*?)\1\s*:(?:\s|$)/.exec(rest);
  return { dash, col: m[1].length + m[2].length, key: km ? km[2] : null };
}

/**
 * Key path of the position described by `cursor` on line `lineIdx`. The
 * cursor line contributes only its list marker: its key is what is being
 * typed or edited, not an ancestor.
 */
function pathAt(lines: readonly string[], lineIdx: number, cursor: LineInfo): string[] {
  const path: string[] = [];
  let indent = cursor.col;
  if (cursor.dash >= 0 && cursor.dash < indent) {
    path.unshift(LIST_ITEM);
    indent = cursor.dash;
  }
  // Walk upwards; a line is an ancestor when it sits left of the current indent.
  for (let i = lineIdx - 1; i >= 0; i--) {
    const info = parseLine(lines[i]);
    if (!info) continue;
    if (info.key !== null && info.col < indent) {
      path.unshift(info.key);
      indent = info.col;
    }
    if (info.dash >= 0 && info.dash < indent) {
      path.unshift(LIST_ITEM);
      indent = info.dash;
    }
  }
  return path;
}

function cursorInfo(prefix: string): LineInfo {
  const m = /^(\s*)((?:-\s+)*)([\s\S]*)$/.exec(prefix);
  const lead = m ? m[1].length : 0;
  const dashes = m ? m[2] : "";
  return { dash: dashes ? lead : -1, col: lead + dashes.length, key: null };
}

/** YAML key path at `offset`, e.g. ["services","web","healthcheck"]. */
export function yamlKeyPath(doc: string, offset: number): string[] {
  const before = doc.slice(0, offset);
  const lineIdx = before.split("\n").length - 1;
  const prefix = before.slice(before.lastIndexOf("\n") + 1);
  return pathAt(doc.split("\n"), lineIdx, cursorInfo(prefix));
}

/**
 * Completions at `offset`: keys of the mapping the cursor is in, or the
 * enumerated values after `key: `. Null when nothing applies.
 */
export function composeSuggestions(doc: string, offset: number): ComposeSuggestions | null {
  const before = doc.slice(0, offset);
  const lineIdx = before.split("\n").length - 1;
  const prefix = before.slice(before.lastIndexOf("\n") + 1);
  const lines = doc.split("\n");
  const cursor = cursorInfo(prefix);

  const valueMatch = /^(["']?)([^"'#\s:][^"':]*?)\1\s*:\s*(["']?[^\s#]*)$/.exec(prefix.slice(cursor.col));
  if (valueMatch) {
    const [, , key, raw] = valueMatch;
    const parent = lookupNode(pathAt(lines, lineIdx, cursor));
    const values = parent?.keys?.[key]?.values;
    if (!values) return null;
    const partial = raw.replace(/^["']/, "");
    const options = Object.entries(values)
      .filter(([v]) => v.startsWith(partial))
      .map(([v, desc]) => ({ label: v, insert: v, kind: "value" as const, description: desc }));
    return options.length ? { from: offset - raw.length, options } : null;
  }

  const keyMatch = /^["']?[\w.-]*$/.exec(prefix.slice(cursor.col));
  if (!keyMatch) return null;
  const rest = prefix.slice(cursor.col);
  const keys = lookupNode(pathAt(lines, lineIdx, cursor))?.keys;
  if (!keys) return null;
  const partial = rest.replace(/^["']/, "");
  const options = Object.entries(keys)
    .filter(([k]) => k !== "*" && k.startsWith(partial))
    .map(([k, child]) => ({
      label: k,
      // "x-" is a prefix for user names, so it is inserted without a colon.
      insert: k.endsWith("-") ? k : `${k}: `,
      kind: "key" as const,
      description: child.desc,
    }));
  return options.length ? { from: offset - rest.length, options } : null;
}
