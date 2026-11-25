/**
 * Pure model of a bundled-binary update (engine, CLI, compose, caddy, kind):
 * the normalised check result, the live install run built from
 * `engine-install-progress` events, and the one status every update panel
 * shows. No React, no Tauri — the store in `binaryUpdates.ts` wires it up.
 */
import type { InstallProgress, UpdateTarget } from "../types";

export type RunStatus = "running" | "done" | "error";

export interface UpdateLogLine {
  at: string;
  text: string;
}

/** One install/update run, as the progress panel renders it. */
export interface UpdateRun {
  version: string;
  plan: string[];
  step: number | null;
  label: string;
  fraction: number | null;
  detail: string | null;
  logs: UpdateLogLine[];
  status: RunStatus;
  error: string | null;
  startedAt: number;
  finishedAt: number | null;
}

/** What a release check found, the same shape for every binary. */
export interface BinaryInfo {
  installed: string | null;
  latest: string;
  updateAvailable: boolean;
  /** Version the app ships pinned checksums for (engine, compose). */
  pinned?: string | null;
}

export interface BinaryState {
  info: BinaryInfo | null;
  checking: boolean;
  /** When the last check finished (ms), 0 = never. */
  checkedAt: number;
  /** Message of the last failed check, cleared by a successful one. */
  checkError: string | null;
  installing: boolean;
  run: UpdateRun | null;
  /** An install that failed before the backend reported any progress. */
  installError: string | null;
}

export const EMPTY_BINARY_STATE: BinaryState = {
  info: null,
  checking: false,
  checkedAt: 0,
  checkError: null,
  installing: false,
  run: null,
  installError: null,
};

export const UPDATE_TARGETS: readonly UpdateTarget[] = ["engine", "cli", "compose", "caddy", "kind"];

const MAX_LOG_LINES = 400;

/** Folds one progress event into the run it belongs to. */
export function applyProgress(prev: UpdateRun | null, p: InstallProgress, at: string, now: number): UpdateRun {
  // A new version, or a run that already ended, starts a fresh transcript.
  const fresh = !prev || prev.version !== p.version || prev.status !== "running";
  const base = fresh ? [] : prev.logs;
  const logs = p.log ? [...base, { at, text: p.log }].slice(-MAX_LOG_LINES) : base;
  const status: RunStatus = p.error ? "error" : p.done ? "done" : "running";
  return {
    version: p.version,
    plan: p.plan,
    step: p.step ?? (fresh ? null : prev.step),
    label: p.label,
    fraction: p.fraction ?? (fresh ? null : prev.fraction),
    detail: p.detail ?? null,
    logs,
    status,
    error: p.error ?? null,
    startedAt: fresh ? now : prev.startedAt,
    finishedAt: status === "running" ? null : now,
  };
}

export type BinaryStatusKind =
  | "updating"
  | "checking"
  | "update"
  | "missing"
  | "current"
  | "failed"
  | "unknown";

export interface BinaryStatus {
  kind: BinaryStatusKind;
  label: string;
}

/** The badge an update panel shows; an install in flight wins over everything. */
export function binaryStatus(s: BinaryState): BinaryStatus {
  if (s.installing || s.run?.status === "running") {
    const pct = s.run?.fraction != null ? ` ${Math.round(s.run.fraction * 100)}%` : "";
    return { kind: "updating", label: `Updating${pct}` };
  }
  if (s.checking && !s.info) return { kind: "checking", label: "Checking…" };
  if (s.info) {
    if (!s.info.installed) return { kind: "missing", label: "Not installed" };
    if (s.info.updateAvailable) return { kind: "update", label: "Update available" };
    return { kind: "current", label: "Up to date" };
  }
  if (s.checkError) return { kind: "failed", label: "Check failed" };
  return { kind: "unknown", label: "Not checked" };
}

/** True when the panel should offer its install/update button. */
export function canApply(s: BinaryState): boolean {
  return !!s.info && (s.info.updateAvailable || !s.info.installed);
}

/** "just now", "5 min ago", "2 h ago" — for the "checked …" hint. */
export function ago(then: number, now: number): string {
  const s = Math.max(0, Math.round((now - then) / 1000));
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.round(h / 24)} d ago`;
}

/** Strips a leading `v` so every panel prints versions the same way. */
export const bareVersion = (v: string | null | undefined): string | null => (v ? v.replace(/^v/i, "") : null);

/** Human message for the errors release checks usually hit. */
export function humanCheckError(e: unknown): string {
  const s = String(e);
  if (/403|forbidden|rate/i.test(s) && /github/i.test(s)) return "GitHub refused the request (rate limited). Try again later.";
  if (/429/.test(s)) return "Rate limited by the release server. Try again later.";
  if (/timeout|timed out/i.test(s)) return "Request timed out. Check your connection.";
  if (/network|connect|dns/i.test(s)) return "Network error. Check your connection.";
  return s;
}
