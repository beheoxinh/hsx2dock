/**
 * Published-port presentation.
 *
 * Docker publishes `-p 8080:80` on every interface (`0.0.0.0` and `[::]`).
 * Those listeners are DNAT rules, so they bypass ufw/firewalld — worth knowing
 * about, which is why a wildcard binding is marked on the card, but not worth a
 * page-wide banner. This module is the single place that decides which
 * bindings count as public and how a binding is labelled, so the card badge
 * and any future surface cannot drift apart.
 *
 * Pure logic only: no DOM, no Tauri IPC, no engine.
 */
import type { PublishedPort } from "../types";

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

/**
 * A binding as the card should render it: one row per published port, with the
 * bits the user actually needs (which host port maps where, and whether it is
 * reachable beyond this machine).
 */
export interface PortChip {
  /** Host port to open in the browser; null for an exposed-only port. */
  hostPort: number | null;
  containerPort: number;
  protocol: string;
  /** True when bound on every interface (reachable from the network). */
  public: boolean;
  /** `8080 → 80` or just `443` when the port is not published. */
  label: string;
  /** URL to open, when the binding is reachable from this machine. */
  url: string | null;
}

/** The port fields `portChips` reads — a subset of `ContainerItem`. */
export interface PortSource {
  published_ports?: PublishedPort[];
  ports?: string;
}

/**
 * Turn a container's structured bindings into render-ready chips.
 *
 * Loopback bindings get a `localhost` URL; wildcard bindings get one too, since
 * a listener on every interface is by definition reachable on localhost. Ports
 * that are only exposed (never published) have no URL.
 *
 * Falls back to parsing the legacy `ports` string for containers whose summary
 * came from an older backend, so the card never regresses to raw text.
 */
export function portChips(container: PortSource): PortChip[] {
  const structured = container.published_ports ?? [];
  if (structured.length > 0) {
    return structured.map((p) => {
      const proto = (p.protocol ?? "").trim() || "tcp";
      const publicBinding = isPublicBinding(p.host_ip);
      return {
        hostPort: p.host_port,
        containerPort: p.container_port,
        protocol: proto,
        public: publicBinding,
        label: `${p.host_port} → ${p.container_port}`,
        // Only TCP has a browsable scheme; udp/sctp ports have no URL.
        url: proto === "tcp" ? `http://localhost:${p.host_port}` : null,
      };
    });
  }

  // Legacy fallback: `8080->80/tcp, 443/tcp`.
  return parseLegacyPorts(container.ports ?? "");
}

/** Parse `docker ps`-style port text when no structured data is available. */
function parseLegacyPorts(ports: string): PortChip[] {
  if (!ports?.trim()) return [];

  return ports
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean)
    .flatMap((part) => {
      const [left, right] = part.includes("->") ? part.split("->") : [null, part];
      const [portText, protoText] = (right ?? "").split("/");
      const containerPort = Number.parseInt(portText ?? "", 10);
      if (Number.isNaN(containerPort)) return [];

      const proto = (protoText ?? "tcp").trim() || "tcp";
      // `0.0.0.0:8080`, `127.0.0.1:8080`, or bare `8080`.
      const hostPort = left
        ? Number.parseInt(left.split(":").pop() ?? "", 10)
        : NaN;
      const published = !Number.isNaN(hostPort);
      const ip = left?.includes(":") ? left.slice(0, left.lastIndexOf(":")) : "";
      const isPublic = published && isPublicBinding(ip);

      return [
        {
          hostPort: published ? hostPort : null,
          containerPort,
          protocol: proto,
          public: isPublic,
          label: published
            ? `${hostPort} → ${containerPort}`
            : `${containerPort}`,
          url: published && proto === "tcp" ? `http://localhost:${hostPort}` : null,
        },
      ];
    });
}
