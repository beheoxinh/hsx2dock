/** Unit tests for small shared helpers. Run with: node --test src/lib/utils.test.ts */
import test from "node:test";
import assert from "node:assert/strict";

import { sortNames } from "./utils.ts";

test("sortNames orders network names A-Z, case-insensitively and naturally", () => {
  const input = ["hsc-net", "hsc-mocknet", "Bridge", "net10", "net2"];
  assert.deepEqual(sortNames(input), ["Bridge", "hsc-mocknet", "hsc-net", "net2", "net10"]);
  // Input is left alone; order does not depend on the input order.
  assert.deepEqual(input, ["hsc-net", "hsc-mocknet", "Bridge", "net10", "net2"]);
  assert.deepEqual(sortNames(["hsc-mocknet", "hsc-net"]), sortNames(["hsc-net", "hsc-mocknet"]));
  assert.deepEqual(sortNames(undefined), []);
});
