/**
 * Unit tests for the container-group folder semantics.
 *
 * Pure logic only — no DOM, no engine, no Tauri IPC. Run with:
 *   node --test src/lib/containerGroups.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  canonicalContainerIds,
  dropIndexAt,
  groupOf,
  moveMember,
  reorderMember,
} from "./containerGroups.ts";
import type { ContainerGroup, ContainerItem } from "../types/index.ts";

const container = (id: string, name = `/${id}`): ContainerItem => ({
  id,
  short_id: id.slice(0, 6),
  name,
  image: "alpine:3",
  state: "running",
  status: "Up",
  created: 0,
  ports: "",
});

const group = (id: string, members: string[]): ContainerGroup => ({
  id,
  name: id,
  container_ids: members,
  auto_start: false,
});

const containers = [container("aaa111"), container("bbb222"), container("ccc333")];
const map = canonicalContainerIds(containers);

test("moveMember takes a container out of every group before inserting it", () => {
  const groups = [group("g1", ["aaa111"]), group("g2", [])];
  const next = moveMember(groups, map, "aaa111", "g2");

  assert.deepEqual(next.find((g) => g.id === "g1")!.container_ids, []);
  assert.deepEqual(next.find((g) => g.id === "g2")!.container_ids, ["aaa111"]);
  assert.equal(groupOf(next, map, "aaa111")?.id, "g2");
});

test("moveMember with a null group releases the container to Isolated", () => {
  const next = moveMember([group("g1", ["aaa111", "bbb222"])], map, "aaa111", null);
  assert.deepEqual(next[0].container_ids, ["bbb222"]);
  assert.equal(groupOf(next, map, "aaa111"), undefined);
});

test("moveMember drops stale aliases that resolve to the same container", () => {
  // A group may hold both the full id and the name of the same container.
  const next = moveMember([group("g1", ["aaa111", "/aaa111"])], map, "aaa111", "g1");
  assert.deepEqual(next[0].container_ids, ["aaa111"]);
});

test("reorderMember rewrites the stored order", () => {
  const groups = [group("g1", ["aaa111", "bbb222", "ccc333"])];
  const next = reorderMember(groups, map, "ccc333", "g1", ["ccc333", "aaa111", "bbb222"]);
  assert.deepEqual(next[0].container_ids, ["ccc333", "aaa111", "bbb222"]);
});

test("reorderMember is a no-op when the order would not change", () => {
  const groups = [group("g1", ["aaa111", "bbb222"])];
  const same = reorderMember(groups, map, "bbb222", "g1", ["aaa111", "bbb222"]);
  assert.equal(same, groups, "unchanged order must not dirty the saved config");
});

test("reorderMember refuses a container that is not in the group", () => {
  const groups = [group("g1", ["aaa111"]), group("g2", ["bbb222"])];
  const next = reorderMember(groups, map, "bbb222", "g1", ["bbb222", "aaa111"]);
  assert.equal(next, groups);
});

test("reorderMember drops ids that no longer resolve to a container", () => {
  const groups = [group("g1", ["aaa111", "bbb222"])];
  const next = reorderMember(groups, map, "bbb222", "g1", ["bbb222", "ghost", "aaa111"]);
  assert.deepEqual(next[0].container_ids, ["bbb222", "aaa111"]);
});

test("dropIndexAt counts the cards whose midpoint the pointer passed", () => {
  const card = (id: string, top: number, height = 40) =>
    ({ dataset: { containerCard: id }, getBoundingClientRect: () => ({ top, height }) }) as unknown as HTMLElement;
  const cards = [card("aaa111", 100), card("bbb222", 160), card("ccc333", 220)];

  // Above every midpoint → insert at the top.
  assert.equal(dropIndexAt(cards, 90, null), 0);
  // Past the first midpoint only → index 1.
  assert.equal(dropIndexAt(cards, 125, null), 1);
  // Past every midpoint → append.
  assert.equal(dropIndexAt(cards, 400, null), 3);
});

test("dropIndexAt ignores the dragged card itself", () => {
  const card = (id: string, top: number) =>
    ({ dataset: { containerCard: id }, getBoundingClientRect: () => ({ top, height: 40 }) }) as unknown as HTMLElement;
  const cards = [card("aaa111", 100), card("bbb222", 160), card("ccc333", 220)];

  // Dragging the middle card: its own slot must not count toward the index.
  assert.equal(dropIndexAt(cards, 400, "bbb222"), 2);
});
