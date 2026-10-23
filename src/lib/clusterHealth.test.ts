/**
 * Unit tests for the header's cluster health badges.
 *
 * Pure derivation only — no DOM, no engine, no Tauri IPC. Run with:
 *   node --test src/lib/clusterHealth.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import { summarizeClusters } from "./clusterHealth.ts";
import type { KindCluster } from "../types/index.ts";

const cluster = (over: Partial<KindCluster> = {}): KindCluster => ({
  name: "dev",
  running: false,
  node_count: 1,
  running_nodes: 0,
  created: null,
  has_kubeconfig: true,
  uptime_seconds: null,
  last_started_at: null,
  pod_count: null,
  running_pod_count: null,
  api_ready: null,
  cpu_percent: null,
  mem_used_mb: null,
  cpu_quota_m: null,
  mem_limit_mb: null,
  host_mem_total_mb: null,
  ...over,
});

test("no running clusters must never render as healthy", () => {
  // Regression: the label ends in "Running", so StatusBadge's text heuristic
  // used to paint `0/1 Running` green and pulse it.
  const h = summarizeClusters([cluster({ name: "dev" })], true);
  assert.equal(h.clusters.label, "0/1 Running");
  assert.equal(h.clusters.tone, "neutral");
  assert.equal(h.clusters.pulse, false);
});

test("all clusters running is success and pulses", () => {
  const h = summarizeClusters([cluster({ running: true, running_nodes: 1 })], true);
  assert.equal(h.clusters.label, "1 Running");
  assert.equal(h.clusters.tone, "success");
  assert.equal(h.clusters.pulse, true);
});

test("a partial count is a warning and does not pulse", () => {
  const h = summarizeClusters(
    [cluster({ name: "a", running: true }), cluster({ name: "b" })],
    true
  );
  assert.equal(h.clusters.label, "1/2 Running");
  assert.equal(h.clusters.tone, "warning");
  assert.equal(h.clusters.pulse, false);
});

test("no clusters at all shows None while the engine is up", () => {
  assert.equal(summarizeClusters([], true).clusters.label, "None");
  assert.equal(summarizeClusters([], false).clusters.label, "—");
});

test("pods stay hidden until a running control plane reports counts", () => {
  // Stopped clusters carry null counts; nothing to show.
  const stopped = summarizeClusters([cluster()], true);
  assert.equal(stopped.pods, null);

  const live = summarizeClusters(
    [cluster({ running: true, pod_count: 9, running_pod_count: 8 })],
    true
  );
  assert.deepEqual(live.pods, { label: "8/9 Pods", tone: "info" });

  const allUp = summarizeClusters(
    [cluster({ running: true, pod_count: 4, running_pod_count: 4 })],
    true
  );
  assert.deepEqual(allUp.pods, { label: "4/4 Pods", tone: "success" });
});

test("one unreachable control plane flags the whole group", () => {
  const h = summarizeClusters(
    [
      cluster({ name: "a", running: true, api_ready: true }),
      cluster({ name: "b", running: true, api_ready: false }),
    ],
    true
  );
  assert.deepEqual(h.api, { label: "API 1 down", tone: "danger" });
});

test("API badge is absent while every cluster is stopped", () => {
  const h = summarizeClusters([cluster()], true);
  assert.equal(h.api, null);
});

test("tooltip lists pods and API state per cluster", () => {
  const h = summarizeClusters(
    [cluster({ name: "dev", running: true, running_nodes: 1, pod_count: 5, running_pod_count: 5, api_ready: true })],
    true
  );
  assert.equal(h.detail, "dev: running · 5/5 pods · API ready");
  assert.equal(summarizeClusters([], true).detail, "No kind clusters");
});
