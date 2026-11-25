/**
 * Pure helpers for the kind cluster wizard: the default spec, field
 * validation hints, preset handling and the local registry ref mapping.
 * Mirrors `kind_spec` / `kind_registry` on the Rust side; the backend stays
 * the authority (its preview `errors` decide whether a cluster can be made).
 */
import type { ClusterPreset, ClusterSpec, HostMountSpec, PortMappingSpec } from "../types";

export const MAX_NODES = 10;
export const CONTROL_PLANE_OPTIONS = [1, 3, 5] as const;
/** Host ports the app reserves for its own API forwards. */
export const RESERVED_HOST_PORTS = { min: 16443, max: 16999 } as const;
/** Ephemeral range that is never forwarded to the host. */
export const UNFORWARDED_HOST_PORTS = { min: 32768, max: 60999 } as const;
export const PORT_RANGE_HINT =
  "Host ports 16443–16999 are reserved; 32768–60999 are not forwarded to the host.";
/** Rough working-set estimate per kind node, shown as a hint only. */
export const RAM_PER_NODE_GB = 1.5;
export const LOCAL_REGISTRY_HOST = "localhost:5001";

const NAME_PATTERN = /^[a-z0-9][a-z0-9.-]*$/;
const CIDR_PATTERN = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})\/(\d{1,2})$/;
const FEATURE_GATE_PATTERN = /^[A-Za-z][A-Za-z0-9]*=(true|false)$/;

export function defaultClusterSpec(): ClusterSpec {
  return {
    image: null,
    controlPlanes: 1,
    workers: 0,
    ingress: false,
    ingressHttpPort: 18080,
    ingressHttpsPort: 18443,
    portMappings: [],
    mounts: [],
    podSubnet: null,
    serviceSubnet: null,
    featureGates: [],
    localRegistry: false,
  };
}

/** Spec pre-filled from Settings → Kubernetes defaults (total node count, image). */
export function specFromDefaults(totalNodes: number, image: string): ClusterSpec {
  const nodes = Math.min(MAX_NODES, Math.max(1, Math.round(totalNodes) || 1));
  return { ...defaultClusterSpec(), image: image.trim() || null, workers: nodes - 1 };
}

/** Deep copy, so editing a form never mutates a preset or a saved spec. */
export function cloneSpec(spec: ClusterSpec): ClusterSpec {
  return {
    ...spec,
    portMappings: spec.portMappings.map((p) => ({ ...p })),
    mounts: spec.mounts.map((m) => ({ ...m })),
    featureGates: [...spec.featureGates],
  };
}

export function totalNodes(spec: ClusterSpec): number {
  return spec.controlPlanes + spec.workers;
}

/** Largest worker count allowed next to the chosen control-plane count. */
export function maxWorkers(controlPlanes: number): number {
  return MAX_NODES - controlPlanes;
}

export function sameSpec(a: ClusterSpec, b: ClusterSpec): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Applies a preset: a fresh copy of its spec, so the preset itself stays untouched. */
export function applyPreset(preset: ClusterPreset): ClusterSpec {
  return cloneSpec({ ...defaultClusterSpec(), ...preset.spec });
}

/** A user preset built from the wizard's current spec. */
export function presetFromSpec(name: string, description: string, spec: ClusterSpec): ClusterPreset {
  return {
    name: name.trim(),
    description: description.trim(),
    builtin: false,
    spec: cloneSpec(spec),
  };
}

export function validateClusterName(name: string): string | null {
  const value = name.trim();
  if (!value) return "Name is required.";
  // Mirrors `check_cluster_name` in kind.rs: the name reaches kind arguments
  // and node-state paths.
  if (!NAME_PATTERN.test(value) || value.includes("..")) {
    return "Use lowercase letters, digits, '-' and '.', starting with a letter or digit.";
  }
  if (value.length > 50) return "At most 50 characters.";
  return null;
}

export function validatePresetName(name: string): string | null {
  return name.trim() ? null : "Preset name is required.";
}

export function validateContainerPort(port: number): string | null {
  return Number.isInteger(port) && port >= 1 && port <= 65535 ? null : "Port must be 1–65535.";
}

/** Host port check; the reserved and unforwarded ranges get a specific message. */
export function validateHostPort(port: number): string | null {
  const base = validateContainerPort(port);
  if (base) return base;
  if (port >= RESERVED_HOST_PORTS.min && port <= RESERVED_HOST_PORTS.max) {
    return "Port is reserved by the app (16443–16999).";
  }
  if (port >= UNFORWARDED_HOST_PORTS.min && port <= UNFORWARDED_HOST_PORTS.max) {
    return "Port is not forwarded to the host (32768–60999).";
  }
  return null;
}

/** Empty means "no subnet" (kind default); anything else must be an IPv4 CIDR. */
export function validateCidr(cidr: string | null): string | null {
  const value = (cidr ?? "").trim();
  if (!value) return null;
  const m = CIDR_PATTERN.exec(value);
  if (!m) return "Use IPv4 CIDR notation, e.g. 10.244.0.0/16.";
  const octets = m.slice(1, 5).map(Number);
  const prefix = Number(m[5]);
  if (octets.some((o) => o > 255) || prefix > 32) return "Not a valid IPv4 CIDR.";
  return null;
}

export function validateFeatureGate(gate: string): string | null {
  return FEATURE_GATE_PATTERN.test(gate.trim()) ? null : "Use Name=true or Name=false.";
}

export function validateMount(mount: HostMountSpec): string | null {
  if (!mount.hostPath.trim()) return "Host path is required.";
  if (!mount.containerPath.trim().startsWith("/")) return "Container path must be absolute.";
  return null;
}

/**
 * Every field-level problem in the spec, as readable lines. Backend preview
 * errors are separate; these are the checks the form can make up front.
 */
export function validateSpec(spec: ClusterSpec): string[] {
  const issues: string[] = [];
  if (!(CONTROL_PLANE_OPTIONS as readonly number[]).includes(spec.controlPlanes)) {
    issues.push("Control planes must be 1, 3 or 5.");
  }
  if (totalNodes(spec) > MAX_NODES) issues.push(`At most ${MAX_NODES} nodes in total.`);
  if (spec.ingress) {
    for (const port of [spec.ingressHttpPort, spec.ingressHttpsPort]) {
      const issue = validateHostPort(port);
      if (issue) issues.push(`Ingress port ${port}: ${issue}`);
    }
  }
  spec.portMappings.forEach((p, i) => {
    const c = validateContainerPort(p.containerPort);
    if (c) issues.push(`Port mapping ${i + 1} container port: ${c}`);
    const h = validateHostPort(p.hostPort);
    if (h) issues.push(`Port mapping ${i + 1} host port: ${h}`);
  });
  const hostPorts = [
    ...(spec.ingress ? [spec.ingressHttpPort, spec.ingressHttpsPort] : []),
    ...spec.portMappings.map((p) => p.hostPort),
  ];
  const seen = new Set<number>();
  for (const port of hostPorts) {
    if (seen.has(port)) issues.push(`Host port ${port} is used more than once.`);
    seen.add(port);
  }
  const podIssue = validateCidr(spec.podSubnet);
  if (podIssue) issues.push(`Pod subnet: ${podIssue}`);
  const svcIssue = validateCidr(spec.serviceSubnet);
  if (svcIssue) issues.push(`Service subnet: ${svcIssue}`);
  spec.featureGates.forEach((g) => {
    const issue = validateFeatureGate(g);
    if (issue) issues.push(`Feature gate "${g}": ${issue}`);
  });
  spec.mounts.forEach((m, i) => {
    const issue = validateMount(m);
    if (issue) issues.push(`Mount ${i + 1}: ${issue}`);
  });
  return issues;
}

/** Starting values for a new row in the port mappings table. */
export const NEW_PORT_MAPPING: PortMappingSpec = { containerPort: 80, hostPort: 8080, protocol: "TCP" };
export const NEW_MOUNT: HostMountSpec = { hostPath: "", containerPath: "", readOnly: false };

/**
 * Mirror of the backend's local registry ref: `nginx:1.27` becomes
 * `localhost:5001/nginx:1.27`, a registry host is dropped, `library/` is
 * stripped and a missing tag becomes `:latest`. Null for an empty reference.
 */
export function localRegistryRef(image: string): string | null {
  const trimmed = image.trim();
  if (!trimmed || /\s/.test(trimmed)) return null;
  const noDigest = trimmed.split("@")[0];
  const slash = noDigest.indexOf("/");
  const first = slash === -1 ? "" : noDigest.slice(0, slash);
  const rest = slash === -1 ? noDigest : noDigest.slice(slash + 1);
  const hasHost = first.includes(".") || first.includes(":") || first === "localhost";
  const path = (hasHost ? rest : noDigest).replace(/^library\//, "");
  const last = path.slice(path.lastIndexOf("/") + 1);
  return `${LOCAL_REGISTRY_HOST}/${last.includes(":") ? path : `${path}:latest`}`;
}
