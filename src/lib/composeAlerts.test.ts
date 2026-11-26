/**
 * Unit tests for the compose alert list helpers.
 * Run with: node --test src/lib/composeAlerts.test.ts
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { alertsFor, clearAlerts, mergeAlerts, MAX_ALERTS } from "./composeAlerts.ts";
import type { ComposeAlert } from "../types";

const alert = (over: Partial<ComposeAlert> = {}): ComposeAlert => ({
  project: "shop",
  service: "web",
  container: "shop-web-1",
  kind: "unhealthy",
  message: "healthcheck failed",
  time: 100,
  ...over,
});

test("merge sorts newest first and drops exact duplicates", () => {
  const seeded = [alert({ time: 100 })];
  const live = [alert({ time: 100 }), alert({ time: 200, kind: "crashed" })];
  const merged = mergeAlerts(seeded, live);
  assert.deepEqual(
    merged.map((a) => [a.time, a.kind]),
    [
      [200, "crashed"],
      [100, "unhealthy"],
    ]
  );
});

test("merge caps the list", () => {
  const many = Array.from({ length: MAX_ALERTS + 5 }, (_, i) => alert({ time: i, container: `c${i}` }));
  assert.equal(mergeAlerts(many).length, MAX_ALERTS);
});

test("alertsFor filters by project", () => {
  const list = [alert(), alert({ project: "blog", time: 300 })];
  assert.deepEqual(
    alertsFor(list, "blog").map((a) => a.project),
    ["blog"]
  );
});

test("clear without a project empties the list; with one only that project", () => {
  const list = [alert(), alert({ project: "blog" })];
  assert.deepEqual(
    clearAlerts(list, "shop").map((a) => a.project),
    ["blog"]
  );
  assert.deepEqual(clearAlerts(list), []);
});
