/**
 * Unit tests for the compose stats helpers.
 * Run with: node --test src/lib/composeStats.test.ts
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { pickStats, projectUsage } from "./composeStats.ts";
import type { ComposeProjectInfo, ContainerStats } from "../types";

const st = (cpu: number, mem = 100): ContainerStats => ({
  cpu_percent: cpu,
  mem_usage: mem,
  mem_limit: 1000,
  net_rx: 1,
  net_tx: 2,
});

test("projectUsage sums only the project's containers that have stats", () => {
  const project = { services: [{ id: "a" }, { id: "b" }, { id: "c" }] } as unknown as ComposeProjectInfo;
  const u = projectUsage(project, { a: st(1.5, 10), c: st(2, 30), z: st(99, 99) });
  assert.deepEqual(u, { cpu: 3.5, mem: 40, sampled: 2 });
});

test("pickStats returns prev when the subset is unchanged by value", () => {
  const prev = pickStats({}, { a: st(1), b: st(2) }, ["a"]);
  const next = pickStats(prev, { a: st(1), b: st(5) }, ["a"]);
  assert.equal(next, prev);
});

test("pickStats returns a new record when a value of the subset changes", () => {
  const prev = pickStats({}, { a: st(1) }, ["a"]);
  const next = pickStats(prev, { a: st(2) }, ["a"]);
  assert.notEqual(next, prev);
  assert.equal(next.a.cpu_percent, 2);
});

test("pickStats returns a new record when the id set differs", () => {
  const prev = pickStats({}, { a: st(1), b: st(1) }, ["a", "b"]);
  assert.notEqual(pickStats(prev, { a: st(1), b: st(1) }, ["a"]), prev);
  const missing = pickStats({}, { a: st(1) }, ["a", "b"]);
  assert.deepEqual(Object.keys(missing), ["a"]);
});
