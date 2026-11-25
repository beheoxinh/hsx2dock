import React from "react";
import {
  Activity,
  AlertTriangle,
  Camera,
  Copy,
  Ellipsis,
  FileCheck2,
  FileCode,
  FileKey2,
  FileMinus,
  FilePlus,
  Gauge,
  Hash,
  Hexagon,
  Loader2,
  Play,
  Plus,
  Rocket,
  Settings2,
  Square,
  Star,
  Terminal,
  TerminalSquare,
  Trash2,
} from "lucide-react";
import { Card } from "../ui/Card";
import { Button } from "../ui/Button";
import { Switch } from "../ui/Switch";
import { useToast } from "../ui/Toast";
import { ClusterSparkline } from "./ClusterSparkline";
import { ClusterOverflowMenu, type OverflowItem } from "./ClusterOverflowMenu";
import { AlertBadge } from "./AlertsPanel";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import {
  HEALTH_LABEL,
  accentFor,
  accessWarning,
  clusterHealth,
  hostTerminalBlocker,
  type ClusterHealthState,
} from "../../lib/k8sClusterList";
import { cn, formatBytes, formatDuration, formatRelativeTime } from "../../lib/utils";
import type {
  ClusterAccess,
  ClusterAlert,
  KindCluster,
  KindClusterPrefs,
  KindProgressPayload,
  PortForwardInfo,
} from "../../types";

export interface ClusterCardHistory {
  pods: number[];
  cpu: number[];
  mem: number[];
  disk: number[];
}

export interface ClusterCardProps {
  cluster: KindCluster;
  prefs: KindClusterPrefs;
  /** undefined = not fetched yet, null = fetch failed. */
  access: ClusterAccess | null | undefined;
  history?: ClusterCardHistory;
  /** This cluster's port-forwards only (the page filters them). */
  forwards: PortForwardInfo[];
  /** Active incident alerts for this cluster. */
  alerts: ClusterAlert[];
  /** Another kind operation is in progress; most actions wait for it. */
  busy: boolean;
  /** This cluster's power operation is the one in progress. */
  powerBusy: boolean;
  kindInstalled: boolean;
  prefsLoaded: boolean;
  /** kind output for this cluster only. */
  logLines: KindProgressPayload[];
  onToggleAutoStart: () => void;
  onOpenMonitor: () => void;
  onTogglePower: () => void;
  onOpenShell: () => void;
  onApply: (tab: "yaml" | "quick") => void;
  onPrefs: () => void;
  onCopyKubeconfig: () => void;
  onAddHostKubeconfig: () => void;
  onRemoveHostKubeconfig: () => void;
  onOpenHostTerminal: () => void;
  onCopyContext: () => void;
  onAddWorker: () => void;
  onOpenSnapshots: () => void;
  onDelete: () => void;
  onClearLog: () => void;
  onMenuOpenChange: (open: boolean) => void;
}

/** Forward states that still carry traffic or are on their way to doing so. */
const LIVE_FORWARD_STATES = new Set(["active", "reconnecting", "starting"]);

/** Status icon colours: the icon before the name is the status indicator. */
const STATE_STYLE: Record<ClusterHealthState, { icon: string; box: string }> = {
  running: { icon: "text-emerald-400", box: "bg-emerald-500/10 border-emerald-500/40" },
  degraded: { icon: "text-amber-400", box: "bg-amber-500/10 border-amber-500/40" },
  starting: { icon: "text-sky-400", box: "bg-sky-500/10 border-sky-500/40" },
  stopped: { icon: "text-zinc-500", box: "bg-zinc-800/60 border-zinc-700" },
};

type BadgeTone = "neutral" | "info" | "warning" | "accent" | "muted";

const BADGE_TONE: Record<BadgeTone, string> = {
  neutral: "text-zinc-300 bg-zinc-800/80 border-zinc-700",
  info: "text-sky-300 bg-sky-500/10 border-sky-500/30",
  warning: "text-amber-300 bg-amber-500/10 border-amber-500/40",
  accent: "text-indigo-300 bg-indigo-500/10 border-indigo-500/30",
  muted: "text-zinc-500 bg-zinc-900 border-zinc-800",
};

/** Square icon badge; the hint is the tooltip and the accessible name. */
const IconBadge: React.FC<{ hint: string; tone: BadgeTone; children: React.ReactNode }> = ({
  hint,
  tone,
  children,
}) => (
  <span
    role="img"
    aria-label={hint}
    title={hint}
    className={cn(
      "inline-flex h-6 w-6 shrink-0 cursor-help items-center justify-center border",
      BADGE_TONE[tone]
    )}
  >
    {children}
  </span>
);

/** Quick action as a compact icon button; the title is also its accessible name. */
const IconAction: React.FC<{
  title: string;
  onClick: () => void;
  disabled?: boolean;
  children: React.ReactNode;
}> = ({ title, onClick, disabled, children }) => (
  <Button
    variant="outline"
    size="icon"
    onClick={onClick}
    disabled={disabled}
    title={title}
    aria-label={title}
    className="h-8 w-8 text-zinc-300"
  >
    {children}
  </Button>
);

/** One live resource chart: label and value on top, sparkline filling the tile. */
const StatTile: React.FC<{
  label: string;
  value: string;
  sub?: string;
  title?: string;
  color: string;
  data: number[];
  max?: number;
  live: boolean;
}> = ({ label, value, sub, title, color, data, max, live }) => (
  <div
    title={title}
    className={cn(
      "flex min-w-0 flex-col border border-zinc-800 bg-zinc-950/50 px-3 pt-2 pb-1.5",
      !live && "opacity-50"
    )}
  >
    <div className="flex items-baseline justify-between gap-2 min-w-0">
      <span className="text-[11px] uppercase tracking-wider text-zinc-500 truncate">{label}</span>
      <span className="text-sm font-mono font-semibold text-white shrink-0 tabular-nums">{value}</span>
    </div>
    <div className="h-4 text-[10px] font-mono text-zinc-500 truncate">{sub ?? ""}</div>
    <ClusterSparkline data={live ? data : []} color={color} max={max} heightClass="mt-auto h-8" />
  </div>
);

/** One cell of the info panel; borders between cells come from the parent. */
const InfoCell: React.FC<{
  label: string;
  children: React.ReactNode;
  title?: string;
  className?: string;
}> = ({ label, children, title, className }) => (
  <div className={cn("min-w-0 px-3 py-2", className)} title={title}>
    <dt className="text-[10px] uppercase tracking-wider text-zinc-500">{label}</dt>
    <dd className="mt-0.5 font-mono text-xs text-zinc-200 min-w-0">{children}</dd>
  </div>
);

/**
 * One cluster as a full-width row card:
 *  - header: status icon, name, tags, auto-start and icon badges;
 *  - body: a fixed-width info panel beside a 4 → 2 → 1 grid of resource charts;
 *  - footer: the action buttons.
 */
export const ClusterCard: React.FC<ClusterCardProps> = (props) => {
  const {
    cluster: c,
    prefs,
    access,
    history,
    busy,
    powerBusy,
    kindInstalled,
    prefsLoaded,
    logLines,
  } = props;
  const toast = useToast();

  const state = clusterHealth(c);
  const stateStyle = STATE_STYLE[state];
  const accent = accentFor(prefs.color);
  const tags = prefs.tags ?? [];
  const running = c.running;
  const apiStarting = running && c.api_ready === false;
  const hostBlocker = hostTerminalBlocker(running, access);
  const warning = running && access ? accessWarning(access) : null;
  const shellBlocked = busy || !running || c.api_ready === false;
  const shellTitle = !running
    ? "Start the cluster to use kubectl"
    : c.api_ready === false
      ? "Waiting for the API server to become ready"
      : undefined;
  const monitorTitle = !running
    ? "Start the cluster to open monitoring"
    : c.api_ready === false
      ? "Waiting for the API server to become ready"
      : "Open the full monitoring screen for this cluster";

  const stateHint =
    `${HEALTH_LABEL[state]}` +
    (state === "degraded" ? ` — ${c.running_nodes}/${c.node_count} nodes running` : "") +
    (apiStarting ? " — API server starting" : "");

  const caps = [
    prefs.nodeCpuM > 0 ? `${(prefs.nodeCpuM / 1000).toFixed(1)} CPU` : null,
    prefs.nodeMemoryMb > 0 ? `${prefs.nodeMemoryMb} MB` : null,
  ].filter(Boolean);

  const timeLabel = running ? "Uptime" : c.last_started_at ? "Last run" : "Age";
  const timeValue = running
    ? formatDuration(c.uptime_seconds)
    : c.last_started_at
      ? formatRelativeTime(c.last_started_at)
      : c.created
        ? formatRelativeTime(c.created)
        : "—";

  const apiValue = !running
    ? "—"
    : access === undefined
      ? "checking…"
      : access === null
        ? "unavailable"
        : access.apiPort != null
          ? `127.0.0.1:${access.apiPort}`
          : "not published";
  const apiDot = !running || !access ? "bg-zinc-600" : access.apiReachable ? "bg-emerald-400" : "bg-amber-400";
  const apiHint = !running
    ? "Host access is available while the cluster runs"
    : access
      ? access.apiReachable
        ? `${access.context} answers on the host`
        : "No answer from the host on this port yet"
      : undefined;

  // The API port shows in its own row; only extra forwards are listed here.
  const forwards = (c.published_ports ?? []).filter(
    (p) => !(p.container_port === 6443 && access?.apiPort != null && p.host_port === access.apiPort)
  );

  const menuItems: OverflowItem[] = [
    {
      key: "copy-context",
      label: "Copy context name",
      icon: <Hash className="w-3.5 h-3.5" />,
      onSelect: props.onCopyContext,
      title: `Copy kind-${c.name} to the clipboard`,
    },
    {
      key: "copy-kubeconfig",
      label: "Copy kubeconfig path",
      icon: <Copy className="w-3.5 h-3.5" />,
      onSelect: props.onCopyKubeconfig,
      disabled: busy || !kindInstalled || !c.has_kubeconfig,
      title: c.has_kubeconfig
        ? "Copy the KUBECONFIG path for this cluster"
        : "This cluster has no kubeconfig yet",
    },
    {
      key: "host-add",
      label: "Add to ~/.kube/config",
      icon: <FilePlus className="w-3.5 h-3.5" />,
      onSelect: props.onAddHostKubeconfig,
      disabled: busy || !running,
      title: !running
        ? "Start the cluster first"
        : "Merge this cluster into your kubeconfig and make it the current context (a backup is kept)",
      dividerBefore: true,
    },
    {
      key: "host-remove",
      label: "Remove from ~/.kube/config",
      icon: <FileMinus className="w-3.5 h-3.5" />,
      onSelect: props.onRemoveHostKubeconfig,
      danger: true,
      disabled: busy || !access?.inHostKubeconfig,
      title: access?.inHostKubeconfig
        ? `Remove the kind-${c.name} entries from your kubeconfig (a backup is kept)`
        : "This cluster is not in your kubeconfig",
    },
    {
      key: "host-terminal",
      label: "Open host terminal",
      icon: <Terminal className="w-3.5 h-3.5" />,
      onSelect: props.onOpenHostTerminal,
      disabled: busy || !!hostBlocker,
      title: hostBlocker ?? "Open a terminal window on the host with KUBECONFIG set to this cluster",
      dividerBefore: true,
    },
    {
      key: "add-worker",
      label: "Add worker node",
      icon: <Plus className="w-3.5 h-3.5" />,
      onSelect: props.onAddWorker,
      disabled: busy || !running,
      title: running
        ? "Add a worker node to this cluster (takes about a minute)"
        : "Start the cluster first",
    },
    {
      key: "snapshots",
      label: "Snapshots…",
      icon: <Camera className="w-3.5 h-3.5" />,
      onSelect: props.onOpenSnapshots,
      disabled: busy,
      title: "Take, restore and clone snapshots of this cluster",
    },
    {
      key: "prefs",
      label: "Preferences",
      icon: <Settings2 className="w-3.5 h-3.5" />,
      onSelect: props.onPrefs,
      disabled: busy,
      title: "Colour, tags, auto-start and per-node caps",
      dividerBefore: true,
    },
    {
      key: "delete",
      label: "Delete cluster",
      icon: <Trash2 className="w-3.5 h-3.5" />,
      onSelect: props.onDelete,
      danger: true,
      disabled: busy,
      title: "Delete this cluster and its node containers",
      dividerBefore: true,
    },
  ];

  const copyLog = async () => {
    const text = logLines.map((l) => l.line).join("\n");
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      toast.success(
        "Kind output copied",
        `${logLines.length} line${logLines.length === 1 ? "" : "s"} on the clipboard.`
      );
    } catch (e) {
      toast.error("Copy failed", String(e));
    }
  };

  const live = running && c.pod_count != null;
  const disk = c.disk_size_bytes ?? null;
  const activeForwards = props.forwards.filter((f) => LIVE_FORWARD_STATES.has(f.status));

  const openForward = async (url: string) => {
    try {
      await openExternal(url);
    } catch (e) {
      toast.error("Could not open link", String(e));
    }
  };

  return (
    <Card
      className="p-0 hover:border-zinc-700 transition-colors"
      style={{ borderLeftWidth: 4, borderLeftColor: accent }}
      data-health={state}
    >
      {/* Header: status icon, name, tags, auto-start, icon badges */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-800/80 min-w-0">
        <span
          role="img"
          aria-label={`Status: ${stateHint}`}
          title={stateHint}
          className={cn("relative flex h-8 w-8 shrink-0 items-center justify-center border", stateStyle.box)}
        >
          {state === "starting" || powerBusy ? (
            <Loader2 className={cn("w-4 h-4 animate-spin", stateStyle.icon)} aria-hidden="true" />
          ) : (
            <Hexagon className={cn("w-4 h-4", stateStyle.icon)} aria-hidden="true" />
          )}
        </span>
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
          <h3 className="font-semibold text-white truncate max-w-full" title={c.name}>
            {c.name}
          </h3>
          {tags.map((t) => (
            <span
              key={t}
              className="shrink-0 border border-zinc-700 bg-zinc-800/80 px-1.5 py-0.5 text-[10px] text-zinc-300"
            >
              {t}
            </span>
          ))}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <Switch
            checked={prefs.autoStart}
            onChange={props.onToggleAutoStart}
            disabled={busy || !prefsLoaded}
            label="Auto-start"
            title="Start this cluster when the Docker engine comes up (the master Auto-start switch above must also be on)"
            className="mr-2 shrink-0"
          />
          <AlertBadge alerts={props.alerts} />
          {apiStarting && (
            <IconBadge
              tone="warning"
              hint="The API server is still starting — monitoring opens as soon as /readyz passes"
            >
              <Loader2 className="w-3.5 h-3.5 animate-spin" aria-hidden="true" />
            </IconBadge>
          )}
          {warning && (
            <IconBadge tone="warning" hint={warning}>
              <AlertTriangle className="w-3.5 h-3.5" aria-hidden="true" />
            </IconBadge>
          )}
          {c.has_kubeconfig && (
            <IconBadge tone="neutral" hint="The app has a kubeconfig for this cluster">
              <FileKey2 className="w-3.5 h-3.5" aria-hidden="true" />
            </IconBadge>
          )}
          {access?.inHostKubeconfig && (
            <IconBadge
              tone={access.hostCurrent ? "info" : "neutral"}
              hint={
                access.hostCurrent
                  ? `${access.context} is the current context in ${access.hostKubeconfig}`
                  : `${access.context} is in ${access.hostKubeconfig}`
              }
            >
              {access.hostCurrent ? (
                <Star className="w-3.5 h-3.5" aria-hidden="true" />
              ) : (
                <FileCheck2 className="w-3.5 h-3.5" aria-hidden="true" />
              )}
            </IconBadge>
          )}
          {running && access && !access.hostKubectl && (
            <IconBadge tone="muted" hint="kubectl is not on the host PATH; the host terminal will say so">
              <TerminalSquare className="w-3.5 h-3.5" aria-hidden="true" />
            </IconBadge>
          )}
          {caps.length > 0 && (
            <IconBadge tone="accent" hint={`Per-node resource caps: ${caps.join(" · ")}`}>
              <Gauge className="w-3.5 h-3.5" aria-hidden="true" />
            </IconBadge>
          )}
        </div>
      </div>

      {/* Body: info panel beside the resource charts */}
      <div className="grid grid-cols-1 gap-3 p-4 lg:grid-cols-[15rem_minmax(0,1fr)]">
        <dl className="grid w-full max-w-sm grid-cols-2 self-start border border-zinc-800 bg-zinc-950/40 lg:max-w-none">
          <InfoCell label={timeLabel} className="border-r border-zinc-800">
            <span className="block truncate">{timeValue}</span>
          </InfoCell>
          <InfoCell
            label="Nodes"
            title={
              c.node_count > 1
                ? `${c.running_nodes} of ${c.node_count} node containers running (control-plane + workers)`
                : "Single-node cluster: the control-plane also runs workloads"
            }
          >
            <span className={c.running_nodes < c.node_count && running ? "text-amber-300" : undefined}>
              {c.running_nodes}/{c.node_count}
            </span>
          </InfoCell>
          <InfoCell label="API server" title={apiHint} className="col-span-2 border-t border-zinc-800">
            <span className="flex items-center gap-1.5 min-w-0">
              <span aria-hidden="true" className={cn("h-1.5 w-1.5 shrink-0 rounded-full", apiDot)} />
              <span className="truncate">{apiValue}</span>
            </span>
          </InfoCell>
          {forwards.length > 0 && (
            <InfoCell label="Forwarded" className="col-span-2 border-t border-zinc-800">
              <span className="mt-0.5 flex flex-wrap gap-1">
                {forwards.map((p, idx) => {
                  const hostPart = p.host_port
                    ? `${p.host_ip && p.host_ip !== "0.0.0.0" && p.host_ip !== "::" ? `${p.host_ip}:` : ""}${p.host_port}`
                    : "—";
                  const url =
                    p.protocol === "tcp" && p.host_port > 0
                      ? `http://${p.host_ip && p.host_ip !== "0.0.0.0" && p.host_ip !== "::" ? p.host_ip : "localhost"}:${p.host_port}`
                      : null;
                  const body = (
                    <>
                      <span className="truncate">{hostPart}</span>
                      <span className="text-zinc-600">→</span>
                      <span className="text-zinc-400 shrink-0">
                        {p.container_port}/{p.protocol}
                      </span>
                    </>
                  );
                  const chip =
                    "inline-flex max-w-full items-center gap-1 border px-1.5 py-0.5 font-mono text-[10px]";
                  return url ? (
                    <a
                      key={idx}
                      href={url}
                      target="_blank"
                      rel="noreferrer"
                      className={cn(
                        chip,
                        "border-sky-500/30 bg-sky-500/10 text-sky-300 hover:bg-sky-500/20 hover:border-sky-500/50 transition-colors"
                      )}
                      title={`Open ${url} (node :${p.container_port}/${p.protocol})`}
                    >
                      {body}
                    </a>
                  ) : (
                    <span
                      key={idx}
                      className={cn(chip, "border-zinc-700 bg-zinc-800/80 text-zinc-300")}
                      title={`Host port ${hostPart} forwards to node :${p.container_port}/${p.protocol}`}
                    >
                      {body}
                    </span>
                  );
                })}
              </span>
            </InfoCell>
          )}
        </dl>

        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-4">
          <StatTile
            label="Pods"
            value={live ? String(c.running_pod_count ?? 0) : "—"}
            sub={live ? `${c.running_pod_count ?? 0} of ${c.pod_count} running` : undefined}
            color="#34d399"
            data={history?.pods ?? []}
            max={Math.max(1, c.pod_count ?? 1)}
            live={live}
          />
          <StatTile
            label="CPU"
            value={running && c.cpu_percent != null ? `${c.cpu_percent.toFixed(1)}%` : "—"}
            sub={running ? "of host CPU" : undefined}
            title="Share of the host CPU used by the node containers"
            color="#60a5fa"
            data={history?.cpu ?? []}
            live={running}
          />
          <StatTile
            label="Memory"
            value={running && c.mem_used_mb != null ? `${Math.round(c.mem_used_mb)} MiB` : "—"}
            sub={
              running && c.mem_limit_mb
                ? `of ${Math.round(c.mem_limit_mb)} MiB cap`
                : running && c.host_mem_total_mb
                  ? `of ${(c.host_mem_total_mb / 1024).toFixed(0)} GiB host`
                  : undefined
            }
            color="#a78bfa"
            data={history?.mem ?? []}
            live={running}
          />
          <StatTile
            label="Disk"
            value={disk != null && disk > 0 ? formatBytes(disk) : "—"}
            sub={disk != null && disk > 0 ? "images + node state" : undefined}
            title="Space used on the host by the node containers and their containerd state (refreshed every minute)"
            color="#f59e0b"
            data={history?.disk ?? []}
            live={disk != null && disk > 0}
          />
        </div>
      </div>

      {/* Active port-forwards for this cluster; hidden when there are none. */}
      {activeForwards.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 border-t border-zinc-800/80 px-4 py-2 text-xs">
          <span className="text-zinc-400">
            Port-forwards:{" "}
            <span className="font-mono tabular-nums text-zinc-200">{activeForwards.length}</span>{" "}
            active
          </span>
          {activeForwards.slice(0, 3).map((f) => (
            <button
              key={f.id}
              type="button"
              onClick={() => openForward(f.url)}
              title={`Open ${f.url} in the browser`}
              className="max-w-full truncate rounded-none border border-sky-500/30 bg-sky-500/10 px-1.5 py-0.5 font-mono text-[10px] text-sky-300 transition-colors hover:border-sky-500/50 hover:bg-sky-500/20 outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
            >
              {f.url}
            </button>
          ))}
          {activeForwards.length > 3 && (
            <span className="text-[10px] text-zinc-500">+{activeForwards.length - 3} more</span>
          )}
        </div>
      )}

      {/* Footer: actions */}
      <div className="flex flex-wrap items-center justify-end gap-2 border-t border-zinc-800/80 px-4 py-2.5">
        <div className="flex flex-wrap items-center gap-1.5">
          <IconAction
            title={monitorTitle}
            onClick={props.onOpenMonitor}
            disabled={busy || !running || c.api_ready === false}
          >
            <Activity className="w-3.5 h-3.5" />
          </IconAction>
          <IconAction
            title={running ? "Stop all node containers (data is kept)" : "Start this cluster's node containers"}
            onClick={props.onTogglePower}
            disabled={busy || !kindInstalled}
          >
            {powerBusy ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : running ? (
              <Square className="w-3.5 h-3.5" />
            ) : (
              <Play className="w-3.5 h-3.5" />
            )}
          </IconAction>
          <IconAction
            title={shellTitle ?? "Open a kubectl shell inside the control-plane node"}
            onClick={props.onOpenShell}
            disabled={shellBlocked}
          >
            <Terminal className="w-3.5 h-3.5" />
          </IconAction>
          <IconAction
            title={shellTitle ?? "Apply a YAML manifest to this cluster"}
            onClick={() => props.onApply("yaml")}
            disabled={shellBlocked}
          >
            <FileCode className="w-3.5 h-3.5" />
          </IconAction>
          <IconAction
            title={shellTitle ?? "Quick deploy an image as a Deployment"}
            onClick={() => props.onApply("quick")}
            disabled={shellBlocked}
          >
            <Rocket className="w-3.5 h-3.5" />
          </IconAction>
          <ClusterOverflowMenu
            label={`More actions for ${c.name}`}
            icon={<Ellipsis className="w-4 h-4" />}
            items={menuItems}
            disabled={false}
            onOpenChange={props.onMenuOpenChange}
          />
        </div>
      </div>

      {/* Kind output for this cluster — streams while an operation runs. */}
      {logLines.length > 0 && (
        <div className="mx-4 mb-4 border border-zinc-800 bg-zinc-950 overflow-hidden">
          <div className="flex items-center justify-between px-3.5 py-2 border-b border-zinc-800/80 bg-zinc-900/50">
            <span className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
              kind output — {c.name}
            </span>
            <div className="flex items-center gap-3">
              <button
                type="button"
                onClick={copyLog}
                className="text-[11px] text-zinc-500 hover:text-zinc-300 outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
              >
                Copy
              </button>
              <button
                type="button"
                onClick={props.onClearLog}
                className="text-[11px] text-zinc-500 hover:text-zinc-300 outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
              >
                Clear
              </button>
            </div>
          </div>
          <div
            ref={(el) => {
              if (el) el.scrollTop = el.scrollHeight;
            }}
            className="px-3.5 py-2.5 max-h-40 overflow-y-auto font-mono text-xs leading-relaxed space-y-0.5 select-text"
          >
            {logLines.map((l, i) => (
              <div
                key={i}
                className={/error|failed|exit status/i.test(l.line) ? "text-rose-400" : "text-zinc-400"}
              >
                {l.line}
              </div>
            ))}
          </div>
        </div>
      )}
    </Card>
  );
};
