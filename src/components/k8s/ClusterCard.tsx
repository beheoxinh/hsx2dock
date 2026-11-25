import React from "react";
import {
  Activity,
  Copy,
  Ellipsis,
  FileCode,
  FileMinus,
  FilePlus,
  Hash,
  Hexagon,
  Loader2,
  Play,
  Rocket,
  Settings2,
  Square,
  Terminal,
  Trash2,
} from "lucide-react";
import { Card } from "../ui/Card";
import { Button } from "../ui/Button";
import { StatusBadge } from "../ui/StatusBadge";
import { Switch } from "../ui/Switch";
import { useToast } from "../ui/Toast";
import { ClusterSparkline } from "./ClusterSparkline";
import { ClusterAccessPanel } from "./ClusterAccessPanel";
import { ClusterOverflowMenu, type OverflowItem } from "./ClusterOverflowMenu";
import {
  HEALTH_LABEL,
  HEALTH_TONE,
  accentFor,
  clusterHealth,
  hostTerminalBlocker,
} from "../../lib/k8sClusterList";
import { formatBytes, formatDuration, formatRelativeTime } from "../../lib/utils";
import type { ClusterAccess, KindCluster, KindClusterPrefs, KindProgressPayload } from "../../types";

export interface ClusterCardHistory {
  pods: number[];
  cpu: number[];
  mem: number[];
}

export interface ClusterCardProps {
  cluster: KindCluster;
  prefs: KindClusterPrefs;
  /** undefined = not fetched yet, null = fetch failed. */
  access: ClusterAccess | null | undefined;
  history?: ClusterCardHistory;
  selected: boolean;
  /** Another kind operation is in progress; most actions wait for it. */
  busy: boolean;
  /** This cluster's power operation is the one in progress. */
  powerBusy: boolean;
  kindInstalled: boolean;
  prefsLoaded: boolean;
  /** kind output for this cluster only. */
  logLines: KindProgressPayload[];
  onToggleSelected: () => void;
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
  onDelete: () => void;
  onClearLog: () => void;
  onMenuOpenChange: (open: boolean) => void;
}

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

/** One live stat in the row: label and value on one line, sparkline under it. */
const StatTile: React.FC<{
  label: string;
  value: string;
  title?: string;
  color: string;
  data: number[];
  max?: number;
}> = ({ label, value, title, color, data, max }) => (
  <div
    title={title}
    className="w-28 shrink-0 rounded-xl border border-zinc-800/80 bg-zinc-950/40 px-2.5 py-1.5 min-w-0"
  >
    <div className="flex items-baseline justify-between gap-2 min-w-0">
      <p className="text-[11px] text-zinc-500 truncate">{label}</p>
      <p className="text-xs text-white font-mono font-medium shrink-0">{value}</p>
    </div>
    <ClusterSparkline data={data} color={color} max={max} heightClass="h-6" />
  </div>
);

/**
 * One cluster as a full-width row: identity on the left, meta / host access /
 * live stats in the middle, switch and quick actions on the right. On narrow
 * screens the three parts stack.
 */
export const ClusterCard: React.FC<ClusterCardProps> = (props) => {
  const {
    cluster: c,
    prefs,
    access,
    history,
    selected,
    busy,
    powerBusy,
    kindInstalled,
    prefsLoaded,
    logLines,
  } = props;
  const toast = useToast();

  const state = clusterHealth(c);
  const accent = accentFor(prefs.color);
  const tags = prefs.tags ?? [];
  const running = c.running;
  const apiStarting = running && c.api_ready === false;
  const hostBlocker = hostTerminalBlocker(running, access);
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

  return (
    <Card
      className="p-4 hover:border-zinc-700 transition-colors"
      style={{ borderLeftWidth: 4, borderLeftColor: accent }}
      data-health={state}
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:gap-4">
        {/* Identity: selection, icon, name with tags, health badges */}
        <div className="flex items-center gap-2.5 min-w-0 lg:w-64 lg:shrink-0">
          <input
            type="checkbox"
            checked={selected}
            onChange={props.onToggleSelected}
            disabled={busy}
            aria-label={`Select cluster ${c.name}`}
            title="Select for bulk start/stop"
            className="rounded border-zinc-700 w-4 h-4 accent-blue-500 shrink-0"
          />
          <div
            className="w-8 h-8 rounded-lg border flex items-center justify-center shrink-0"
            style={{ backgroundColor: `${accent}1a`, borderColor: `${accent}66` }}
          >
            <Hexagon className="w-4 h-4" style={{ color: accent }} aria-hidden="true" />
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-1.5 min-w-0">
              <p className="font-semibold text-white truncate" title={c.name}>
                {c.name}
              </p>
              {tags.map((t) => (
                <span
                  key={t}
                  className="text-[10px] px-1.5 py-0.5 rounded-full bg-zinc-800/80 text-zinc-300 border border-zinc-700/60 shrink-0"
                >
                  {t}
                </span>
              ))}
            </div>
            <div className="flex flex-wrap items-center gap-1.5 mt-1">
              <StatusBadge
                status={HEALTH_LABEL[state]}
                tone={HEALTH_TONE[state]}
                pulse={state === "running" || state === "starting"}
              />
              {apiStarting && (
                <span
                  title="The API server is still starting — monitoring opens as soon as /readyz passes"
                  className="text-[10px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-300 border border-amber-500/30"
                >
                  API starting…
                </span>
              )}
              {c.has_kubeconfig && (
                <span className="text-[10px] px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-400 border border-zinc-700/60 font-mono">
                  kubeconfig
                </span>
              )}
              {(prefs.nodeCpuM > 0 || prefs.nodeMemoryMb > 0) && (
                <span
                  title="Per-node resource caps"
                  className="text-[10px] px-1.5 py-0.5 rounded bg-indigo-500/10 text-indigo-300 border border-indigo-500/30 font-mono"
                >
                  {[
                    prefs.nodeCpuM > 0 ? `${(prefs.nodeCpuM / 1000).toFixed(1)} cpu` : null,
                    prefs.nodeMemoryMb > 0 ? `${prefs.nodeMemoryMb} MB` : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              )}
            </div>
          </div>
        </div>

        {/* Middle: meta, host access, forwarded ports, then live stats */}
        <div className="flex-1 min-w-0 flex flex-col gap-2 lg:flex-row lg:items-center lg:gap-4">
          <div className="flex-1 min-w-0 space-y-1">
            <p className="text-xs text-zinc-500">
              {running ? "Uptime" : c.last_started_at ? "Last run" : "Age"}{" "}
              <span className="font-mono text-zinc-300">
                {running
                  ? formatDuration(c.uptime_seconds)
                  : c.last_started_at
                    ? formatRelativeTime(c.last_started_at)
                    : c.created
                      ? formatRelativeTime(c.created)
                      : "—"}
              </span>
              <span className="mx-2 text-zinc-700">·</span>
              Nodes{" "}
              <span className="font-mono text-zinc-300">
                {c.running_nodes}/{c.node_count}
              </span>
              {c.disk_size_bytes != null && c.disk_size_bytes > 0 && (
                <>
                  <span className="mx-2 text-zinc-700">·</span>
                  Disk <span className="font-mono text-zinc-300">{formatBytes(c.disk_size_bytes)}</span>
                </>
              )}
            </p>
            <ClusterAccessPanel running={running} access={access} />
            {c.published_ports && c.published_ports.length > 0 && (
              <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
                <span className="text-zinc-500 font-medium">Forwarded:</span>
                {c.published_ports.map((p, idx) => {
                  const hostPart = p.host_port ? `${p.host_ip ? `${p.host_ip}:` : ""}${p.host_port}` : "—";
                  const isHttp =
                    p.protocol === "tcp" && (p.host_port === 80 || p.host_port === 443 || p.host_port >= 1024);
                  const url = isHttp
                    ? `http://${p.host_ip && p.host_ip !== "0.0.0.0" && p.host_ip !== "::" ? p.host_ip : "localhost"}:${p.host_port}`
                    : null;
                  return url ? (
                    <a
                      key={idx}
                      href={url}
                      target="_blank"
                      rel="noreferrer"
                      className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-sky-500/10 border border-sky-500/25 text-sky-300 font-mono text-[10px] hover:bg-sky-500/20 hover:border-sky-500/40 transition-colors"
                      title={`Open ${url} in browser (maps to node :${p.container_port}/${p.protocol})`}
                    >
                      <span>{hostPart}</span>
                      <span className="text-sky-400/60">→</span>
                      <span className="text-zinc-400">{p.container_port}/{p.protocol}</span>
                    </a>
                  ) : (
                    <span
                      key={idx}
                      className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-zinc-800/80 border border-zinc-700/60 text-zinc-300 font-mono text-[10px]"
                      title={`Host port ${hostPart} forwards to node container :${p.container_port}/${p.protocol}`}
                    >
                      <span>{hostPart}</span>
                      <span className="text-zinc-500">→</span>
                      <span className="text-zinc-400">{p.container_port}/{p.protocol}</span>
                    </span>
                  );
                })}
              </div>
            )}
          </div>

          {running && (
            <div className="flex items-center gap-2 shrink-0">
              <StatTile
                label={`Pods${c.pod_count != null ? ` ${c.running_pod_count ?? 0}/${c.pod_count}` : ""}`}
                value={c.pod_count != null ? String(c.running_pod_count ?? 0) : "—"}
                color="#34d399"
                data={history?.pods ?? []}
                max={Math.max(1, c.pod_count ?? 1)}
              />
              <StatTile
                label="CPU"
                value={c.cpu_percent != null ? `${c.cpu_percent.toFixed(1)}%` : "—"}
                title="Share of the host CPU"
                color="#60a5fa"
                data={history?.cpu ?? []}
              />
              <StatTile
                label="Memory"
                value={c.mem_used_mb != null ? `${Math.round(c.mem_used_mb)} MiB` : "—"}
                title={
                  c.mem_used_mb != null && c.mem_limit_mb
                    ? `of ${Math.round(c.mem_limit_mb)} MiB cap`
                    : undefined
                }
                color="#a78bfa"
                data={history?.mem ?? []}
              />
            </div>
          )}
        </div>

        {/* Actions: auto-start, quick icon buttons, overflow menu */}
        <div className="flex items-center gap-1.5 shrink-0 flex-wrap lg:flex-nowrap">
          <Switch
            checked={prefs.autoStart}
            onChange={props.onToggleAutoStart}
            disabled={busy || !prefsLoaded}
            label="Auto-start"
            title="Start this cluster when the Docker engine comes up (the master Auto-start switch above must also be on)"
            className="mr-2 shrink-0"
          />
          <IconAction title={monitorTitle} onClick={props.onOpenMonitor} disabled={busy || !running || c.api_ready === false}>
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
        <div className="mt-3 rounded-xl border border-zinc-800 bg-zinc-950 overflow-hidden">
          <div className="flex items-center justify-between px-3.5 py-2 border-b border-zinc-800/80 bg-zinc-900/50">
            <span className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
              kind output — {c.name}
            </span>
            <div className="flex items-center gap-3">
              <button
                type="button"
                onClick={copyLog}
                className="text-[11px] text-zinc-500 hover:text-zinc-300 outline-none focus-visible:ring-2 focus-visible:ring-blue-500 rounded"
              >
                Copy
              </button>
              <button
                type="button"
                onClick={props.onClearLog}
                className="text-[11px] text-zinc-500 hover:text-zinc-300 outline-none focus-visible:ring-2 focus-visible:ring-blue-500 rounded"
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
