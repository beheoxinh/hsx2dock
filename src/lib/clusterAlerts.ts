/**
 * Pure helpers for cluster incident alerts. No React, no IPC.
 */
import type { ClusterAlert, ClusterAlertKind, K8sAlertPrefs } from "../types";

/** Re-fetch interval for the alerts panel while the document is visible. */
export const ALERTS_REFRESH_MS = 30_000;

const KIND_LABELS: Record<ClusterAlertKind, string> = {
  "crash-loop": "Crash loop",
  "image-pull": "Image pull",
  oom: "OOM kill",
  "restart-spike": "Restart spike",
  "node-not-ready": "Node not ready",
  "api-down": "API server down",
  "pvc-pending": "PVC pending",
};

const PREF_KEYS: Record<ClusterAlertKind, keyof K8sAlertPrefs> = {
  "crash-loop": "crash_loop",
  "image-pull": "image_pull",
  oom: "oom",
  "restart-spike": "restart_spike",
  "node-not-ready": "node_not_ready",
  "api-down": "api_down",
  "pvc-pending": "pvc_pending",
};

export function alertKindLabel(kind: ClusterAlertKind): string {
  return KIND_LABELS[kind];
}

export function alertPrefKey(kind: ClusterAlertKind): keyof K8sAlertPrefs {
  return PREF_KEYS[kind];
}

/** The most serious severity present, or null when there are no alerts. */
export function highestSeverity(
  alerts: readonly ClusterAlert[]
): "error" | "warning" | null {
  if (alerts.some((a) => a.severity === "error")) return "error";
  if (alerts.some((a) => a.severity === "warning")) return "warning";
  return null;
}

/** Errors first, then oldest `since`, then by key. Returns a new array. */
export function sortAlerts(alerts: readonly ClusterAlert[]): ClusterAlert[] {
  const rank = (a: ClusterAlert) => (a.severity === "error" ? 0 : 1);
  return [...alerts].sort((a, b) => {
    const byRank = rank(a) - rank(b);
    if (byRank !== 0) return byRank;
    if (a.since !== b.since) return a.since - b.since;
    return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
  });
}

/**
 * One line per alert in input order: "<kind label> <object>: <message>".
 * Newlines inside a message are flattened so each alert stays on one line.
 * Beyond `max` lines a "+N more" line is appended.
 */
export function alertTooltip(alerts: readonly ClusterAlert[], max = 6): string {
  const lines = alerts
    .slice(0, max)
    .map((a) => `${alertKindLabel(a.kind)} ${a.object}: ${a.message}`.replace(/\s*\n\s*/g, " "));
  if (alerts.length > max) lines.push(`+${alerts.length - max} more`);
  return lines.join("\n");
}
