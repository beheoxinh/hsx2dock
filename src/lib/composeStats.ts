/**
 * Per-project resource numbers from the container stats tick, and helpers that
 * keep those numbers referentially stable when nothing changed, so memoised
 * rows and the drawer do not re-render on every tick.
 */
import type { ComposeProjectInfo, ContainerStats } from "../types";

export interface ProjectUsage {
  cpu: number;
  mem: number;
  /** Containers of the project that appeared in the tick. */
  sampled: number;
}

/** CPU and memory of the project's running containers, from the last stats tick. */
export function projectUsage(project: ComposeProjectInfo, stats: Record<string, ContainerStats>): ProjectUsage {
  let cpu = 0;
  let mem = 0;
  let sampled = 0;
  for (const c of project.services) {
    const s = stats[c.id];
    if (!s) continue;
    sampled += 1;
    cpu += s.cpu_percent;
    mem += s.mem_usage;
  }
  return { cpu, mem, sampled };
}

function sameStats(a: ContainerStats, b: ContainerStats): boolean {
  return (
    a.cpu_percent === b.cpu_percent &&
    a.mem_usage === b.mem_usage &&
    a.mem_limit === b.mem_limit &&
    a.net_rx === b.net_rx &&
    a.net_tx === b.net_tx
  );
}

/**
 * The subset of `next` for the container `ids`. Returns `prev` itself when the
 * subset has the same keys with equal values, so a consumer that only cares
 * about its own containers keeps its reference across unrelated ticks.
 */
export function pickStats(
  prev: Record<string, ContainerStats>,
  next: Record<string, ContainerStats>,
  ids: readonly string[]
): Record<string, ContainerStats> {
  const out: Record<string, ContainerStats> = {};
  for (const id of ids) {
    const s = next[id];
    if (s) out[id] = s;
  }
  const outKeys = Object.keys(out);
  const unchanged =
    outKeys.length === Object.keys(prev).length &&
    outKeys.every((id) => prev[id] !== undefined && sameStats(prev[id], out[id]));
  return unchanged ? prev : out;
}
