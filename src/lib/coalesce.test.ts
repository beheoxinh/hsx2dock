/**
 * Unit tests for the shared single-flight + TTL fetch.
 * Run with: node --test src/lib/coalesce.test.ts
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { createCoalescedFetch } from "./coalesce.ts";

/** A loader whose calls resolve only when the test releases them. */
function gatedLoader<T>() {
  const calls: { resolve: (v: T) => void; reject: (e: unknown) => void }[] = [];
  const load = () =>
    new Promise<T>((resolve, reject) => {
      calls.push({ resolve, reject });
    });
  return { load, calls };
}

test("concurrent callers share one in-flight request", async () => {
  const g = gatedLoader<number>();
  const f = createCoalescedFetch(g.load, 8000);
  const a = f.get();
  const b = f.get();
  assert.equal(g.calls.length, 1);
  g.calls[0].resolve(7);
  assert.deepEqual(await Promise.all([a, b]), [7, 7]);
});

test("a finished value is reused within the TTL and refetched after it", async () => {
  let t = 1000;
  let n = 0;
  const f = createCoalescedFetch(async () => ++n, 8000, () => t);
  assert.equal(await f.get(), 1);
  t += 7999;
  assert.equal(await f.get(), 1);
  t += 2;
  assert.equal(await f.get(), 2);
  assert.equal(n, 2);
});

test("invalidate forces the next get to refetch", async () => {
  let n = 0;
  const f = createCoalescedFetch(async () => ++n, 8000);
  assert.equal(await f.get(), 1);
  f.invalidate();
  assert.equal(await f.get(), 2);
});

test("a request started before invalidate is not cached and not joined", async () => {
  const g = gatedLoader<string>();
  const f = createCoalescedFetch(g.load, 8000);
  const stale = f.get();
  f.invalidate();
  const fresh = f.get();
  assert.equal(g.calls.length, 2, "get after invalidate starts a new request");
  g.calls[0].resolve("before");
  assert.equal(await stale, "before");
  g.calls[1].resolve("after");
  assert.equal(await fresh, "after");
  // The stale answer must not have been cached over the fresh one.
  assert.equal(await f.get(), "after");
  assert.equal(g.calls.length, 2);
});

test("a failed request is not cached and the next get retries", async () => {
  const g = gatedLoader<number>();
  const f = createCoalescedFetch(g.load, 8000);
  const first = f.get();
  g.calls[0].reject(new Error("engine down"));
  await assert.rejects(first, /engine down/);
  const second = f.get();
  assert.equal(g.calls.length, 2);
  g.calls[1].resolve(3);
  assert.equal(await second, 3);
});
