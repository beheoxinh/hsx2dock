/**
 * Shared list sorting for the resource pages (images, volumes, networks,
 * gateway routes, container groups): a key plus a direction, natural and
 * case-insensitive text order, missing values always last, and a stable
 * name tie-break so equal rows never swap between refreshes.
 */

export type SortDir = "asc" | "desc";

export interface SortState<K extends string> {
  key: K;
  dir: SortDir;
}

export type SortValue = string | number | boolean | null | undefined;

const collator = new Intl.Collator("en", { sensitivity: "base", numeric: true });

/** Natural, case-insensitive order (`web2` before `web10`); exact order breaks ties. */
export function compareText(a: string, b: string): number {
  return collator.compare(a, b) || (a < b ? -1 : a > b ? 1 : 0);
}

function isMissing(v: SortValue): boolean {
  return v === null || v === undefined || v === "" || (typeof v === "number" && Number.isNaN(v));
}

function compareValue(a: SortValue, b: SortValue): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  if (typeof a === "boolean" && typeof b === "boolean") return a === b ? 0 : a ? -1 : 1;
  return compareText(String(a), String(b));
}

/**
 * Sorted copy of `items`. `getters[key]` gives the sort value; rows without
 * one go last in either direction; `name` orders equal rows A→Z.
 */
export function sortItems<T, K extends string>(
  items: readonly T[],
  getters: Record<K, (item: T) => SortValue>,
  state: SortState<K>,
  name: (item: T) => string
): T[] {
  const get = getters[state.key];
  const sign = state.dir === "desc" ? -1 : 1;
  return items
    .map((item, i) => ({ item, i, v: get ? get(item) : undefined, n: name(item) }))
    .sort((x, y) => {
      const mx = isMissing(x.v);
      const my = isMissing(y.v);
      if (mx !== my) return mx ? 1 : -1;
      const byValue = mx ? 0 : sign * compareValue(x.v, y.v);
      return byValue || compareText(x.n, y.n) || x.i - y.i;
    })
    .map((e) => e.item);
}

export interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** Saved sort for one list, or `fallback` when missing or no longer valid. */
export function readSort<K extends string>(
  store: KeyValueStore | null | undefined,
  storageKey: string,
  keys: readonly K[],
  fallback: SortState<K>
): SortState<K> {
  if (!store) return { ...fallback };
  try {
    const raw = store.getItem(storageKey);
    if (!raw) return { ...fallback };
    const parsed = JSON.parse(raw) as Partial<SortState<string>> | null;
    const key = keys.includes(parsed?.key as K) ? (parsed!.key as K) : fallback.key;
    const dir = parsed?.dir === "asc" || parsed?.dir === "desc" ? parsed.dir : fallback.dir;
    return { key, dir };
  } catch {
    return { ...fallback };
  }
}

export function writeSort<K extends string>(
  store: KeyValueStore | null | undefined,
  storageKey: string,
  state: SortState<K>
): void {
  try {
    store?.setItem(storageKey, JSON.stringify(state));
  } catch {
    /* storage full or blocked: the choice just isn't remembered */
  }
}
