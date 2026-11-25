import { test } from "node:test";
import assert from "node:assert/strict";
import { compareText, readSort, sortItems, writeSort, type KeyValueStore } from "./listSort.ts";

interface Row {
  name: string;
  size?: number;
  driver?: string;
}

const getters = {
  name: (r: Row) => r.name,
  size: (r: Row) => r.size,
  driver: (r: Row) => r.driver,
};

test("compareText is natural and case-insensitive", () => {
  const names = ["web10", "Web2", "api", "web1"];
  assert.deepEqual([...names].sort(compareText), ["api", "web1", "Web2", "web10"]);
});

test("sortItems orders by key and direction, missing values last", () => {
  const rows: Row[] = [
    { name: "b", size: 10 },
    { name: "a" },
    { name: "c", size: 30 },
    { name: "d", size: 20 },
  ];
  const asc = sortItems(rows, getters, { key: "size", dir: "asc" }, (r) => r.name);
  assert.deepEqual(asc.map((r) => r.name), ["b", "d", "c", "a"]);
  const desc = sortItems(rows, getters, { key: "size", dir: "desc" }, (r) => r.name);
  assert.deepEqual(desc.map((r) => r.name), ["c", "d", "b", "a"]);
});

test("sortItems breaks ties by name and keeps the input untouched", () => {
  const rows: Row[] = [
    { name: "zeta", driver: "bridge" },
    { name: "Alpha", driver: "bridge" },
    { name: "mid", driver: "host" },
  ];
  const copy = [...rows];
  const out = sortItems(rows, getters, { key: "driver", dir: "desc" }, (r) => r.name);
  assert.deepEqual(out.map((r) => r.name), ["mid", "Alpha", "zeta"]);
  assert.deepEqual(rows, copy);
});

function memStore(init: Record<string, string> = {}): KeyValueStore {
  const m = new Map(Object.entries(init));
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v) };
}

test("readSort falls back on missing, stale or broken values", () => {
  const keys = ["name", "size"] as const;
  const fb = { key: "name" as const, dir: "asc" as const };
  assert.deepEqual(readSort(null, "k", keys, fb), fb);
  assert.deepEqual(readSort(memStore({ k: "{oops" }), "k", keys, fb), fb);
  assert.deepEqual(readSort(memStore({ k: '{"key":"gone","dir":"desc"}' }), "k", keys, fb), {
    key: "name",
    dir: "desc",
  });
  const store = memStore();
  writeSort(store, "k", { key: "size", dir: "desc" });
  assert.deepEqual(readSort(store, "k", keys, fb), { key: "size", dir: "desc" });
});
