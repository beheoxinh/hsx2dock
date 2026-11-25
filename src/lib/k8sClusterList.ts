/**
 * Pure helpers for the Kubernetes cluster list: health, filtering, sorting,
 * colour/tag normalisation, bulk-run concurrency and list-view persistence.
 *
 * No React and no Tauri IPC here, so every rule is covered by
 * k8sClusterList.test.ts (run with `npm test`).
 */
import type { ClusterAccess, KindCluster, KindClusterPrefs } from "../types";
import type { HealthTone } from "./clusterHealth";

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

export type ClusterHealthState = "running" | "degraded" | "starting" | "stopped";

/** A running cluster younger than this may still be coming up ("starting"). */
export const STARTUP_GRACE_SECONDS = 120;

/**
 * Health of one cluster, from the snapshot the backend already sends.
 *
 * - stopped:  not running.
 * - starting: running, but still inside the startup grace window and a node
 *             is missing, or the API has not answered yet.
 * - degraded: running past the grace window with a node missing, the API not
 *             ready (or never reported), or pods reported but none running.
 * - running:  everything the snapshot can see is healthy.
 *
 * Partially running pods are deliberately NOT degraded: completed Jobs are
 * normal, and a partial count would cry wolf. Only "zero running out of some
 * pods" is treated as a problem.
 */
export function clusterHealth(c: KindCluster): ClusterHealthState {
  if (!c.running) return "stopped";
  const young = c.uptime_seconds != null && c.uptime_seconds < STARTUP_GRACE_SECONDS;
  if (c.running_nodes < c.node_count) return young ? "starting" : "degraded";
  if (c.api_ready !== true) return young ? "starting" : "degraded";
  if (c.pod_count != null && c.pod_count > 0 && (c.running_pod_count ?? 0) === 0) {
    return "degraded";
  }
  return "running";
}

export const HEALTH_LABEL: Record<ClusterHealthState, string> = {
  running: "Running",
  degraded: "Degraded",
  starting: "Starting",
  stopped: "Stopped",
};

export const HEALTH_TONE: Record<ClusterHealthState, HealthTone> = {
  running: "success",
  degraded: "warning",
  starting: "info",
  stopped: "neutral",
};

// ---------------------------------------------------------------------------
// Filter / sort
// ---------------------------------------------------------------------------

export type StatusFilter = "all" | "running" | "stopped" | "degraded";
export const STATUS_FILTERS: readonly StatusFilter[] = ["all", "running", "stopped", "degraded"];

/**
 * "Running" is the superset of live clusters that are not degraded, so a
 * starting cluster shows under it. "Degraded" is the subset that needs
 * attention.
 */
export function matchesStatus(state: ClusterHealthState, filter: StatusFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "running":
      return state === "running" || state === "starting";
    case "stopped":
      return state === "stopped";
    case "degraded":
      return state === "degraded";
  }
}

export type SortKey = "name" | "status" | "created" | "cpu" | "memory";
export const SORT_KEYS: readonly SortKey[] = ["name", "status", "created", "cpu", "memory"];

const STATE_ORDER: Record<ClusterHealthState, number> = {
  degraded: 0,
  starting: 1,
  running: 2,
  stopped: 3,
};

/**
 * Returns a new sorted array. Name is the tiebreaker everywhere. Numeric keys
 * are descending (newest / busiest first) and clusters without a value sink
 * to the bottom. Status puts problems first.
 */
export function sortClusters(list: readonly KindCluster[], key: SortKey): KindCluster[] {
  const byName = (a: KindCluster, b: KindCluster) => a.name.localeCompare(b.name);
  const descending =
    (get: (c: KindCluster) => number | null | undefined) =>
    (a: KindCluster, b: KindCluster): number => {
      const av = get(a);
      const bv = get(b);
      if (av == null && bv == null) return byName(a, b);
      if (av == null) return 1;
      if (bv == null) return -1;
      return bv - av || byName(a, b);
    };
  const comparators: Record<SortKey, (a: KindCluster, b: KindCluster) => number> = {
    name: byName,
    status: (a, b) =>
      STATE_ORDER[clusterHealth(a)] - STATE_ORDER[clusterHealth(b)] || byName(a, b),
    created: descending((c) => c.created),
    cpu: descending((c) => c.cpu_percent),
    memory: descending((c) => c.mem_used_mb),
  };
  return [...list].sort(comparators[key]);
}

export function statusCounts(list: readonly KindCluster[]): Record<StatusFilter, number> {
  const counts: Record<StatusFilter, number> = { all: list.length, running: 0, stopped: 0, degraded: 0 };
  for (const c of list) {
    const state = clusterHealth(c);
    if (matchesStatus(state, "running")) counts.running++;
    if (state === "stopped") counts.stopped++;
    if (state === "degraded") counts.degraded++;
  }
  return counts;
}

/** Sorted unique tags across all clusters' prefs. */
export function collectTags(prefs: Iterable<KindClusterPrefs | undefined>): string[] {
  const seen = new Set<string>();
  for (const p of prefs) {
    for (const t of p?.tags ?? []) seen.add(t);
  }
  return [...seen].sort((a, b) => a.localeCompare(b));
}

export interface ClusterListFilter {
  /** Free text, matched against the name and the tags (case-insensitive). */
  query: string;
  status: StatusFilter;
  /** Exact tag, or null for any. */
  tag: string | null;
}

export function filterClusters(
  list: readonly KindCluster[],
  prefsOf: (name: string) => KindClusterPrefs | undefined,
  filter: ClusterListFilter
): KindCluster[] {
  const q = filter.query.trim().toLowerCase();
  return list.filter((c) => {
    const tags = prefsOf(c.name)?.tags ?? [];
    if (filter.tag !== null && !tags.includes(filter.tag)) return false;
    if (!matchesStatus(clusterHealth(c), filter.status)) return false;
    if (!q) return true;
    return (
      c.name.toLowerCase().includes(q) || tags.some((t) => t.toLowerCase().includes(q))
    );
  });
}

// ---------------------------------------------------------------------------
// Prefs normalisation (mirrors the backend rules so we never send a value it
// would reject)
// ---------------------------------------------------------------------------

export const MAX_TAGS = 8;
export const MAX_TAG_LENGTH = 24;

/** Trim, drop empties and duplicates, cap length and count. */
export function normalizeTags(input: readonly string[]): string[] {
  const out: string[] = [];
  for (const raw of input) {
    const tag = raw.trim().slice(0, MAX_TAG_LENGTH).trim();
    if (!tag || out.includes(tag)) continue;
    out.push(tag);
    if (out.length === MAX_TAGS) break;
  }
  return out;
}

/** Card accent palette offered in the prefs dialog. */
export const CLUSTER_COLOR_PALETTE: readonly string[] = [
  "#3b82f6",
  "#10b981",
  "#f59e0b",
  "#ef4444",
  "#8b5cf6",
  "#ec4899",
  "#06b6d4",
  "#84cc16",
];

/** Theme accent used when a cluster has no colour. */
export const DEFAULT_ACCENT = "#3b82f6";

const HEX_COLOR = /^#[0-9a-fA-F]{6}$/;

/** `#rrggbb` (lower-cased) or "" for default. Anything else is dropped. */
export function normalizeColor(input: string | null | undefined): string {
  if (!input) return "";
  const t = input.trim();
  return HEX_COLOR.test(t) ? t.toLowerCase() : "";
}

export function accentFor(color: string | null | undefined): string {
  return normalizeColor(color) || DEFAULT_ACCENT;
}

// ---------------------------------------------------------------------------
// Host access
// ---------------------------------------------------------------------------

/** How long a fetched `ClusterAccess` is reused before asking again. */
export const ACCESS_TTL_MS = 30_000;

export function isAccessStale(fetchedAt: number | undefined, now: number, ttl = ACCESS_TTL_MS): boolean {
  return fetchedAt === undefined || now - fetchedAt >= ttl;
}

/** One-line warning for the access panel, or null when all is well. */
export function accessWarning(a: ClusterAccess): string | null {
  if (a.ephemeralPort) {
    return "Not reachable from host — recreate the cluster to get a host-reachable API port";
  }
  if (a.apiPort == null) return "No host port is published for the API server";
  if (!a.apiReachable) return "The API port is not answering on the host yet";
  return null;
}

/**
 * Why "Open host terminal" must be disabled, or null when it may be used.
 * `access` is undefined while the first fetch is in flight, null when the
 * fetch failed.
 */
export function hostTerminalBlocker(
  running: boolean,
  access: ClusterAccess | null | undefined
): string | null {
  if (!running) return "Start the cluster to open a terminal";
  if (access === undefined) return "Checking host access…";
  if (access === null) return "Host access is unavailable";
  if (!access.apiReachable) return "The API server is not reachable from the host";
  return null;
}

// ---------------------------------------------------------------------------
// Bulk runs
// ---------------------------------------------------------------------------

export type SettleResult<T> = { item: T; ok: true } | { item: T; ok: false; error: unknown };

/**
 * Runs `worker` over `items` with at most `limit` in flight. Failures do not
 * stop the run; each item reports its own outcome. Results keep input order.
 */
export async function settleWithConcurrency<T>(
  items: readonly T[],
  limit: number,
  worker: (item: T) => Promise<void>,
  onProgress?: (done: number, total: number) => void
): Promise<SettleResult<T>[]> {
  const total = items.length;
  const results = new Array<SettleResult<T>>(total);
  const lanes = Math.max(1, Math.min(Math.floor(limit) || 1, total));
  let next = 0;
  let done = 0;

  const runLane = async (): Promise<void> => {
    while (next < total) {
      const i = next++;
      const item = items[i];
      try {
        await worker(item);
        results[i] = { item, ok: true };
      } catch (error) {
        results[i] = { item, ok: false, error };
      }
      done++;
      onProgress?.(done, total);
    }
  };

  await Promise.all(Array.from({ length: lanes }, () => runLane()));
  return results;
}

// ---------------------------------------------------------------------------
// List view persistence (sort + filters only; the search text is not kept)
// ---------------------------------------------------------------------------

export interface ListView {
  status: StatusFilter;
  sort: SortKey;
  tag: string | null;
}

export const DEFAULT_LIST_VIEW: ListView = { status: "all", sort: "name", tag: null };
export const LIST_VIEW_KEY = "hsx2dock.k8s.clusterList";

export interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** window.localStorage, or null when storage is unavailable (private mode, tests). */
export function browserStorage(): KeyValueStore | null {
  try {
    return typeof window !== "undefined" && window.localStorage ? window.localStorage : null;
  } catch {
    return null;
  }
}

/** Reads the saved view, falling back per field on anything unexpected. */
export function readListView(store: KeyValueStore | null | undefined): ListView {
  if (!store) return { ...DEFAULT_LIST_VIEW };
  try {
    const raw = store.getItem(LIST_VIEW_KEY);
    if (!raw) return { ...DEFAULT_LIST_VIEW };
    const parsed = JSON.parse(raw) as Record<string, unknown> | null;
    if (!parsed || typeof parsed !== "object") return { ...DEFAULT_LIST_VIEW };
    return {
      status: STATUS_FILTERS.includes(parsed.status as StatusFilter)
        ? (parsed.status as StatusFilter)
        : DEFAULT_LIST_VIEW.status,
      sort: SORT_KEYS.includes(parsed.sort as SortKey)
        ? (parsed.sort as SortKey)
        : DEFAULT_LIST_VIEW.sort,
      tag: typeof parsed.tag === "string" && parsed.tag ? parsed.tag : null,
    };
  } catch {
    return { ...DEFAULT_LIST_VIEW };
  }
}

export function writeListView(store: KeyValueStore | null | undefined, view: ListView): void {
  if (!store) return;
  try {
    store.setItem(LIST_VIEW_KEY, JSON.stringify(view));
  } catch {
    // Quota or disabled storage: the view simply is not remembered.
  }
}
