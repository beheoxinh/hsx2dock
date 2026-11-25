import { useSyncExternalStore } from "react";
import { api, events } from "./api";
import { containersEqual, reconcileContainers, sizesEqual } from "./containerGroups";
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
/**
 * Bumped on every local or echoed group write. A group list fetched before a
 * bump is older than what the store now holds and must not replace it.
 */
let groupsSeq = 0;
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
      if (snapshot.ready && containersEqual(snapshot.containers, containers)) {
        return;
      }
      emit({ containers: reconcileContainers(snapshot.containers, containers), ready: true });
    })
    .catch(() => {
      // No Tauri bridge (tests, plain browser) — polling still works.
      ticksAttached = false;
    });
  events
    .onContainerGroupsChanged((groups) => {
      // While our own saves are queued the optimistic list is newer than any
      // echo; the last save's echo lands once the queue drains.
      if (pendingGroupSaves > 0) return;
      groupsSeq += 1;
      emit({ groups });
    })
    .catch(() => {});
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
let forcedAfter: Promise<void> | null = null;

export function primeContainerStore(force = false): Promise<void> {
  attachTicks();
  if (inflight) {
    // A forced refresh (engine start, endpoint switch, recreate) must not be
    // satisfied by a fetch that began before the change.
    if (!force) return inflight;
    if (!forcedAfter) {
      forcedAfter = inflight.then(() => {
        forcedAfter = null;
        return primeContainerStore(true);
      });
    }
    return forcedAfter;
  }
  if (snapshot.ready && !force) return Promise.resolve();

  emit({ loading: true, error: null });
  const run = (async () => {
    // The list, groups and routes are cheap; disk usage is a per-container
    // inspect pass. Paint the list as soon as it arrives and let the DISK
    // figures fill in afterwards instead of holding the whole page back.
    const seqAtStart = groupsSeq;
    const [containers, groups, routes] = await Promise.allSettled([
      api.listContainers(true),
      api.listContainerGroups(),
      api.listGatewayRoutes(),
    ]);
    const containersOk = containers.status === "fulfilled";
    // The group fetch may have been sent before a save: keep the store's list
    // whenever a save is queued or landed since this load began.
    const groupsFresh =
      groups.status === "fulfilled" && pendingGroupSaves === 0 && groupsSeq === seqAtStart;

    emit({
      containers: containersOk
        ? reconcileContainers(snapshot.containers, containers.value)
        : snapshot.containers,
      groups: groupsFresh ? groups.value : snapshot.groups,
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
      if (!containersEqual(snapshot.containers, containers)) {
        emit({ containers: reconcileContainers(snapshot.containers, containers), ready: true });
      }
    } catch {
      /* engine offline — keep the last known list */
    }
    try {
      const sizes = await api.getContainerSizes();
      const nextSizes = sizes ?? {};
      if (!sizesEqual(snapshot.sizes, nextSizes)) {
        emit({ sizes: nextSizes });
      }
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
      const nextSizes = sizes ?? {};
      if (!sizesEqual(snapshot.sizes, nextSizes)) {
        emit({ sizes: nextSizes });
      }
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
  groupsSeq += 1;
  emit({ groups });
}

let pendingGroupSaves = 0;
let groupSaveChain: Promise<unknown> = Promise.resolve();

/**
 * Show the edit at once and persist saves strictly in order. Concurrent saves
 * (fast drags, a rename while a drop is saving) used to race: an older save
 * finishing last put its stale list back on screen. A failed save reloads the
 * list from the backend and rethrows so the caller can report it.
 */
export function saveContainerGroups(groups: ContainerGroup[]): Promise<void> {
  groupsSeq += 1;
  emit({ groups });
  pendingGroupSaves += 1;
  const run = groupSaveChain.then(() => api.saveContainerGroups(groups));
  groupSaveChain = run.catch(() => {});
  return run.then(
    () => {
      pendingGroupSaves -= 1;
    },
    async (err) => {
      pendingGroupSaves -= 1;
      try {
        const seqAtFailure = groupsSeq;
        const fresh = await api.listContainerGroups();
        // Later saves still queued hold newer edits than the backend list.
        if (pendingGroupSaves === 0 && groupsSeq === seqAtFailure) {
          groupsSeq += 1;
          emit({ groups: fresh });
        }
      } catch {
        // keep the optimistic list; the next prime corrects it
      }
      throw err;
    }
  );
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
    if (typeof document !== "undefined" && document.hidden) return;
    void refreshContainerSizes();
  }, 120_000);
}

/** React binding. Components re-render whenever the snapshot changes. */
export function useContainerStore(): ContainerStoreSnapshot {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
