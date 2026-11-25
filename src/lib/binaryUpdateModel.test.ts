import { test } from "node:test";
import assert from "node:assert/strict";
import {
  EMPTY_BINARY_STATE,
  ago,
  applyProgress,
  bareVersion,
  binaryStatus,
  canApply,
  humanCheckError,
  type BinaryState,
} from "./binaryUpdateModel.ts";
import type { InstallProgress } from "../types/index.ts";

const ev = (p: Partial<InstallProgress>): InstallProgress => ({
  target: "engine",
  version: "28.1.0",
  label: "Downloading",
  fraction: null,
  plan: ["Download", "Verify", "Install"],
  step: 0,
  ...p,
});

test("applyProgress starts a run and accumulates its log", () => {
  const a = applyProgress(null, ev({ log: "fetching", fraction: 0.2 }), "10:00:00", 1000);
  assert.equal(a.status, "running");
  assert.equal(a.startedAt, 1000);
  assert.deepEqual(a.logs, [{ at: "10:00:00", text: "fetching" }]);

  const b = applyProgress(a, ev({ log: "verified", step: 1, fraction: null }), "10:00:05", 6000);
  assert.equal(b.logs.length, 2);
  assert.equal(b.step, 1);
  assert.equal(b.fraction, 0.2, "a missing fraction keeps the last one");
  assert.equal(b.startedAt, 1000);
  assert.equal(b.finishedAt, null);
});

test("applyProgress marks done and error, and a finished run is not continued", () => {
  const run = applyProgress(null, ev({ log: "x" }), "t", 1);
  const done = applyProgress(run, ev({ done: true }), "t", 9);
  assert.equal(done.status, "done");
  assert.equal(done.finishedAt, 9);

  const again = applyProgress(done, ev({ log: "new" }), "t", 20);
  assert.equal(again.status, "running");
  assert.equal(again.startedAt, 20);
  assert.deepEqual(again.logs.map((l) => l.text), ["new"]);

  const failed = applyProgress(again, ev({ error: "boom" }), "t", 30);
  assert.equal(failed.status, "error");
  assert.equal(failed.error, "boom");
});

test("applyProgress restarts on a different version and caps the log", () => {
  let run = applyProgress(null, ev({ log: "0" }), "t", 0);
  for (let i = 1; i < 450; i++) run = applyProgress(run, ev({ log: String(i) }), "t", i);
  assert.equal(run.logs.length, 400);
  assert.equal(run.logs.at(-1)?.text, "449");
  const other = applyProgress(run, ev({ version: "29.0.0", log: "y" }), "t", 500);
  assert.equal(other.logs.length, 1);
});

const st = (p: Partial<BinaryState>): BinaryState => ({ ...EMPTY_BINARY_STATE, ...p });

test("binaryStatus prefers an install in flight", () => {
  const run = applyProgress(null, ev({ fraction: 0.42 }), "t", 0);
  assert.deepEqual(binaryStatus(st({ run, info: { installed: "1", latest: "2", updateAvailable: true } })), {
    kind: "updating",
    label: "Updating 42%",
  });
  assert.equal(binaryStatus(st({ installing: true })).kind, "updating");
});

test("binaryStatus covers the check outcomes", () => {
  assert.equal(binaryStatus(st({})).kind, "unknown");
  assert.equal(binaryStatus(st({ checking: true })).kind, "checking");
  assert.equal(binaryStatus(st({ checkError: "x" })).kind, "failed");
  assert.equal(binaryStatus(st({ info: { installed: null, latest: "1", updateAvailable: false } })).kind, "missing");
  assert.equal(binaryStatus(st({ info: { installed: "1", latest: "2", updateAvailable: true } })).kind, "update");
  assert.equal(binaryStatus(st({ info: { installed: "2", latest: "2", updateAvailable: false } })).kind, "current");
  // A re-check keeps showing the known answer instead of blanking it.
  assert.equal(
    binaryStatus(st({ checking: true, info: { installed: "2", latest: "2", updateAvailable: false } })).kind,
    "current"
  );
});

test("canApply only for an update or a missing binary", () => {
  assert.equal(canApply(st({})), false);
  assert.equal(canApply(st({ info: { installed: "2", latest: "2", updateAvailable: false } })), false);
  assert.equal(canApply(st({ info: { installed: "1", latest: "2", updateAvailable: true } })), true);
  assert.equal(canApply(st({ info: { installed: null, latest: "2", updateAvailable: false } })), true);
});

test("ago, bareVersion and humanCheckError", () => {
  assert.equal(ago(0, 10_000), "just now");
  assert.equal(ago(0, 5 * 60_000), "5 min ago");
  assert.equal(ago(0, 3 * 3_600_000), "3 h ago");
  assert.equal(ago(0, 72 * 3_600_000), "3 d ago");
  assert.equal(bareVersion("v2.7.1"), "2.7.1");
  assert.equal(bareVersion(null), null);
  assert.match(humanCheckError("GitHub API 403 rate limit"), /rate limited/);
  assert.match(humanCheckError("operation timed out"), /timed out/);
  assert.equal(humanCheckError("weird"), "weird");
});
