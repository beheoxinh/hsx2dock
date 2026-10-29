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
 * Pure cache helpers for the Dashboard store.
 *
 * Kept apart from `dashboardStore` (which imports the Tauri `api`) so the
 * merge/diff/parse rules can be unit-tested without a bridge.
 */

export interface DashboardData {
  engineInfo: EngineInfo | null;
  diskUsage: DfReport | null;
  images: ImageItem[];
  volumes: VolumeItem[];
  networks: NetworkItem[];
  kindStatus: KindInstallStatus | null;
  embeddedBinaries: EmbeddedBinaryInfo[];
  clusters: KindCluster[];
}

export type DashboardDataPatch = Partial<DashboardData>;

/** Panels the dashboard caches; order is irrelevant, used for diffing. */
const DATA_KEYS: (keyof DashboardData)[] = [
  "engineInfo",
  "diskUsage",
  "images",
  "volumes",
  "networks",
  "kindStatus",
  "embeddedBinaries",
  "clusters",
];

/** localStorage schema version — bump to invalidate old payloads. */
export const DASHBOARD_CACHE_VERSION = 1;

/**
 * Structural equality for cached panel values. The payloads are JSON-derived
 * (engine/Docker API responses), so serializing is both accurate and cheap at
 * this size — and it catches nested changes an identity check would miss.
 */
export function sameDashboardValue(a: unknown, b: unknown): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
}

/**
 * Keep only the fields that actually differ from the cached snapshot.
 *
 * A poll that returns the same figures as last time yields `{}`, which lets
 * the store skip the emit entirely: no re-render, no flicker, no redundant
 * localStorage write. Fields missing from `patch` are left untouched so a
 * status-only refresh can never blank the cached lists.
 */
export function changedDashboardFields(
  current: DashboardData,
  patch: DashboardDataPatch
): DashboardDataPatch {
  const changed: DashboardDataPatch = {};
  for (const key of DATA_KEYS) {
    if (!(key in patch)) continue;
    if (!sameDashboardValue(current[key], patch[key])) {
      (changed as Record<string, unknown>)[key] = patch[key];
    }
  }
  return changed;
}

export interface CachedDashboard {
  data: DashboardData;
  statusFetchedAt: number;
  listsFetchedAt: number;
}

function arrayOf<T>(value: unknown): T[] {
  return Array.isArray(value) ? (value as T[]) : [];
}

/**
 * Parse a persisted dashboard payload. Returns `null` for anything malformed
 * or written by an older schema — a bad cache must degrade to a normal fetch,
 * never crash the opening screen.
 */
export function parseCachedDashboard(raw: string | null): CachedDashboard | null {
  if (!raw) return null;
  try {
    const data = JSON.parse(raw) as Record<string, unknown>;
    if (!data || data.v !== DASHBOARD_CACHE_VERSION) return null;
    return {
      data: {
        engineInfo: (data.engineInfo as EngineInfo | null) ?? null,
        diskUsage: (data.diskUsage as DfReport | null) ?? null,
        images: arrayOf<ImageItem>(data.images),
        volumes: arrayOf<VolumeItem>(data.volumes),
        networks: arrayOf<NetworkItem>(data.networks),
        kindStatus: (data.kindStatus as KindInstallStatus | null) ?? null,
        embeddedBinaries: arrayOf<EmbeddedBinaryInfo>(data.embeddedBinaries),
        clusters: arrayOf<KindCluster>(data.clusters),
      },
      statusFetchedAt:
        typeof data.statusFetchedAt === "number" ? data.statusFetchedAt : 0,
      listsFetchedAt:
        typeof data.listsFetchedAt === "number" ? data.listsFetchedAt : 0,
    };
  } catch {
    return null;
  }
}

/** Serialize the cache in the shape `parseCachedDashboard` expects. */
export function serializeDashboardCache(
  data: DashboardData,
  statusFetchedAt: number,
  listsFetchedAt: number
): string {
  return JSON.stringify({
    v: DASHBOARD_CACHE_VERSION,
    statusFetchedAt,
    listsFetchedAt,
    engineInfo: data.engineInfo,
    diskUsage: data.diskUsage,
    images: data.images,
    volumes: data.volumes,
    networks: data.networks,
    kindStatus: data.kindStatus,
    embeddedBinaries: data.embeddedBinaries,
    clusters: data.clusters,
  });
}
