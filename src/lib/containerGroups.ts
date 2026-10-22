import type { ContainerGroup, ContainerItem } from "../types";

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
