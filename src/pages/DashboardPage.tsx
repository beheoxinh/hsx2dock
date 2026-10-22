import React, { useEffect, useRef, useState } from "react";
import {
  Boxes,
  HardDrive,
  Layers,
  Network,
  Server,
  Trash2,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { useToast } from "../components/ui/Toast";
import { useModal } from "../components/ui/ModalContext";
import { api, events } from "../lib/api";
import { useSeries } from "../hooks/useSeries";
import { AreaChart } from "../components/charts/AreaChart";
import { DonutChart } from "../components/charts/DonutChart";
import { cn, formatBytes, formatDuration } from "../lib/utils";
import type {
  ContainerItem,
  ContainerStats,
  DaemonState,
  DfReport,
  EngineInfo,
  ImageItem,
  KindCluster,
  NetworkItem,
  VolumeItem,
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

  const [engineInfo, setEngineInfo] = useState<EngineInfo | null>(null);
  const [diskUsage, setDiskUsage] = useState<DfReport | null>(null);
  const [containers, setContainers] = useState<ContainerItem[]>([]);
  const [images, setImages] = useState<ImageItem[]>([]);
  const [volumes, setVolumes] = useState<VolumeItem[]>([]);
  const [networks, setNetworks] = useState<NetworkItem[]>([]);
  const [clusters, setClusters] = useState<KindCluster[]>([]);
  const [stats, setStats] = useState<Record<string, ContainerStats>>({});
  const [pruning, setPruning] = useState(false);

  // Kind node ids, so their CPU/memory never inflates the user's charts.
  const kindIdsRef = useRef<Set<string>>(new Set());

  const { samples, push, reset } = useSeries<StatSample>(MAX_SAMPLES);

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
    if (!isRunning) {
      setClusters([]);
      return;
    }
    let disposed = false;
    const load = () => {
      api
        .listKindClusters()
        .then((list) => {
          if (!disposed) setClusters(list);
        })
        .catch(() => {
          /* engine went away mid-poll — keep the last known snapshot */
        });
    };
    load();
    const timer = window.setInterval(load, 10_000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [isRunning]);

  const loadData = async () => {
    try {
      const [infoR, dfR, cR, iR, vR, nR] = await Promise.allSettled([
        api.getEngineInfo(),
        api.getDiskUsage(),
        api.listContainers(true),
        api.listImages(),
        api.listVolumes(),
        api.listNetworks(),
      ]);
      if (infoR.status === "fulfilled") setEngineInfo(infoR.value);
      if (dfR.status === "fulfilled") setDiskUsage(dfR.value);
      if (cR.status === "fulfilled") setContainers(cR.value);
      if (iR.status === "fulfilled") setImages(iR.value);
      if (vR.status === "fulfilled") setVolumes(vR.value);
      if (nR.status === "fulfilled") setNetworks(nR.value);
    } catch (e) {
      toast.error("Failed to load dashboard data", String(e));
    }
  };

  useEffect(() => {
    loadData();

    let unlistenContainers: (() => void) | undefined;
    let unlistenStats: (() => void) | undefined;
    let disposed = false;
    let prevNet: Record<string, { rx: number; tx: number }> = {};
    let lastTs = Date.now();

    events.onContainersTick((updated) => {
      setContainers(updated);
    }).then((u) => {
      if (disposed) u();
      else unlistenContainers = u;
    });

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

      setStats(tick);
      push({
        cpu: counted ? cpuSum / counted : 0,
        mem: memSum,
        rxRate,
        txRate,
      });
    }).then((u) => {
      if (disposed) u();
      else unlistenStats = u;
    });

    return () => {
      disposed = true;
      if (unlistenContainers) unlistenContainers();
      if (unlistenStats) unlistenStats();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [daemonState]);

  useEffect(() => {
    if (!isRunning) {
      reset();
      setStats({});
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
      await loadData();
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

  const specRows: [string, string][] = [
    ["Version", engineInfo?.version ?? "—"],
    ["Storage driver", engineInfo?.driver ?? "—"],
    [
      "Platform",
      engineInfo?.os && engineInfo?.arch ? `${engineInfo.os}/${engineInfo.arch}` : "—",
    ],
    ["CPU cores", engineInfo ? String(engineInfo.cpus) : "—"],
    ["Total memory", totalMem ? formatBytes(totalMem) : "—"],
  ];

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 overflow-y-auto p-4">
      {/* KPI strip — every metric appears exactly once */}
      <div className="grid shrink-0 auto-rows-fr grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
        {kpiItems.map((k) => {
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
            <AreaChart
              series={[{ name: "CPU", color: "#22d3ee", data: cpuSeries }]}
              formatValue={(v) => `${v.toFixed(0)}%`}
              yMax={100}
              emptyHint={isRunning ? "Collecting samples…" : "Engine not running"}
              timeHint={`last ${MAX_SAMPLES} samples (≈4 min)`}
            />
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
            <AreaChart
              series={[{ name: "Memory", color: "#a3e635", data: memSeries }]}
              formatValue={(v) => formatBytes(v)}
              yMax={totalMem > 0 ? totalMem : undefined}
              emptyHint={isRunning ? "Collecting samples…" : "Engine not running"}
              timeHint={`last ${MAX_SAMPLES} samples (≈4 min)`}
            />
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
            <AreaChart
              series={[
                { name: "Download", color: "#38bdf8", data: rxSeries },
                { name: "Upload", color: "#fb923c", data: txSeries },
              ]}
              formatValue={(v) => `${formatBytes(v)}/s`}
              emptyHint={isRunning ? "Collecting samples…" : "Engine not running"}
              timeHint={`last ${MAX_SAMPLES} samples (≈4 min)`}
            />
          </div>
        </Card>

        <Card className="flex min-h-[132px] flex-col gap-2 p-3 [@media(max-height:820px)]:min-h-[104px]">
          <div className="flex shrink-0 items-center justify-between gap-2">
            <div className="min-w-0">
              <h3 className="text-sm font-semibold text-white">Kind Clusters</h3>
              <p className="truncate text-[11px] text-zinc-500">
                {clusters.length === 0
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
            {clusters.length === 0 && (
              <div className="flex h-full flex-col items-center justify-center gap-1 text-center">
                <Boxes className="h-7 w-7 text-zinc-600" />
                <p className="text-[11px] text-zinc-500">
                  {isRunning
                    ? "Create a cluster on the Kubernetes page"
                    : "Start the engine to manage clusters"}
                </p>
              </div>
            )}
            {clusters.slice(0, 8).map((c) => (
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
                <span className="shrink-0 text-[10px] tabular-nums text-zinc-400">
                  {c.running
                    ? `${c.running_nodes}/${c.node_count} nodes${
                        c.uptime_seconds != null ? ` · ${formatDuration(c.uptime_seconds)}` : ""
                      }`
                    : "stopped"}
                </span>
              </button>
            ))}
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
                {formatBytes(diskUsage?.total_reclaimable ?? 0)} reclaimable
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
            <DonutChart
              segments={storageSegments}
              total={storageTotal}
              centerLabel="Total"
            />
          </div>
        </Card>

        <Card className="flex min-h-[150px] flex-col gap-2 p-3 [@media(max-height:820px)]:min-h-[120px]">
          <div className="flex shrink-0 items-center justify-between gap-2">
            <h3 className="text-sm font-semibold text-white">Engine</h3>
          </div>
          <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto text-xs">
            {specRows.map(([label, value]) => (
              <div key={label} className="flex items-center justify-between gap-3">
                <span className="text-zinc-400">{label}</span>
                <span className="truncate tabular-nums text-zinc-200">{value}</span>
              </div>
            ))}
          </div>
        </Card>

        <Card className="flex min-h-[150px] flex-col gap-2 p-3 lg:col-span-2 xl:col-span-1 [@media(max-height:820px)]:min-h-[120px]">
          <div className="flex shrink-0 items-center justify-between gap-2">
            <h3 className="text-sm font-semibold text-white">Containers</h3>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onNavigate("containers")}
              className="text-[11px] text-zinc-400 hover:text-zinc-100"
            >
              View all
            </Button>
          </div>
          <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto">
            {appContainers.slice(0, 8).map((c) => {
              const s = stats[c.id];
              return (
                <div
                  key={c.id}
                  className="flex items-center justify-between gap-3 rounded-lg border border-zinc-800/60 bg-zinc-900/40 px-2.5 py-1.5"
                >
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-xs text-zinc-200">
                      {c.name || c.short_id}
                    </div>
                    <div className="truncate text-[10px] text-zinc-500">{c.image}</div>
                  </div>
                  <div className="shrink-0 text-right">
                    <div className="text-[11px] tabular-nums text-zinc-300">
                      {s
                        ? `${(s.cpu_percent ?? 0).toFixed(1)}% · ${formatBytes(s.mem_usage ?? 0)}`
                        : c.state}
                    </div>
                    <div className="text-[10px] text-zinc-500">{c.status}</div>
                  </div>
                </div>
              );
            })}
            {appContainers.length === 0 && (
              <div className="py-6 text-center text-xs text-zinc-600">
                No containers yet — run one from the Containers page.
              </div>
            )}
          </div>
        </Card>
      </div>
    </div>
  );
};
