import { test } from "node:test";
import assert from "node:assert/strict";
import { joinArgs, splitArgs } from "./shellArgs.ts";

test("quoted arguments stay whole", () => {
  assert.deepEqual(splitArgs(`nginx -g "daemon off;"`), ["nginx", "-g", "daemon off;"]);
  assert.deepEqual(splitArgs(`sh -c 'echo "hi there"'`), ["sh", "-c", `echo "hi there"`]);
  assert.deepEqual(splitArgs(`a  ""  b`), ["a", "", "b"]);
});

test("join round-trips through split", () => {
  const args = ["nginx", "-g", "daemon off;", "it's", ""];
  assert.deepEqual(splitArgs(joinArgs(args)), args);
});
