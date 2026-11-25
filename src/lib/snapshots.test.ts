/**
 * Unit tests for the snapshot helpers. Run with:
 *   node --test src/lib/snapshots.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import { isSingleControlPlane, validateCloneName } from "./snapshots.ts";
import type { ClusterSnapshot } from "../types/index.ts";

const snap = (nodes: string[]): ClusterSnapshot => ({
  id: "20261125-101500",
  cluster: "dev",
  created: "2026-11-25T10:15:00+07:00",
  note: null,
  nodeImage: "kindest/node:v1.31.0",
  nodes,
  sizeBytes: 1024,
});

test("one control plane plus workers can be cloned", () => {
  assert.equal(isSingleControlPlane(snap(["dev-control-plane", "dev-worker", "dev-worker2"])), true);
});

test("a single control-plane node with no workers can be cloned", () => {
  assert.equal(isSingleControlPlane(snap(["dev-control-plane"])), true);
});

test("an HA snapshot (several control planes) cannot be cloned", () => {
  assert.equal(
    isSingleControlPlane(snap(["dev-control-plane", "dev-control-plane2", "dev-control-plane3", "dev-worker"])),
    false
  );
});

test("a snapshot with no control plane cannot be cloned", () => {
  assert.equal(isSingleControlPlane(snap(["dev-worker", "dev-worker2"])), false);
  assert.equal(isSingleControlPlane(snap([])), false);
});

test("worker names are never counted as control planes", () => {
  assert.equal(isSingleControlPlane(snap(["dev-worker", "dev-worker2", "dev-control-plane"])), true);
});

test("validateCloneName accepts a valid new name", () => {
  assert.equal(validateCloneName("dev-copy", ["dev", "prod"]), null);
  assert.equal(validateCloneName("  dev.2  ", ["dev"]), null);
});

test("validateCloneName rejects a name that already exists", () => {
  const msg = validateCloneName("dev", ["dev", "prod"]);
  assert.match(msg ?? "", /already exists/);
  assert.match(validateCloneName(" prod ", ["dev", "prod"]) ?? "", /already exists/);
});

test("validateCloneName reuses the wizard's format rule", () => {
  assert.equal(validateCloneName("", []), "Name is required.");
  assert.equal(validateCloneName("   ", []), "Name is required.");
  assert.match(validateCloneName("bad name", []) ?? "", /letters, digits/);
  assert.match(validateCloneName("under_score", []) ?? "", /letters, digits/);
});
