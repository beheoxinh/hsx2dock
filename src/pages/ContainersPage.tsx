import React, { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Play,
  Square,
  RotateCw,
  Trash2,
  Terminal,
  FileText,
  Search,
  Plus,
  Activity,
  Cpu,
  Layers,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusBadge } from "../components/ui/StatusBadge";
import { useToast } from "../components/ui/Toast";
import { api, events } from "../lib/api";
import { formatBytes, truncateId } from "../lib/utils";
import type { ContainerItem, ContainerStats } from "../types";

interface ContainersPageProps {
  onOpenDetails: (id: string, tab?: "logs" | "inspect" | "stats") => void;
  onOpenWizard: () => void;
}

export const ContainersPage: React.FC<ContainersPageProps> = ({
  onOpenDetails,
  onOpenWizard,
}) => {
  const [containers, setContainers] = useState<ContainerItem[]>([]);
  const [statsMap, setStatsMap] = useState<Record<string, ContainerStats>>({});
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [actionLoading, setActionLoading] = useState<Record<string, boolean>>({});
  const toast = useToast();

  const loadContainers = async () => {
    try {
      setLoading(true);
      const data = await api.listContainers(true);
      setContainers(data);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadContainers();

    let unlistenStats: (() => void) | undefined;
    events.onContainerStatsTick((stats) => {
      setStatsMap((prev) => ({ ...prev, ...stats }));
    }).then((u) => (unlistenStats = u));

    return () => {
      if (unlistenStats) unlistenStats();
    };
  }, []);

  const handleAction = async (c: ContainerItem, action: "start" | "stop" | "restart" | "remove") => {
    const cName = c.name.replace(/^\//, "");
    try {
      setActionLoading((prev) => ({ ...prev, [c.id]: true }));
      if (action === "start") {
        await api.startContainer(c.id);
        toast.success("Container started", `${cName} is now running.`);
      }
      if (action === "stop") {
        await api.stopContainer(c.id);
        toast.info("Container stopped", `${cName} has been stopped.`);
      }
      if (action === "restart") {
        await api.restartContainer(c.id);
        toast.success("Container restarted", `${cName} rebooted successfully.`);
      }
      if (action === "remove") {
        if (!confirm(`Are you sure you want to remove container '${cName}'?`)) return;
        await api.removeContainer(c.id, true);
        toast.success("Container removed", `${cName} was deleted.`);
      }
      await loadContainers();
    } catch (e) {
      toast.error("Operation failed", String(e));
    } finally {
      setActionLoading((prev) => ({ ...prev, [c.id]: false }));
    }
  };

  const filteredContainers = containers.filter(
    (c) =>
      c.name.toLowerCase().includes(search.toLowerCase()) ||
      c.image.toLowerCase().includes(search.toLowerCase()) ||
      c.id.toLowerCase().includes(search.toLowerCase())
  );

  return (
    <div className="p-8 space-y-6 max-w-7xl mx-auto h-full flex flex-col">
      {/* Action Bar */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">Containers</h2>
          <p className="text-sm text-zinc-400 mt-1">
            Manage instances, stream live performance telemetry, and launch shells
          </p>
        </div>

        <div className="flex items-center gap-3">
          <div className="relative w-64">
            <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
            <input
              type="text"
              placeholder="Search containers..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-full h-9 pl-9 pr-4 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white placeholder-zinc-400 focus:outline-none focus:border-blue-500 transition-colors"
            />
          </div>

          <Button variant="primary" size="sm" onClick={onOpenWizard} className="gap-2">
            <Plus className="w-4 h-4" />
            <span>Create Container</span>
          </Button>
        </div>
      </div>

      {/* Containers List / Cards */}
      <div className="flex-1 overflow-y-auto space-y-3 pb-8">
        <AnimatePresence>
          {filteredContainers.map((c) => {
            const isRunning = c.state === "running";
            const stats = statsMap[c.id];
            const isBusy = !!actionLoading[c.id];

            return (
              <motion.div
                key={c.id}
                layout
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, scale: 0.98 }}
                transition={{ duration: 0.2 }}
              >
                <Card className="flex flex-col lg:flex-row lg:items-center justify-between gap-5 p-4 hover:border-zinc-700/80 transition-all">
                  {/* Left: Container Basic Info */}
                  <div className="flex items-start gap-4 min-w-[280px]">
                    <div className="w-10 h-10 rounded-xl bg-zinc-800/80 border border-zinc-700/60 flex items-center justify-center text-zinc-300">
                      <Layers className="w-5 h-5" />
                    </div>
                    <div>
                      <div className="flex items-center gap-2.5">
                        <span className="font-semibold text-white tracking-tight">
                          {c.name.replace(/^\//, "")}
                        </span>
                        <StatusBadge status={c.state} />
                      </div>
                      <div className="text-xs text-zinc-400 mt-1 flex items-center gap-2">
                        <span className="font-mono text-zinc-400">{truncateId(c.id)}</span>
                        <span>•</span>
                        <span className="truncate max-w-[200px]" title={c.image}>
                          {c.image}
                        </span>
                      </div>
                    </div>
                  </div>

                  {/* Middle: Live Stats / Ports */}
                  <div className="flex-1 grid grid-cols-2 sm:grid-cols-3 gap-4 text-xs bg-zinc-950/50 border border-zinc-800/50 rounded-xl p-3">
                    <div>
                      <span className="text-zinc-500 block text-[10px] uppercase font-semibold">
                        Ports
                      </span>
                      <span className="text-zinc-200 truncate block font-mono mt-0.5">
                        {c.ports || "—"}
                      </span>
                    </div>

                    <div>
                      <span className="text-zinc-500 block text-[10px] uppercase font-semibold">
                        CPU
                      </span>
                      <span className="text-zinc-200 flex items-center gap-1 font-mono mt-0.5">
                        <Cpu className="w-3.5 h-3.5 text-blue-400" />
                        {isRunning && stats ? `${stats.cpu_percent.toFixed(1)}%` : "—"}
                      </span>
                    </div>

                    <div>
                      <span className="text-zinc-500 block text-[10px] uppercase font-semibold">
                        Memory
                      </span>
                      <span className="text-zinc-200 flex items-center gap-1 font-mono mt-0.5">
                        <Activity className="w-3.5 h-3.5 text-emerald-400" />
                        {isRunning && stats ? formatBytes(stats.mem_usage) : "—"}
                      </span>
                    </div>
                  </div>

                  {/* Right: Actions */}
                  <div className="flex items-center gap-1.5 self-end lg:self-center">
                    {isRunning ? (
                      <>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => handleAction(c, "stop")}
                          disabled={isBusy}
                          title="Stop Container"
                          className="text-amber-400 hover:text-amber-300 hover:bg-amber-500/10"
                        >
                          <Square className="w-4 h-4 fill-current" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => handleAction(c, "restart")}
                          disabled={isBusy}
                          title="Restart Container"
                          className="text-blue-400 hover:text-blue-300 hover:bg-blue-500/10"
                        >
                          <RotateCw className="w-4 h-4" />
                        </Button>
                      </>
                    ) : (
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => handleAction(c, "start")}
                        disabled={isBusy}
                        title="Start Container"
                        className="text-emerald-400 hover:text-emerald-300 hover:bg-emerald-500/10"
                      >
                        <Play className="w-4 h-4 fill-current" />
                      </Button>
                    )}

                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => onOpenDetails(c.id, "logs")}
                      title="View Logs"
                      className="text-zinc-400 hover:text-white"
                    >
                      <FileText className="w-4 h-4" />
                    </Button>

                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => onOpenDetails(c.id, "inspect")}
                      title="Inspect JSON"
                      className="text-zinc-400 hover:text-white"
                    >
                      <Terminal className="w-4 h-4" />
                    </Button>

                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => handleAction(c, "remove")}
                      disabled={isBusy}
                      title="Delete Container"
                      className="text-zinc-400 hover:text-rose-400 hover:bg-rose-500/10"
                    >
                      <Trash2 className="w-4 h-4" />
                    </Button>
                  </div>
                </Card>
              </motion.div>
            );
          })}
        </AnimatePresence>

        {!loading && filteredContainers.length === 0 && (
          <div className="py-16 text-center text-zinc-400">
            <Layers className="w-10 h-10 mx-auto text-zinc-500 mb-3" />
            <p className="font-medium text-white">No containers matched</p>
            <p className="text-xs text-zinc-400 mt-1">
              {search ? "Try searching with a different keyword" : "Get started by launching a new container image"}
            </p>
          </div>
        )}
      </div>
    </div>
  );
};
