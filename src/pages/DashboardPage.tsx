import React, { useEffect, useState } from "react";
import { motion } from "framer-motion";
import {
  Box,
  Layers,
  Cpu,
  ArrowUpRight,
  Database,
  Trash2,
  HardDrive,
  Activity,
  PlusCircle,
  Download,
  FolderGit2,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusBadge } from "../components/ui/StatusBadge";
import { api, events } from "../lib/api";
import { formatBytes } from "../lib/utils";
import type {
  ContainerItem,
  EngineInfo,
  DfReport,
  DaemonState,
} from "../types";

interface DashboardPageProps {
  daemonState: DaemonState;
  onNavigate: (tab: any) => void;
}

export const DashboardPage: React.FC<DashboardPageProps> = ({
  daemonState,
  onNavigate,
}) => {
  const [engineInfo, setEngineInfo] = useState<EngineInfo | null>(null);
  const [diskUsage, setDiskUsage] = useState<DfReport | null>(null);
  const [containers, setContainers] = useState<ContainerItem[]>([]);
  const [, setLoading] = useState(true);
  const [pruning, setPruning] = useState(false);

  const loadData = async () => {
    try {
      setLoading(true);
      const [info, df, cList] = await Promise.allSettled([
        api.getEngineInfo(),
        api.getDiskUsage(),
        api.listContainers(true),
      ]);

      if (info.status === "fulfilled") setEngineInfo(info.value);
      if (df.status === "fulfilled") setDiskUsage(df.value);
      if (cList.status === "fulfilled") setContainers(cList.value);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadData();

    let unlistenContainers: (() => void) | undefined;
    events.onContainersTick((updatedList) => {
      setContainers(updatedList);
    }).then((u) => (unlistenContainers = u));

    return () => {
      if (unlistenContainers) unlistenContainers();
    };
  }, [daemonState]);

  const handlePrune = async () => {
    if (!confirm("Are you sure you want to prune unused Docker resources?")) return;
    try {
      setPruning(true);
      await api.pruneSystem();
      await loadData();
    } catch (e) {
      alert("Prune error: " + e);
    } finally {
      setPruning(false);
    }
  };

  const runningCount = containers.filter((c) => c.state === "running").length;

  const cards = [
    {
      title: "Containers",
      value: `${runningCount} / ${containers.length}`,
      subtext: `${runningCount} running currently`,
      icon: Box,
      color: "from-blue-500/20 to-blue-600/5 text-blue-400 border-blue-500/30",
      action: () => onNavigate("containers"),
    },
    {
      title: "Images",
      value: engineInfo ? engineInfo.images : "-",
      subtext: diskUsage ? formatBytes(diskUsage.images_size) : "Storage used",
      icon: Layers,
      color: "from-purple-500/20 to-purple-600/5 text-purple-400 border-purple-500/30",
      action: () => onNavigate("images"),
    },
    {
      title: "Disk Reclaimable",
      value: diskUsage ? formatBytes(diskUsage.total_reclaimable) : "-",
      subtext: "Unused cache & layers",
      icon: Database,
      color: "from-amber-500/20 to-amber-600/5 text-amber-400 border-amber-500/30",
      action: handlePrune,
    },
    {
      title: "System Hardware",
      value: engineInfo ? `${engineInfo.cpus} Cores` : "-",
      subtext: engineInfo ? formatBytes(engineInfo.memory_total) : "Memory",
      icon: Cpu,
      color: "from-emerald-500/20 to-emerald-600/5 text-emerald-400 border-emerald-500/30",
    },
  ];

  return (
    <div className="p-8 space-y-8 max-w-7xl mx-auto overflow-y-auto h-full">
      {/* Page Title */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">System Dashboard</h2>
          <p className="text-sm text-zinc-400 mt-1">
            Realtime overview of containers, volumes, and engine resource consumption
          </p>
        </div>
        <div className="flex gap-3">
          <Button
            variant="outline"
            size="sm"
            onClick={handlePrune}
            isLoading={pruning}
            className="gap-2"
          >
            <Trash2 className="w-3.5 h-3.5 text-rose-400" />
            <span>Prune System</span>
          </Button>
        </div>
      </div>

      {/* Metrics Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-5">
        {cards.map((c, i) => {
          const Icon = c.icon;
          return (
            <motion.div
              key={c.title}
              initial={{ opacity: 0, y: 15 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: i * 0.05, duration: 0.3 }}
            >
              <Card
                className={`relative overflow-hidden cursor-pointer group hover:scale-[1.02] bg-gradient-to-br ${c.color}`}
                onClick={c.action}
              >
                <div className="flex items-center justify-between">
                  <span className="text-xs font-semibold uppercase tracking-wider text-zinc-400">
                    {c.title}
                  </span>
                  <div className="p-2 rounded-xl bg-zinc-950/40 border border-zinc-800/80">
                    <Icon className="w-4 h-4" />
                  </div>
                </div>
                <div className="mt-4">
                  <div className="text-3xl font-extrabold tracking-tight text-white">
                    {c.value}
                  </div>
                  <div className="text-xs text-zinc-400 mt-1 flex items-center justify-between">
                    <span>{c.subtext}</span>
                    <ArrowUpRight className="w-3.5 h-3.5 opacity-0 group-hover:opacity-100 transition-opacity" />
                  </div>
                </div>
              </Card>
            </motion.div>
          );
        })}
      </div>

      {/* Recent Containers and Engine Info */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Containers Table Summary */}
        <Card className="lg:col-span-2">
          <div className="flex items-center justify-between pb-4 border-b border-zinc-800/80">
            <div>
              <h3 className="font-semibold text-white">Active Containers</h3>
              <p className="text-xs text-zinc-400">Quick view of running instances</p>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onNavigate("containers")}
              className="text-xs gap-1"
            >
              View all
              <ArrowUpRight className="w-3.5 h-3.5" />
            </Button>
          </div>

          <div className="divide-y divide-zinc-800/40 mt-2">
            {containers.slice(0, 5).map((container) => (
              <div
                key={container.id}
                className="py-3 flex items-center justify-between group hover:bg-zinc-800/30 px-3 rounded-xl transition-colors cursor-pointer"
                onClick={() => onNavigate("containers")}
              >
                <div className="flex items-center gap-3">
                  <div className="w-8 h-8 rounded-lg bg-zinc-800 flex items-center justify-center text-zinc-400">
                    <Box className="w-4 h-4" />
                  </div>
                  <div>
                    <div className="font-medium text-sm text-white group-hover:text-blue-400 transition-colors">
                      {container.name.replace(/^\//, "")}
                    </div>
                    <div className="text-xs text-zinc-400">{container.image}</div>
                  </div>
                </div>
                <div className="flex items-center gap-4">
                  <span className="text-xs text-zinc-400 hidden sm:inline-block">
                    {container.ports || "No ports exposed"}
                  </span>
                  <StatusBadge status={container.state} />
                </div>
              </div>
            ))}

            {containers.length === 0 && (
              <div className="py-8 text-center text-zinc-400 text-sm">
                No containers found. Create or run a new container to see it here.
              </div>
            )}
          </div>
        </Card>

        {/* Engine Specification */}
        <Card>
          <div className="pb-4 border-b border-zinc-800/80">
            <h3 className="font-semibold text-white">Engine Specifications</h3>
            <p className="text-xs text-zinc-400">Environment and platform info</p>
          </div>
          <div className="space-y-3 mt-4 text-xs">
            <div className="flex justify-between py-1.5 border-b border-zinc-800/40">
              <span className="text-zinc-400">Engine Version</span>
              <span className="font-medium text-zinc-200">{engineInfo?.version || "-"}</span>
            </div>
            <div className="flex justify-between py-1.5 border-b border-zinc-800/40">
              <span className="text-zinc-400">Storage Driver</span>
              <span className="font-medium text-zinc-200">{engineInfo?.driver || "-"}</span>
            </div>
            <div className="flex justify-between py-1.5 border-b border-zinc-800/40">
              <span className="text-zinc-400">Operating System</span>
              <span className="font-medium text-zinc-200">{engineInfo?.os || "-"}</span>
            </div>
            <div className="flex justify-between py-1.5 border-b border-zinc-800/40">
              <span className="text-zinc-400">Architecture</span>
              <span className="font-medium text-zinc-200">{engineInfo?.arch || "-"}</span>
            </div>
            <div className="flex justify-between py-1.5 border-b border-zinc-800/40">
              <span className="text-zinc-400">Total Containers</span>
              <span className="font-medium text-zinc-200">{engineInfo?.containers || 0}</span>
            </div>
            <div className="flex justify-between py-1.5 border-b border-zinc-800/40">
              <span className="text-zinc-400">Running / Paused</span>
              <span className="font-medium text-emerald-400">
                {engineInfo?.containers_running || 0} / {engineInfo?.containers_paused || 0}
              </span>
            </div>
          </div>
        </Card>
      </div>

      {/* Storage Breakdown & Quick Actions */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Storage Breakdown Bar */}
        <Card className="lg:col-span-2 space-y-4">
          <div className="flex items-center justify-between pb-3 border-b border-zinc-800/80">
            <div className="flex items-center gap-2.5">
              <HardDrive className="w-4 h-4 text-emerald-400" />
              <h3 className="font-semibold text-white">Docker Storage Utilization</h3>
            </div>
            <span className="text-xs text-zinc-400">
              Total: {formatBytes(
                (diskUsage?.images_size || 0) +
                (diskUsage?.containers_size || 0) +
                (diskUsage?.volumes_size || 0) +
                (diskUsage?.build_cache_size || 0)
              )}
            </span>
          </div>

          <div className="space-y-3">
            <div>
              <div className="flex justify-between text-xs mb-1">
                <span className="text-zinc-400">Images ({formatBytes(diskUsage?.images_size)})</span>
                <span className="text-purple-400 font-mono">
                  {diskUsage?.images_size ? formatBytes(diskUsage.images_size) : "0 B"}
                </span>
              </div>
              <div className="h-2 w-full bg-zinc-800 rounded-full overflow-hidden">
                <div
                  className="h-full bg-purple-500 rounded-full"
                  style={{
                    width: `${Math.min(
                      100,
                      Math.max(
                        5,
                        ((diskUsage?.images_size || 0) /
                          Math.max(
                            1,
                            (diskUsage?.images_size || 0) +
                              (diskUsage?.containers_size || 0) +
                              (diskUsage?.volumes_size || 0) +
                              (diskUsage?.build_cache_size || 0)
                          )) * 100
                      )
                    )}%`,
                  }}
                />
              </div>
            </div>

            <div>
              <div className="flex justify-between text-xs mb-1">
                <span className="text-zinc-400">Containers Writable Layer ({formatBytes(diskUsage?.containers_size)})</span>
                <span className="text-blue-400 font-mono">
                  {diskUsage?.containers_size ? formatBytes(diskUsage.containers_size) : "0 B"}
                </span>
              </div>
              <div className="h-2 w-full bg-zinc-800 rounded-full overflow-hidden">
                <div
                  className="h-full bg-blue-500 rounded-full"
                  style={{
                    width: `${Math.min(
                      100,
                      Math.max(
                        3,
                        ((diskUsage?.containers_size || 0) /
                          Math.max(
                            1,
                            (diskUsage?.images_size || 0) +
                              (diskUsage?.containers_size || 0) +
                              (diskUsage?.volumes_size || 0) +
                              (diskUsage?.build_cache_size || 0)
                          )) * 100
                      )
                    )}%`,
                  }}
                />
              </div>
            </div>

            <div>
              <div className="flex justify-between text-xs mb-1">
                <span className="text-zinc-400">Volumes ({formatBytes(diskUsage?.volumes_size)})</span>
                <span className="text-amber-400 font-mono">
                  {diskUsage?.volumes_size ? formatBytes(diskUsage.volumes_size) : "0 B"}
                </span>
              </div>
              <div className="h-2 w-full bg-zinc-800 rounded-full overflow-hidden">
                <div
                  className="h-full bg-amber-500 rounded-full"
                  style={{
                    width: `${Math.min(
                      100,
                      Math.max(
                        3,
                        ((diskUsage?.volumes_size || 0) /
                          Math.max(
                            1,
                            (diskUsage?.images_size || 0) +
                              (diskUsage?.containers_size || 0) +
                              (diskUsage?.volumes_size || 0) +
                              (diskUsage?.build_cache_size || 0)
                          )) * 100
                      )
                    )}%`,
                  }}
                />
              </div>
            </div>

            <div>
              <div className="flex justify-between text-xs mb-1">
                <span className="text-zinc-400">Build Cache ({formatBytes(diskUsage?.build_cache_size)})</span>
                <span className="text-emerald-400 font-mono">
                  {diskUsage?.build_cache_size ? formatBytes(diskUsage.build_cache_size) : "0 B"}
                </span>
              </div>
              <div className="h-2 w-full bg-zinc-800 rounded-full overflow-hidden">
                <div
                  className="h-full bg-emerald-500 rounded-full"
                  style={{
                    width: `${Math.min(
                      100,
                      Math.max(
                        2,
                        ((diskUsage?.build_cache_size || 0) /
                          Math.max(
                            1,
                            (diskUsage?.images_size || 0) +
                              (diskUsage?.containers_size || 0) +
                              (diskUsage?.volumes_size || 0) +
                              (diskUsage?.build_cache_size || 0)
                          )) * 100
                      )
                    )}%`,
                  }}
                />
              </div>
            </div>
          </div>
        </Card>

        {/* Quick Actions Shortcuts */}
        <Card className="space-y-3">
          <div className="pb-3 border-b border-zinc-800/80 flex items-center gap-2">
            <Activity className="w-4 h-4 text-blue-400" />
            <h3 className="font-semibold text-white">Quick Shortcuts</h3>
          </div>
          <div className="grid grid-cols-1 gap-2.5 pt-1">
            <button
              onClick={() => onNavigate("containers")}
              className="flex items-center justify-between p-3 rounded-xl bg-zinc-800/40 hover:bg-zinc-800/80 border border-zinc-800/80 transition-colors text-left group"
            >
              <div className="flex items-center gap-3">
                <PlusCircle className="w-4 h-4 text-blue-400" />
                <span className="text-xs font-medium text-zinc-200">Create Container</span>
              </div>
              <ArrowUpRight className="w-3.5 h-3.5 text-zinc-500 group-hover:text-white transition-colors" />
            </button>

            <button
              onClick={() => onNavigate("images")}
              className="flex items-center justify-between p-3 rounded-xl bg-zinc-800/40 hover:bg-zinc-800/80 border border-zinc-800/80 transition-colors text-left group"
            >
              <div className="flex items-center gap-3">
                <Download className="w-4 h-4 text-purple-400" />
                <span className="text-xs font-medium text-zinc-200">Pull Image from Hub</span>
              </div>
              <ArrowUpRight className="w-3.5 h-3.5 text-zinc-500 group-hover:text-white transition-colors" />
            </button>

            <button
              onClick={() => onNavigate("compose")}
              className="flex items-center justify-between p-3 rounded-xl bg-zinc-800/40 hover:bg-zinc-800/80 border border-zinc-800/80 transition-colors text-left group"
            >
              <div className="flex items-center gap-3">
                <FolderGit2 className="w-4 h-4 text-indigo-400" />
                <span className="text-xs font-medium text-zinc-200">Deploy Compose Project</span>
              </div>
              <ArrowUpRight className="w-3.5 h-3.5 text-zinc-500 group-hover:text-white transition-colors" />
            </button>
          </div>
        </Card>
      </div>
    </div>
  );
};
