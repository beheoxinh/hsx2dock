import type { ContainerItem, GatewayRoute } from "../types";

/** `host:port` of a route target, scheme and trailing slash stripped. */
export function parseTarget(target: string): { host: string; port: number } | null {
  const clean = target.trim().replace(/^https?:\/\//, "").replace(/\/+$/, "");
  const i = clean.lastIndexOf(":");
  if (i <= 0) return null;
  const port = Number(clean.slice(i + 1));
  if (!Number.isInteger(port) || port <= 0 || port > 65535) return null;
  return { host: clean.slice(0, i), port };
}

export interface TargetDrift {
  /** The mapped container no longer publishes the route's target port. */
  drifted: boolean;
  /** A replacement target when exactly one TCP host port is published. */
  suggestion: string | null;
}

/**
 * A mapped route stores a fixed `host:port`. Recreating its container (a
 * compose up with a random host port, a re-run on another port) silently
 * left the route dialling a port nothing listens on, so the domain answered
 * 502. This compares the target with what the container publishes now.
 */
export function mappedTargetDrift(
  route: GatewayRoute,
  container: ContainerItem | undefined
): TargetDrift {
  const none = { drifted: false, suggestion: null };
  if (!container || container.state !== "running") return none;
  const target = parseTarget(route.target);
  if (!target) return none;
  const ports = (container.published_ports ?? []).filter(
    (p) => p.host_port > 0 && (p.protocol || "tcp").toLowerCase() === "tcp"
  );
  // Nothing published: the target may be a host-network service — no claim.
  if (ports.length === 0) return none;
  if (ports.some((p) => p.host_port === target.port)) return none;
  const hostPorts = [...new Set(ports.map((p) => p.host_port))];
  return {
    drifted: true,
    suggestion: hostPorts.length === 1 ? `${target.host}:${hostPorts[0]}` : null,
  };
}

/** The container a mapped route points at, by name, id or short id. */
export function findMappedContainer(
  route: GatewayRoute,
  containers: ContainerItem[]
): ContainerItem | undefined {
  const want = route.container_name?.trim();
  if (!want) return undefined;
  return containers.find((c) => {
    const name = c.name.replace(/^\//, "");
    return name === want || c.id === want || c.short_id === want;
  });
}
