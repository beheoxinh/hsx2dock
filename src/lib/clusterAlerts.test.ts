/**
 * Unit tests for the cluster alert helpers.
 *
 * Run with:
 *   node --test src/lib/clusterAlerts.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  ALERTS_REFRESH_MS,
  alertKindLabel,
  alertPrefKey,
  alertTooltip,
  highestSeverity,
  sortAlerts,
} from "./clusterAlerts.ts";
import type { ClusterAlert, ClusterAlertKind } from "../types/index.ts";

function alert(over: Partial<ClusterAlert> & { key: string }): ClusterAlert {
  return {
    cluster: "dev",
    kind: "crash-loop",
    severity: "warning",
    namespace: "default",
    object: "pod/web",
    message: "restarting",
    since: 100,
    ...over,
  };
}

test("alertKindLabel returns a human label for every kind", () => {
  const expected: Record<ClusterAlertKind, string> = {
    "crash-loop": "Crash loop",
    "image-pull": "Image pull",
    oom: "OOM kill",
    "restart-spike": "Restart spike",
    "node-not-ready": "Node not ready",
    "api-down": "API server down",
    "pvc-pending": "PVC pending",
  };
  for (const [kind, label] of Object.entries(expected)) {
    assert.equal(alertKindLabel(kind as ClusterAlertKind), label);
  }
});

test("alertPrefKey maps hyphenated kinds to snake_case prefs keys", () => {
  assert.equal(alertPrefKey("crash-loop"), "crash_loop");
  assert.equal(alertPrefKey("image-pull"), "image_pull");
  assert.equal(alertPrefKey("oom"), "oom");
  assert.equal(alertPrefKey("restart-spike"), "restart_spike");
  assert.equal(alertPrefKey("node-not-ready"), "node_not_ready");
  assert.equal(alertPrefKey("api-down"), "api_down");
  assert.equal(alertPrefKey("pvc-pending"), "pvc_pending");
});

test("ALERTS_REFRESH_MS is 30 seconds", () => {
  assert.equal(ALERTS_REFRESH_MS, 30_000);
});

test("highestSeverity is null for no alerts", () => {
  assert.equal(highestSeverity([]), null);
});

test("highestSeverity reports warning when only warnings exist", () => {
  assert.equal(highestSeverity([alert({ key: "a" })]), "warning");
});

test("highestSeverity reports error when any error exists", () => {
  const list = [alert({ key: "a" }), alert({ key: "b", severity: "error" })];
  assert.equal(highestSeverity(list), "error");
});

test("sortAlerts puts errors first, then oldest since, then key", () => {
  const list = [
    alert({ key: "w-old", severity: "warning", since: 1 }),
    alert({ key: "e-new", severity: "error", since: 50 }),
    alert({ key: "e-old", severity: "error", since: 10 }),
    alert({ key: "b", severity: "error", since: 10 }),
    alert({ key: "a", severity: "warning", since: 1 }),
  ];
  assert.deepEqual(
    sortAlerts(list).map((a) => a.key),
    ["b", "e-old", "e-new", "a", "w-old"]
  );
});

test("sortAlerts does not mutate its input", () => {
  const list = [alert({ key: "z", since: 9 }), alert({ key: "y", since: 1 })];
  const before = list.map((a) => a.key);
  const sorted = sortAlerts(list);
  assert.deepEqual(list.map((a) => a.key), before);
  assert.notEqual(sorted, list);
});

test("alertTooltip renders one line per alert", () => {
  const list = [
    alert({ key: "a", kind: "oom", object: "pod/db", message: "killed" }),
    alert({ key: "b", kind: "api-down", object: "apiserver", message: "no answer" }),
  ];
  assert.equal(
    alertTooltip(list),
    "OOM kill pod/db: killed\nAPI server down apiserver: no answer"
  );
});

test("alertTooltip flattens newlines inside a message", () => {
  const list = [alert({ key: "a", message: "line one\nline two" })];
  assert.equal(alertTooltip(list), "Crash loop pod/web: line one line two");
});

test("alertTooltip appends +N more beyond the default max of 6", () => {
  const list = Array.from({ length: 8 }, (_, i) => alert({ key: `k${i}`, object: `pod/p${i}` }));
  const lines = alertTooltip(list).split("\n");
  assert.equal(lines.length, 7);
  assert.equal(lines[0], "Crash loop pod/p0: restarting");
  assert.equal(lines[5], "Crash loop pod/p5: restarting");
  assert.equal(lines[6], "+2 more");
});

test("alertTooltip respects a custom max and gives no suffix at exactly max", () => {
  const list = [alert({ key: "a" }), alert({ key: "b" })];
  assert.equal(alertTooltip(list, 2).split("\n").length, 2);
  assert.equal(alertTooltip(list, 1), "Crash loop pod/web: restarting\n+1 more");
});

test("alertTooltip on empty input is an empty string", () => {
  assert.equal(alertTooltip([]), "");
});
