/**
 * Unit tests for the Kubernetes cluster list helpers.
 *
 * Pure logic only — no DOM, no engine, no Tauri IPC. Run with:
 *   node --test src/lib/k8sClusterList.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  ACCESS_TTL_MS,
  DEFAULT_LIST_VIEW,
  LIST_VIEW_KEY,
  accentFor,
  accessWarning,
  clusterHealth,
  collectTags,
  filterClusters,
  hostTerminalBlocker,
  isAccessStale,
  normalizeColor,
  normalizeTags,
  readListView,
  settleWithConcurrency,
  sortClusters,
  statusCounts,
  writeListView,
  type KeyValueStore,
} from "./k8sClusterList.ts";
import type { ClusterAccess, KindCluster, KindClusterPrefs } from "../types/index.ts";

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

const healthy = (over: Partial<KindCluster> = {}): KindCluster =>
  cluster({
    running: true,
    running_nodes: 1,
    uptime_seconds: 3600,
    api_ready: true,
    pod_count: 10,
    running_pod_count: 9,
    ...over,
  });

const access = (over: Partial<ClusterAccess> = {}): ClusterAccess => ({
  context: "kind-dev",
  apiPort: 6443,
  apiReachable: true,
  ephemeralPort: false,
  hostKubeconfig: "/home/u/.kube/config",
  inHostKubeconfig: true,
  hostCurrent: false,
  hostKubectl: true,
  ...over,
});

// ---------------------------------------------------------------------------
// clusterHealth
// ---------------------------------------------------------------------------

test("a stopped cluster is stopped whatever its counts say", () => {
  assert.equal(clusterHealth(cluster({ api_ready: true, running_nodes: 0 })), "stopped");
});

test("a fully healthy running cluster is running", () => {
  assert.equal(clusterHealth(healthy()), "running");
});

test("a young cluster with missing nodes or no API answer is starting", () => {
  assert.equal(
    clusterHealth(healthy({ node_count: 3, running_nodes: 1, uptime_seconds: 30 })),
    "starting"
  );
  assert.equal(clusterHealth(healthy({ api_ready: false, uptime_seconds: 60 })), "starting");
  // Unknown uptime is not treated as young: a silent API is degraded.
  assert.equal(clusterHealth(healthy({ api_ready: null, uptime_seconds: null })), "degraded");
});

test("past the grace window a missing node or an unready API is degraded", () => {
  assert.equal(
    clusterHealth(healthy({ node_count: 3, running_nodes: 2, uptime_seconds: 3600 })),
    "degraded"
  );
  assert.equal(clusterHealth(healthy({ api_ready: false, uptime_seconds: 3600 })), "degraded");
});

test("the grace boundary is exclusive: 120 s old is no longer starting", () => {
  assert.equal(clusterHealth(healthy({ api_ready: false, uptime_seconds: 119 })), "starting");
  assert.equal(clusterHealth(healthy({ api_ready: false, uptime_seconds: 120 })), "degraded");
});

test("pods reported but none running is degraded, a partial count is not", () => {
  assert.equal(
    clusterHealth(healthy({ pod_count: 12, running_pod_count: 0 })),
    "degraded"
  );
  // Completed Jobs make a partial count normal.
  assert.equal(clusterHealth(healthy({ pod_count: 12, running_pod_count: 5 })), "running");
});

// ---------------------------------------------------------------------------
// filter / sort / counts
// ---------------------------------------------------------------------------

const fleet = [
  cluster({ name: "alpha", created: 100, cpu_percent: 5, mem_used_mb: 200 }),
  healthy({ name: "bravo", created: 300, cpu_percent: 40, mem_used_mb: 900 }),
  healthy({ name: "charlie", created: 200, api_ready: false, uptime_seconds: 3600, cpu_percent: null }),
  healthy({ name: "delta", created: 50, node_count: 2, running_nodes: 1, uptime_seconds: 10 }),
];

test("status counts: running includes starting, degraded is its own bucket", () => {
  assert.deepEqual(statusCounts(fleet), {
    all: 4,
    running: 2,
    stopped: 1,
    degraded: 1,
  });
});

test("status filter buckets", () => {
  const names = (f: "all" | "running" | "stopped" | "degraded") =>
    filterClusters(fleet, () => undefined, { query: "", status: f, tag: null }).map((c) => c.name);
  assert.deepEqual(names("all"), ["alpha", "bravo", "charlie", "delta"]);
  assert.deepEqual(names("running"), ["bravo", "delta"]);
  assert.deepEqual(names("stopped"), ["alpha"]);
  assert.deepEqual(names("degraded"), ["charlie"]);
});

test("search matches the name and tags, case-insensitively and trimmed", () => {
  const prefs: Record<string, KindClusterPrefs> = {
    bravo: { autoStart: false, nodeCpuM: 0, nodeMemoryMb: 0, tags: ["Frontend"] },
  };
  const of = (n: string) => prefs[n];
  const match = (query: string) =>
    filterClusters(fleet, of, { query, status: "all", tag: null }).map((c) => c.name);
  assert.deepEqual(match("  ALP "), ["alpha"]);
  assert.deepEqual(match("front"), ["bravo"]);
  assert.deepEqual(match(""), ["alpha", "bravo", "charlie", "delta"]);
});

test("tag filter is an exact match and combines with status", () => {
  const prefs: Record<string, KindClusterPrefs> = {
    bravo: { autoStart: false, nodeCpuM: 0, nodeMemoryMb: 0, tags: ["ci"] },
    charlie: { autoStart: false, nodeCpuM: 0, nodeMemoryMb: 0, tags: ["ci", "x"] },
    alpha: { autoStart: false, nodeCpuM: 0, nodeMemoryMb: 0, tags: ["c"] },
  };
  const of = (n: string) => prefs[n];
  assert.deepEqual(
    filterClusters(fleet, of, { query: "", status: "all", tag: "ci" }).map((c) => c.name),
    ["bravo", "charlie"]
  );
  assert.deepEqual(
    filterClusters(fleet, of, { query: "", status: "running", tag: "ci" }).map((c) => c.name),
    ["bravo"]
  );
});

test("collectTags returns sorted unique tags", () => {
  const tags = collectTags([
    { autoStart: false, nodeCpuM: 0, nodeMemoryMb: 0, tags: ["b", "a"] },
    undefined,
    { autoStart: false, nodeCpuM: 0, nodeMemoryMb: 0, tags: ["a", "c"] },
  ]);
  assert.deepEqual(tags, ["a", "b", "c"]);
});

test("sort by name is alphabetical", () => {
  assert.deepEqual(sortClusters(fleet, "name").map((c) => c.name), ["alpha", "bravo", "charlie", "delta"]);
});

test("sort by status puts degraded, starting, running, stopped in that order", () => {
  const list = [
    cluster({ name: "stop" }),
    healthy({ name: "run" }),
    healthy({ name: "start", uptime_seconds: 5, api_ready: false }),
    healthy({ name: "bad", api_ready: false, uptime_seconds: 3600 }),
  ];
  assert.deepEqual(sortClusters(list, "status").map((c) => c.name), ["bad", "start", "run", "stop"]);
});

test("numeric sorts are descending and push unknown values last", () => {
  assert.deepEqual(sortClusters(fleet, "created").map((c) => c.name), ["bravo", "charlie", "alpha", "delta"]);
  assert.deepEqual(sortClusters(fleet, "cpu").map((c) => c.name), ["bravo", "alpha", "charlie", "delta"]);
  assert.deepEqual(sortClusters(fleet, "memory").map((c) => c.name), ["bravo", "alpha", "charlie", "delta"]);
});

test("sortClusters does not mutate its input", () => {
  const input = [...fleet];
  sortClusters(input, "name");
  assert.deepEqual(input.map((c) => c.name), fleet.map((c) => c.name));
});

// ---------------------------------------------------------------------------
// normalisation
// ---------------------------------------------------------------------------

test("normalizeTags trims, drops empties and duplicates, caps length and count", () => {
  assert.deepEqual(normalizeTags(["  a ", "", "a", "b"]), ["a", "b"]);
  assert.deepEqual(normalizeTags(["x".repeat(40)]), ["x".repeat(24)]);
  const many = Array.from({ length: 12 }, (_, i) => `t${i}`);
  assert.equal(normalizeTags(many).length, 8);
  assert.equal(normalizeTags(many)[0], "t0");
});

test("normalizeColor accepts only #rrggbb and lower-cases it", () => {
  assert.equal(normalizeColor("#3B82F6"), "#3b82f6");
  assert.equal(normalizeColor(" #10b981 "), "#10b981");
  assert.equal(normalizeColor("#fff"), "");
  assert.equal(normalizeColor("red"), "");
  assert.equal(normalizeColor(""), "");
  assert.equal(normalizeColor(undefined), "");
});

test("accentFor falls back to the theme accent", () => {
  assert.equal(accentFor("#ef4444"), "#ef4444");
  assert.equal(accentFor(""), "#3b82f6");
  assert.equal(accentFor("nonsense"), "#3b82f6");
});

// ---------------------------------------------------------------------------
// host access
// ---------------------------------------------------------------------------

test("accessWarning: ephemeral port wins, then missing port, then unreachable", () => {
  assert.match(accessWarning(access({ ephemeralPort: true })) ?? "", /recreate the cluster/);
  assert.match(accessWarning(access({ apiPort: null })) ?? "", /No host port/);
  assert.match(accessWarning(access({ apiReachable: false })) ?? "", /not answering/);
  assert.equal(accessWarning(access()), null);
});

test("isAccessStale honours the TTL and treats a missing entry as stale", () => {
  assert.equal(isAccessStale(undefined, 1_000), true);
  assert.equal(isAccessStale(1_000, 1_000 + ACCESS_TTL_MS - 1), false);
  assert.equal(isAccessStale(1_000, 1_000 + ACCESS_TTL_MS), true);
});

test("hostTerminalBlocker explains each reason in order", () => {
  assert.match(hostTerminalBlocker(false, access()) ?? "", /Start the cluster/);
  assert.match(hostTerminalBlocker(true, undefined) ?? "", /Checking/);
  assert.match(hostTerminalBlocker(true, null) ?? "", /unavailable/);
  assert.match(hostTerminalBlocker(true, access({ apiReachable: false })) ?? "", /not reachable/);
  assert.equal(hostTerminalBlocker(true, access()), null);
});

// ---------------------------------------------------------------------------
// bulk runs
// ---------------------------------------------------------------------------

test("settleWithConcurrency never exceeds the limit", async () => {
  let inFlight = 0;
  let peak = 0;
  const items = Array.from({ length: 7 }, (_, i) => i);
  await settleWithConcurrency(items, 2, async () => {
    inFlight++;
    peak = Math.max(peak, inFlight);
    await new Promise((r) => setTimeout(r, 5));
    inFlight--;
  });
  assert.equal(peak, 2);
});

test("settleWithConcurrency keeps going after a failure and reports each outcome in input order", async () => {
  const progress: Array<[number, number]> = [];
  const results = await settleWithConcurrency(
    ["a", "b", "c"],
    2,
    async (name) => {
      await new Promise((r) => setTimeout(r, name === "a" ? 10 : 1));
      if (name === "b") throw new Error("boom");
    },
    (done, total) => progress.push([done, total])
  );
  assert.deepEqual(results.map((r) => r.item), ["a", "b", "c"]);
  assert.equal(results[0].ok, true);
  assert.equal(results[1].ok, false);
  assert.equal(results[2].ok, true);
  if (!results[1].ok) assert.match(String(results[1].error), /boom/);
  assert.deepEqual(progress[progress.length - 1], [3, 3]);
  assert.equal(progress.length, 3);
});

test("settleWithConcurrency on an empty list resolves to an empty result", async () => {
  assert.deepEqual(await settleWithConcurrency([], 2, async () => {}), []);
});

// ---------------------------------------------------------------------------
// list view persistence
// ---------------------------------------------------------------------------

const memoryStore = (initial: Record<string, string> = {}): KeyValueStore & { data: Record<string, string> } => {
  const data = { ...initial };
  return {
    data,
    getItem: (k) => (k in data ? data[k] : null),
    setItem: (k, v) => {
      data[k] = v;
    },
  };
};

test("list view round-trips through the store", () => {
  const store = memoryStore();
  writeListView(store, { status: "degraded", sort: "memory", tag: "ci" });
  assert.deepEqual(readListView(store), { status: "degraded", sort: "memory", tag: "ci" });
});

test("list view falls back to defaults for missing, corrupt or unknown values", () => {
  assert.deepEqual(readListView(null), DEFAULT_LIST_VIEW);
  assert.deepEqual(readListView(memoryStore()), DEFAULT_LIST_VIEW);
  assert.deepEqual(readListView(memoryStore({ [LIST_VIEW_KEY]: "{not json" })), DEFAULT_LIST_VIEW);
  assert.deepEqual(readListView(memoryStore({ [LIST_VIEW_KEY]: "null" })), DEFAULT_LIST_VIEW);
  assert.deepEqual(
    readListView(memoryStore({ [LIST_VIEW_KEY]: JSON.stringify({ status: "bogus", sort: "cpu", tag: 3 }) })),
    { status: "all", sort: "cpu", tag: null }
  );
});

test("a throwing store never breaks reads or writes", () => {
  const broken: KeyValueStore = {
    getItem: () => {
      throw new Error("denied");
    },
    setItem: () => {
      throw new Error("quota");
    },
  };
  assert.deepEqual(readListView(broken), DEFAULT_LIST_VIEW);
  assert.doesNotThrow(() => writeListView(broken, DEFAULT_LIST_VIEW));
});
