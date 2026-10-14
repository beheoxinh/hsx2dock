import React, { useEffect, useState } from "react";
import { motion } from "framer-motion";
import {
  Box,
  Layers,
  Cpu,
  ArrowUpRight,
  Database,
  Trash2,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusBadge } from "../components/ui/StatusBadge";
import { api } from "../lib/api";
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
          </div>
        </Card>
      </div>
    </div>
  );
};
