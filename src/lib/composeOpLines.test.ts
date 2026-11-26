/**
 * Unit tests for batched compose progress lines.
 * Run with: node --test src/lib/composeOpLines.test.ts
 */
import { test } from "node:test";
import assert from "node:assert/strict";
import { appendOpLines, MAX_OP_LINES } from "./composeOpLines.ts";
import type { ComposeOp } from "../components/compose/ComposeProgressPanel";

const op = (id: string, lines: string[] = []): ComposeOp => ({
  id,
  project: "shop",
  title: id,
  status: "running",
  lines,
  started: 0,
});

test("an empty batch returns the same array", () => {
  const ops = [op("a")];
  assert.equal(appendOpLines(ops, []), ops);
});

test("lines for several ops land in order on the right op", () => {
  const ops = [op("a"), op("b")];
  const out = appendOpLines(ops, [
    { opId: "a", line: "1" },
    { opId: "b", line: "x" },
    { opId: "a", line: "2" },
  ]);
  assert.deepEqual(out[0].lines, ["1", "2"]);
  assert.deepEqual(out[1].lines, ["x"]);
  assert.equal(ops[0].lines.length, 0, "input is not mutated");
});

test("lines for an unknown op are ignored and leave the array untouched", () => {
  const ops = [op("a")];
  assert.equal(appendOpLines(ops, [{ opId: "zzz", line: "x" }]), ops);
});

test("the line tail is capped at MAX_OP_LINES", () => {
  const full = Array.from({ length: MAX_OP_LINES }, (_, i) => `l${i}`);
  const out = appendOpLines([op("a", full)], [{ opId: "a", line: "new" }]);
  assert.equal(out[0].lines.length, MAX_OP_LINES);
  assert.equal(out[0].lines[0], "l1");
  assert.equal(out[0].lines[MAX_OP_LINES - 1], "new");
});
