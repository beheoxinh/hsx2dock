import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import type {
  ContainerItem,
  ContainerStats,
  ContainerDetails,
  ImageItem,
  ImageHistoryEntry,
  SearchResult,
  PullProgressPayload,
  VolumeItem,
  NetworkItem,
  ComposeProjectInfo,
  DfReport,
  EngineInfo,
  EngineUpdateInfo,
  EngineInstallProgress,
  CliShimStatus,
  CliStatus,
  CliUpdateInfo,
  ComposeUpdateInfo,
  EmbeddedBinaryInfo,
  CaddyInstallStatus,
  CaddyUpdateInfo,
  GatewayRoute,
  HostsSyncStatus,
  HostsFileModel,
  EngineDoctorReport,
  RepairReport,
  AppConfig,
  McpStatus,
  McpToolInfo,
  McpResourceInfo,
  McpPromptInfo,
  CliBridgeStatus,
  DaemonState,
  RunConfig,
  BuildRequest,
  BuildProgressPayload,
  BuildContextInfo,
  EndpointStatus,
  EndpointInput,
  ResourceLimits,
  ExecOutputPayload,
  RegistryInput,
  RegistryStatus,
  RegistryTestResult,
  PushProgressPayload,
  KindCluster,
  KindInstallStatus,
  KindProgressPayload,
  KindUpdateInfo,
  K8sNodeInfo,
  K8sPodInfo,
  K8sEventInfo,
  K8sWorkloadInfo,
  K8sMetrics,
  MetricsServerStatus,
  K8sClusterSummary,
  K8sServiceInfo,
  K8sConfigInfo,
  K8sPvcInfo,
  K8sNamespaceInfo,
  K8sIngressInfo,
  K8sPvInfo,
  KindClusterPrefs,
  ContainerGroup,
} from "../types";

export const api = {
  // System
  getEngineStatus: () => invoke<DaemonState>("get_engine_status"),
  checkEngineUpdate: () => invoke<EngineUpdateInfo>("check_engine_update"),
  installEngineUpdate: (version: string) =>
    invoke<string>("install_engine_update", { version }),
  checkDockerCli: () => invoke<CliStatus>("check_docker_cli"),
  checkCliUpdate: () => invoke<CliUpdateInfo>("check_cli_update"),
  installCliUpdate: (version: string) =>
    invoke<string>("install_cli_update", { version }),
  toggleCliShim: (enabled: boolean) => invoke<CliShimStatus>("toggle_cli_shim", { enabled }),
  checkComposeUpdate: () => invoke<ComposeUpdateInfo>("check_compose_update"),
  installComposeUpdate: (version: string) =>
    invoke<string>("install_compose_update", { version }),
  /** Read-only engine health: install, data root, snapshotter links, plugins. */
  diagnoseEngine: () => invoke<EngineDoctorReport>("diagnose_engine"),
  /**
   * Apply repairs. Omit `checkIds` to run every auto-repairable fix (the same
   * set the engine applies before each start).
   */
  repairEngine: (checkIds?: string[]) =>
    invoke<RepairReport>("repair_engine", { checkIds: checkIds ?? null }),
  getEmbeddedBinaries: () => invoke<EmbeddedBinaryInfo[]>("get_embedded_binaries"),
  toggleEngine: () => invoke<DaemonState>("toggle_engine"),
  getEngineInfo: () => invoke<EngineInfo | null>("get_engine_info"),
  getDiskUsage: () => invoke<DfReport>("get_disk_usage"),
  pruneSystem: () => invoke<number>("prune_system"),
  getAppConfig: () => invoke<AppConfig>("get_app_config"),
  saveAppConfig: (config: AppConfig) => invoke<void>("save_app_config", { config }),

  // MCP front-end (Settings → MCP, and the MCP page)
  mcpStatus: () => invoke<McpStatus>("mcp_status"),
  /** Enables/disables the loopback HTTP transport; `port` omitted = keep current. */
  mcpSetEnabled: (enabled: boolean, port?: number) =>
    invoke<McpStatus>("mcp_set_enabled", { enabled, port }),
  mcpRotateToken: () => invoke<McpStatus>("mcp_rotate_token"),
  /** Returns the bearer token itself — call only when the user asks to see it. */
  mcpToken: () => invoke<string>("mcp_token"),
  mcpCatalogue: () => invoke<McpToolInfo[]>("mcp_catalogue"),
  mcpResources: () => invoke<McpResourceInfo[]>("mcp_resources"),
  mcpPrompts: () => invoke<McpPromptInfo[]>("mcp_prompts"),
  getCliBridgeStatus: () => invoke<CliBridgeStatus>("get_cli_bridge_status"),
  toggleCliBridge: (enabled: boolean) => invoke<CliBridgeStatus>("toggle_cli_bridge", { enabled }),

  // Gateway (Caddy reverse proxy & virtual domains)
  getGatewayStatus: () => invoke<CaddyInstallStatus>("get_gateway_status"),
  checkCaddyUpdate: () => invoke<CaddyUpdateInfo>("check_caddy_update"),
  installCaddyUpdate: (version: string) =>
    invoke<void>("install_caddy_update", { version }),
  setGatewayPorts: (httpPort: number, httpsPort: number) =>
    invoke<void>("set_gateway_ports", { httpPort, httpsPort }),
  listGatewayRoutes: () => invoke<GatewayRoute[]>("list_gateway_routes"),
  saveGatewayRoutes: (routes: GatewayRoute[]) =>
    invoke<void>("save_gateway_routes", { routes }),
  toggleGateway: (enabled: boolean) => invoke<boolean>("toggle_gateway", { enabled }),
  trustGatewayCa: () => invoke<void>("trust_gateway_ca"),
  checkHostsSync: () => invoke<HostsSyncStatus>("check_hosts_sync"),
  syncHostsFile: () => invoke<void>("sync_hosts_file"),
  setGatewayAutoStart: (autoStart: boolean) =>
    invoke<void>("set_gateway_auto_start", { autoStart }),
  getHostsFileModel: () => invoke<HostsFileModel>("get_hosts_file_model"),
  saveRawHosts: (newContent: string) => invoke<void>("save_raw_hosts", { newContent }),
  clearAppHosts: () => invoke<void>("clear_app_hosts"),

  // Remote engine endpoints
  listEndpoints: () => invoke<EndpointStatus[]>("list_endpoints"),
  testEndpoint: (ep: EndpointInput) => invoke<EndpointStatus>("test_endpoint", { ep }),
  saveEndpoint: (ep: EndpointInput) => invoke<void>("save_endpoint", { ep }),
  removeEndpoint: (id: string) => invoke<void>("remove_endpoint", { id }),
  setActiveEndpoint: (id: string) => invoke<void>("set_active_endpoint", { id }),
  getActiveEndpoint: () => invoke<EndpointStatus | null>("get_active_endpoint"),

  // Registry logins (Docker Hub, ghcr, GitLab, Harbor, custom)
  listRegistries: () => invoke<RegistryStatus[]>("list_registries"),
  testRegistryAuth: (input: RegistryInput) =>
    invoke<RegistryTestResult>("test_registry_auth", { input }),
  saveRegistryAuth: (input: RegistryInput) =>
    invoke<void>("save_registry_auth", { input }),
  /** Marks one login as the default push target; `""` clears the choice. */
  setDefaultRegistry: (id: string) =>
    invoke<void>("set_default_registry", { id }),
  removeRegistryAuth: (id: string) => invoke<void>("remove_registry_auth", { id }),
  pushImage: (image: string) => invoke<void>("push_image", { image }),
  checkDataRootPath: (path: string) =>
    invoke<{
      exists: boolean;
      is_dir: boolean;
      is_empty: boolean;
      current_data_root: string;
      fs_error?: string | null;
    }>("check_data_root_path", { path }),
  migrateDataRoot: (targetPath: string, moveData: boolean, clearOld: boolean) =>
    invoke<void>("migrate_data_root", {
      targetPath,
      moveData,
      clearOld,
    }),

  // Containers
  listContainers: (all: boolean = true) => invoke<ContainerItem[]>("list_containers", { all }),
  startContainer: (id: string) => invoke<void>("start_container", { id }),
  stopContainer: (id: string) => invoke<void>("stop_container", { id }),
  restartContainer: (id: string) => invoke<void>("restart_container", { id }),
  pauseContainer: (id: string) => invoke<void>("pause_container", { id }),
  unpauseContainer: (id: string) => invoke<void>("unpause_container", { id }),
  removeContainer: (id: string, force: boolean = false) => invoke<void>("remove_container", { id, force }),
  renameContainer: (id: string, newName: string) => invoke<void>("rename_container", { id, newName }),
  inspectContainer: (id: string) => invoke<ContainerDetails>("inspect_container", { id }),
  getContainerLogs: (id: string, tail: string = "200", timestamps?: boolean) =>
    invoke<string>("get_container_logs", { id, tail, timestamps }),
  getContainerStats: (id: string) => invoke<ContainerStats>("get_container_stats", { id }),
  getContainerSizes: () => invoke<Record<string, number>>("get_container_sizes"),
  runContainer: (config: RunConfig) => invoke<string>("run_container", { config }),
  updateContainerResources: (id: string, limits: ResourceLimits) =>
    invoke<void>("update_container_resources", { id, limits }),
  recreateContainer: (id: string, config: RunConfig) =>
    invoke<string>("recreate_container", { id, config }),

  // Interactive exec / debug
  startInteractiveExec: (
    id: string,
    cmd: string[] | null,
    cols: number,
    rows: number
  ) => invoke<string>("start_interactive_exec", { id, cmd, cols, rows }),
  debugContainer: (id: string, cmd: string[] | null, cols: number, rows: number) =>
    invoke<string>("debug_container", { id, cmd, cols, rows }),
  writeExecStdin: (sessionId: string, data: string) =>
    invoke<void>("write_exec_stdin", { sessionId, data }),
  resizeExecSession: (sessionId: string, cols: number, rows: number) =>
    invoke<void>("resize_exec_session", { sessionId, cols, rows }),
  stopExecSession: (sessionId: string) =>
    invoke<void>("stop_exec_session", { sessionId }),

  // Images
  listImages: () => invoke<ImageItem[]>("list_images"),
  removeImage: (id: string, force: boolean = false) => invoke<void>("remove_image", { id, force }),
  tagImage: (id: string, repo: string, tag: string) => invoke<void>("tag_image", { id, repo, tag }),
  pruneImages: () => invoke<[number, number]>("prune_images"),
  getImageHistory: (name: string) => invoke<ImageHistoryEntry[]>("get_image_history", { name }),
  searchHub: (term: string) => invoke<SearchResult[]>("search_hub", { term }),
  pullImage: (image: string) => invoke<void>("pull_image", { image }),
  buildImage: (req: BuildRequest) => invoke<void>("build_image", { req }),
  cancelImageBuild: () => invoke<void>("cancel_image_build"),
  validateBuildContext: (path: string) =>
    invoke<BuildContextInfo>("validate_build_context", { path }),
  previewBuildCommand: (req: BuildRequest) =>
    invoke<string>("preview_build_command", { req }),

  // Volumes
  listVolumes: () => invoke<VolumeItem[]>("list_volumes"),
  createVolume: (name: string, driver: string = "local") =>
    invoke<void>("create_volume", { name, driver }),
  removeVolume: (name: string, force: boolean = false) =>
    invoke<void>("remove_volume", { name, force }),
  pruneVolumes: () => invoke<number>("prune_volumes"),

  // Networks
  listNetworks: () => invoke<NetworkItem[]>("list_networks"),
  createNetwork: (
    name: string,
    driver: string = "bridge",
    subnet?: string,
    gateway?: string,
    internal?: boolean,
    attachable?: boolean
  ) =>
    invoke<void>("create_network", {
      name,
      driver,
      subnet,
      gateway,
      internal,
      attachable,
    }),
  removeNetwork: (id: string) => invoke<void>("remove_network", { id }),
  pruneNetworks: () => invoke<number>("prune_networks"),
  repairContainerNetwork: (
    containerId: string,
    oldNetwork: string,
    targetNetwork: string,
    recreateIfMissing: boolean
  ) =>
    invoke<void>("repair_container_network", {
      containerId,
      oldNetwork,
      targetNetwork,
      recreateIfMissing,
    }),

  // Compose
  listComposeProjects: () => invoke<ComposeProjectInfo[]>("list_compose_projects"),
  composeUp: (filePath: string) => invoke<string>("compose_up", { filePath }),
  composeDown: (filePath: string) => invoke<string>("compose_down", { filePath }),
  composeLogs: (filePath: string, tail: string = "100") =>
    invoke<string>("compose_logs", { filePath, tail }),

  // Kind (Kubernetes IN Docker) — isolated binary + app-managed kubeconfig
  kindStatus: () => invoke<KindInstallStatus>("kind_status"),
  installKind: () => invoke<void>("install_kind"),
  checkKindUpdate: () => invoke<KindUpdateInfo>("check_kind_update"),
  installKindUpdate: (version: string) =>
    invoke<void>("install_kind_update", { version }),
  listKindClusters: () => invoke<KindCluster[]>("list_kind_clusters"),
  createKindCluster: (name: string, image?: string, nodes?: number) =>
    invoke<void>("create_kind_cluster", { name, image, nodes }),
  deleteKindCluster: (name: string) => invoke<void>("delete_kind_cluster", { name }),
  startKindCluster: (name: string) => invoke<void>("start_kind_cluster", { name }),
  stopKindCluster: (name: string) => invoke<void>("stop_kind_cluster", { name }),
  getKindKubeconfig: (name: string) => invoke<string>("get_kind_kubeconfig", { name }),
  loadImageToCluster: (cluster: string, image: string) =>
    invoke<void>("load_image_to_cluster", { cluster, image }),

  // Cluster monitoring (kubectl queries inside the control-plane node)
  getClusterSummary: (cluster: string) =>
    invoke<K8sClusterSummary>("get_cluster_summary", { cluster }),
  setClusterPrefs: (cluster: string, prefs: KindClusterPrefs) =>
    invoke<void>("set_cluster_prefs", { cluster, prefs }),
  getPodLogs: (cluster: string, namespace: string, name: string, tail?: number) =>
    invoke<string>("get_pod_logs", { cluster, namespace, name, tail }),
  getResourceYaml: (
    cluster: string,
    kind: string,
    namespace: string | null,
    name: string
  ) => invoke<string>("get_resource_yaml", { cluster, kind, namespace, name }),
  getResourceEvents: (cluster: string, namespace: string, name: string) =>
    invoke<K8sEventInfo[]>("get_resource_events", { cluster, namespace, name }),
  getWorkloadLogs: (
    cluster: string,
    kind: string,
    namespace: string,
    name: string,
    tail?: number
  ) => invoke<string>("get_workload_logs", { cluster, kind, namespace, name, tail }),
  deletePod: (cluster: string, namespace: string, name: string, force = false) =>
    invoke<void>("delete_pod", { cluster, namespace, name, force }),
  deleteK8sResource: (cluster: string, kind: string, namespace: string | null, name: string) =>
    invoke<void>("delete_k8s_resource", { cluster, kind, namespace, name }),
  scaleWorkload: (cluster: string, kind: string, namespace: string, name: string, replicas: number) =>
    invoke<void>("scale_workload", { cluster, kind, namespace, name, replicas }),
  restartWorkload: (cluster: string, kind: string, namespace: string, name: string) =>
    invoke<void>("restart_workload", { cluster, kind, namespace, name }),
  createNamespace: (cluster: string, name: string) =>
    invoke<void>("create_namespace", { cluster, name }),
  getClusterServices: (cluster: string) =>
    invoke<K8sServiceInfo[]>("get_cluster_services", { cluster }),
  getClusterConfigResources: (cluster: string, includeSecrets = true) =>
    invoke<K8sConfigInfo[]>("get_cluster_config_resources", { cluster, includeSecrets }),
  getClusterPvcs: (cluster: string) =>
    invoke<K8sPvcInfo[]>("get_cluster_pvcs", { cluster }),
  getClusterNamespaces: (cluster: string) =>
    invoke<K8sNamespaceInfo[]>("get_cluster_namespaces", { cluster }),
  getClusterIngresses: (cluster: string) =>
    invoke<K8sIngressInfo[]>("get_cluster_ingresses", { cluster }),
  getClusterPvs: (cluster: string) =>
    invoke<K8sPvInfo[]>("get_cluster_pvs", { cluster }),
  setNodeSchedulable: (cluster: string, node: string, schedulable: boolean) =>
    invoke<void>("set_node_schedulable", { cluster, node, schedulable }),
  drainNode: (cluster: string, node: string) =>
    invoke<string>("drain_node", { cluster, node }),
  getClusterNodes: (cluster: string) =>
    invoke<K8sNodeInfo[]>("get_cluster_nodes", { cluster }),
  getClusterPods: (cluster: string, namespace?: string) =>
    invoke<K8sPodInfo[]>("get_cluster_pods", { cluster, namespace }),
  getClusterEvents: (cluster: string) =>
    invoke<K8sEventInfo[]>("get_cluster_events", { cluster }),
  getClusterWorkloads: (cluster: string) =>
    invoke<K8sWorkloadInfo[]>("get_cluster_workloads", { cluster }),
  getClusterMetrics: (cluster: string) =>
    invoke<K8sMetrics>("get_cluster_metrics", { cluster }),
  installMetricsServer: (cluster: string) =>
    invoke<MetricsServerStatus>("install_metrics_server", { cluster }),
  metricsServerStatus: (cluster: string) =>
    invoke<MetricsServerStatus>("metrics_server_status", { cluster }),
  detectPodShell: (
    cluster: string,
    namespace: string,
    name: string,
    container?: string | null
  ) =>
    invoke<string | null>("detect_pod_shell", {
      cluster,
      namespace,
      name,
      container: container ?? null,
    }),

  // Container groups (bulk start/stop, optional auto-start with the engine)
  listContainerGroups: () => invoke<ContainerGroup[]>("list_container_groups"),
  saveContainerGroups: (groups: ContainerGroup[]) =>
    invoke<void>("save_container_groups", { groups }),
  startContainerGroup: (id: string) => invoke<string>("start_container_group", { id }),
  stopContainerGroup: (id: string) => invoke<string>("stop_container_group", { id }),

  // Compose project lifecycle addressed by project name (no compose file)
  composeProjectAction: (project: string, action: "start" | "stop" | "restart" | "down") =>
    invoke<string>("compose_project_action", { project, action }),
};

// Event Listeners helper
export const events = {
  onEngineStateChanged: (cb: (state: DaemonState) => void): Promise<UnlistenFn> =>
    listen<DaemonState>("engine-state-changed", (event) => cb(event.payload)),

  /** Unified progress for every bundled binary (engine, CLI, compose, kind). */
  onInstallProgress: (
    cb: (progress: EngineInstallProgress) => void
  ): Promise<UnlistenFn> =>
    listen<EngineInstallProgress>("engine-install-progress", (event) =>
      cb(event.payload)
    ),

  onContainerStatsTick: (
    cb: (stats: Record<string, ContainerStats>) => void
  ): Promise<UnlistenFn> =>
    listen<Record<string, ContainerStats>>("container-stats-tick", (event) =>
      cb(event.payload)
    ),

  onPullProgress: (cb: (payload: PullProgressPayload) => void): Promise<UnlistenFn> =>
    listen<PullProgressPayload>("pull-progress", (event) => cb(event.payload)),

  onPushProgress: (cb: (payload: PushProgressPayload) => void): Promise<UnlistenFn> =>
    listen<PushProgressPayload>("push-progress", (event) => cb(event.payload)),

  onBuildProgress: (cb: (payload: BuildProgressPayload) => void): Promise<UnlistenFn> =>
    listen<BuildProgressPayload>("build-progress", (event) => cb(event.payload)),

  onEndpointChanged: (cb: (id: string) => void): Promise<UnlistenFn> =>
    listen<string>("endpoint-changed", (event) => cb(event.payload)),

  onContainersTick: (cb: (containers: ContainerItem[]) => void): Promise<UnlistenFn> =>
    listen<ContainerItem[]>("containers-tick", (event) => cb(event.payload)),

  onExecOutput: (cb: (payload: ExecOutputPayload) => void): Promise<UnlistenFn> =>
    listen<ExecOutputPayload>("exec-output", (event) => cb(event.payload)),

  onKindProgress: (cb: (payload: KindProgressPayload) => void): Promise<UnlistenFn> =>
    listen<KindProgressPayload>("kind-progress", (event) => cb(event.payload)),

  /**
   * kind progress used to ride its own event; it now shares the unified
   * `engine-install-progress` channel, filtered by `target`.
   */
  onKindInstallProgress: (
    cb: (progress: EngineInstallProgress) => void
  ): Promise<UnlistenFn> =>
    listen<EngineInstallProgress>("engine-install-progress", (event) => {
      if (event.payload.target === "kind") cb(event.payload);
    }),

  /** Tray quick actions ask the shell to switch pages (`tray-navigate`). */
  onTrayNavigate: (cb: (tab: string) => void): Promise<UnlistenFn> =>
    listen<string>("tray-navigate", (event) => cb(event.payload)),
};
