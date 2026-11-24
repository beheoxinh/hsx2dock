import type {
  ContainerGroup,
  ContainerItem,
  ContainerStats,
  PublishedPort,
} from "../types";

/**
 * Maps every token a stored group might hold (full id, short id, container
 * name with or without the leading slash) to the canonical container id.
 * Groups persist across restarts, so a stored token can predate a rename.
 */
export function canonicalContainerIds(
  containers: ContainerItem[]
): Map<string, string> {
  const map = new Map<string, string>();
  for (const c of containers) {
    map.set(c.id, c.id);
    map.set(c.short_id, c.id);
    map.set(c.name, c.id);
    map.set(c.name.replace(/^\//, ""), c.id);
  }
  return map;
}

const canonical = (map: Map<string, string>, token: string) => map.get(token) ?? token;

/** The group a container currently belongs to, if any. */
export function groupOf(
  groups: ContainerGroup[],
  map: Map<string, string>,
  containerId: string
): ContainerGroup | undefined {
  return groups.find((g) =>
    g.container_ids.some((token) => canonical(map, token) === containerId)
  );
}

/**
 * Moves a container into `groupId`, or out of every group when `groupId` is
 * null. A container lives in exactly one group — like a file in one folder —
 * so the move removes it from every other group first. Tokens that resolve to
 * the same container are dropped too, which keeps stale aliases from
 * duplicating a member.
 */
export function moveMember(
  groups: ContainerGroup[],
  map: Map<string, string>,
  containerId: string,
  groupId: string | null
): ContainerGroup[] {
  return groups.map((g) => {
    const ids = g.container_ids.filter(
      (token) => canonical(map, token) !== containerId
    );
    if (g.id !== groupId) return { ...g, container_ids: ids };
    return { ...g, container_ids: [...ids, containerId] };
  });
}

/**
 * Reorders a container within the group that already holds it.
 *
 * Members are the group's cards in display order, so this rewrites the stored
 * token list to match what the user just dragged. It bails out when the
 * container is unknown, is not in `groupId`, or the new position is where the
 * card already was — a no-op must not dirty the saved config.
 */
export function reorderMember(
  groups: ContainerGroup[],
  map: Map<string, string>,
  containerId: string,
  groupId: string,
  orderedIds: string[]
): ContainerGroup[] {
  const group = groups.find((g) => g.id === groupId);
  if (!group) return groups;
  const current = group.container_ids.filter((token) => map.has(token));
  if (!current.some((token) => canonical(map, token) === containerId)) {
    return groups;
  }
  const next = orderedIds.filter((id) => map.has(id));
  const same =
    next.length === current.length &&
    next.every((id, i) => canonical(map, current[i]) === id);
  if (same) return groups;
  return groups.map((g) => (g.id === groupId ? { ...g, container_ids: next } : g));
}

/**
 * Picks the member a pointer is hovering over inside a group's card stack.
 *
 * Folders accept drops anywhere, but *order* only means something between two
 * cards, so the target card is the one whose midpoint the pointer has passed —
 * dropping in the empty space under the stack appends.
 */
export function dropIndexAt(
  cardElements: HTMLElement[],
  y: number,
  draggedId: string | null
): number {
  let index = 0;
  for (let i = 0; i < cardElements.length; i++) {
    const el = cardElements[i];
    if (draggedId !== null && el.dataset.containerCard === draggedId) continue;
    const box = el.getBoundingClientRect();
    if (y >= box.top + box.height / 2) index++;
  }
  return index;
}

/** Removes every token that points at a container which no longer exists. */
export function pruneMissingMembers(
  groups: ContainerGroup[],
  map: Map<string, string>
): ContainerGroup[] {
  return groups.map((g) => {
    const ids = g.container_ids.filter((token) => map.has(token));
    return ids.length === g.container_ids.length
      ? g
      : { ...g, container_ids: ids };
  });
}

/**
 * Fast equality comparison for container snapshots. Prevents redundant React re-renders
 * across background poll intervals when container items haven't changed.
 */
export function containersEqual(a: ContainerItem[], b: ContainerItem[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (!containerEqual(a[i], b[i])) return false;
  }
  return true;
}

const sameList = (a?: string[], b?: string[]) =>
  a === b ||
  ((a?.length ?? 0) === (b?.length ?? 0) &&
    (a ?? []).every((x, i) => x === (b ?? [])[i]));

const samePorts = (a?: PublishedPort[], b?: PublishedPort[]) =>
  a === b ||
  ((a?.length ?? 0) === (b?.length ?? 0) &&
    (a ?? []).every((p, i) => {
      const q = (b ?? [])[i];
      return (
        p.host_ip === q.host_ip &&
        p.host_port === q.host_port &&
        p.container_port === q.container_port &&
        p.protocol === q.protocol
      );
    }));

/** Field-level equality of one container row (what the cards render). */
export function containerEqual(ca: ContainerItem, cb: ContainerItem): boolean {
  return (
    ca === cb ||
    (ca.id === cb.id &&
      ca.state === cb.state &&
      ca.status === cb.status &&
      ca.name === cb.name &&
      ca.image === cb.image &&
      ca.ports === cb.ports &&
      ca.compose_project === cb.compose_project &&
      ca.kind_cluster === cb.kind_cluster &&
      ca.compose_service === cb.compose_service &&
      sameList(ca.networks, cb.networks) &&
      sameList(ca.broken_networks, cb.broken_networks) &&
      samePorts(ca.published_ports, cb.published_ports))
  );
}

/**
 * Keeps the previous object for every row that did not change, so a tick that
 * flips one container re-renders one memoised card instead of all of them.
 */
export function reconcileContainers(
  prev: ContainerItem[],
  next: ContainerItem[]
): ContainerItem[] {
  if (prev.length === 0) return next;
  const byId = new Map(prev.map((c) => [c.id, c]));
  let reused = 0;
  const out = next.map((c) => {
    const old = byId.get(c.id);
    if (old && containerEqual(old, c)) {
      reused++;
      return old;
    }
    return c;
  });
  // Same rows in the same order: keep the old array too.
  if (reused === prev.length && reused === next.length && out.every((c, i) => c === prev[i])) {
    return prev;
  }
  return out;
}

/**
 * Replaces the live stats map with this tick's sample (containers that went
 * away drop out instead of accumulating forever) while reusing the previous
 * entry for every container whose figures did not change.
 */
export function reconcileStats(
  prev: Record<string, ContainerStats>,
  next: Record<string, ContainerStats>
): Record<string, ContainerStats> {
  const out: Record<string, ContainerStats> = {};
  let changed = Object.keys(prev).length !== Object.keys(next).length;
  for (const [id, s] of Object.entries(next)) {
    const old = prev[id];
    if (
      old &&
      old.cpu_percent === s.cpu_percent &&
      old.mem_usage === s.mem_usage &&
      old.mem_limit === s.mem_limit &&
      old.net_rx === s.net_rx &&
      old.net_tx === s.net_tx
    ) {
      out[id] = old;
    } else {
      out[id] = s;
      changed = true;
    }
  }
  return changed ? out : prev;
}

export interface RouteLike {
  domain: string;
  target: string;
  path?: string | null;
  container_name?: string | null;
}

/**
 * Gateway routes that point at each container (by name or published port),
 * computed once per list change instead of once per card per render. Rows
 * whose matches did not change keep the previous array, so memoised cards
 * stay put.
 */
export function routesByContainer<R extends RouteLike>(
  containers: ContainerItem[],
  routes: R[],
  prev?: Map<string, R[]>
): Map<string, R[]> {
  const out = new Map<string, R[]>();
  if (routes.length === 0) return out;
  for (const c of containers) {
    const cleanName = c.name.replace(/^\//, "");
    const ports = new Set<string>();
    for (const p of c.published_ports ?? []) {
      ports.add(`127.0.0.1:${p.host_port}`);
      ports.add(`localhost:${p.host_port}`);
    }
    const matched = routes.filter(
      (r) => r.container_name === cleanName || ports.has(r.target)
    );
    if (matched.length === 0) continue;
    const old = prev?.get(c.id);
    out.set(
      c.id,
      old && old.length === matched.length && old.every((r, i) => r === matched[i])
        ? old
        : matched
    );
  }
  return out;
}

/**
 * Compares two size maps to avoid re-rendering container cards when byte counts are unchanged.
 */
export function sizesEqual(a: Record<string, number>, b: Record<string, number>): boolean {
  if (a === b) return true;
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return false;
  for (const k of aKeys) {
    if (a[k] !== b[k]) return false;
  }
  return true;
}

