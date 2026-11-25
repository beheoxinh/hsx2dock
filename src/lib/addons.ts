/**
 * Pure helpers for the kind addon catalog (Addons tab) and ingress hints.
 * No React here: the components import these.
 */
import type { AddonStatus } from "../types";

/** Backend ids from src-tauri/src/commands/kind_addons.rs (ADDONS). */
export const DASHBOARD_ADDON_ID = "dashboard";
export const INGRESS_ADDON_ID = "ingress-nginx";

export type AddonTone = "success" | "warning" | "info" | "danger" | "neutral";

const BUSY_STATES: ReadonlySet<string> = new Set(["installing", "uninstalling"]);

/** Install or uninstall is running in the backend right now. */
export function isAddonBusy(state: string): boolean {
  return BUSY_STATES.has(state);
}

export function addonStateTone(state: string): AddonTone {
  switch (state) {
    case "ready":
      return "success";
    case "progressing":
    case "installing":
    case "uninstalling":
      return "info";
    case "error":
      return "danger";
    case "not-installed":
    case "unknown":
    default:
      return "neutral";
  }
}

/** Fast polling while any addon is busy, relaxed otherwise. */
export function addonPollIntervalMs(addons: readonly AddonStatus[]): number {
  return addons.some((a) => isAddonBusy(a.state)) ? 3000 : 15000;
}

/**
 * Rewrites a leading http:// to https://, keeping host and port.
 * Anything else is returned unchanged.
 */
export function toHttpsUrl(url: string): string {
  return url.replace(/^http:\/\//i, "https://");
}
