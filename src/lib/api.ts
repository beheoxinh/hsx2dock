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
  ComposeUpdateInfo,
  AppConfig,
  CliBridgeStatus,
  DaemonState,
  RunConfig,
} from "../types";

export const api = {
  // System
  getEngineStatus: () => invoke<DaemonState>("get_engine_status"),
  checkEngineUpdate: () => invoke<EngineUpdateInfo>("check_engine_update"),
  installEngineUpdate: (version: string) =>
    invoke<string>("install_engine_update", { version }),
  checkDockerCli: () => invoke<CliStatus>("check_docker_cli"),
  toggleCliShim: (enabled: boolean) => invoke<CliShimStatus>("toggle_cli_shim", { enabled }),
  checkComposeUpdate: () => invoke<ComposeUpdateInfo>("check_compose_update"),
  installComposeUpdate: (version: string) =>
    invoke<string>("install_compose_update", { version }),
  toggleEngine: () => invoke<DaemonState>("toggle_engine"),
  getEngineInfo: () => invoke<EngineInfo | null>("get_engine_info"),
  getDiskUsage: () => invoke<DfReport>("get_disk_usage"),
  pruneSystem: () => invoke<number>("prune_system"),
  getAppConfig: () => invoke<AppConfig>("get_app_config"),
  saveAppConfig: (config: AppConfig) => invoke<void>("save_app_config", { config }),
  getCliBridgeStatus: () => invoke<CliBridgeStatus>("get_cli_bridge_status"),
  toggleCliBridge: (enabled: boolean) => invoke<CliBridgeStatus>("toggle_cli_bridge", { enabled }),
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
  getContainerLogs: (id: string, tail: string = "200") => invoke<string>("get_container_logs", { id, tail }),
  getContainerStats: (id: string) => invoke<ContainerStats>("get_container_stats", { id }),
  runContainer: (config: RunConfig) => invoke<string>("run_container", { config }),
  execContainer: (id: string, cmd: string) => invoke<string>("exec_container", { id, cmd }),

  // Images
  listImages: () => invoke<ImageItem[]>("list_images"),
  removeImage: (id: string, force: boolean = false) => invoke<void>("remove_image", { id, force }),
  tagImage: (id: string, repo: string, tag: string) => invoke<void>("tag_image", { id, repo, tag }),
  pruneImages: () => invoke<[number, number]>("prune_images"),
  getImageHistory: (name: string) => invoke<ImageHistoryEntry[]>("get_image_history", { name }),
  searchHub: (term: string) => invoke<SearchResult[]>("search_hub", { term }),
  pullImage: (image: string) => invoke<void>("pull_image", { image }),

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

  // Compose
  listComposeProjects: () => invoke<ComposeProjectInfo[]>("list_compose_projects"),
  composeUp: (filePath: string) => invoke<string>("compose_up", { filePath }),
  composeDown: (filePath: string) => invoke<string>("compose_down", { filePath }),
  composeLogs: (filePath: string, tail: string = "100") =>
    invoke<string>("compose_logs", { filePath, tail }),
};

// Event Listeners helper
export const events = {
  onEngineStateChanged: (cb: (state: DaemonState) => void): Promise<UnlistenFn> =>
    listen<DaemonState>("engine-state-changed", (event) => cb(event.payload)),

  onEngineInstallProgress: (
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

  onContainersTick: (cb: (containers: ContainerItem[]) => void): Promise<UnlistenFn> =>
    listen<ContainerItem[]>("containers-tick", (event) => cb(event.payload)),
};
