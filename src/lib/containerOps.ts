import { useSyncExternalStore } from "react";
import { api, events } from "./api";
import {
  getContainerStore,
  refreshContainers,
  subscribeContainerStore,
} from "./containerStore";
import type { ContainerItem, ContainerOp, GroupOp } from "../types";

/**
 * Which containers have a lifecycle action in flight, so their cards can show a
 * spinner instead of a clickable button.
 *
 * Two sources feed it:
 * - backend `container-op` / `group-op` events (group starts, auto-start on
 *   engine boot, tray actions), re-hydrated from `list_container_ops` on start;
 * - actions the page runs directly (`trackContainerOp`).
 *
 * A finished action does not clear the spinner at once: the container list
 * reaches the UI on the next tick, and dropping the spinner before that would
 * flash the old button back for up to two seconds. The entry "settles" until
 * the store shows the expected state (or a short deadline passes).
 */
export type PendingPhase = "queued" | "running" | "settling";

export interface PendingOp {
  op: string;
  phase: PendingPhase;
  group?: string | null;
  /** Deadline (ms epoch) for the settling phase. */
  until?: number;
  /** When the entry entered queued/running (ms epoch), for the watchdog. */
  since?: number;
}

export interface OpNotice {
  title: string;
  detail: string;
}

const SETTLE_MS = 6_000;

let containerOps: Record<string, PendingOp> = {};
let groupOps: Record<string, GroupOp> = {};
const listeners = new Set<() => void>();
const noticeListeners = new Set<(n: OpNotice) => void>();
let attached = false;
let sweepTimer: ReturnType<typeof setTimeout> | null = null;
let refreshTimer: ReturnType<typeof setTimeout> | null = null;

function notify() {
  for (const l of listeners) l();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function notice(n: OpNotice) {
  for (const l of noticeListeners) l(n);
}

/** Toast hook-up: the app shell shows failures that happen in the background. */
export function subscribeOpNotices(cb: (n: OpNotice) => void): () => void {
  noticeListeners.add(cb);
  return () => {
    noticeListeners.delete(cb);
  };
}

function sameOp(a: PendingOp, b: PendingOp): boolean {
  return a.op === b.op && a.phase === b.phase && a.group === b.group && a.until === b.until && a.since === b.since;
}

function setContainerOp(id: string, next: PendingOp | null) {
  const prev = containerOps[id];
  if (next && next.phase !== "settling" && next.since === undefined) {
    next = { ...next, since: prev?.phase === next.phase ? prev.since : Date.now() };
  }
  if (!next && !prev) return;
  // Repeated identical events (every "running" re-send) must not copy the map
  // and re-render every subscribed card.
  if (next && prev && sameOp(prev, next)) return;
  const copy = { ...containerOps };
  if (next) copy[id] = next;
  else delete copy[id];
  containerOps = copy;
  notify();
}

/** One coalesced list refresh after a burst of completions. */
function scheduleRefresh() {
  if (refreshTimer) return;
  refreshTimer = setTimeout(() => {
    refreshTimer = null;
    void refreshContainers();
  }, 250);
}

/** True once the live list reflects what the finished op was meant to do. */
function settled(op: string, c: ContainerItem | undefined): boolean {
  if (op === "remove") return !c;
  if (!c) return true;
  switch (op) {
    case "start":
    case "restart":
    case "unpause":
      return c.state === "running";
    case "stop":
      return c.state !== "running" && c.state !== "restarting";
    case "pause":
      return c.state === "paused";
    default:
      return true;
  }
}

/** Drops settling entries whose container caught up or whose deadline passed. */
function sweep() {
  sweepTimer = null;
  const now = Date.now();
  const { containers } = getContainerStore();
  const byId = new Map(containers.map((c) => [c.id, c]));
  const drop: string[] = [];
  let nextDeadline = Infinity;
  for (const [id, entry] of Object.entries(containerOps)) {
    if (entry.phase !== "settling") continue;
    if ((entry.until ?? 0) <= now || settled(entry.op, byId.get(id))) {
      drop.push(id);
    } else {
      nextDeadline = Math.min(nextDeadline, entry.until ?? now);
    }
  }
  if (drop.length > 0) {
    const copy = { ...containerOps };
    for (const id of drop) delete copy[id];
    containerOps = copy;
    notify();
  }
  if (nextDeadline !== Infinity) armSweep(nextDeadline - now);
}

function armSweep(delay: number) {
  if (sweepTimer) clearTimeout(sweepTimer);
  sweepTimer = setTimeout(sweep, Math.max(50, delay));
}

function settle(id: string, op: string, group?: string | null) {
  setContainerOp(id, { op, phase: "settling", group, until: Date.now() + SETTLE_MS });
  scheduleRefresh();
  armSweep(SETTLE_MS);
}

function applyContainerOp(ev: ContainerOp) {
  switch (ev.phase) {
    case "queued":
    case "running":
      setContainerOp(ev.id, { op: ev.op, phase: ev.phase, group: ev.group });
      break;
    case "done":
      settle(ev.id, ev.op, ev.group);
      break;
    case "failed": {
      setContainerOp(ev.id, null);
      scheduleRefresh();
      // Group failures are summarised once by the group event; a lone
      // failure (tray, auto-start) is reported here.
      if (!ev.group) {
        const c = getContainerStore().containers.find((x) => x.id === ev.id);
        notice({
          title: `Could not ${ev.op} ${c?.name ?? ev.id.slice(0, 12)}`,
          detail: ev.error ?? "Unknown error",
        });
      }
      break;
    }
  }
}

function applyGroupOp(ev: GroupOp) {
  if (ev.group === AUTOSTART_PASS && ev.phase !== "done" && ev.phase !== "failed") {
    // The boot stop pass has no card; only its summary matters.
    return;
  }
  const t = requestTimers.get(ev.group);
  if (t) {
    clearTimeout(t);
    requestTimers.delete(ev.group);
  }
  if (ev.phase === "done" || ev.phase === "failed") {
    groupStarted.delete(ev.group);
    if (groupOps[ev.group]) {
      const copy = { ...groupOps };
      delete copy[ev.group];
      groupOps = copy;
      notify();
    }
    // Members whose own "done" event was lost (webview reload, a pass the
    // backend summarised) must not keep spinning.
    settleGroup(ev.group);
    scheduleRefresh();
    const failed = ev.phase === "failed" || (ev.summary ?? "").includes("failed:");
    if (failed) {
      notice({
        title: `Could not ${ev.op} every container`,
        detail: ev.summary ?? "Unknown error",
      });
    }
    return;
  }
  if (!groupStarted.has(ev.group)) groupStarted.set(ev.group, Date.now());
  groupOps = { ...groupOps, [ev.group]: ev };
  notify();
}

/**
 * Attach the backend listeners once for the app lifetime and restore any
 * operation already running (e.g. auto-start began before the window opened).
 */
export function startContainerOpsSync() {
  if (attached) return;
  attached = true;
  // The snapshot below answers after the listeners are live; an id that
  // already got an event is newer than the snapshot and must not be
  // overwritten by it (a "done" followed by a stale "running" stuck the
  // spinner forever).
  const fresh = new Set<string>();
  events
    .onContainerOp((ev) => {
      fresh.add(ev.id);
      applyContainerOp(ev);
    })
    .catch(() => {
      attached = false;
    });
  events
    .onGroupOp((ev) => {
      fresh.add(`group:${ev.group}`);
      applyGroupOp(ev);
    })
    .catch(() => undefined);
  if (!watchdogTimer) watchdogTimer = setInterval(watchdog, WATCHDOG_EVERY_MS);
  // Settling entries clear as soon as the list shows the new state.
  subscribeContainerStore(() => {
    if (Object.values(containerOps).some((e) => e.phase === "settling")) sweep();
  });
  api
    .listContainerOps()
    .then((snap) => {
      for (const op of snap.containers) if (!fresh.has(op.id)) applyContainerOp(op);
      for (const g of snap.groups) if (!fresh.has(`group:${g.group}`)) applyGroupOp(g);
    })
    .catch(() => undefined);
}

const AUTOSTART_PASS = "__autostart__";
const STUCK_AFTER_MS = 120_000;
/** Past this, an entry is dropped even if the backend still claims it. */
const HARD_STUCK_MS = 600_000;
const WATCHDOG_EVERY_MS = 30_000;
let watchdogTimer: ReturnType<typeof setInterval> | null = null;
/** When each group pass first showed as busy (ms epoch), for the watchdog. */
const groupStarted = new Map<string, number>();

/**
 * Entries queued/running for minutes are re-checked against the backend: an
 * op it no longer reports finished while its event was lost, so the entry
 * settles (and clears once the list agrees) instead of spinning forever.
 * Entries past HARD_STUCK_MS are dropped even when the backend (or the
 * request itself) cannot answer, so a wedged op cannot pin a card forever.
 */
function watchdog() {
  const now = Date.now();
  for (const [group, since] of groupStarted) {
    if (now - since > HARD_STUCK_MS && groupOps[group]) {
      releaseGroup(group);
      clearGroup(group);
    }
  }
  const stale = Object.entries(containerOps).filter(
    ([, e]) => e.phase !== "settling" && now - (e.since ?? now) > STUCK_AFTER_MS
  );
  if (stale.length === 0) return;
  for (const [id, e] of stale) {
    if (now - (e.since ?? now) > HARD_STUCK_MS && containerOps[id] === e) {
      setContainerOp(id, null);
    }
  }
  api
    .listContainerOps()
    .then((snap) => {
      const live = new Set(snap.containers.map((c) => c.id));
      for (const [id, e] of stale) {
        if (!live.has(id) && containerOps[id] === e) settle(id, e.op, e.group);
      }
    })
    .catch(() => undefined);
}

/** True when an action is already pending for this container. */
export function isContainerBusy(id: string): boolean {
  return !!containerOps[id];
}

/**
 * Runs a page-initiated action with the same pending/settling lifecycle as
 * backend bulk operations. Returns false (without running) when the container
 * already has an action in flight — a second click must not double-issue.
 */
export async function trackContainerOp(
  id: string,
  op: string,
  run: () => Promise<unknown>
): Promise<boolean> {
  if (containerOps[id]) return false;
  setContainerOp(id, { op, phase: "running" });
  try {
    await run();
    settle(id, op);
    return true;
  } catch (e) {
    setContainerOp(id, null);
    throw e;
  }
}

/** Cancels a pending local entry (e.g. the user dismissed a confirm dialog). */
export function clearContainerOp(id: string) {
  setContainerOp(id, null);
}

/**
 * Marks containers as queued for a bulk action the backend runs outside the
 * ops registry (compose CLI), until their events or the deadline clear them.
 */
export function markQueued(ids: string[], op: string, group: string) {
  if (ids.length === 0) return;
  const copy = { ...containerOps };
  for (const id of ids) {
    if (!copy[id]) copy[id] = { op, phase: "queued", group, since: Date.now() };
  }
  containerOps = copy;
  notify();
}

/** How long a request placeholder may wait for the backend's first event. */
const REQUEST_TIMEOUT_MS = 30_000;
const requestTimers = new Map<string, ReturnType<typeof setTimeout>>();

/**
 * Shows a group as busy the moment it is clicked. The backend resolves the
 * members before it reports progress, and without this the buttons stayed
 * clickable in that gap. The first `group-op` event replaces the placeholder.
 */
export function requestGroup(group: string, op: string) {
  if (groupOps[group]) return false;
  groupStarted.set(group, Date.now());
  groupOps = {
    ...groupOps,
    [group]: { group, op, phase: "queued", total: 0, completed: 0, summary: null },
  };
  notify();
  const prev = requestTimers.get(group);
  if (prev) clearTimeout(prev);
  requestTimers.set(
    group,
    setTimeout(() => {
      requestTimers.delete(group);
      if (groupOps[group]?.phase === "queued") releaseGroup(group);
    }, REQUEST_TIMEOUT_MS)
  );
  return true;
}

/** Ends a group's busy state started by `requestGroup` (or a bulk call). */
export function releaseGroup(group: string) {
  const t = requestTimers.get(group);
  if (t) {
    clearTimeout(t);
    requestTimers.delete(group);
  }
  groupStarted.delete(group);
  if (!groupOps[group]) return;
  const copy = { ...groupOps };
  delete copy[group];
  groupOps = copy;
  notify();
}

/** True while a group pass (or a request for one) is pending. */
export function isGroupBusy(group: string): boolean {
  return !!groupOps[group];
}

/** Settles every entry still owned by `group` (bulk call returned). */
export function settleGroup(group: string) {
  for (const [id, e] of Object.entries(containerOps)) {
    if (e.group === group && e.phase !== "settling") settle(id, e.op, group);
  }
}

/** Drops every entry still owned by `group` (bulk call failed outright). */
export function clearGroup(group: string) {
  const drop = Object.keys(containerOps).filter((id) => containerOps[id]?.group === group);
  if (drop.length === 0) return;
  const copy = { ...containerOps };
  for (const id of drop) delete copy[id];
  containerOps = copy;
  notify();
}

/** Per-card binding: only the card whose entry changed re-renders. */
export function useContainerOp(id: string): PendingOp | undefined {
  return useSyncExternalStore(
    subscribe,
    () => containerOps[id],
    () => containerOps[id]
  );
}

/** Progress of a group's bulk pass, if one is running. */
export function useGroupOp(id: string): GroupOp | undefined {
  return useSyncExternalStore(
    subscribe,
    () => groupOps[id],
    () => groupOps[id]
  );
}

/** Test hook: reset module state between cases. */
export function __resetContainerOpsForTests() {
  containerOps = {};
  groupOps = {};
  for (const t of requestTimers.values()) clearTimeout(t);
  if (watchdogTimer) clearInterval(watchdogTimer);
  watchdogTimer = null;
  requestTimers.clear();
  groupStarted.clear();
  listeners.clear();
  noticeListeners.clear();
  attached = false;
}

export const __internals = { settled };
