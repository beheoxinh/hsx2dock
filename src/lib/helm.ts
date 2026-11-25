/**
 * Helm helpers for the Kubernetes Helm tab. Pure logic only: no React, no IPC.
 */

export type HelmTone = "success" | "warning" | "info" | "danger" | "neutral";

/** Repositories offered as one-click adds in the install dialog. */
export const QUICK_REPOS: { name: string; url: string }[] = [
  { name: "bitnami", url: "https://charts.bitnami.com/bitnami" },
  { name: "ingress-nginx", url: "https://kubernetes.github.io/ingress-nginx" },
];

/**
 * A trailing version such as "-4.11.0", "-v17.0.5" or "-1.2.3-rc.1". A bare
 * number such as "-2048" is not a version, so "app-2048" keeps its name.
 */
const VERSION_SUFFIX = /-(?:v\d+(?:\.\d+)*|\d+(?:\.\d+)+)(?:[-+][0-9A-Za-z.-]+)?$/;

/** Strips a trailing version from a chart reference: "redis-v17.0.5" -> "redis". */
export function defaultChartRef(chart: string): string {
  const trimmed = chart.trim();
  const base = trimmed.replace(VERSION_SUFFIX, "");
  return base === "" ? trimmed : base;
}

export const RELEASE_NAME_MAX = 53;
const RELEASE_NAME = /^[a-z](?:[a-z0-9-]*[a-z0-9])?$/;

/** Helm release names: lowercase letters, digits and '-', starting with a letter, ending alphanumeric, at most 53 characters. */
export function isValidReleaseName(name: string): boolean {
  return name.length <= RELEASE_NAME_MAX && RELEASE_NAME.test(name);
}

export function helmStatusTone(status: string): HelmTone {
  const s = status.trim().toLowerCase();
  if (s === "deployed") return "success";
  if (s.startsWith("pending")) return "info";
  if (s === "failed") return "danger";
  if (s === "uninstalling" || s === "superseded") return "warning";
  return "neutral";
}

/** Drops fractional seconds and the zone suffix from Helm's timestamp: "2026-11-20 10:00:00.123 +0000 UTC" -> "2026-11-20 10:00:00". */
export function shortUpdated(updated: string): string {
  return updated.trim().replace(/\.\d+/, "").replace(/\s+[+-]\d{4}(?:\s+[A-Z]+)?$/, "");
}

/** Helm prints "null" or "{}" for a release whose user values are empty. Those are not a values file. */
export function cleanValuesText(text: string): string {
  const t = text.trim();
  return t === "" || t === "null" || t === "{}" ? "" : text;
}

/** Highest revision in a release history, or 0 when empty. The current revision is always the highest. */
export function currentRevision(history: { revision: number }[]): number {
  return history.reduce((max, h) => Math.max(max, h.revision), 0);
}

export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
