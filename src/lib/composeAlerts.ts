import type { ComposeAlert } from "../types";

/** Keeps the page's alert list bounded; older alerts fall off the end. */
export const MAX_ALERTS = 100;

/**
 * Merges alert lists (seeded history plus live events), newest first.
 * The same alert can arrive twice (seed request racing a live event), so
 * identical rows collapse into one.
 */
export function mergeAlerts(...lists: ComposeAlert[][]): ComposeAlert[] {
  const seen = new Set<string>();
  const out: ComposeAlert[] = [];
  for (const a of lists.flat()) {
    const key = `${a.project}|${a.service}|${a.container}|${a.kind}|${a.time}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(a);
  }
  out.sort((x, y) => y.time - x.time);
  return out.slice(0, MAX_ALERTS);
}

/** Alerts of one project, newest first. */
export function alertsFor(alerts: ComposeAlert[], project: string): ComposeAlert[] {
  return alerts.filter((a) => a.project === project);
}

/** Drops one project's alerts, or every alert when `project` is omitted. */
export function clearAlerts(alerts: ComposeAlert[], project?: string): ComposeAlert[] {
  return project === undefined ? [] : alerts.filter((a) => a.project !== project);
}

/** Human label for an alert kind, shown in the alert list. */
export const ALERT_KIND_LABEL: Record<ComposeAlert["kind"], string> = {
  unhealthy: "Unhealthy",
  crashed: "Crashed",
  "restart-loop": "Restart loop",
};
