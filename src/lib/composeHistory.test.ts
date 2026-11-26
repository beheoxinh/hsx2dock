/**
 * Unit tests for the sparkline ring buffer and the per-project history store.
 * Pure logic only. Run with: node --test src/lib/composeHistory.test.ts
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRing, historyOf, HISTORY_SAMPLES, pruneHistory, recordSample } from "./composeHistory.ts";

test("ring keeps insertion order before it is full", () => {
  const r = createRing(4);
  r.push(1);
  r.push(2);
  assert.deepEqual(r.values(), [1, 2]);
  assert.equal(r.size, 2);
});

test("ring overwrites the oldest sample past capacity", () => {
  const r = createRing(3);
  for (const v of [1, 2, 3, 4, 5]) r.push(v);
  assert.deepEqual(r.values(), [3, 4, 5]);
  assert.equal(r.size, 3);
});

test("ring with capacity 1 keeps only the newest value", () => {
  const r = createRing(1);
  r.push(7);
  r.push(8);
  assert.deepEqual(r.values(), [8]);
});

test("default capacity is the sample budget of the store", () => {
  const r = createRing();
  for (let i = 0; i < HISTORY_SAMPLES + 10; i++) r.push(i);
  const v = r.values();
  assert.equal(v.length, HISTORY_SAMPLES);
  assert.equal(v[0], 10);
  assert.equal(v[v.length - 1], HISTORY_SAMPLES + 9);
});

test("history store records both series per project", () => {
  recordSample("shop|/a", 1.5, 100);
  recordSample("shop|/a", 2.5, 200);
  assert.deepEqual(historyOf("shop|/a"), { cpu: [1.5, 2.5], mem: [100, 200] });
});

test("unknown project has empty history", () => {
  assert.deepEqual(historyOf("nope|/x"), { cpu: [], mem: [] });
});

test("prune drops projects that are gone", () => {
  recordSample("gone|/g", 1, 1);
  recordSample("kept|/k", 1, 1);
  pruneHistory(new Set(["kept|/k"]));
  assert.deepEqual(historyOf("gone|/g"), { cpu: [], mem: [] });
  assert.equal(historyOf("kept|/k").cpu.length, 1);
});

test("historyOf returns the same object until a new sample lands", () => {
  const key = "identity-test|/tmp/identity";
  recordSample(key, 1, 10);
  const first = historyOf(key);
  assert.equal(historyOf(key), first, "no new sample: same reference");
  recordSample(key, 2, 20);
  const second = historyOf(key);
  assert.notEqual(second, first);
  assert.deepEqual(second.cpu, [1, 2]);
  pruneHistory(new Set());
});
