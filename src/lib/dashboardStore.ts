import { useSyncExternalStore } from "react";
import { api } from "./api";
import type {
  DfReport,
  EmbeddedBinaryInfo,
  EngineInfo,
  ImageItem,
  KindCluster,
  KindInstallStatus,
  NetworkItem,
  VolumeItem,
} from "../types";

/**
 * App-wide cache for the Dashboard.
 *
 * The dashboard is the screen the app opens on, so fetching on mount meant its
 * tiles, donut and spec panel sat empty while Docker answered. Priming this
 * store at startup (the same way the Containers cache is warmed) lets the page
 * paint from memory instead of flashing placeholders on every visit.
 *
 * Containers are intentionally not duplicated here: the Containers cache is
 * already primed and kept in sync by `containerStore`, so the dashboard reads
 * that snapshot directly.
 */
export interface DashboardStoreSnapshot {
  engineInfo: EngineInfo | null;
  diskUsage: DfReport | null;
  images: ImageItem[];
  volumes: VolumeItem[];
  networks: NetworkItem[];
  kindStatus: KindInstallStatus | null;
  embeddedBinaries: EmbeddedBinaryInfo[];
  clusters: KindCluster[];
  /** True once the first fetch has completed, successful or not. */
  ready: boolean;
  loading: boolean;
}

const EMPTY: DashboardStoreSnapshot = {
  engineInfo: null,
  diskUsage: null,
  images: [],
  volumes: [],
  networks: [],
  kindStatus: null,
  embeddedBinaries: [],
  clusters: [],
  ready: false,
  loading: false,
};

let snapshot = EMPTY;
const listeners = new Set<() => void>();
let inflight: Promise<void> | null = null;
let clustersInflight: Promise<void> | null = null;

function emit(next: Partial<DashboardStoreSnapshot>) {
  snapshot = { ...snapshot, ...next };
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): DashboardStoreSnapshot {
  return snapshot;
}

/**
 * Fetch everything the dashboard shows in one pass.
 *
 * Idempotent: a call while a fetch is already running returns that promise,
 * and a call after the cache is warm is a no-op unless `force` is set. A
 * rejected panel keeps its previous value instead of blanking the page.
 */
export function primeDashboardStore(force = false): Promise<void> {
  if (inflight) return inflight;
  if (snapshot.ready && !force) return Promise.resolve();

  emit({ loading: true });
  const run = (async () => {
    const [infoR, dfR, iR, vR, nR, kindR, binR, clusterR] =
      await Promise.allSettled([
        api.getEngineInfo(),
        api.getDiskUsage(),
        api.listImages(),
        api.listVolumes(),
        api.listNetworks(),
        api.kindStatus(),
        api.getEmbeddedBinaries(),
        api.listKindClusters(),
      ]);

    const next: Partial<DashboardStoreSnapshot> = {};
    if (infoR.status === "fulfilled") next.engineInfo = infoR.value;
    if (dfR.status === "fulfilled") next.diskUsage = dfR.value;
    if (iR.status === "fulfilled") next.images = iR.value;
    if (vR.status === "fulfilled") next.volumes = vR.value;
    if (nR.status === "fulfilled") next.networks = nR.value;
    if (kindR.status === "fulfilled") next.kindStatus = kindR.value;
    if (binR.status === "fulfilled") next.embeddedBinaries = binR.value;
    if (clusterR.status === "fulfilled") next.clusters = clusterR.value;

    emit({ ...next, ready: true, loading: false });
  })();
  inflight = run.finally(() => {
    inflight = null;
  });
  return inflight;
}

/**
 * Refresh just the Kind cluster list (the dashboard card polls this every few
 * seconds while the engine runs). Kept separate so the poll never re-inspects
 * images or disk usage.
 */
export function refreshDashboardClusters(): Promise<void> {
  if (clustersInflight) return clustersInflight;
  const run = api
    .listKindClusters()
    .then((clusters) => {
      emit({ clusters });
    })
    .catch(() => {
      // Engine went away mid-poll — keep the last known snapshot.
    })
    .finally(() => {
      clustersInflight = null;
    });
  clustersInflight = run;
  return run;
}

/** Clear engine-specific figures when the daemon stops. */
export function resetDashboardEngineData() {
  emit({
    engineInfo: null,
    diskUsage: null,
    images: [],
    volumes: [],
    networks: [],
    kindStatus: null,
    embeddedBinaries: [],
    clusters: [],
  });
}

/** React binding. Components re-render whenever the snapshot changes. */
export function useDashboardStore(): DashboardStoreSnapshot {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
