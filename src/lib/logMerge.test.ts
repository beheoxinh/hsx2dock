/**
 * Unit tests for the incremental group-log merge. Run with:
 *   node --test src/lib/logMerge.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import { lineSeconds, mergeLogChunk, nextSince, parseDockerTime } from "./logMerge.ts";

const T = (sec: number, nanos: string) =>
  new Date(sec * 1000).toISOString().replace(/\.\d{3}Z$/, `.${nanos}Z`);
const BASE = 1_795_000_000;

test("lineSeconds reads the leading Docker timestamp only", () => {
  assert.equal(lineSeconds(`${T(BASE, "123456789")} hello`), BASE);
  assert.equal(lineSeconds("no timestamp here"), null);
  assert.equal(lineSeconds("2026 is a year"), null);
});

test("nanosecond timestamps are read to the millisecond", () => {
  assert.equal(
    parseDockerTime("2026-11-25T05:07:56.592092330Z"),
    Date.UTC(2026, 10, 25, 5, 7, 56, 592)
  );
  assert.equal(parseDockerTime("2026-11-25T05:07:56Z"), Date.UTC(2026, 10, 25, 5, 7, 56));
});

test("a first chunk is a fresh tail capped to the newest lines", () => {
  const chunk = [1, 2, 3].map((i) => `${T(BASE + i, "0")} line ${i}`).join("\n") + "\n";
  const buf = mergeLogChunk(undefined, chunk, 2);
  assert.deepEqual(
    buf.lines.map((l) => l.split(" ").slice(1).join(" ")),
    ["line 2", "line 3"]
  );
  assert.equal(buf.lastSec, BASE + 3);
  assert.equal(nextSince(buf), BASE + 3);
  assert.equal(nextSince(undefined), undefined);
});

test("the boundary second re-read by `since` is not duplicated", () => {
  const a = `${T(BASE, "100000000")} a`;
  const b = `${T(BASE, "200000000")} b`;
  const first = mergeLogChunk(undefined, `${a}\n${b}\n`, 100);

  // `since=BASE` returns every line of that second again plus the new one.
  const c = `${T(BASE, "300000000")} c`;
  const d = `${T(BASE + 1, "0")} d`;
  const next = mergeLogChunk(first, `${a}\n${b}\n${c}\n${d}\n`, 100);
  assert.deepEqual(next.lines, [a, b, c, d]);
  assert.equal(next.lastSec, BASE + 1);
});

test("a line repeated within the boundary second is kept once per occurrence", () => {
  const x = `${T(BASE, "500000000")} tick`;
  const first = mergeLogChunk(undefined, `${x}\n`, 100);
  // The same exact line logged twice: one was shown, one is new.
  const next = mergeLogChunk(first, `${x}\n${x}\n`, 100);
  assert.deepEqual(next.lines, [x, x]);
});

test("an empty refresh returns the same buffer object", () => {
  const first = mergeLogChunk(undefined, `${T(BASE, "1")} only\n`, 100);
  assert.equal(mergeLogChunk(first, `${T(BASE, "1")} only\n`, 100), first);
  assert.equal(mergeLogChunk(first, "", 100), first);
});

test("the merged buffer stays capped", () => {
  let buf = mergeLogChunk(undefined, `${T(BASE, "0")} 0\n`, 3);
  for (let i = 1; i <= 5; i++) {
    buf = mergeLogChunk(buf, `${T(BASE + i, "0")} ${i}\n`, 3);
  }
  assert.equal(buf.lines.length, 3);
  assert.ok(buf.lines[2].endsWith(" 5"));
});
