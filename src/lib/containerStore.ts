import { useSyncExternalStore } from "react";
import { api, events } from "./api";
import type {
  ContainerGroup,
  ContainerItem,
  GatewayRoute,
} from "../types";

/**
 * App-wide cache for the Containers screen.
 *
 * The page used to fetch everything on mount, which meant every visit showed
 * an empty shell while Docker answered (`list_containers` + a full
 * `container_sizes` inspect pass). The store is primed once when the app
 * starts — and again whenever the engine or endpoint changes — so opening the
 * tab renders from memory instantly. `containers-tick` keeps it live.
 *
 * It is deliberately small: no reducers, no selectors. One snapshot object,
 * replaced on every change so `useSyncExternalStore` can compare by identity.
 */
export interface ContainerStoreSnapshot {
  containers: ContainerItem[];
  sizes: Record<string, number>;
  groups: ContainerGroup[];
  gatewayRoutes: GatewayRoute[];
  /** At least one fetch settled — `containers` is trustworthy even when empty. */
  ready: boolean;
  /** A fetch is in flight (used for refresh affordances, not for skeletons). */
  loading: boolean;
  error: string | null;
}

const EMPTY: ContainerStoreSnapshot = {
  containers: [],
  sizes: {},
  groups: [],
  gatewayRoutes: [],
  ready: false,
  loading: false,
  error: null,
};

let snapshot = EMPTY;
const listeners = new Set<() => void>();
let inflight: Promise<void> | null = null;
let sizesInflight: Promise<void> | null = null;
let ticksAttached = false;
let pollTimer: ReturnType<typeof setInterval> | null = null;

function emit(next: Partial<ContainerStoreSnapshot>) {
  snapshot = { ...snapshot, ...next };
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot(): ContainerStoreSnapshot {
  return snapshot;
}

/** The backend broadcasts the whole list on a timer; mirror it into the cache. */
function attachTicks() {
  if (ticksAttached) return;
  ticksAttached = true;
  events
    .onContainersTick((containers) => {
      emit({ containers, ready: true });
    })
    .catch(() => {
      // No Tauri bridge (tests, plain browser) — polling still works.
      ticksAttached = false;
    });
}

/** Read the cache outside React (event handlers, imperative refreshes). */
export function getContainerStore(): ContainerStoreSnapshot {
  return snapshot;
}

/** Subscribe to cache changes outside React. */
export const subscribeContainerStore = subscribe;

/**
 * Fetch containers, sizes, groups and gateway routes once.
 *
 * Idempotent: a call while a fetch is already running returns that promise,
 * and a call after the cache is warm is a no-op unless `force` is set.
 */
export function primeContainerStore(force = false): Promise<void> {
  attachTicks();
  if (inflight) return inflight;
  if (snapshot.ready && !force) return Promise.resolve();

  emit({ loading: true, error: null });
  const run = (async () => {
    // The list, groups and routes are cheap; disk usage is a per-container
    // inspect pass. Paint the list as soon as it arrives and let the DISK
    // figures fill in afterwards instead of holding the whole page back.
    const [containers, groups, routes] = await Promise.allSettled([
      api.listContainers(true),
      api.listContainerGroups(),
      api.listGatewayRoutes(),
    ]);
    const containersOk = containers.status === "fulfilled";

    emit({
      containers: containersOk ? containers.value : snapshot.containers,
      groups: groups.status === "fulfilled" ? groups.value : snapshot.groups,
      gatewayRoutes:
        routes.status === "fulfilled" ? routes.value : snapshot.gatewayRoutes,
      // A failed first fetch must not look like "no containers": stay on the
      // skeleton until the backend tick or a retry produces a real list.
      ready: containersOk || snapshot.ready,
      loading: false,
      error: containersOk ? null : String(containers.reason),
    });

    try {
      const sizes = await api.getContainerSizes();
      emit({ sizes: sizes ?? {} });
    } catch {
      /* keep the previous figures — the list is already usable */
    }
  })();

  inflight = run.finally(() => {
    inflight = null;
  });
  return inflight;
}

/** Re-read the container list (and sizes) after a lifecycle action. */
export function refreshContainers(): Promise<void> {
  if (inflight) return inflight;
  const run = (async () => {
    try {
      const containers = await api.listContainers(true);
      emit({ containers, ready: true });
    } catch {
      /* engine offline — keep the last known list */
    }
    try {
      const sizes = await api.getContainerSizes();
      emit({ sizes: sizes ?? {} });
    } catch {
      /* keep the previous figures */
    }
  })();
  inflight = run.finally(() => {
    inflight = null;
  });
  return inflight;
}

/** Disk usage only — cheap enough to poll, unlike the full prime. */
export function refreshContainerSizes(): Promise<void> {
  if (sizesInflight) return sizesInflight;
  const run = (async () => {
    try {
      const sizes = await api.getContainerSizes();
      emit({ sizes: sizes ?? {} });
    } catch {
      /* engine offline — keep the last known figures */
    }
  })();
  sizesInflight = run.finally(() => {
    sizesInflight = null;
  });
  return sizesInflight;
}

/** The page owns group edits; mirror them into the cache immediately. */
export function setContainerGroups(groups: ContainerGroup[]) {
  emit({ groups });
}

export function setGatewayRoutes(routes: GatewayRoute[]) {
  emit({ gatewayRoutes: routes });
}

/**
 * Keep the cache warm for the whole app lifetime: attach the backend tick and
 * poll disk usage on a slow interval. Idempotent — safe to call on every mount.
 */
export function startContainerStoreSync() {
  attachTicks();
  void primeContainerStore();
  if (pollTimer) return;
  pollTimer = setInterval(() => {
    void refreshContainerSizes();
  }, 30_000);
}

/** React binding. Components re-render whenever the snapshot changes. */
export function useContainerStore(): ContainerStoreSnapshot {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
