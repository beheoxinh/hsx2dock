import type { ComposeProjectInfo, ComposeService, ComposeServiceSpec } from "../types";

export type ComposeProjectStatus = "running" | "partial" | "stopped" | "error";

/** One service of a project: what the file declares joined with its containers. */
export interface ComposeServiceRow {
  name: string;
  spec: ComposeServiceSpec | null;
  containers: ComposeService[];
  running: number;
  /** Declared only behind a profile (not started by a plain `up`). */
  optional: boolean;
}

export function isRunning(c: ComposeService): boolean {
  return c.state.toLowerCase() === "running";
}

/** Declared services and container-only services, sorted by name. */
export function serviceRows(project: ComposeProjectInfo): ComposeServiceRow[] {
  const rows = new Map<string, ComposeServiceRow>();
  for (const spec of project.declared ?? []) {
    rows.set(spec.name, {
      name: spec.name,
      spec,
      containers: [],
      running: 0,
      optional: spec.profiles.length > 0,
    });
  }
  for (const c of project.services) {
    const name = c.service || c.name;
    let row = rows.get(name);
    if (!row) {
      row = { name, spec: null, containers: [], running: 0, optional: false };
      rows.set(name, row);
    }
    row.containers.push(c);
    if (isRunning(c)) row.running += 1;
  }
  return [...rows.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/**
 * Running/total services. Profile-only services count only while they have
 * containers, so a project with an unused `tools` profile still reads as
 * fully running.
 */
export function projectCounts(project: ComposeProjectInfo): { running: number; total: number } {
  let running = 0;
  let total = 0;
  for (const row of serviceRows(project)) {
    if (row.optional && row.containers.length === 0) continue;
    total += 1;
    if (row.running > 0) running += 1;
  }
  return { running, total };
}

export function projectStatus(project: ComposeProjectInfo): ComposeProjectStatus {
  const { running, total } = projectCounts(project);
  if (project.config_error && project.services.length === 0) return "error";
  if (running === 0) return "stopped";
  return running >= total ? "running" : "partial";
}

export interface HostPort {
  published: number;
  target: number;
  protocol: string;
}

/**
 * Published ports from a container's ports text
 * (`0.0.0.0:8080->80/tcp, :::8080->80/tcp, 53/udp`), one per published port.
 */
export function publishedPorts(ports: string): HostPort[] {
  const out: HostPort[] = [];
  for (const part of ports.split(",")) {
    const m = part.trim().match(/(\d+)->(\d+)\/(\w+)$/);
    if (!m) continue;
    const p = { published: Number(m[1]), target: Number(m[2]), protocol: m[3] };
    if (!out.some((q) => q.published === p.published && q.protocol === p.protocol)) out.push(p);
  }
  return out;
}

/** Every published port of a project's containers. */
export function projectPorts(project: ComposeProjectInfo): HostPort[] {
  const out: HostPort[] = [];
  for (const c of project.services) {
    for (const p of publishedPorts(c.ports)) {
      if (!out.some((q) => q.published === p.published && q.protocol === p.protocol)) out.push(p);
    }
  }
  return out.sort((a, b) => a.published - b.published);
}

export type HealthSummary = "healthy" | "unhealthy" | "starting" | "none";

export function projectHealth(project: ComposeProjectInfo): HealthSummary {
  const states = project.services.map((c) => c.health).filter(Boolean);
  if (states.includes("unhealthy")) return "unhealthy";
  if (states.includes("starting")) return "starting";
  if (states.includes("healthy")) return "healthy";
  return "none";
}

export type ComposeStatusFilter = "all" | ComposeProjectStatus;
export type ComposeSortKey = "name" | "status" | "services";

const STATUS_RANK: Record<ComposeProjectStatus, number> = {
  error: 0,
  partial: 1,
  running: 2,
  stopped: 3,
};

export function filterProjects(
  projects: ComposeProjectInfo[],
  query: string,
  status: ComposeStatusFilter
): ComposeProjectInfo[] {
  const q = query.trim().toLowerCase();
  return projects.filter((p) => {
    if (status !== "all" && projectStatus(p) !== status) return false;
    if (!q) return true;
    if (p.name.toLowerCase().includes(q) || p.path.toLowerCase().includes(q)) return true;
    return serviceRows(p).some(
      (r) => r.name.toLowerCase().includes(q) || (r.spec?.image ?? "").toLowerCase().includes(q)
    );
  });
}

export function sortProjects(
  projects: ComposeProjectInfo[],
  key: ComposeSortKey
): ComposeProjectInfo[] {
  const byName = (a: ComposeProjectInfo, b: ComposeProjectInfo) => a.name.localeCompare(b.name);
  const list = [...projects];
  if (key === "status") {
    list.sort((a, b) => STATUS_RANK[projectStatus(a)] - STATUS_RANK[projectStatus(b)] || byName(a, b));
  } else if (key === "services") {
    list.sort((a, b) => projectCounts(b).total - projectCounts(a).total || byName(a, b));
  } else {
    list.sort(byName);
  }
  return list;
}

/**
 * Layer of each service in the dependency graph: services with no
 * dependencies are layer 0, every other one sits one layer right of its
 * deepest dependency. Cycles (invalid in compose, but possible in a draft)
 * are cut where they close.
 */
export function dependencyLayers(specs: ComposeServiceSpec[]): Map<string, number> {
  const byName = new Map(specs.map((s) => [s.name, s]));
  const layers = new Map<string, number>();
  const visiting = new Set<string>();
  const visit = (name: string): number => {
    const known = layers.get(name);
    if (known !== undefined) return known;
    if (visiting.has(name)) return 0;
    visiting.add(name);
    let layer = 0;
    for (const dep of byName.get(name)?.depends_on ?? []) {
      if (byName.has(dep)) layer = Math.max(layer, visit(dep) + 1);
    }
    visiting.delete(name);
    layers.set(name, layer);
    return layer;
  };
  for (const s of specs) visit(s.name);
  return layers;
}

/** A default gateway domain for a service: `web.shop.localhost`. */
export function defaultDomain(project: string, service: string): string {
  const clean = (s: string) =>
    s
      .toLowerCase()
      .replace(/[^a-z0-9-]+/g, "-")
      .replace(/^-+|-+$/g, "");
  const svc = clean(service);
  const proj = clean(project);
  return [svc, proj].filter(Boolean).join(".") + ".localhost";
}

/** A short, unique id to correlate `compose-progress` events with one run. */
export function newOpId(): string {
  return `op-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}
