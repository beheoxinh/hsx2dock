/**
 * Pure helpers for the cluster history charts. No React, no IPC.
 */
import type { HistoryRange, HistorySample } from "../types";

export const HISTORY_RANGES: readonly HistoryRange[] = ["1h", "6h", "24h", "7d"];

/** How often the history panel re-fetches while the document is visible. */
export const HISTORY_REFRESH_MS = 60_000;

/** Most recent samples kept per chart so the line stays readable. */
const MAX_CHART_POINTS = 40;

/**
 * Samples sorted oldest first, capped to the most recent `MAX_CHART_POINTS`.
 * Returns a new array; the input is not mutated.
 */
export function recentHistory(samples: readonly HistorySample[]): HistorySample[] {
  const sorted = [...samples].sort((a, b) => a.t - b.t);
  return sorted.slice(-MAX_CHART_POINTS);
}

/**
 * Chart series for the dashboard card sparklines. `disk` is always empty:
 * the backend history does not carry disk usage, so the caller merges it.
 */
export function historyToCardSeries(samples: readonly HistorySample[]): {
  pods: number[];
  cpu: number[];
  mem: number[];
  disk: number[];
} {
  const recent = recentHistory(samples);
  return {
    pods: recent.map((s) => s.runningPods),
    cpu: recent.map((s) => s.cpuPercent),
    mem: recent.map((s) => s.memMb),
    disk: [],
  };
}
