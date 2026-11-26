/** Unit tests for small shared helpers. Run with: node --test src/lib/utils.test.ts */
import test from "node:test";
import assert from "node:assert/strict";

import { retainKeys, sortNames } from "./utils.ts";

test("retainKeys drops keys outside the keep set and keeps the object when nothing goes", () => {
  const rec = { a: 1, b: 2, c: 3 };
  const kept = retainKeys(rec, new Set(["a", "c", "zzz"]));
  assert.deepEqual(kept, { a: 1, c: 3 });
  assert.deepEqual(rec, { a: 1, b: 2, c: 3 });
  assert.equal(retainKeys(rec, new Set(["a", "b", "c"])), rec);
  assert.deepEqual(retainKeys({}, new Set()), {});
});

test("sortNames orders network names A-Z, case-insensitively and naturally", () => {
  const input = ["hsc-net", "hsc-mocknet", "Bridge", "net10", "net2"];
  assert.deepEqual(sortNames(input), ["Bridge", "hsc-mocknet", "hsc-net", "net2", "net10"]);
  // Input is left alone; order does not depend on the input order.
  assert.deepEqual(input, ["hsc-net", "hsc-mocknet", "Bridge", "net10", "net2"]);
  assert.deepEqual(sortNames(["hsc-mocknet", "hsc-net"]), sortNames(["hsc-net", "hsc-mocknet"]));
  assert.deepEqual(sortNames(undefined), []);
});
