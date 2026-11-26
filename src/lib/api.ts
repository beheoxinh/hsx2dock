import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { createCoalescedFetch } from "./coalesce";
import type {
  ContainerOp,
  ContainerOpsSnapshot,
  GroupOp,
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
  ComposeUpOptions,
  ComposeDownOptions,
  ComposeServiceAction,
  ComposeValidation,
  ComposeEnvFile,
  ComposeProgressPayload,
  ComposePullOptions,
  ComposeBuildOptions,
  ComposeImageUpdate,
  HealthDetails,
  ComposeProjectVolume,
  ComposeEvent,
  ComposeAlert,
  ComposeWatchEnded,
  KindOrphanState,
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
  PrunedGatewayRoutes,
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
  ClusterSpec,
  ClusterPreset,
  ClusterConfigPreview,
  NodeImageOption,
  LocalRegistryStatus,
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
  ClusterAccess,
  ManifestDiff,
  SelectorLogs,
  ContainerGroup,
  K8sToolStatus,
  PortForwardInfo,
  PortForwardSpec,
  AddonStatus,
  IngressInfo,
  ServiceLink,
  ServiceLinkRequest,
  ServiceLinkCheck,
  HelmRelease,
  HelmReleaseDetail,
  HelmInstallRequest,
  HelmRepo,
  HelmChart,
  ClusterSnapshot,
  ClusterAlert,
  HistoryRange,
  HistorySample,
} from "../types";

/**
 * The Kind cluster list is polled by the header, the dashboard and the
 * Kubernetes page at the same 10s cadence. One shared fetch serves all of them:
 * concurrent polls join the request in flight, and a poll within 8s of the last
 * answer reuses it. Cluster create/delete/start/stop/worker changes call
 * `invalidate` so the next read is fresh.
 */
const kindClusters = createCoalescedFetch(() => invoke<KindCluster[]>("list_kind_clusters"), 8_000);

/** Runs a cluster-changing call and drops the shared list once it settles. */
const kindChanged = <T,>(pending: Promise<T>): Promise<T> =>
  pending.finally(() => kindClusters.invalidate());

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
  /** `fresh` bypasses the backend's 30s cache (after a prune or a manual refresh). */
  getDiskUsage: (fresh = false) => invoke<DfReport>("get_disk_usage", { fresh }),
  pruneSystem: () => invoke<number>("prune_system"),
  getAppConfig: () => invoke<AppConfig>("get_app_config"),
  saveAppConfig: (config: AppConfig) => invoke<void>("save_app_config", { config }),
  /** Saves only these fields, merged into the live config on the backend. */
  patchAppConfig: (patch: Partial<AppConfig>) => invoke<void>("patch_app_config", { patch }),

  // MCP front-end (Settings → MCP, and the MCP page)
  mcpStatus: () => invoke<McpStatus>("mcp_status"),
  /** Enables/disables the loopback HTTP transport; `port` omitted = keep current. */
  mcpSetEnabled: (enabled: boolean, port?: number) =>
    invoke<McpStatus>("mcp_set_enabled", { enabled, port }),
  mcpRotateToken: () => invoke<McpStatus>("mcp_rotate_token"),
  mcpSetReadOnly: (readOnly: boolean) => invoke<McpStatus>("mcp_set_read_only", { readOnly }),
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
  probeGatewayUpstreams: () =>
    invoke<{ id: string; reachable: boolean }[]>("probe_gateway_upstreams"),
  syncHostsFile: () => invoke<void>("sync_hosts_file"),
  setGatewayAutoStart: (autoStart: boolean) =>
    invoke<void>("set_gateway_auto_start", { autoStart }),
  setGatewayAutoClearOrphans: (enabled: boolean) =>
    invoke<void>("set_gateway_auto_clear_orphans", { enabled }),
  clearOrphanGatewayRoutes: () => invoke<PrunedGatewayRoutes>("clear_orphan_gateway_routes"),
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
  getContainerLogs: (id: string, tail: string = "200", timestamps?: boolean, since?: number) =>
    invoke<string>("get_container_logs", { id, tail, timestamps, since }),
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
  /** Ends every exec session (sessions left over from before a webview reload). */
  closeAllExecSessions: () => invoke<void>("close_all_exec_sessions"),

  // Images
  listImages: () => invoke<ImageItem[]>("list_images"),
  removeImage: (id: string, force: boolean = false) => invoke<void>("remove_image", { id, force }),
  tagImage: (id: string, repo: string, tag: string) => invoke<void>("tag_image", { id, repo, tag }),
  pruneImages: (danglingOnly?: boolean) =>
    invoke<[number, number]>("prune_images", { danglingOnly }),
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
  composeUp: (filePath: string, options?: ComposeUpOptions, opId?: string) =>
    invoke<string>("compose_up", { filePath, options: options ?? null, opId: opId ?? null }),
  composeDown: (filePath: string, options?: ComposeDownOptions, opId?: string) =>
    invoke<string>("compose_down", { filePath, options: options ?? null, opId: opId ?? null }),
  composeLogs: (filePath: string, tail: string = "100", services?: string[]) =>
    invoke<string>("compose_logs", { filePath, tail, services: services ?? null }),
  composeServiceAction: (
    project: string,
    filePath: string | null,
    service: string,
    action: ComposeServiceAction,
    replicas?: number,
    opId?: string
  ) =>
    invoke<string>("compose_service_action", {
      project,
      filePath,
      service,
      action,
      replicas: replicas ?? null,
      opId: opId ?? null,
    }),
  composePinProject: (filePath: string) => invoke<string>("compose_pin_project", { filePath }),
  composeUnpinProject: (filePath: string) =>
    invoke<string>("compose_unpin_project", { filePath }),
  composeScanDir: (dir: string, depth?: number) =>
    invoke<string[]>("compose_scan_dir", { dir, depth: depth ?? null }),
  composeOpenFolder: (filePath: string) => invoke<void>("compose_open_folder", { filePath }),
  composeReadFile: (filePath: string) => invoke<string>("compose_read_file", { filePath }),
  composeValidate: (filePath: string, content?: string, envFile?: string) =>
    invoke<ComposeValidation>("compose_validate", {
      filePath,
      content: content ?? null,
      envFile: envFile ?? null,
    }),
  composeSaveFile: (filePath: string, content: string, original: string) =>
    invoke<void>("compose_save_file", { filePath, content, original }),
  composeReadEnv: (filePath: string, envFile?: string) =>
    invoke<ComposeEnvFile>("compose_read_env", { filePath, envFile: envFile ?? null }),
  composeSaveEnv: (filePath: string, envFile: string | null, content: string, original: string | null) =>
    invoke<void>("compose_save_env", { filePath, envFile, content, original }),
  composePull: (filePath: string, options?: ComposePullOptions, opId?: string) =>
    invoke<string>("compose_pull", { filePath, options: options ?? null, opId: opId ?? null }),
  composeBuild: (filePath: string, options?: ComposeBuildOptions, opId?: string) =>
    invoke<string>("compose_build", { filePath, options: options ?? null, opId: opId ?? null }),
  /** Registry digest check per service image (cached 15 min unless `force`). */
  composeImageUpdates: (filePath: string, force?: boolean) =>
    invoke<ComposeImageUpdate[]>("compose_image_updates", { filePath, force: force ?? null }),
  /** `compose config` as YAML: override file and `.env` applied. */
  composeMergedConfig: (filePath: string, envFile?: string) =>
    invoke<string>("compose_merged_config", { filePath, envFile: envFile ?? null }),
  containerHealth: (id: string) => invoke<HealthDetails>("container_health", { id }),
  composeProjectVolumes: (project: string) =>
    invoke<ComposeProjectVolume[]>("compose_project_volumes", { project }),
  /** Archives a volume (default folder ~/hsx2dock-backups/volumes); returns the file. */
  volumeBackup: (volume: string, destDir?: string, opId?: string) =>
    invoke<string>("volume_backup", { volume, destDir: destDir ?? null, opId: opId ?? null }),
  /** Replaces a volume's content with an archive; refused while a running container uses it. */
  volumeRestore: (volume: string, archive: string, opId?: string) =>
    invoke<void>("volume_restore", { volume, archive, opId: opId ?? null }),
  /** Packs compose file(s) and optionally `.env` (default folder ~/hsx2dock-backups/projects). */
  composeExport: (filePath: string, destDir: string | null, includeEnv: boolean, maskSecrets: boolean) =>
    invoke<string>("compose_export", { filePath, destDir, includeEnv, maskSecrets }),
  /** Unpacks an export into `<destDir>/<project>` and pins it; returns the compose file. */
  composeImport: (archive: string, destDir: string) =>
    invoke<string>("compose_import", { archive, destDir }),
  /** Writes `<dir>/compose.yaml` (+ optional `.env`), validates and pins it. */
  composeCreateProject: (dir: string, content: string, env?: string) =>
    invoke<string>("compose_create_project", { dir, content, env: env ?? null }),
  /** A compose file describing existing containers (for review before saving). */
  composeFromContainers: (ids: string[], project: string) =>
    invoke<string>("compose_from_containers", { ids, project }),
  composeRecentAlerts: () => invoke<ComposeAlert[]>("compose_recent_alerts"),
  composeClearAlerts: (project?: string) =>
    invoke<void>("compose_clear_alerts", { project: project ?? null }),
  /** Starts `docker compose watch`; output streams as compose-progress under `opId`. */
  composeWatchStart: (filePath: string, opId: string) =>
    invoke<void>("compose_watch_start", { filePath, opId }),
  composeWatchStop: (filePath: string) => invoke<void>("compose_watch_stop", { filePath }),
  kindOrphanStates: () => invoke<KindOrphanState[]>("kind_orphan_states"),
  kindCleanOrphanStates: (paths: string[]) => invoke<number>("kind_clean_orphan_states", { paths }),

  // Kind (Kubernetes IN Docker) — isolated binary + app-managed kubeconfig
  kindStatus: () => invoke<KindInstallStatus>("kind_status"),
  installKind: () => invoke<void>("install_kind"),
  checkKindUpdate: () => invoke<KindUpdateInfo>("check_kind_update"),
  installKindUpdate: (version: string) =>
    invoke<void>("install_kind_update", { version }),
  listKindClusters: () => kindClusters.get(),
  createKindCluster: (name: string, image?: string, nodes?: number) =>
    kindChanged(invoke<void>("create_kind_cluster", { name, image, nodes })),
  /** Advanced create: a wizard spec, or a hand-edited kind config (wins over spec). */
  createKindClusterAdvanced: (name: string, spec: ClusterSpec, config?: string) =>
    kindChanged(invoke<void>("create_kind_cluster", { name, spec, config: config ?? null })),
  previewClusterConfig: (spec: ClusterSpec, config?: string) =>
    invoke<ClusterConfigPreview>("preview_cluster_config", { spec, config: config ?? null }),
  listKindNodeImages: () => invoke<NodeImageOption[]>("list_kind_node_images"),
  listClusterPresets: () => invoke<ClusterPreset[]>("list_cluster_presets"),
  saveClusterPreset: (preset: ClusterPreset) =>
    invoke<ClusterPreset[]>("save_cluster_preset", { preset }),
  deleteClusterPreset: (name: string) => invoke<ClusterPreset[]>("delete_cluster_preset", { name }),
  addClusterWorker: (cluster: string) => kindChanged(invoke<string>("add_cluster_worker", { cluster })),
  removeClusterNode: (cluster: string, node: string) =>
    kindChanged(invoke<void>("remove_cluster_node", { cluster, node })),
  // Shared local registry (kind-registry on localhost:5001)
  localRegistryStatus: () => invoke<LocalRegistryStatus>("registry_status"),
  ensureLocalRegistry: () => invoke<LocalRegistryStatus>("ensure_registry"),
  removeLocalRegistry: (deleteData: boolean) => invoke<void>("remove_registry", { deleteData }),
  connectRegistryToCluster: (cluster: string) =>
    invoke<void>("connect_registry_to_cluster", { cluster }),
  pushToLocalRegistry: (image: string) => invoke<string>("push_to_local_registry", { image }),
  // Port-forwards (host kubectl, app-managed)
  k8sToolsStatus: () => invoke<K8sToolStatus[]>("k8s_tools_status"),
  installK8sTools: () => invoke<K8sToolStatus[]>("install_k8s_tools"),
  listPortForwards: (cluster?: string) =>
    invoke<PortForwardInfo[]>("list_port_forwards", { cluster: cluster ?? null }),
  startPortForward: (spec: PortForwardSpec) => invoke<PortForwardInfo>("start_port_forward", { spec }),
  stopPortForward: (id: string, forget = false) => invoke<void>("stop_port_forward", { id, forget }),
  // Addons, ingress, service links
  listClusterAddons: (cluster: string) => invoke<AddonStatus[]>("list_cluster_addons", { cluster }),
  installClusterAddon: (cluster: string, addon: string) =>
    invoke<void>("install_cluster_addon", { cluster, addon }),
  uninstallClusterAddon: (cluster: string, addon: string) =>
    invoke<void>("uninstall_cluster_addon", { cluster, addon }),
  dashboardToken: (cluster: string) => invoke<string>("dashboard_token", { cluster }),
  listClusterIngresses: (cluster: string) =>
    invoke<IngressInfo[]>("list_cluster_ingresses", { cluster }),
  exposeIngressHost: (cluster: string, host: string, expose: boolean) =>
    invoke<void>("expose_ingress_host", { cluster, host, expose }),
  linkClusterService: (req: ServiceLinkRequest) => invoke<ServiceLink>("link_cluster_service", { req }),
  listServiceLinks: (cluster: string) => invoke<ServiceLink[]>("list_service_links", { cluster }),
  unlinkClusterService: (cluster: string, namespace: string, name: string) =>
    invoke<void>("unlink_cluster_service", { cluster, namespace, name }),
  checkServiceLink: (cluster: string, namespace: string, name: string) =>
    invoke<ServiceLinkCheck[]>("check_service_link", { cluster, namespace, name }),
  // Helm
  helmListReleases: (cluster: string) => invoke<HelmRelease[]>("helm_list_releases", { cluster }),
  helmReleaseDetail: (cluster: string, namespace: string, name: string) =>
    invoke<HelmReleaseDetail>("helm_release_detail", { cluster, namespace, name }),
  helmInstall: (req: HelmInstallRequest) => invoke<string>("helm_install", { req }),
  helmRollback: (cluster: string, namespace: string, name: string, revision: number) =>
    invoke<void>("helm_rollback", { cluster, namespace, name, revision }),
  helmUninstall: (cluster: string, namespace: string, name: string) =>
    invoke<void>("helm_uninstall", { cluster, namespace, name }),
  helmRepoList: () => invoke<HelmRepo[]>("helm_repo_list"),
  helmRepoAdd: (name: string, url: string) => invoke<void>("helm_repo_add", { name, url }),
  helmRepoRemove: (name: string) => invoke<void>("helm_repo_remove", { name }),
  helmRepoUpdate: () => invoke<string>("helm_repo_update"),
  helmSearch: (query: string, versions = false) =>
    invoke<HelmChart[]>("helm_search", { query, versions }),
  helmShowValues: (chart: string, version?: string) =>
    invoke<string>("helm_show_values", { chart, version: version ?? null }),
  // Snapshots / clone
  listClusterSnapshots: (cluster?: string) =>
    invoke<ClusterSnapshot[]>("list_cluster_snapshots", { cluster: cluster ?? null }),
  createClusterSnapshot: (cluster: string, note?: string) =>
    invoke<string>("create_cluster_snapshot", { cluster, note: note ?? null }),
  deleteClusterSnapshot: (cluster: string, id: string) =>
    invoke<void>("delete_cluster_snapshot", { cluster, id }),
  restoreClusterSnapshot: (cluster: string, id: string, newName?: string) =>
    invoke<string>("restore_cluster_snapshot", { req: { cluster, id, newName: newName ?? null } }),
  // Alerts and history
  listClusterAlerts: (cluster?: string) =>
    invoke<ClusterAlert[]>("list_cluster_alerts", { cluster: cluster ?? null }),
  takeAlertFocus: () => invoke<string | null>("take_alert_focus"),
  clusterHistory: (cluster: string, range: HistoryRange) =>
    invoke<HistorySample[]>("cluster_history", { cluster, range }),
  deleteKindCluster: (name: string) => kindChanged(invoke<void>("delete_kind_cluster", { name })),
  startKindCluster: (name: string) => kindChanged(invoke<void>("start_kind_cluster", { name })),
  stopKindCluster: (name: string) => kindChanged(invoke<void>("stop_kind_cluster", { name })),
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
  /** Host reachability + kubeconfig state of a cluster. */
  getClusterAccess: (cluster: string) =>
    invoke<ClusterAccess>("get_cluster_access", { cluster }),
  /** Merge into ~/.kube/config (backed up, becomes current context). */
  exportKubeconfigToHost: (cluster: string) =>
    invoke<string>("export_kubeconfig_to_host", { cluster }),
  removeKubeconfigFromHost: (cluster: string) =>
    invoke<string>("remove_kubeconfig_from_host", { cluster }),
  /** Opens a host terminal window with KUBECONFIG set to the cluster. */
  openClusterTerminal: (cluster: string) =>
    invoke<string>("open_cluster_terminal", { cluster }),
  /** Server-side dry-run diff; nothing is changed. */
  diffClusterYaml: (cluster: string, yaml: string) =>
    invoke<ManifestDiff>("diff_cluster_yaml", { cluster, yaml }),
  /** `kubectl apply -f -`; returns kubectl's output. Max 90 KiB. */
  applyClusterYaml: (cluster: string, yaml: string) =>
    invoke<string>("apply_cluster_yaml", { cluster, yaml }),
  /** Logs of all pods matching a selector, or a workload's selector. */
  getSelectorLogs: (
    cluster: string,
    namespace: string,
    opts: {
      selector?: string;
      workloadKind?: string;
      workloadName?: string;
      tail?: number;
      sinceTime?: string;
    }
  ) =>
    invoke<SelectorLogs>("get_selector_logs", {
      cluster,
      namespace,
      selector: opts.selector,
      workloadKind: opts.workloadKind,
      workloadName: opts.workloadName,
      tail: opts.tail,
      sinceTime: opts.sinceTime,
    }),
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
  /** Container/group lifecycle operations currently in flight (spinners). */
  listContainerOps: () => invoke<ContainerOpsSnapshot>("list_container_ops"),

  // Compose project lifecycle addressed by project name (no compose file)
  composeProjectAction: (
    project: string,
    action: "start" | "stop" | "restart" | "down",
    opId?: string
  ) => invoke<string>("compose_project_action", { project, action, opId: opId ?? null }),
};

/**
 * Turns a pending `listen()` into a synchronous disposer.
 *
 * `listen()` resolves asynchronously, so the common
 * `.then((u) => (unlisten = u))` + cleanup pattern leaks the listener when the
 * component unmounts before the promise settles (fast tab switches, StrictMode).
 * The disposer returned here also unsubscribes a listener that resolves late.
 */
export function disposeListener(pending: Promise<UnlistenFn>): () => void {
  let disposed = false;
  let unlisten: UnlistenFn | undefined;
  pending
    .then((u) => {
      if (disposed) u();
      else unlisten = u;
    })
    .catch(() => {
      /* no Tauri bridge (tests, plain browser) */
    });
  return () => {
    disposed = true;
    unlisten?.();
    unlisten = undefined;
  };
}

// Event Listeners helper
export const events = {
  /** One container moved through queued → running → done/failed. */
  onContainerOp: (cb: (op: ContainerOp) => void): Promise<UnlistenFn> =>
    listen<ContainerOp>("container-op", (event) => cb(event.payload)),

  /** A group start/stop pass progressed or finished. */
  onGroupOp: (cb: (op: GroupOp) => void): Promise<UnlistenFn> =>
    listen<GroupOp>("group-op", (event) => cb(event.payload)),

  /** The saved group list changed (this window, another one, or MCP). */
  onContainerGroupsChanged: (cb: (groups: ContainerGroup[]) => void): Promise<UnlistenFn> =>
    listen<ContainerGroup[]>("container-groups-changed", (event) => cb(event.payload)),

  onEngineStateChanged: (cb: (state: DaemonState) => void): Promise<UnlistenFn> =>
    listen<DaemonState>("engine-state-changed", (event) => cb(event.payload)),

  /** Unified progress for every bundled binary (engine, CLI, compose, kind). */
  onInstallProgress: (
    cb: (progress: EngineInstallProgress) => void
  ): Promise<UnlistenFn> =>
    listen<EngineInstallProgress>("engine-install-progress", (event) =>
      cb(event.payload)
    ),

  /** One output line of a running compose operation started with an `opId`. */
  onComposeProgress: (cb: (payload: ComposeProgressPayload) => void): Promise<UnlistenFn> =>
    listen<ComposeProgressPayload>("compose-progress", (event) => cb(event.payload)),
  /** Container event of a compose project (from the daemon's event stream). */
  onComposeEvent: (cb: (e: ComposeEvent) => void): Promise<UnlistenFn> =>
    listen<ComposeEvent>("compose-event", (event) => cb(event.payload)),
  onComposeAlert: (cb: (a: ComposeAlert) => void): Promise<UnlistenFn> =>
    listen<ComposeAlert>("compose-alert", (event) => cb(event.payload)),
  onComposeWatchEnded: (cb: (e: ComposeWatchEnded) => void): Promise<UnlistenFn> =>
    listen<ComposeWatchEnded>("compose-watch-ended", (event) => cb(event.payload)),

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

  /** The background sweeper removed mapped routes whose container is gone. */
  onGatewayRoutesPruned: (cb: (p: PrunedGatewayRoutes) => void): Promise<UnlistenFn> =>
    listen<PrunedGatewayRoutes>("gateway-routes-pruned", (event) => cb(event.payload)),

  /** Tray quick actions ask the shell to switch pages (`tray-navigate`). */
  onTrayNavigate: (cb: (tab: string) => void): Promise<UnlistenFn> =>
    listen<string>("tray-navigate", (event) => cb(event.payload)),
};
