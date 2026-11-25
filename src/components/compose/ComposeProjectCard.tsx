import React from "react";
import {
  AlertTriangle,
  Copy,
  Ellipsis,
  FileKey2,
  FolderOpen,
  HeartPulse,
  Layers,
  Loader2,
  PanelRightOpen,
  Pin,
  PinOff,
  Play,
  Power,
  RotateCw,
  SlidersHorizontal,
  Square,
  Tags,
} from "lucide-react";
import { ClusterOverflowMenu, type OverflowItem } from "../k8s/ClusterOverflowMenu";
import { IconAction, IconBadge, InfoCell, PortChip, STATUS_STYLE } from "./composeUi";
import {
  projectCounts,
  projectHealth,
  projectPorts,
  projectStatus,
} from "../../lib/composeProject";
import { cn, formatBytes } from "../../lib/utils";
import type { ComposeProjectInfo, ContainerStats } from "../../types";

export interface ComposeProjectCardProps {
  project: ComposeProjectInfo;
  stats: Record<string, ContainerStats>;
  busy: boolean;
  onOpen: () => void;
  onUp: () => void;
  onUpOptions: () => void;
  onRestart: () => void;
  onStop: () => void;
  onDown: () => void;
  onTogglePin: () => void;
  onCopyPath: () => void;
  onOpenFolder: () => void;
}

/** CPU and memory of the project's running containers, from the last stats tick. */
export function projectUsage(project: ComposeProjectInfo, stats: Record<string, ContainerStats>) {
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

const SOURCE_HINT: Record<string, string> = {
  settings: "The default compose file from Settings",
  pinned: "Saved in the project list",
  labels: "Found from running containers' compose labels (not saved)",
};

/**
 * One compose project as a full-width row card: status icon, name and file,
 * icon badges and actions on top; an info strip with services, ports, health
 * and resource use below. Clicking the card opens the details drawer.
 */
export const ComposeProjectCard: React.FC<ComposeProjectCardProps> = (props) => {
  const { project: p, stats, busy } = props;
  const status = projectStatus(p);
  const style = STATUS_STYLE[status];
  const { running, total } = projectCounts(p);
  const ports = projectPorts(p);
  const health = projectHealth(p);
  const usage = projectUsage(p, stats);
  const hasFile = Boolean(p.path) && !p.config_error;
  const anyRunning = running > 0;
  const noFileHint = "This project has no readable compose file; only start/stop/restart by project name work";

  const menu: OverflowItem[] = [
    {
      key: "options",
      label: "Up with options…",
      icon: <SlidersHorizontal className="h-3.5 w-3.5" />,
      onSelect: props.onUpOptions,
      disabled: busy || !hasFile,
      title: hasFile ? "Choose profiles, env file and recreate options" : noFileHint,
    },
    {
      key: "pin",
      label: p.pinned ? "Remove from list" : "Save to list",
      icon: p.pinned ? <PinOff className="h-3.5 w-3.5" /> : <Pin className="h-3.5 w-3.5" />,
      onSelect: props.onTogglePin,
      disabled: !p.path || p.source === "settings",
      title:
        p.source === "settings"
          ? "This is the default file from Settings"
          : p.pinned
            ? "Forget the project here; files and containers stay"
            : "Keep this project listed even when it is not running",
      dividerBefore: true,
    },
    {
      key: "copy",
      label: "Copy file path",
      icon: <Copy className="h-3.5 w-3.5" />,
      onSelect: props.onCopyPath,
      disabled: !p.path,
    },
    {
      key: "folder",
      label: "Open folder",
      icon: <FolderOpen className="h-3.5 w-3.5" />,
      onSelect: props.onOpenFolder,
      disabled: !p.working_dir,
    },
    {
      key: "down",
      label: "Down (remove containers)…",
      icon: <Power className="h-3.5 w-3.5" />,
      onSelect: props.onDown,
      disabled: busy || !hasFile || p.services.length === 0,
      danger: true,
      dividerBefore: true,
      title: hasFile ? "Stop and remove the project's containers and networks" : noFileHint,
    },
  ];

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={props.onOpen}
      onKeyDown={(e) => {
        if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          props.onOpen();
        }
      }}
      className="group cursor-pointer border border-zinc-800 bg-zinc-900/70 transition-colors hover:border-zinc-700 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/60"
    >
      <div className="flex items-center gap-3 border-b border-zinc-800/80 px-4 py-3">
        <span
          role="img"
          aria-label={`Status: ${style.label}`}
          title={`${style.label} — ${running}/${total} services running`}
          className={cn("flex h-8 w-8 shrink-0 items-center justify-center border", style.box)}
        >
          {busy ? (
            <Loader2 className={cn("h-4 w-4 animate-spin", style.icon)} aria-hidden="true" />
          ) : (
            <Layers className={cn("h-4 w-4", style.icon)} aria-hidden="true" />
          )}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <h3 className="truncate font-semibold text-white" title={p.name}>
              {p.name}
            </h3>
            {p.pinned && <Pin className="h-3 w-3 shrink-0 text-indigo-400" aria-label="Saved" />}
          </div>
          <p className="truncate font-mono text-[11px] text-zinc-500" title={p.path || SOURCE_HINT[p.source ?? "labels"]}>
            {p.path || "compose file unknown"}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {p.config_error && (
            <IconBadge tone="danger" hint={`Compose file problem: ${p.config_error}`}>
              <AlertTriangle className="h-3.5 w-3.5" aria-hidden="true" />
            </IconBadge>
          )}
          {health !== "none" && (
            <IconBadge
              tone={health === "healthy" ? "success" : health === "starting" ? "info" : "warning"}
              hint={`Healthchecks: ${health}`}
            >
              <HeartPulse className="h-3.5 w-3.5" aria-hidden="true" />
            </IconBadge>
          )}
          {p.env_file && (
            <IconBadge tone="neutral" hint={`Has an env file: ${p.env_file}`}>
              <FileKey2 className="h-3.5 w-3.5" aria-hidden="true" />
            </IconBadge>
          )}
          {(p.profiles?.length ?? 0) > 0 && (
            <IconBadge tone="accent" hint={`Profiles: ${p.profiles!.join(", ")}`}>
              <Tags className="h-3.5 w-3.5" aria-hidden="true" />
            </IconBadge>
          )}
          <span className="mx-1 h-6 w-px bg-zinc-800" aria-hidden="true" />
          {status === "running" ? (
            <IconAction title="Restart all services" onClick={props.onRestart} disabled={busy}>
              <RotateCw className="h-4 w-4" />
            </IconAction>
          ) : (
            <IconAction
              title={hasFile ? "Up: create and start all services" : noFileHint}
              onClick={props.onUp}
              disabled={busy || (!hasFile && p.services.length === 0)}
              className="text-emerald-300"
            >
              <Play className="h-4 w-4" />
            </IconAction>
          )}
          <IconAction title="Stop all services (containers are kept)" onClick={props.onStop} disabled={busy || !anyRunning}>
            <Square className="h-4 w-4" />
          </IconAction>
          <IconAction title="Open details" onClick={props.onOpen}>
            <PanelRightOpen className="h-4 w-4" />
          </IconAction>
          <span onClick={(e) => e.stopPropagation()}>
            <ClusterOverflowMenu
              items={menu}
              label={`More actions for ${p.name}`}
              icon={<Ellipsis className="h-4 w-4" />}
            />
          </span>
        </div>
      </div>

      <dl className="grid grid-cols-2 divide-zinc-800 sm:grid-cols-3 lg:grid-cols-[8rem_8rem_minmax(0,1fr)_7rem_7rem_7rem] lg:divide-x">
        <InfoCell label="Services" title="Services with at least one running container / services in use">
          <span className={cn(status === "partial" && "text-amber-300")}>
            {running}/{total}
          </span>
        </InfoCell>
        <InfoCell label="Containers">
          {p.services.filter((c) => c.state === "running").length}/{p.services.length}
        </InfoCell>
        <InfoCell label="Ports" className="col-span-2 sm:col-span-1">
          {ports.length === 0 ? (
            <span className="text-zinc-600">—</span>
          ) : (
            <span className="flex flex-wrap gap-1">
              {ports.slice(0, 6).map((port) => (
                <PortChip key={`${port.published}/${port.protocol}`} port={port} small />
              ))}
              {ports.length > 6 && <span className="text-zinc-500">+{ports.length - 6}</span>}
            </span>
          )}
        </InfoCell>
        <InfoCell label="CPU" title="Sum over the project's running containers">
          {usage.sampled > 0 ? `${usage.cpu.toFixed(1)}%` : "—"}
        </InfoCell>
        <InfoCell label="Memory" title="Sum over the project's running containers">
          {usage.sampled > 0 ? formatBytes(usage.mem) : "—"}
        </InfoCell>
        <InfoCell label="Source" title={SOURCE_HINT[p.source ?? "labels"]}>
          <span className="text-zinc-400">{p.source === "labels" ? "labels" : p.source === "settings" ? "settings" : "saved"}</span>
        </InfoCell>
      </dl>
    </div>
  );
};
