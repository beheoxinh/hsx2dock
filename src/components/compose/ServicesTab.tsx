import React, { useCallback, useEffect, useState } from "react";
import {
  ArrowUpCircle,
  Box,
  ChevronDown,
  ChevronRight,
  CloudOff,
  Download,
  Globe,
  Hammer,
  HeartPulse,
  Loader2,
  Play,
  RefreshCcw,
  RotateCw,
  ScanSearch,
  Scaling,
  ScrollText,
  Square,
  Tag,
  TerminalSquare,
} from "lucide-react";
import { IconAction, IconBadge, PortChip, openUrl } from "./composeUi";
import { HealthDetails } from "./HealthDetails";
import { api } from "../../lib/api";
import { publishedPorts, type ComposeServiceRow } from "../../lib/composeProject";
import { cn, formatBytes } from "../../lib/utils";
import type { ComposeImageUpdate, ComposeServiceAction, ContainerStats, GatewayRoute } from "../../types";

export interface ServicesTabProps {
  rows: ComposeServiceRow[];
  stats: Record<string, ContainerStats>;
  routes: GatewayRoute[];
  hasFile: boolean;
  /** The project's compose file; image checks need it. */
  filePath: string;
  busy: boolean;
  onAction: (row: ComposeServiceRow, action: ComposeServiceAction) => void;
  onScale: (row: ComposeServiceRow) => void;
  onExpose: (row: ComposeServiceRow) => void;
  onLogs: (row: ComposeServiceRow) => void;
  onTerminal: (row: ComposeServiceRow) => void;
  /** Resolves true when the pull succeeded. */
  onPull: (row: ComposeServiceRow) => Promise<boolean>;
  onBuild: (row: ComposeServiceRow) => Promise<boolean>;
}

function rowState(row: ComposeServiceRow): { dot: string; label: string } {
  if (row.containers.length === 0)
    return row.optional
      ? { dot: "bg-zinc-700", label: "Not created (behind a profile)" }
      : { dot: "bg-zinc-600", label: "Not created" };
  if (row.running === row.containers.length) return { dot: "bg-emerald-400", label: "Running" };
  if (row.running > 0) return { dot: "bg-amber-400", label: `${row.running}/${row.containers.length} running` };
  const states = [...new Set(row.containers.map((c) => c.state))].join(", ");
  return { dot: "bg-zinc-500", label: states || "Stopped" };
}

function health(row: ComposeServiceRow): string {
  const states = row.containers.map((c) => c.health).filter(Boolean);
  if (states.includes("unhealthy")) return "unhealthy";
  if (states.includes("starting")) return "starting";
  if (states.includes("healthy")) return "healthy";
  return row.spec?.healthcheck ? "defined" : "";
}

const HEALTH_TONE: Record<string, string> = {
  healthy: "text-emerald-300",
  unhealthy: "text-rose-300",
  starting: "text-sky-300",
  defined: "text-zinc-500",
};

/** Every service of the project, one row each, with its own actions. */
export const ServicesTab: React.FC<ServicesTabProps> = ({
  rows,
  stats,
  routes,
  hasFile,
  filePath,
  busy,
  onAction,
  onScale,
  onExpose,
  onLogs,
  onTerminal,
  onPull,
  onBuild,
}) => {
  const needFile = "Needs the project's compose file";
  const canCheck = hasFile && Boolean(filePath);
  // Image status per service name; loaded when the tab opens (the backend caches it).
  const [updates, setUpdates] = useState<Record<string, ComposeImageUpdate>>({});
  const [checking, setChecking] = useState(false);
  const [checkError, setCheckError] = useState<string | null>(null);
  // Healthcheck details are opened per service; a set keeps several open at once.
  const [openHealth, setOpenHealth] = useState<Set<string>>(new Set());

  const checkImages = useCallback(
    async (force: boolean) => {
      setChecking(true);
      try {
        const list = await api.composeImageUpdates(filePath, force);
        setUpdates(Object.fromEntries(list.map((u) => [u.service, u])));
        setCheckError(null);
      } catch (e) {
        setCheckError(String(e));
      } finally {
        setChecking(false);
      }
    },
    [filePath]
  );

  useEffect(() => {
    if (canCheck) void checkImages(false);
  }, [canCheck, checkImages]);

  const toggleHealth = (name: string) =>
    setOpenHealth((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });

  if (rows.length === 0) {
    return <p className="p-6 text-center text-sm text-zinc-500">No services.</p>;
  }
  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 text-[11px] text-zinc-500">
          Image status compares the registry digest with the local image (cached for 15 minutes).
        </p>
        {checkError && (
          <span className="max-w-[40%] truncate text-[11px] text-rose-300" title={checkError}>
            {checkError}
          </span>
        )}
        <IconAction
          title="Check images against the registry now (bypasses the cache)"
          onClick={() => void checkImages(true)}
          disabled={!canCheck || checking}
        >
          {checking ? <Loader2 className="h-4 w-4 animate-spin" /> : <ScanSearch className="h-4 w-4" />}
        </IconAction>
      </div>
    <div className="divide-y divide-zinc-800 border border-zinc-800">
        <div className="hidden grid-cols-[minmax(0,1.4fr)_5rem_minmax(0,1fr)_6rem_5rem_6rem_auto] gap-3 bg-zinc-900/60 px-3 py-1.5 text-[10px] uppercase tracking-wider text-zinc-500 lg:grid">
          <span>Service</span>
          <span>Replicas</span>
          <span>Ports</span>
          <span>Health</span>
          <span>CPU</span>
          <span>Memory</span>
          <span className="text-right">Actions</span>
        </div>
        {rows.map((row) => {
          const st = rowState(row);
          const h = health(row);
          const ports = row.containers.length
            ? row.containers.flatMap((c) => publishedPorts(c.ports)).filter(
                (p, i, all) => all.findIndex((q) => q.published === p.published && q.protocol === p.protocol) === i
              )
            : [];
          const declaredPorts = (row.spec?.ports ?? []).filter((p) => p.published);
          let cpu = 0;
          let mem = 0;
          let sampled = 0;
          for (const c of row.containers) {
            const s = stats[c.id];
            if (!s) continue;
            sampled += 1;
            cpu += s.cpu_percent;
            mem += s.mem_usage;
          }
          const names = new Set(row.containers.map((c) => c.name));
          const exposed = routes.filter((r) => r.container_name && names.has(r.container_name));
          const running = row.running > 0;
          const replicasWanted = row.spec?.replicas ?? 1;
          const img = updates[row.name];
          const withHealth = row.containers.filter((c) => c.health);
          const healthOpen = openHealth.has(row.name);
          return (
            <div key={row.name}>
              <div
                className="grid grid-cols-1 items-center gap-2 px-3 py-2 text-xs lg:grid-cols-[minmax(0,1.4fr)_5rem_minmax(0,1fr)_6rem_5rem_6rem_auto] lg:gap-3"
              >
                <div className="flex min-w-0 items-center gap-2">
                  {withHealth.length > 0 ? (
                    <button
                      type="button"
                      onClick={() => toggleHealth(row.name)}
                      aria-expanded={healthOpen}
                      aria-label={`${healthOpen ? "Hide" : "Show"} healthcheck of ${row.name}`}
                      title={`${healthOpen ? "Hide" : "Show"} healthcheck details`}
                      className="flex h-5 w-5 shrink-0 items-center justify-center text-zinc-400 hover:text-zinc-100"
                    >
                      {healthOpen ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
                    </button>
                  ) : (
                    <span className="h-5 w-5 shrink-0" aria-hidden="true" />
                  )}
                  <span
                    role="img"
                    aria-label={st.label}
                    title={st.label}
                    className={cn("h-2 w-2 shrink-0 rounded-full", st.dot)}
                  />
                  <div className="min-w-0">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <span className="truncate font-medium text-zinc-100" title={row.name}>
                        {row.name}
                      </span>
                      {row.spec?.build && (
                        <IconBadge tone="neutral" hint="Built from a Dockerfile (build:)">
                          <Hammer className="h-3 w-3" aria-hidden="true" />
                        </IconBadge>
                      )}
                      {row.optional && (
                        <IconBadge tone="accent" hint={`Profiles: ${row.spec!.profiles.join(", ")}`}>
                          <Tag className="h-3 w-3" aria-hidden="true" />
                        </IconBadge>
                      )}
                      {!row.spec && (
                        <IconBadge tone="warning" hint="Not in the compose file (orphaned or the file is unknown)">
                          <Box className="h-3 w-3" aria-hidden="true" />
                        </IconBadge>
                      )}
                      {/* Registry check: "local" and "current" need no badge. */}
                      {img?.status === "update" && (
                        <IconBadge tone="warning" hint="Registry has a newer image (digest differs)">
                          <ArrowUpCircle className="h-3 w-3" aria-hidden="true" />
                        </IconBadge>
                      )}
                      {img?.status === "missing" && (
                        <span
                          className="shrink-0 border border-zinc-700 px-1 font-mono text-[10px] text-zinc-400"
                          title="The image is not on this machine yet"
                        >
                          not pulled
                        </span>
                      )}
                      {img?.status === "error" && (
                        <IconBadge tone="muted" hint={`Registry check failed: ${img.error}`}>
                          <CloudOff className="h-3 w-3" aria-hidden="true" />
                        </IconBadge>
                      )}
                      {exposed.map((r) => (
                        <button
                          key={r.id}
                          type="button"
                          onClick={() => openUrl(`http://${r.domain}${r.path ?? ""}`)}
                          title={`Exposed at ${r.domain}${r.path ?? ""} → ${r.target}${r.enabled ? "" : " (disabled)"}`}
                          className="inline-flex max-w-[12rem] items-center gap-1 border border-indigo-500/30 bg-indigo-500/10 px-1.5 py-0.5 font-mono text-[10px] text-indigo-300 hover:bg-indigo-500/20"
                        >
                          <Globe className="h-3 w-3 shrink-0" />
                          <span className="truncate">{r.domain}</span>
                        </button>
                      ))}
                    </div>
                    <p className="truncate font-mono text-[10px] text-zinc-500" title={row.spec?.image || row.containers[0]?.image}>
                      {row.spec?.image || row.containers[0]?.image || "—"}
                    </p>
                  </div>
                </div>
                <span className="font-mono text-zinc-300" title={`${row.running} running of ${row.containers.length} containers; the file asks for ${replicasWanted}`}>
                  {row.running}/{row.containers.length || replicasWanted}
                </span>
                <span className="flex min-w-0 flex-wrap gap-1">
                  {ports.length > 0
                    ? ports.map((p) => <PortChip key={`${p.published}/${p.protocol}`} port={p} small />)
                    : declaredPorts.length > 0
                      ? declaredPorts.map((p) => (
                          <span key={`${p.published}-${p.target}`} className="font-mono text-[10px] text-zinc-500" title="Declared, not running">
                            {p.published}→{p.target}
                          </span>
                        ))
                      : <span className="text-zinc-600">—</span>}
                </span>
                <span className={cn("flex items-center gap-1 font-mono", HEALTH_TONE[h] ?? "text-zinc-600")}>
                  {h ? (
                    <>
                      <HeartPulse className="h-3 w-3" aria-hidden="true" />
                      {h}
                    </>
                  ) : (
                    "—"
                  )}
                </span>
                <span className="font-mono text-zinc-300">{sampled ? `${cpu.toFixed(1)}%` : "—"}</span>
                <span className="font-mono text-zinc-300">{sampled ? formatBytes(mem) : "—"}</span>
                <div className="flex flex-wrap items-center justify-end gap-1">
                  {running ? (
                    <IconAction title={`Stop ${row.name}`} onClick={() => onAction(row, "stop")} disabled={busy}>
                      <Square className="h-3.5 w-3.5" />
                    </IconAction>
                  ) : row.containers.length > 0 ? (
                    <IconAction title={`Start ${row.name}`} onClick={() => onAction(row, "start")} disabled={busy} className="text-emerald-300">
                      <Play className="h-3.5 w-3.5" />
                    </IconAction>
                  ) : (
                    <IconAction
                      title={hasFile ? `Create and start ${row.name}` : needFile}
                      onClick={() => onAction(row, "up")}
                      disabled={busy || !hasFile}
                      className="text-emerald-300"
                    >
                      <Play className="h-3.5 w-3.5" />
                    </IconAction>
                  )}
                  {row.spec?.build ? (
                    <IconAction
                      title={hasFile ? `Build the image of ${row.name} (build:)` : needFile}
                      onClick={() => void onBuild(row)}
                      disabled={busy || !hasFile}
                    >
                      <Hammer className="h-3.5 w-3.5" />
                    </IconAction>
                  ) : row.spec ? (
                    <IconAction
                      title={hasFile ? `Pull the image of ${row.name}` : needFile}
                      onClick={() => void onPull(row).then((ok) => ok && void checkImages(true))}
                      disabled={busy || !hasFile}
                      className={img?.status === "update" ? "border-amber-500/60 text-amber-300" : undefined}
                    >
                      <Download className="h-3.5 w-3.5" />
                    </IconAction>
                  ) : null}
                  <IconAction title={`Restart ${row.name}`} onClick={() => onAction(row, "restart")} disabled={busy || !running}>
                    <RotateCw className="h-3.5 w-3.5" />
                  </IconAction>
                  <IconAction
                    title={hasFile ? `Recreate ${row.name} (new container from the current file and image)` : needFile}
                    onClick={() => onAction(row, "recreate")}
                    disabled={busy || !hasFile || !row.spec}
                  >
                    <RefreshCcw className="h-3.5 w-3.5" />
                  </IconAction>
                  <IconAction
                    title={
                      !hasFile
                        ? needFile
                        : row.spec?.container_name
                          ? "A fixed container_name allows only one replica"
                          : `Scale ${row.name}`
                    }
                    onClick={() => onScale(row)}
                    disabled={busy || !hasFile || !row.spec || Boolean(row.spec?.container_name)}
                  >
                    <Scaling className="h-3.5 w-3.5" />
                  </IconAction>
                  <IconAction title={`Logs of ${row.name}`} onClick={() => onLogs(row)} disabled={row.containers.length === 0}>
                    <ScrollText className="h-3.5 w-3.5" />
                  </IconAction>
                  <IconAction title={running ? `Shell into ${row.name}` : "Start the service to open a shell"} onClick={() => onTerminal(row)} disabled={!running}>
                    <TerminalSquare className="h-3.5 w-3.5" />
                  </IconAction>
                  <IconAction title={`Expose ${row.name} on a domain through the gateway`} onClick={() => onExpose(row)}>
                    <Globe className="h-3.5 w-3.5" />
                  </IconAction>
                </div>
              </div>
              {healthOpen && (
                <div className="flex flex-col gap-3 border-t border-zinc-800 bg-zinc-950/60 px-3 py-3 pl-9">
                  {withHealth.map((c) => (
                    <div key={c.id} className="flex flex-col gap-1.5">
                      <p className="font-mono text-[11px] text-zinc-500" title={c.name}>
                        {c.name}
                      </p>
                      <HealthDetails containerId={c.id} />
                    </div>
                  ))}
                </div>
              )}
              </div>
          );
        })}
    </div>
    </div>
  );
};
