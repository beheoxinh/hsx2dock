import { useSyncExternalStore } from "react";
import { api } from "./api";
import {
  changedDashboardFields,
  parseCachedDashboard,
  serializeDashboardCache,
  type DashboardData,
  type DashboardDataPatch,
} from "./dashboardCache";

/**
 * App-wide cache for the Dashboard.
 *
 * The dashboard is the screen the app opens on, so fetching on mount meant its
 * tiles, donut and spec panel sat empty while Docker answered. This store:
 *
 *  - hydrates from localStorage, so even a cold app start paints the last
 *    known figures before Docker answers;
 *  - caches the slow-moving resource lists (images/volumes/networks) for
 *    minutes and only re-fetches the engine status on a short interval;
 *  - diffs every response against the cache and emits only when something
 *    actually changed, so a poll that returns the same values causes no
 *    re-render at all.
 *
 * Containers are intentionally not duplicated here: the Containers cache is
 * already primed and kept in sync by `containerStore`, so the dashboard reads
 * that snapshot directly.
 */
export interface DashboardStoreSnapshot extends DashboardData {
  /** True once there is something to show — hydrated cache or a first fetch. */
  ready: boolean;
  loading: boolean;
}

/** Engine figures change as containers run: revalidate them often. */
const STATUS_TTL_MS = 30_000;
/** Resource lists only change when images/volumes/networks are edited. */
const LISTS_TTL_MS = 5 * 60_000;

const STORAGE_KEY = "hsx2dock.dashboardCache";

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

function readStorage(): string | null {
  try {
    return typeof localStorage === "undefined"
      ? null
      : localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

const cached = parseCachedDashboard(readStorage());

let snapshot: DashboardStoreSnapshot = cached
  ? { ...EMPTY, ...cached.data, ready: true }
  : EMPTY;
let statusFetchedAt = cached?.statusFetchedAt ?? 0;
let listsFetchedAt = cached?.listsFetchedAt ?? 0;

const listeners = new Set<() => void>();
let inflight: Promise<void> | null = null;
let clustersInflight: Promise<void> | null = null;

function persist() {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(
      STORAGE_KEY,
      serializeDashboardCache(snapshot, statusFetchedAt, listsFetchedAt)
    );
  } catch {
    /* storage disabled or full — the cache is a convenience, not a requirement */
  }
}

function commit(next: Partial<DashboardStoreSnapshot>) {
  snapshot = { ...snapshot, ...next };
  for (const listener of listeners) listener();
}

/**
 * Apply a fetched patch: bump the freshness stamps, drop unchanged fields, and
 * only notify React (and rewrite localStorage) when something moved.
 */
function applyData(
  patch: DashboardDataPatch,
  opts: { status?: boolean; lists?: boolean }
): boolean {
  const changed = changedDashboardFields(snapshot, patch);
  const now = Date.now();
  if (opts.status) statusFetchedAt = now;
  if (opts.lists) listsFetchedAt = now;

  if (snapshot.ready && Object.keys(changed).length === 0) {
    // Same values as before: persist the new freshness stamps, but leave the
    // snapshot identity alone so no component re-renders.
    persist();
    return false;
  }

  commit({ ...changed, ready: true, loading: false });
  persist();
  return true;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): DashboardStoreSnapshot {
  return snapshot;
}

/**
 * Make sure the cache is populated and reasonably fresh.
 *
 * Splits the work so routine revalidation is cheap: engine status, disk usage,
 * Kind info and bundled binaries are refreshed on a short TTL, while the
 * resource lists (images/volumes/networks) are only re-read every few minutes
 * or when `force` is set (engine start/stop, endpoint change, prune).
 *
 * Idempotent: a call while a fetch is already running returns that promise,
 * and a call on a fresh cache is a no-op.
 */
export function primeDashboardStore(force = false): Promise<void> {
  if (inflight) return inflight;

  const now = Date.now();
  const cold = !snapshot.ready;
  const statusDue = cold || force || now - statusFetchedAt > STATUS_TTL_MS;
  const listsDue = cold || force || now - listsFetchedAt > LISTS_TTL_MS;
  if (!statusDue && !listsDue) return Promise.resolve();

  if (cold) commit({ loading: true });

  const run = (async () => {
    const statusPromise = statusDue
      ? Promise.allSettled([
          api.getEngineInfo(),
          api.getDiskUsage(),
          api.kindStatus(),
          api.getEmbeddedBinaries(),
          api.listKindClusters(),
        ])
      : null;
    const listsPromise = listsDue
      ? Promise.allSettled([
          api.listImages(),
          api.listVolumes(),
          api.listNetworks(),
        ])
      : null;

    const [statusR, listsR] = await Promise.all([statusPromise, listsPromise]);
    const patch: DashboardDataPatch = {};

    if (statusR) {
      const [infoR, dfR, kindR, binR, clusterR] = statusR;
      if (infoR.status === "fulfilled") patch.engineInfo = infoR.value;
      if (dfR.status === "fulfilled") patch.diskUsage = dfR.value;
      if (kindR.status === "fulfilled") patch.kindStatus = kindR.value;
      if (binR.status === "fulfilled") patch.embeddedBinaries = binR.value;
      if (clusterR.status === "fulfilled") patch.clusters = clusterR.value;
    }

    if (listsR) {
      const [imagesR, volumesR, networksR] = listsR;
      if (imagesR.status === "fulfilled") patch.images = imagesR.value;
      if (volumesR.status === "fulfilled") patch.volumes = volumesR.value;
      if (networksR.status === "fulfilled") patch.networks = networksR.value;
    }

    applyData(patch, { status: statusDue, lists: listsDue });
  })();
  inflight = run.finally(() => {
    inflight = null;
  });
  return inflight;
}

/**
 * Refresh just the Kind cluster list (the dashboard card polls this every few
 * seconds while the engine runs). Kept separate so the poll never re-inspects
 * images or disk usage, and diffed so an unchanged list causes no re-render.
 */
export function refreshDashboardClusters(): Promise<void> {
  if (clustersInflight) return clustersInflight;
  const run = api
    .listKindClusters()
    .then((clusters) => {
      const changed = changedDashboardFields(snapshot, { clusters });
      if (Object.keys(changed).length === 0) return;
      commit({ ...changed, ready: true });
      persist();
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
  statusFetchedAt = 0;
  listsFetchedAt = 0;
  commit({
    engineInfo: null,
    diskUsage: null,
    images: [],
    volumes: [],
    networks: [],
    kindStatus: null,
    embeddedBinaries: [],
    clusters: [],
  });
  persist();
}

/** React binding. Components re-render whenever the snapshot changes. */
export function useDashboardStore(): DashboardStoreSnapshot {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
