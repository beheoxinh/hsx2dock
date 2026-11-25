/**
 * Unit tests for the cluster history helpers.
 *
 * Run with:
 *   node --test src/lib/clusterHistory.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  HISTORY_RANGES,
  HISTORY_REFRESH_MS,
  historyToCardSeries,
  recentHistory,
} from "./clusterHistory.ts";
import type { HistorySample } from "../types/index.ts";

function sample(t: number, extra: Partial<HistorySample> = {}): HistorySample {
  return { t, cpuPercent: t, memMb: t * 2, pods: t + 1, runningPods: t, ...extra };
}

test("HISTORY_RANGES lists the four supported windows in order", () => {
  assert.deepEqual([...HISTORY_RANGES], ["1h", "6h", "24h", "7d"]);
});

test("HISTORY_REFRESH_MS is one minute", () => {
  assert.equal(HISTORY_REFRESH_MS, 60_000);
});

test("historyToCardSeries sorts samples by time ascending", () => {
  const series = historyToCardSeries([sample(300), sample(100), sample(200)]);
  assert.deepEqual(series.pods, [100, 200, 300]);
  assert.deepEqual(series.cpu, [100, 200, 300]);
  assert.deepEqual(series.mem, [200, 400, 600]);
});

test("historyToCardSeries maps pods from runningPods and leaves disk empty", () => {
  const series = historyToCardSeries([sample(1, { runningPods: 7, pods: 9 })]);
  assert.deepEqual(series.pods, [7]);
  assert.deepEqual(series.disk, []);
});

test("historyToCardSeries keeps only the last 40 samples", () => {
  const input = Array.from({ length: 55 }, (_, i) => sample(i));
  const series = historyToCardSeries(input);
  assert.equal(series.cpu.length, 40);
  assert.equal(series.pods[0], 15);
  assert.equal(series.pods[39], 54);
});

test("historyToCardSeries caps after sorting, so the newest samples survive", () => {
  const input = Array.from({ length: 50 }, (_, i) => sample(49 - i));
  const series = historyToCardSeries(input);
  assert.equal(series.pods[0], 10);
  assert.equal(series.pods[39], 49);
});

test("historyToCardSeries on empty input returns empty series", () => {
  assert.deepEqual(historyToCardSeries([]), { pods: [], cpu: [], mem: [], disk: [] });
});

test("recentHistory does not mutate its input", () => {
  const input = [sample(3), sample(1), sample(2)];
  const before = input.map((s) => s.t);
  const out = recentHistory(input);
  assert.deepEqual(input.map((s) => s.t), before);
  assert.deepEqual(out.map((s) => s.t), [1, 2, 3]);
});
