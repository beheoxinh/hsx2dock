/**
 * Published-port exposure checks.
 *
 * Docker publishes `-p 8080:80` on every interface (`0.0.0.0` and `[::]`).
 * Those listeners are DNAT rules, so they bypass ufw/firewalld — the same
 * advisory Docker Desktop shows. Binding `127.0.0.1:8080:80` keeps the port on
 * loopback instead. This module is the single place that decides which
 * bindings count as exposed, so the card badge, the page banner and the run
 * wizard cannot drift apart.
 *
 * Pure logic only: no DOM, no Tauri IPC, no engine.
 */
import type { ContainerItem, PublishedPort } from "../types";

/** Bind addresses that mean "listen on every interface". */
const WILDCARD_HOSTS = new Set([
  "",
  "0.0.0.0",
  "::",
  "[::]",
  "*",
  "0:0:0:0:0:0:0:0",
]);

/** True when the engine bound the port on every interface, not just loopback. */
export function isPublicBinding(hostIp?: string | null): boolean {
  return WILDCARD_HOSTS.has((hostIp ?? "").trim().toLowerCase());
}

/** A container's published ports that are reachable beyond the host itself. */
export function publicBindings(
  container: Pick<ContainerItem, "published_ports">
): PublishedPort[] {
  return (container.published_ports ?? []).filter((p) =>
    isPublicBinding(p.host_ip)
  );
}

/** `0.0.0.0:8080→80/tcp` — the form used in tooltips and the banner. */
export function describeBinding(p: PublishedPort): string {
  const ip = p.host_ip.trim() || "0.0.0.0";
  const proto = (p.protocol ?? "").trim() || "tcp";
  return `${ip}:${p.host_port}→${p.container_port}/${proto}`;
}

/** Every container currently publishing at least one port on every interface. */
export function exposedContainers(containers: ContainerItem[]): ContainerItem[] {
  return containers.filter((c) => publicBindings(c).length > 0);
}

/**
 * Compact list for the banner: `web (8080), api (8000, 8001)` — capped so a
 * host with many containers stays readable.
 */
export function exposureSummary(
  containers: ContainerItem[],
  limit = 3
): string {
  const entries = exposedContainers(containers).map((c) => {
    const ports = publicBindings(c)
      .map((p) => p.host_port)
      .join(", ");
    return `${c.name.replace(/^\//, "")} (${ports})`;
  });
  if (entries.length <= limit) return entries.join(", ");
  return `${entries.slice(0, limit).join(", ")} +${entries.length - limit} more`;
}
