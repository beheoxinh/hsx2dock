import React from "react";
import {
  AlertTriangle,
  ArrowUpCircle,
  BellRing,
  Copy,
  Download,
  Ellipsis,
  Eye,
  EyeOff,
  FileKey2,
  FolderOpen,
  GitMerge,
  Hammer,
  HeartPulse,
  Layers,
  Loader2,
  PackageCheck,
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
import { Sparkline } from "./Sparkline";
import {
  projectCounts,
  projectHealth,
  projectPorts,
  projectStatus,
} from "../../lib/composeProject";
import type { ProjectHistory } from "../../lib/composeHistory";
import { cn, formatBytes } from "../../lib/utils";
import type { ComposeImageUpdate, ComposeProjectInfo } from "../../types";

/**
 * Handlers receive their project, so the page can pass one stable function per
 * action instead of a closure per row. Numbers are primitives so that React.memo
 * skips the card when a stats tick did not change this project's values.
 */
type ProjectHandler = (p: ComposeProjectInfo) => void;

export interface ComposeProjectCardProps {
  project: ComposeProjectInfo;
  busy: boolean;
  /** Summed over the project's containers in the last stats tick; sampled = containers seen. */
  cpu: number;
  mem: number;
  sampled: number;
  /** CPU and memory samples recorded for this project (sparklines). */
  history: ProjectHistory;
  /** Unacknowledged compose alerts for this project, and the newest one's text. */
  alertCount: number;
  latestAlert?: string;
  /** Services whose registry image is newer than the local one. */
  updates: ComposeImageUpdate[];
  onOpen: ProjectHandler;
  onUp: ProjectHandler;
  onUpOptions: ProjectHandler;
  onRestart: ProjectHandler;
  onStop: ProjectHandler;
  onDown: ProjectHandler;
  onPull: ProjectHandler;
  onBuild: ProjectHandler;
  onPullUpdates: ProjectHandler;
  onShowAlerts: ProjectHandler;
  onStartWatch: ProjectHandler;
  onStopWatch: ProjectHandler;
  onExport: ProjectHandler;
  onTogglePin: ProjectHandler;
  onCopyPath: ProjectHandler;
  onOpenFolder: ProjectHandler;
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
const ComposeProjectCardView: React.FC<ComposeProjectCardProps> = (props) => {
  const { project: p, busy, history, cpu, mem, sampled } = props;
  // Bind the project into a handler; the menu and buttons take no arguments.
  const act = (fn: ProjectHandler) => () => fn(p);
  const status = projectStatus(p);
  const style = STATUS_STYLE[status];
  const { running, total } = projectCounts(p);
  const ports = projectPorts(p);
  const health = projectHealth(p);
  const hasFile = Boolean(p.path) && !p.config_error;
  const anyRunning = running > 0;
  const noFileHint = "This project has no readable compose file; only start/stop/restart by project name work";
  // Build and watch only make sense when the file declares them.
  const hasBuild = (p.declared ?? []).some((s) => s.build);
  const hasWatch = (p.declared ?? []).some((s) => s.watch);

  const menu: OverflowItem[] = [
    {
      key: "pull",
      label: "Pull images…",
      icon: <Download className="h-3.5 w-3.5" />,
      onSelect: act(props.onPull),
      disabled: busy || !hasFile,
      title: hasFile ? "Download newer images for the declared services" : noFileHint,
    },
    ...(hasBuild
      ? [
          {
            key: "build",
            label: "Build images…",
            icon: <Hammer className="h-3.5 w-3.5" />,
            onSelect: act(props.onBuild),
            disabled: busy || !hasFile,
            title: hasFile ? "Build the services that have a build section" : noFileHint,
          },
        ]
      : []),
    {
      key: "options",
      label: "Up with options…",
      icon: <SlidersHorizontal className="h-3.5 w-3.5" />,
      onSelect: act(props.onUpOptions),
      disabled: busy || !hasFile,
      title: hasFile ? "Choose profiles, env file and recreate options" : noFileHint,
      dividerBefore: true,
    },
    ...(hasWatch
      ? [
          p.watching
            ? {
                key: "watch-stop",
                label: "Stop watch",
                icon: <EyeOff className="h-3.5 w-3.5" />,
                onSelect: act(props.onStopWatch),
                disabled: busy,
                title: "End the docker compose watch session started from here",
                dividerBefore: true,
              }
            : {
                key: "watch-start",
                label: "Start watch",
                icon: <Eye className="h-3.5 w-3.5" />,
                onSelect: act(props.onStartWatch),
                disabled: busy || !hasFile,
                title: hasFile ? "Sync and rebuild services on file changes (docker compose watch)" : noFileHint,
                dividerBefore: true,
              },
        ]
      : []),
    {
      key: "export",
      label: "Export…",
      icon: <PackageCheck className="h-3.5 w-3.5" />,
      onSelect: act(props.onExport),
      disabled: !p.path,
      title: p.path ? "Pack the compose file (and optionally .env) into an archive" : noFileHint,
      dividerBefore: true,
    },
    {
      key: "pin",
      label: p.pinned ? "Remove from list" : "Save to list",
      icon: p.pinned ? <PinOff className="h-3.5 w-3.5" /> : <Pin className="h-3.5 w-3.5" />,
      onSelect: act(props.onTogglePin),
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
      onSelect: act(props.onCopyPath),
      disabled: !p.path,
    },
    {
      key: "folder",
      label: "Open folder",
      icon: <FolderOpen className="h-3.5 w-3.5" />,
      onSelect: act(props.onOpenFolder),
      disabled: !p.working_dir,
    },
    {
      key: "down",
      label: "Down (remove containers)…",
      icon: <Power className="h-3.5 w-3.5" />,
      onSelect: act(props.onDown),
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
      onClick={act(props.onOpen)}
      onKeyDown={(e) => {
        if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          props.onOpen(p);
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
          {props.alertCount > 0 && (
            // Clickable badge: opens the alert list; the newest message is the hint.
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                props.onShowAlerts(p);
              }}
              title={props.latestAlert ? `${props.alertCount} alert(s). Latest: ${props.latestAlert}` : "Alerts"}
              aria-label={`${props.alertCount} compose alert(s), show list`}
              className="inline-flex h-6 shrink-0 items-center gap-1 rounded-none border border-rose-500/50 bg-rose-500/10 px-1.5 font-mono text-[11px] text-rose-300 hover:bg-rose-500/20"
            >
              <BellRing className="h-3.5 w-3.5" aria-hidden="true" />
              {props.alertCount}
            </button>
          )}
          {props.updates.length > 0 && (
            // Clicking pulls just the services that have a newer image.
            <button
              type="button"
              disabled={busy}
              onClick={(e) => {
                e.stopPropagation();
                props.onPullUpdates(p);
              }}
              title={`Pull newer images: ${props.updates.map((u) => u.service).join(", ")}`}
              aria-label={`${props.updates.length} image updates, pull them`}
              className="inline-flex h-6 shrink-0 items-center gap-1 rounded-none border border-amber-500/40 bg-amber-500/10 px-1.5 text-[11px] text-amber-300 hover:bg-amber-500/20 disabled:opacity-50"
            >
              <ArrowUpCircle className="h-3.5 w-3.5" aria-hidden="true" />
              {props.updates.length} image update{props.updates.length === 1 ? "" : "s"}
            </button>
          )}
          {p.config_error && (
            <IconBadge tone="danger" hint={`Compose file problem: ${p.config_error}`}>
              <AlertTriangle className="h-3.5 w-3.5" aria-hidden="true" />
            </IconBadge>
          )}
          {p.watching && (
            <IconBadge tone="success" hint="compose watch is running: files are synced and services rebuilt on change">
              {/* Static: a watch runs for hours and a pulse repaints every frame. */}
              <Eye className="h-3.5 w-3.5" aria-hidden="true" />
            </IconBadge>
          )}
          {p.override_file && (
            <IconBadge tone="info" hint={`Override file: ${p.override_file} is merged on Up`}>
              <GitMerge className="h-3.5 w-3.5" aria-hidden="true" />
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
            <IconAction title="Restart all services" onClick={act(props.onRestart)} disabled={busy} className="rounded-none">
              <RotateCw className="h-4 w-4" />
            </IconAction>
          ) : (
            <IconAction
              title={hasFile ? "Up: create and start all services" : noFileHint}
              onClick={act(props.onUp)}
              disabled={busy || (!hasFile && p.services.length === 0)}
              className="rounded-none text-emerald-300"
            >
              <Play className="h-4 w-4" />
            </IconAction>
          )}
          <IconAction
            title="Stop all services (containers are kept)"
            onClick={act(props.onStop)}
            disabled={busy || !anyRunning}
            className="rounded-none"
          >
            <Square className="h-4 w-4" />
          </IconAction>
          <IconAction title="Open details" onClick={act(props.onOpen)} className="rounded-none">
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

      <dl className="grid grid-cols-2 divide-zinc-800 sm:grid-cols-3 lg:grid-cols-[8rem_8rem_minmax(0,1fr)_9rem_9rem_7rem] lg:divide-x">
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
        <InfoCell label="CPU" title="Sum over the project's running containers, with the recent trend">
          {sampled > 0 ? (
            <span className="flex items-center gap-2">
              {`${cpu.toFixed(1)}%`}
              <Sparkline values={history.cpu} label="CPU history" className="text-sky-400" />
            </span>
          ) : (
            "—"
          )}
        </InfoCell>
        <InfoCell label="Memory" title="Sum over the project's running containers, with the recent trend">
          {sampled > 0 ? (
            <span className="flex items-center gap-2">
              {formatBytes(mem)}
              <Sparkline values={history.mem} label="Memory history" className="text-violet-400" />
            </span>
          ) : (
            "—"
          )}
        </InfoCell>
        <InfoCell label="Source" title={SOURCE_HINT[p.source ?? "labels"]}>
          <span className="text-zinc-400">{p.source === "labels" ? "labels" : p.source === "settings" ? "settings" : "saved"}</span>
        </InfoCell>
      </dl>
    </div>
  );
};

/** Memoised: skips re-rendering when neither its project nor its values changed. */
export const ComposeProjectCard = React.memo(ComposeProjectCardView);
