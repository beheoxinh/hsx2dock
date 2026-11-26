/**
 * CPU and memory history per compose project, for the card sparklines.
 *
 * A fixed-size ring buffer per series: old samples fall off without shifting
 * an array on every tick. The store is module-level so the history survives
 * the page unmounting (switching tabs) and the sparkline is not empty on
 * return.
 */

/** Samples kept per series; at the stats tick rate this is about a minute and a half. */
export const HISTORY_SAMPLES = 60;

export interface Ring {
  push(value: number): void;
  /** Oldest first. */
  values(): number[];
  readonly size: number;
}

/** Fixed-capacity ring; `push` past capacity overwrites the oldest sample. */
export function createRing(capacity: number = HISTORY_SAMPLES): Ring {
  const cap = Math.max(1, Math.floor(capacity));
  const buf = new Array<number>(cap);
  let head = 0; // next write slot
  let count = 0;
  return {
    push(value: number) {
      buf[head] = value;
      head = (head + 1) % cap;
      if (count < cap) count += 1;
    },
    values() {
      const out: number[] = [];
      const start = count < cap ? 0 : head;
      for (let i = 0; i < count; i++) out.push(buf[(start + i) % cap]);
      return out;
    },
    get size() {
      return count;
    },
  };
}

export interface ProjectHistory {
  cpu: number[];
  mem: number[];
}

interface Series {
  cpu: Ring;
  mem: Ring;
  /** Last snapshot handed out; dropped on the next sample so unchanged series keep their identity. */
  snapshot: ProjectHistory | null;
}

const store = new Map<string, Series>();

/** Records one sample for a project; both series advance together. */
export function recordSample(project: string, cpu: number, mem: number): void {
  let s = store.get(project);
  if (!s) {
    s = { cpu: createRing(), mem: createRing(), snapshot: null };
    store.set(project, s);
  }
  s.cpu.push(cpu);
  s.mem.push(mem);
  s.snapshot = null;
}

const EMPTY: ProjectHistory = { cpu: [], mem: [] };

/**
 * The project's history, oldest first; empty arrays when none was recorded.
 * The same object is returned until a new sample lands, so memoised cards do
 * not re-render for projects that had no new sample.
 */
export function historyOf(project: string): ProjectHistory {
  const s = store.get(project);
  if (!s) return EMPTY;
  if (!s.snapshot) s.snapshot = { cpu: s.cpu.values(), mem: s.mem.values() };
  return s.snapshot;
}

/** Drops history of projects that no longer exist (keeps the store bounded). */
export function pruneHistory(keep: ReadonlySet<string>): void {
  for (const key of [...store.keys()]) {
    if (!keep.has(key)) store.delete(key);
  }
}
