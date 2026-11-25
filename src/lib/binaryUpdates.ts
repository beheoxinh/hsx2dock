/**
 * App-wide store for bundled-binary updates.
 *
 * Check results, the install in flight and its live progress live here, not
 * in a component: an engine update stops and restarts the daemon, and the
 * Settings page (or a card in it) may unmount meanwhile — the run, its log
 * and the "installing" lock must survive that instead of resetting.
 */
import { useSyncExternalStore } from "react";
import { api, events } from "./api";
import {
  EMPTY_BINARY_STATE,
  UPDATE_TARGETS,
  applyProgress,
  humanCheckError,
  type BinaryInfo,
  type BinaryState,
} from "./binaryUpdateModel";
import type { UpdateTarget } from "../types";

/** A successful check is reused for this long before the panel re-asks. */
export const CHECK_TTL_MS = 30 * 60_000;
/** A failed background check backs off this long, quietly. */
export const FAIL_TTL_MS = 10 * 60_000;

type States = Record<UpdateTarget, BinaryState>;

let states: States = Object.fromEntries(
  UPDATE_TARGETS.map((t) => [t, EMPTY_BINARY_STATE])
) as States;
const listeners = new Set<() => void>();
const installedListeners = new Set<(target: UpdateTarget) => void>();
const inflightChecks = new Map<UpdateTarget, Promise<BinaryInfo | null>>();

function patch(target: UpdateTarget, next: Partial<BinaryState>) {
  states = { ...states, [target]: { ...states[target], ...next } };
  listeners.forEach((l) => l());
}

const clock = () =>
  new Date().toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit", second: "2-digit" });

let progressSynced = false;
/** Subscribes once to the shared progress stream; never torn down. */
function ensureProgressSync() {
  if (progressSynced) return;
  progressSynced = true;
  events
    .onInstallProgress((p) => {
      if (!(p.target in states)) return;
      patch(p.target, { run: applyProgress(states[p.target].run, p, clock(), Date.now()), installError: null });
    })
    .catch(() => {
      progressSynced = false;
    });
}

function subscribe(l: () => void) {
  ensureProgressSync();
  listeners.add(l);
  return () => listeners.delete(l);
}

export function getBinaryState(target: UpdateTarget): BinaryState {
  return states[target];
}

/** Live state of one binary's update panel. */
export function useBinaryUpdate(target: UpdateTarget): BinaryState {
  return useSyncExternalStore(subscribe, () => states[target]);
}

/** Every binary at once — for "update available" dots on the tab rail. */
export function useBinaryUpdates(): States {
  return useSyncExternalStore(subscribe, () => states);
}

/** Called after an install succeeds so a card can refresh its own status. */
export function onBinaryInstalled(cb: (target: UpdateTarget) => void): () => void {
  installedListeners.add(cb);
  return () => installedListeners.delete(cb);
}

const CHECKERS: Record<UpdateTarget, () => Promise<BinaryInfo>> = {
  engine: async () => {
    const i = await api.checkEngineUpdate();
    return { installed: i.installed_version, latest: i.latest_version, updateAvailable: i.update_available, pinned: i.pinned_version };
  },
  cli: async () => {
    const i = await api.checkCliUpdate();
    return { installed: i.installed_version, latest: i.latest_version, updateAvailable: i.update_available };
  },
  compose: async () => {
    const i = await api.checkComposeUpdate();
    return { installed: i.installed_version, latest: i.latest_version, updateAvailable: i.update_available, pinned: i.pinned_version };
  },
  caddy: async () => {
    const i = await api.checkCaddyUpdate();
    return { installed: i.installed_version, latest: i.latest_version, updateAvailable: i.update_available };
  },
  kind: async () => {
    const i = await api.checkKindUpdate();
    return { installed: i.installed_version, latest: i.latest_version, updateAvailable: i.update_available };
  },
};

const INSTALLERS: Record<UpdateTarget, (version: string | null) => Promise<unknown>> = {
  engine: (v) => api.installEngineUpdate(v ?? ""),
  cli: (v) => api.installCliUpdate(v ?? ""),
  compose: (v) => api.installComposeUpdate(v ?? ""),
  caddy: (v) => api.installCaddyUpdate(v ?? ""),
  // Without a known release kind installs its pinned default.
  kind: (v) => (v ? api.installKindUpdate(v) : api.installKind()),
};

/**
 * Asks the release channel. Without `force` a recent answer is reused, so
 * opening Settings repeatedly does not hammer GitHub. A background failure is
 * recorded quietly; a forced (clicked) one is thrown for the caller to show.
 */
export async function checkBinary(target: UpdateTarget, opts: { force?: boolean } = {}): Promise<BinaryInfo | null> {
  ensureProgressSync();
  const s = states[target];
  if (s.installing) return s.info;
  if (!opts.force && s.checkedAt) {
    const ttl = s.checkError ? FAIL_TTL_MS : CHECK_TTL_MS;
    if (Date.now() - s.checkedAt < ttl) return s.info;
  }
  const running = inflightChecks.get(target);
  if (running) return running;
  const job = (async () => {
    patch(target, { checking: true });
    try {
      const info = await CHECKERS[target]();
      patch(target, { info, checking: false, checkedAt: Date.now(), checkError: null });
      return info;
    } catch (e) {
      patch(target, { checking: false, checkedAt: Date.now(), checkError: humanCheckError(e) });
      if (opts.force) throw new Error(humanCheckError(e));
      return null;
    } finally {
      inflightChecks.delete(target);
    }
  })();
  inflightChecks.set(target, job);
  return job;
}

/**
 * Installs the latest known release (or the first one). Only one install per
 * binary runs at a time; the result stays in the store for whoever looks.
 */
export async function installBinary(target: UpdateTarget): Promise<string | null> {
  ensureProgressSync();
  const s = states[target];
  if (s.installing) return null;
  const version = s.info?.latest ?? null;
  patch(target, { installing: true, run: null, installError: null });
  try {
    await INSTALLERS[target](version);
    patch(target, { installing: false });
    // The cached answer described the binary just replaced.
    await checkBinary(target, { force: true }).catch(() => {});
    installedListeners.forEach((l) => l(target));
    return states[target].info?.installed ?? version;
  } catch (e) {
    const msg = String(e);
    patch(target, {
      installing: false,
      // The backend usually reported the failure in the run already.
      installError: states[target].run?.error ? null : msg,
    });
    // A failed install may still have changed what is on disk.
    void checkBinary(target, { force: true }).catch(() => {});
    throw new Error(msg);
  }
}

/** Hides a finished run's log. */
export function dismissBinaryRun(target: UpdateTarget) {
  if (states[target].run?.status === "running") return;
  patch(target, { run: null, installError: null });
}
