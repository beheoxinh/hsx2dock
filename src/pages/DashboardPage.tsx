import React, { useEffect, useRef, useState } from "react";
import {
  Boxes,
  HardDrive,
  Layers,
  Network,
  Server,
  Trash2,
  Cpu,
} from "lucide-react";
import { motion } from "framer-motion";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { useToast } from "../components/ui/Toast";
import { useModal } from "../components/ui/ModalContext";
import { StatTileSkeleton, ChartSkeleton, DonutSkeleton, KeyValueSkeleton } from "../components/ui/Skeleton";
import { api, events } from "../lib/api";
import { useContainerStore } from "../lib/containerStore";
import {
  primeDashboardStore,
  refreshDashboardClusters,
  useDashboardStore,
} from "../lib/dashboardStore";
import { useSeries } from "../hooks/useSeries";
import { AreaChart } from "../components/charts/AreaChart";
import { DonutChart } from "../components/charts/DonutChart";
import { cn, formatBytes, formatDuration, formatRelativeTime } from "../lib/utils";
import type {
  ContainerStats,
  DaemonState,
} from "../types";

interface DashboardPageProps {
  daemonState: DaemonState;
  onNavigate: (
    tab:
      | "dashboard"
      | "containers"
      | "images"
      | "volumes"
      | "networks"
      | "kubernetes"
      | "compose"
      | "settings"
  ) => void;
}

interface StatSample {
  cpu: number;
  mem: number;
  rxRate: number;
  txRate: number;
}

const MAX_SAMPLES = 120;

/**
 * Live chart samples outlive dashboard unmounts. The listener below is
 * page-owned, so history pauses while another tab is open; on return the chart
 * is already populated instead of replaying "Collecting samples…" from zero.
 */
const seriesCache: StatSample[] = [];

/**
 * The page fills `main` exactly (the app clips overflow rather than scrolling),
 * so every row is sized to share the available height instead of stacking to a
 * fixed total. Rows are `auto-rows-fr` grids: they split the height evenly and
 * their children can shrink, which is what keeps all panels on screen.
 */
export const DashboardPage: React.FC<DashboardPageProps> = ({
  daemonState,
  onNavigate,
}) => {
  const toast = useToast();
  const modal = useModal();

  // Engine figures come from the app-wide cache primed at startup, and the
  // container list comes from the cache the Containers screen shares, so this
  // page paints from memory instead of fetching on every tab switch.
  const dashboard = useDashboardStore();
  const {
    engineInfo,
    diskUsage,
    images,
    volumes,
    networks,
    kindStatus,
    embeddedBinaries,
    clusters,
  } = dashboard;
  const containers = useContainerStore().containers;
  // First-load gate for the summary tiles: placeholders instead of zeros that
  // snap to real figures a moment later.
  const loading = !dashboard.ready;
  const [pruning, setPruning] = useState(false);
  // The node image is a setting (Settings → Kubernetes), not a constant.
  const [nodeImage, setNodeImage] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    api
      .getAppConfig()
      .then((cfg) => {
        if (alive) setNodeImage(cfg.kind_default_image || null);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  const kindNetwork = networks.find((n) => n.name === "kind");

  // Kind node ids, so their CPU/memory never inflates the user's charts.
  const kindIdsRef = useRef<Set<string>>(new Set());

  const { samples, push, reset } = useSeries<StatSample>(MAX_SAMPLES, seriesCache);

  // Kind node containers live on the Kubernetes tab — exclude them so the
  // dashboard reflects the user's own containers.
  const appContainers = containers.filter((c) => !c.kind_cluster);
  const runningCount = appContainers.filter((c) => c.state === "running").length;
  const isRunning = daemonState === "Running";

  // Keep the Kind container id set current so their stats never reach the
  // user-facing charts (the engine reports every running container).
  useEffect(() => {
    kindIdsRef.current = new Set(
      containers.filter((c) => c.kind_cluster).map((c) => c.id)
    );
  }, [containers]);

  // Cluster status for the dashboard card. Clusters change slowly, so a gentle
  // poll is enough while the engine is up.
  useEffect(() => {
    if (!isRunning) return;
    void refreshDashboardClusters();
    const timer = window.setInterval(() => {
      // A window hidden to the tray polls nothing; the next visible tick catches up.
      if (document.hidden) return;
      void refreshDashboardClusters();
    }, 10_000);
    return () => window.clearInterval(timer);
  }, [isRunning]);

  const loadData = async () => {
    // Cached panels stay on screen; the store only revalidates what is stale
    // and emits when the answer actually differs from the cache.
    await primeDashboardStore();
  };

  useEffect(() => {
    loadData();

    let unlistenStats: (() => void) | undefined;
    let disposed = false;
    let prevNet: Record<string, { rx: number; tx: number }> = {};
    let lastTs = Date.now();

    events.onContainerStatsTick((tick: Record<string, ContainerStats>) => {
      const now = Date.now();
      const dt = Math.max(0.5, (now - lastTs) / 1000);
      lastTs = now;

      const ids = Object.keys(tick);
      let cpuSum = 0;
      let memSum = 0;
      let rxRate = 0;
      let txRate = 0;
      const nextNet: Record<string, { rx: number; tx: number }> = {};

      let counted = 0;
      for (const id of ids) {
        // Kind nodes belong to the Kubernetes tab; they must not move the
        // dashboard's CPU/memory/network charts.
        if (kindIdsRef.current.has(id)) continue;
        counted += 1;
        const s = tick[id];
        const rx = s.net_rx ?? 0;
        const tx = s.net_tx ?? 0;
        const prev = prevNet[id];
        if (prev) {
          rxRate += Math.max(0, rx - prev.rx) / dt;
          txRate += Math.max(0, tx - prev.tx) / dt;
        }
        nextNet[id] = { rx, tx };
        cpuSum += s.cpu_percent ?? 0;
        memSum += s.mem_usage ?? 0;
      }
      prevNet = nextNet;
      // Hidden window: nobody sees the charts, so skip the store write (one
      // re-render per 2s otherwise). The rate baseline above still advances.
      if (document.hidden) return;

      const sample: StatSample = {
        cpu: counted ? cpuSum / counted : 0,
        mem: memSum,
        rxRate,
        txRate,
      };
      seriesCache.push(sample);
      if (seriesCache.length > MAX_SAMPLES) {
        seriesCache.splice(0, seriesCache.length - MAX_SAMPLES);
      }
      push(sample);
    }).then((u) => {
      if (disposed) u();
      else unlistenStats = u;
    });

    return () => {
      disposed = true;
      if (unlistenStats) unlistenStats();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [daemonState]);

  useEffect(() => {
    if (!isRunning) {
      seriesCache.length = 0;
      reset();
    }
  }, [isRunning, reset]);

  const handlePrune = async () => {
    const confirmed = await modal.showConfirm(
      "Prune unused containers, images, volumes and build cache?\n\nThis action cannot be undone.",
      "Prune System",
      "danger"
    );
    if (!confirmed) return;
    try {
      setPruning(true);
      await api.pruneSystem();
      toast.success("Prune completed", "Reclaimable space freed");
      // Prune invalidates disk usage and list contents; bypass the TTLs.
      await primeDashboardStore(true);
    } catch (e) {
      toast.error("Prune error", String(e));
    } finally {
      setPruning(false);
    }
  };

  const totalMem = engineInfo?.memory_total ?? 0;
  const last = samples.length ? samples[samples.length - 1] : undefined;
  const cpuSeries = samples.map((s) => s.cpu);
  const memSeries = samples.map((s) => s.mem);
  const rxSeries = samples.map((s) => s.rxRate);
  const txSeries = samples.map((s) => s.txRate);

  const runningClusters = clusters.filter((c) => c.running).length;

  const kpiItems: {
    icon: typeof Server;
    label: string;
    value: string;
    sub: string;
    color: string;
    glow: string;
    onClick?: () => void;
  }[] = [
    {
      icon: Server,
      label: "Containers",
      value: String(appContainers.length),
      sub: `${runningCount} running`,
      color: "text-blue-400",
      glow: "from-blue-500/15 to-blue-600/5 border-blue-500/25",
      onClick: () => onNavigate("containers"),
    },
    {
      icon: Layers,
      label: "Images",
      value: isRunning ? String(images.length) : "—",
      sub: "local images",
      color: "text-purple-400",
      glow: "from-purple-500/15 to-purple-600/5 border-purple-500/25",
      onClick: () => onNavigate("images"),
    },
    {
      icon: HardDrive,
      label: "Volumes",
      value: String(volumes.filter((v) => !v.protected).length),
      sub: "persistent volumes",
      color: "text-amber-400",
      glow: "from-amber-500/15 to-amber-600/5 border-amber-500/25",
      onClick: () => onNavigate("volumes"),
    },
    {
      icon: Network,
      label: "Networks",
      value: String(networks.filter((n) => !n.protected && n.name !== "kind").length),
      sub: "bridge + custom",
      color: "text-indigo-400",
      glow: "from-indigo-500/15 to-indigo-600/5 border-indigo-500/25",
      onClick: () => onNavigate("networks"),
    },
    {
      icon: HardDrive,
      label: "Disk Use",
      value: diskUsage
        ? formatBytes(
            (diskUsage.images_size ?? 0) +
              (diskUsage.containers_size ?? 0) +
              (diskUsage.volumes_size ?? 0) +
              (diskUsage.build_cache_size ?? 0)
          )
        : "—",
      sub: diskUsage
        ? `${formatBytes(diskUsage.total_reclaimable ?? 0)} reclaimable`
        : "reclaimable",
      color: "text-emerald-400",
      glow: "from-emerald-500/15 to-emerald-600/5 border-emerald-500/25",
    },
  ];

  const storageSegments = diskUsage
    ? [
        { label: "Images", value: diskUsage.images_size ?? 0, color: "#a855f7" },
        { label: "Containers", value: diskUsage.containers_size ?? 0, color: "#3b82f6" },
        { label: "Volumes", value: diskUsage.volumes_size ?? 0, color: "#f59e0b" },
        { label: "Build cache", value: diskUsage.build_cache_size ?? 0, color: "#f43f5e" },
      ]
    : [];
  const storageTotal = storageSegments.reduce((a, s) => a + s.value, 0);

  const dockerSpecRows: [string, string][] = [
    ["Docker Engine", engineInfo?.version ?? "—"],
    ["API Version", engineInfo?.api_version ? `v${engineInfo.api_version}` : "—"],
    ["Storage driver", engineInfo?.driver ?? "—"],
    [
      "Platform",
      engineInfo?.os && engineInfo?.arch ? `${engineInfo.os}/${engineInfo.arch}` : "—",
    ],
    ["CPU cores", engineInfo ? String(engineInfo.cpus) : "—"],
    ["Total memory", totalMem ? formatBytes(totalMem) : "—"],
  ];

  const k8sSpecRows: [string, string][] = [
    ["Kind Binary", kindStatus?.installed ? (kindStatus.version ? `v${kindStatus.version}` : "installed") : "not installed"],
    ["Clusters", clusters.length > 0 ? `${runningClusters}/${clusters.length} running` : "none"],
    ["Kubernetes Node Image", nodeImage ?? "kind default"],
    [
      "Network Driver",
      kindNetwork
        ? `kind ${kindNetwork.driver}${kindNetwork.subnet ? ` (${kindNetwork.subnet})` : ""}`
        : "not created",
    ],
    ["Kubeconfig", kindStatus?.installed ? "isolated (hsx2dock)" : "not created"],
  ];

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 overflow-y-auto p-4">
      {/* KPI strip — every metric appears exactly once */}
      <div className="grid shrink-0 auto-rows-fr grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
        {loading &&
          Array.from({ length: 5 }, (_, i) => (
            <StatTileSkeleton key={`kpi-skeleton-${i}`} />
          ))}
        {!loading && kpiItems.map((k) => {
          const Icon = k.icon;
          const body = (
            <Card
              className={`h-full border bg-gradient-to-br p-3 ${k.glow} ${
                k.onClick ? "transition-colors hover:border-zinc-700/70" : ""
              }`}
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="text-[10px] uppercase tracking-wider text-zinc-500">
                    {k.label}
                  </p>
                  <p className="mt-1 text-xl font-semibold tabular-nums text-white">
                    {k.value}
                  </p>
                  <p className="truncate text-[11px] text-zinc-500">{k.sub}</p>
                </div>
                <div
                  className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-zinc-900/60 ${k.color}`}
                >
                  <Icon className="h-4 w-4" />
                </div>
              </div>
            </Card>
          );
          return k.onClick ? (
            <button key={k.label} onClick={k.onClick} className="text-left">
              {body}
            </button>
          ) : (
            <div key={k.label}>{body}</div>
          );
        })}
      </div>

      {/* Live resource charts — each row shares the leftover height */}
      <div className="grid min-h-0 flex-[1.15] auto-rows-fr grid-cols-1 gap-3 lg:grid-cols-2">
        <Card className="flex min-h-[132px] flex-col gap-2 p-3 [@media(max-height:820px)]:min-h-[104px]">
          <div className="flex shrink-0 items-start justify-between gap-2">
            <div className="min-w-0">
              <h3 className="text-sm font-semibold text-white">CPU Usage</h3>
              <p className="truncate text-[11px] text-zinc-500">
                Average across {runningCount} running{" "}
                {runningCount === 1 ? "container" : "containers"}
              </p>
            </div>
            <span className="shrink-0 text-sm font-medium tabular-nums text-cyan-400">
              {last ? `${last.cpu.toFixed(1)}%` : "—"}
            </span>
          </div>
          <div className="min-h-0 flex-1">
            {isRunning && samples.length === 0 ? (
              // No running container means no stats tick will ever arrive:
              // a pulsing "collecting" placeholder would animate forever.
              runningCount > 0 ? (
                <ChartSkeleton hint="Collecting samples…" />
              ) : (
                <ChartSkeleton hint="No running containers" pulse={false} />
              )
            ) : (
              <motion.div
                className="h-full"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ duration: 0.35 }}
              >
                <AreaChart
                  series={[{ name: "CPU", color: "#22d3ee", data: cpuSeries }]}
                  formatValue={(v) => `${v.toFixed(0)}%`}
                  yMax={100}
                  emptyHint={isRunning ? "Collecting samples…" : "Engine not running"}
                  timeHint={`last ${MAX_SAMPLES} samples (≈4 min)`}
                />
              </motion.div>
            )}
          </div>
        </Card>

        <Card className="flex min-h-[132px] flex-col gap-2 p-3 [@media(max-height:820px)]:min-h-[104px]">
          <div className="flex shrink-0 items-start justify-between gap-2">
            <div className="min-w-0">
              <h3 className="text-sm font-semibold text-white">Memory Usage</h3>
              <p className="truncate text-[11px] text-zinc-500">
                {totalMem > 0
                  ? `${(((last?.mem ?? 0) / totalMem) * 100).toFixed(1)}% of ${formatBytes(totalMem)}`
                  : "Total limit unavailable"}
              </p>
            </div>
            <span className="shrink-0 text-sm font-medium tabular-nums text-lime-400">
              {last ? formatBytes(last.mem) : "—"}
            </span>
          </div>
          <div className="min-h-0 flex-1">
            {isRunning && samples.length === 0 ? (
              // No running container means no stats tick will ever arrive:
              // a pulsing "collecting" placeholder would animate forever.
              runningCount > 0 ? (
                <ChartSkeleton hint="Collecting samples…" />
              ) : (
                <ChartSkeleton hint="No running containers" pulse={false} />
              )
            ) : (
              <motion.div
                className="h-full"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ duration: 0.35 }}
              >
                <AreaChart
                  series={[{ name: "Memory", color: "#a3e635", data: memSeries }]}
                  formatValue={(v) => formatBytes(v)}
                  yMax={totalMem > 0 ? totalMem : undefined}
                  emptyHint={isRunning ? "Collecting samples…" : "Engine not running"}
                  timeHint={`last ${MAX_SAMPLES} samples (≈4 min)`}
                />
              </motion.div>
            )}
          </div>
        </Card>
      </div>

      <div className="grid min-h-0 flex-[1.15] auto-rows-fr grid-cols-1 gap-3 lg:grid-cols-2">
        <Card className="flex min-h-[132px] flex-col gap-2 p-3 [@media(max-height:820px)]:min-h-[104px]">
          <div className="flex shrink-0 items-start justify-between gap-2">
            <div className="min-w-0">
              <h3 className="text-sm font-semibold text-white">Network I/O</h3>
              <p className="truncate text-[11px] text-zinc-500">
                Aggregate throughput across containers
              </p>
            </div>
            <div className="flex shrink-0 gap-3 text-xs tabular-nums">
              <span className="text-sky-400">
                ↓ {last ? `${formatBytes(last.rxRate)}/s` : "—"}
              </span>
              <span className="text-orange-400">
                ↑ {last ? `${formatBytes(last.txRate)}/s` : "—"}
              </span>
            </div>
          </div>
          <div className="min-h-0 flex-1">
            {isRunning && samples.length === 0 ? (
              // No running container means no stats tick will ever arrive:
              // a pulsing "collecting" placeholder would animate forever.
              runningCount > 0 ? (
                <ChartSkeleton hint="Collecting samples…" />
              ) : (
                <ChartSkeleton hint="No running containers" pulse={false} />
              )
            ) : (
              <motion.div
                className="h-full"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ duration: 0.35 }}
              >
                <AreaChart
                  series={[
                    { name: "Download", color: "#38bdf8", data: rxSeries },
                    { name: "Upload", color: "#fb923c", data: txSeries },
                  ]}
                  formatValue={(v) => `${formatBytes(v)}/s`}
                  emptyHint={isRunning ? "Collecting samples…" : "Engine not running"}
                  timeHint={`last ${MAX_SAMPLES} samples (≈4 min)`}
                />
              </motion.div>
            )}
          </div>
        </Card>

        <Card className="flex min-h-[132px] flex-col gap-2 p-3 [@media(max-height:820px)]:min-h-[104px]">
          <div className="flex shrink-0 items-center justify-between gap-2">
            <div className="min-w-0">
              <h3 className="text-sm font-semibold text-white">Kubernetes Clusters</h3>
              <p className="truncate text-[11px] text-zinc-500">
                {loading
                  ? "Loading clusters…"
                  : clusters.length === 0
                    ? isRunning
                      ? "No clusters yet"
                      : "Engine not running"
                    : `${runningClusters} of ${clusters.length} running`}
              </p>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onNavigate("kubernetes")}
              className="shrink-0 text-[11px] text-zinc-400 hover:text-zinc-100"
            >
              View all
            </Button>
          </div>
          <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto">
            {isRunning && loading && (
              <KeyValueSkeleton rows={3} compact />
            )}
            {!loading && clusters.length === 0 && (
              <div className="flex h-full flex-col items-center justify-center gap-1 text-center">
                <Boxes className="h-7 w-7 text-zinc-600" />
                <p className="text-[11px] text-zinc-500">
                  {isRunning
                    ? "Create a cluster on the Kubernetes page"
                    : "Start the engine to manage clusters"}
                </p>
              </div>
            )}
            {clusters.slice(0, 8).map((c) => {
              // Resource usage for running clusters; last run for stopped ones.
              const parts: string[] = [];
              if (c.running) {
                if (c.cpu_percent != null) parts.push(`${c.cpu_percent.toFixed(1)}% CPU`);
                if (c.mem_used_mb != null) parts.push(`${c.mem_used_mb.toFixed(0)} MB RAM`);
                if (c.disk_size_bytes != null && c.disk_size_bytes > 0) {
                  parts.push(formatBytes(c.disk_size_bytes));
                }
              }

              return (
                <button
                  key={c.name}
                  type="button"
                  onClick={() => onNavigate("kubernetes")}
                  className="flex w-full items-center gap-2 rounded-lg border border-zinc-800/60 bg-zinc-900/40 px-2.5 py-1.5 text-left transition-colors hover:bg-zinc-800/60"
                >
                  <span
                    className={cn(
                      "h-1.5 w-1.5 shrink-0 rounded-full",
                      c.running ? "bg-emerald-400" : "bg-zinc-500"
                    )}
                  />
                  <span className="min-w-0 flex-1 truncate text-xs font-medium text-zinc-200">
                    {c.name}
                  </span>
                  <div className="shrink-0 flex items-center gap-2 text-[10px] tabular-nums text-zinc-400">
                    {c.running ? (
                      <>
                        <span className="text-zinc-400">
                          {c.running_nodes}/{c.node_count} nodes
                          {c.uptime_seconds != null ? ` · ${formatDuration(c.uptime_seconds)}` : ""}
                        </span>
                        {parts.length > 0 && (
                          <span className="text-zinc-300 font-mono bg-zinc-800/70 px-1.5 py-0.5 rounded border border-zinc-700/50">
                            {parts.join(" · ")}
                          </span>
                        )}
                      </>
                    ) : (
                      <span className="text-zinc-500">
                        stopped
                        {c.last_started_at ? ` · last run ${formatRelativeTime(c.last_started_at)}` : ""}
                      </span>
                    )}
                  </div>
                </button>
              );
            })}
          </div>
        </Card>
      </div>

      {/* Storage · Engine · Containers */}
      <div className="grid min-h-0 flex-[1.25] auto-rows-fr grid-cols-1 gap-3 lg:grid-cols-2 xl:grid-cols-3">
        <Card className="flex min-h-[150px] flex-col gap-2 p-3 [@media(max-height:820px)]:min-h-[120px]">
          <div className="flex shrink-0 items-start justify-between gap-2">
            <div className="min-w-0">
              <h3 className="text-sm font-semibold text-white">Storage</h3>
              <p className="truncate text-[11px] text-zinc-500">
                {loading
                  ? "Reading disk usage…"
                  : `${formatBytes(diskUsage?.total_reclaimable ?? 0)} reclaimable`}
              </p>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={handlePrune}
              disabled={pruning || !diskUsage || (diskUsage.total_reclaimable ?? 0) <= 0}
              className="shrink-0 text-[11px] text-zinc-400 hover:text-zinc-100"
            >
              <Trash2 className="h-3.5 w-3.5" />
              {pruning ? "Pruning…" : "Prune"}
            </Button>
          </div>
          <div className="min-h-0 flex-1">
            {loading ? (
              <DonutSkeleton />
            ) : (
              <motion.div
                className="h-full"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ duration: 0.35 }}
              >
                <DonutChart
                  segments={storageSegments}
                  total={storageTotal}
                  centerLabel="Total"
                />
              </motion.div>
            )}
          </div>
        </Card>

        {/* Engine Specs (Docker + Kubernetes) */}
        <Card className="flex min-h-[150px] flex-col gap-2 p-3 [@media(max-height:820px)]:min-h-[120px]">
          <div className="flex shrink-0 items-center justify-between gap-2">
            <div className="flex items-center gap-1.5 min-w-0">
              <Server className="w-4 h-4 text-sky-400 shrink-0" />
              <h3 className="text-sm font-semibold text-white truncate">Engines</h3>
            </div>
            <div className="flex items-center gap-1.5 shrink-0">
              <span className="text-[10px] font-mono px-1.5 py-0.5 rounded bg-sky-500/10 text-sky-300 border border-sky-500/20">
                Docker + K8s
              </span>
            </div>
          </div>
          <div className="min-h-0 flex-1 space-y-2.5 overflow-y-auto text-xs pr-1">
            {/* Docker Engine info */}
            <div>
              <div className="flex items-center gap-1.5 text-[11px] font-semibold text-zinc-300 uppercase tracking-wider mb-1">
                <span className="w-1.5 h-1.5 rounded-full bg-blue-400" />
                Docker Engine
              </div>
              <div className="space-y-1 pl-3 border-l border-zinc-800">
                {loading ? (
                  <KeyValueSkeleton rows={6} />
                ) : (
                  dockerSpecRows.map(([label, value]) => (
                    <div key={label} className="flex items-center justify-between gap-3">
                      <span className="text-zinc-400">{label}</span>
                      <span className="truncate tabular-nums text-zinc-200 font-mono text-[11px]">{value}</span>
                    </div>
                  ))
                )}
              </div>
            </div>

            {/* Kubernetes Kind info */}
            <div className="pt-1">
              <div className="flex items-center gap-1.5 text-[11px] font-semibold text-zinc-300 uppercase tracking-wider mb-1">
                <span className="w-1.5 h-1.5 rounded-full bg-sky-400" />
                Kubernetes (Kind)
              </div>
              <div className="space-y-1 pl-3 border-l border-zinc-800">
                {loading ? (
                  <KeyValueSkeleton rows={5} />
                ) : (
                  k8sSpecRows.map(([label, value]) => (
                    <div key={label} className="flex items-center justify-between gap-3">
                      <span className="text-zinc-400">{label}</span>
                      <span className="truncate tabular-nums text-zinc-200 font-mono text-[11px]">{value}</span>
                    </div>
                  ))
                )}
              </div>
            </div>
          </div>
        </Card>

        {/* Embedded & Bundled Binaries */}
        <Card className="flex min-h-[150px] flex-col gap-2 p-3 lg:col-span-2 xl:col-span-1 [@media(max-height:820px)]:min-h-[120px]">
          <div className="flex shrink-0 items-center justify-between gap-2">
            <div className="flex items-center gap-1.5 min-w-0">
              <Cpu className="w-4 h-4 text-purple-400 shrink-0" />
              <h3 className="text-sm font-semibold text-white truncate">Embedded Tools</h3>
            </div>
            <span className="text-[10px] text-zinc-500 font-mono">
              {loading ? "…" : `${embeddedBinaries.length} binaries`}
            </span>
          </div>
          <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto pr-1">
            {embeddedBinaries.map((bin) => (
              <div
                key={bin.name}
                className="flex items-center justify-between gap-2.5 rounded-lg border border-zinc-800/60 bg-zinc-900/40 px-2.5 py-1.5 hover:border-zinc-700/60 transition-colors"
                title={`${bin.name} (${bin.category})\nPath: ${bin.path || "Not installed"}\n${bin.description}`}
              >
                <div className="flex items-center gap-2 min-w-0 flex-1">
                  <span
                    className={cn(
                      "w-1.5 h-1.5 shrink-0 rounded-full",
                      bin.is_active ? "bg-emerald-400" : "bg-zinc-600"
                    )}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="text-xs font-semibold text-white font-mono">
                        {bin.name}
                      </span>
                      <span className="text-[9px] px-1 py-0.2 rounded bg-zinc-800 text-zinc-400 border border-zinc-700/40">
                        {bin.category}
                      </span>
                    </div>
                    <div className="text-[10px] text-zinc-500 truncate mt-0.5">
                      {bin.description}
                    </div>
                  </div>
                </div>

                <div className="shrink-0 text-right">
                  <span
                    className={cn(
                      "text-[11px] font-mono font-medium",
                      bin.version ? "text-purple-300" : "text-zinc-600"
                    )}
                  >
                    {bin.version || "missing"}
                  </span>
                </div>
              </div>
            ))}
            {embeddedBinaries.length === 0 && loading && (
              <KeyValueSkeleton rows={4} compact />
            )}
            {embeddedBinaries.length === 0 && !loading && (
              <div className="py-6 text-center text-xs text-zinc-600">
                No bundled tools found
              </div>
            )}
          </div>
        </Card>
      </div>
    </div>
  );
};
