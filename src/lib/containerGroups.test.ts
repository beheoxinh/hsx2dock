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
  containersEqual,
  dropIndexAt,
  groupOf,
  moveMember,
  reconcileContainers,
  reconcileStats,
  reorderMember,
  routesByContainer,
  sizesEqual,
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

test("containersEqual returns true for identical arrays", () => {
  const c1 = [container("c1"), container("c2")];
  const c2 = [container("c1"), container("c2")];
  assert.equal(containersEqual(c1, c2), true);
});

test("containersEqual detects changes in length", () => {
  const c1 = [container("c1")];
  const c2 = [container("c1"), container("c2")];
  assert.equal(containersEqual(c1, c2), false);
});

test("containersEqual detects changes in container state or status", () => {
  const c1 = [{ ...container("c1"), state: "running", status: "Up 2 hours" }];
  const c2 = [{ ...container("c1"), state: "exited", status: "Exited (0) 5s ago" }];
  assert.equal(containersEqual(c1, c2), false);
});

test("sizesEqual detects differences in container disk sizes", () => {
  assert.equal(sizesEqual({ c1: 1024, c2: 2048 }, { c1: 1024, c2: 2048 }), true);
  assert.equal(sizesEqual({ c1: 1024 }, { c1: 1024, c2: 2048 }), false);
  assert.equal(sizesEqual({ c1: 1024, c2: 2048 }, { c1: 1024, c2: 4096 }), false);
});


test("reconcileContainers keeps unchanged rows and the array itself", () => {
  const prev = [container("a"), container("b")];
  const same = [container("a"), container("b")];
  assert.equal(reconcileContainers(prev, same), prev);

  const changed = [container("a"), { ...container("b"), state: "exited" }];
  const out = reconcileContainers(prev, changed);
  assert.notEqual(out, prev);
  assert.equal(out[0], prev[0], "untouched row keeps its identity");
  assert.equal(out[1], changed[1]);
});

test("reconcileContainers notices network and port changes", () => {
  const prev = [{ ...container("a"), broken_networks: [] as string[] }];
  const next = [{ ...container("a"), broken_networks: ["gone"] }];
  assert.equal(reconcileContainers(prev, next)[0], next[0]);
});

test("reconcileStats drops removed containers and reuses unchanged entries", () => {
  const s = (cpu: number) => ({ cpu_percent: cpu, mem_usage: 1, mem_limit: 2, net_rx: 0, net_tx: 0 });
  const prev = { a: s(1), b: s(2) };
  assert.equal(reconcileStats(prev, { a: s(1), b: s(2) }), prev);
  const next = reconcileStats(prev, { a: s(1) });
  assert.deepEqual(Object.keys(next), ["a"]);
  assert.equal(next.a, prev.a);
});

test("routesByContainer matches by name or published port and stays stable", () => {
  const c = {
    ...container("a", "/web"),
    published_ports: [{ host_ip: "", host_port: 8080, container_port: 80, protocol: "tcp" }],
  };
  const routes = [
    { domain: "web.test", target: "x:1", container_name: "web" },
    { domain: "port.test", target: "127.0.0.1:8080" },
    { domain: "other.test", target: "127.0.0.1:9999" },
  ];
  const first = routesByContainer([c], routes);
  assert.deepEqual(first.get("a")?.map((r) => r.domain), ["web.test", "port.test"]);
  const second = routesByContainer([c], routes, first);
  assert.equal(second.get("a"), first.get("a"));
});
