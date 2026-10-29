/**
 * Unit tests for the dashboard cache merge/parse rules.
 *
 * Pure functions only — no DOM, no engine, no Tauri IPC. Run with:
 *   node --test src/lib/dashboardCache.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  changedDashboardFields,
  parseCachedDashboard,
  serializeDashboardCache,
  type DashboardData,
} from "./dashboardCache.ts";

const data = (over: Partial<DashboardData> = {}): DashboardData => ({
  engineInfo: null,
  diskUsage: null,
  images: [],
  volumes: [],
  networks: [],
  kindStatus: null,
  embeddedBinaries: [],
  clusters: [],
  ...over,
});

const images = (value: unknown): DashboardData["images"] =>
  value as DashboardData["images"];
const clusters = (value: unknown): DashboardData["clusters"] =>
  value as DashboardData["clusters"];
const usage = (value: unknown): DashboardData["diskUsage"] =>
  value as DashboardData["diskUsage"];

test("changedDashboardFields returns only the panels whose value differs", () => {
  const current = data({
    images: images([{ id: "sha256:a" }]),
    clusters: clusters([{ name: "dev" }]),
  });
  const changed = changedDashboardFields(current, {
    images: images([{ id: "sha256:a" }]),
    clusters: clusters([{ name: "dev", running: true }]),
  });
  assert.deepEqual(Object.keys(changed), ["clusters"]);
});

test("a poll that returns the cached values produces no patch", () => {
  const current = data({
    images: images([{ id: "sha256:a" }, { id: "sha256:b" }]),
    diskUsage: usage({ total_reclaimable: 42 }),
  });
  const changed = changedDashboardFields(current, {
    images: images([{ id: "sha256:a" }, { id: "sha256:b" }]),
    diskUsage: usage({ total_reclaimable: 42 }),
  });
  assert.deepEqual(changed, {});
});

test("a status-only patch never touches the cached lists", () => {
  const current = data({ images: images([{ id: "keep-me" }]) });
  const changed = changedDashboardFields(current, {
    diskUsage: usage({ total_reclaimable: 7 }),
  });
  assert.equal("images" in changed, false);
  assert.deepEqual(changed.diskUsage, usage({ total_reclaimable: 7 }));
});

test("parseCachedDashboard rejects absent, malformed and older payloads", () => {
  assert.equal(parseCachedDashboard(null), null);
  assert.equal(parseCachedDashboard("not json"), null);
  assert.equal(parseCachedDashboard(JSON.stringify({ v: 0 })), null);
  assert.equal(parseCachedDashboard(JSON.stringify(["v", 1])), null);
});

test("parseCachedDashboard hydrates a payload and normalizes bad lists", () => {
  const parsed = parseCachedDashboard(
    JSON.stringify({
      v: 1,
      statusFetchedAt: 111,
      listsFetchedAt: 222,
      engineInfo: { version: "27.0.0" },
      images: [{ id: "sha256:a" }],
      volumes: "not-an-array",
      networks: null,
      clusters: [{ name: "dev" }],
    })
  );
  assert.ok(parsed);
  assert.equal(parsed.statusFetchedAt, 111);
  assert.equal(parsed.listsFetchedAt, 222);
  assert.equal(parsed.data.images.length, 1);
  assert.deepEqual(parsed.data.volumes, []);
  assert.deepEqual(parsed.data.networks, []);
  assert.equal(parsed.data.clusters.length, 1);
  assert.equal(parsed.data.embeddedBinaries.length, 0);
});

test("serializeDashboardCache round-trips every cached panel", () => {
  const original = data({
    engineInfo: { version: "27.0.0" } as DashboardData["engineInfo"],
    images: images([{ id: "sha256:a" }]),
    clusters: clusters([{ name: "dev", running: true }]),
  });
  const parsed = parseCachedDashboard(
    serializeDashboardCache(original, 10, 20)
  );
  assert.ok(parsed);
  assert.deepEqual(parsed.data, original);
  assert.equal(parsed.statusFetchedAt, 10);
  assert.equal(parsed.listsFetchedAt, 20);
});
