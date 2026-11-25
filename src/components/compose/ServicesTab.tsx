import React from "react";
import {
  Box,
  Globe,
  Hammer,
  HeartPulse,
  Play,
  RefreshCcw,
  RotateCw,
  Scaling,
  ScrollText,
  Square,
  Tag,
  TerminalSquare,
} from "lucide-react";
import { IconAction, IconBadge, PortChip, openUrl } from "./composeUi";
import { publishedPorts, type ComposeServiceRow } from "../../lib/composeProject";
import { cn, formatBytes } from "../../lib/utils";
import type { ComposeServiceAction, ContainerStats, GatewayRoute } from "../../types";

export interface ServicesTabProps {
  rows: ComposeServiceRow[];
  stats: Record<string, ContainerStats>;
  routes: GatewayRoute[];
  hasFile: boolean;
  busy: boolean;
  onAction: (row: ComposeServiceRow, action: ComposeServiceAction) => void;
  onScale: (row: ComposeServiceRow) => void;
  onExpose: (row: ComposeServiceRow) => void;
  onLogs: (row: ComposeServiceRow) => void;
  onTerminal: (row: ComposeServiceRow) => void;
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
  busy,
  onAction,
  onScale,
  onExpose,
  onLogs,
  onTerminal,
}) => {
  const needFile = "Needs the project's compose file";
  if (rows.length === 0) {
    return <p className="p-6 text-center text-sm text-zinc-500">No services.</p>;
  }
  return (
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
        return (
          <div
            key={row.name}
            className="grid grid-cols-1 items-center gap-2 px-3 py-2 text-xs lg:grid-cols-[minmax(0,1.4fr)_5rem_minmax(0,1fr)_6rem_5rem_6rem_auto] lg:gap-3"
          >
            <div className="flex min-w-0 items-center gap-2">
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
        );
      })}
    </div>
  );
};
