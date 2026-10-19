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
  Pause,
  Copy,
  ExternalLink,
  Bug,
  FileCode2,
  SlidersHorizontal,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusBadge } from "../components/ui/StatusBadge";
import { useToast } from "../components/ui/Toast";
import { useModal } from "../components/ui/ModalContext";
import { ContainerGroupsBar } from "../components/ContainerGroupsBar";
import { api, events } from "../lib/api";
import { formatBytes, truncateId } from "../lib/utils";
import type { ContainerItem, ContainerStats } from "../types";

interface ContainersPageProps {
  onOpenDetails: (
    id: string,
    tab?: "logs" | "inspect" | "stats" | "terminal",
    debug?: boolean,
    edit?: boolean
  ) => void;
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
  const modal = useModal();

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
    let unlistenContainers: (() => void) | undefined;

    events.onContainerStatsTick((stats) => {
      setStatsMap((prev) => ({ ...prev, ...stats }));
    }).then((u) => (unlistenStats = u));

    events.onContainersTick((updatedList) => {
      setContainers(updatedList);
      setLoading(false);
    }).then((u) => (unlistenContainers = u));

    return () => {
      if (unlistenStats) unlistenStats();
      if (unlistenContainers) unlistenContainers();
    };
  }, []);

  const handleAction = async (
    c: ContainerItem,
    action: "start" | "stop" | "restart" | "pause" | "unpause" | "clone" | "remove"
  ) => {
    const cName = c.name.replace(/^\//, "");
    try {
      setActionLoading((prev) => ({ ...prev, [c.id]: true }));
      if (action === "start") {
        await api.startContainer(c.id);
        toast.success("Container started", `${cName} is now running.`);
      } else if (action === "stop") {
        await api.stopContainer(c.id);
        toast.info("Container stopped", `${cName} has been stopped.`);
      } else if (action === "restart") {
        await api.restartContainer(c.id);
        toast.success("Container restarted", `${cName} rebooted successfully.`);
      } else if (action === "pause") {
        await api.pauseContainer(c.id);
        toast.info("Container paused", `${cName} has been paused.`);
      } else if (action === "unpause") {
        await api.unpauseContainer(c.id);
        toast.success("Container unpaused", `${cName} resumed.`);
      } else if (action === "clone") {
        const details = await api.inspectContainer(c.id);
        const newName = `${cName}-clone-${Math.floor(Math.random() * 1000)}`;
        const envVars = (details.env || []).map((e) => {
          const idx = e.indexOf("=");
          return idx !== -1
            ? { key: e.slice(0, idx), value: e.slice(idx + 1) }
            : { key: e, value: "" };
        });
        await api.runContainer({
          image: details.image,
          name: newName,
          env: envVars,
          command: details.command ? details.command.split(" ") : undefined,
          restart_policy: "unless-stopped",
        });
        toast.success("Container cloned", `Created duplicate container: ${newName}`);
      } else if (action === "remove") {
        const confirmed = await modal.showConfirm(`Are you sure you want to remove container '${cName}'?\n\nThis action cannot be undone.`, "Remove Container", "danger");
        if (!confirmed) return;
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

  // Group compose-managed containers under a project header (Docker Desktop
  // style); loose containers keep their original order after their project.
  type Entry =
    | { kind: "project"; project: string; count: number; running: number }
    | { kind: "container"; c: ContainerItem };
  const membersByProject = new Map<string, ContainerItem[]>();
  for (const c of filteredContainers) {
    if (!c.compose_project) continue;
    const arr = membersByProject.get(c.compose_project);
    if (arr) arr.push(c);
    else membersByProject.set(c.compose_project, [c]);
  }
  const entries: Entry[] = [];
  const emitted = new Set<string>();
  for (const c of filteredContainers) {
    const project = c.compose_project;
    if (!project) {
      entries.push({ kind: "container", c });
      continue;
    }
    if (emitted.has(project)) continue;
    emitted.add(project);
    const members = membersByProject.get(project) ?? [];
    entries.push({
      kind: "project",
      project,
      count: members.length,
      running: members.filter((m) => m.state === "running").length,
    });
    for (const m of members) entries.push({ kind: "container", c: m });
  }

  const handleProjectAction = async (
    project: string,
    action: "start" | "stop" | "restart"
  ) => {
    try {
      setActionLoading((prev) => ({ ...prev, [`project:${project}`]: true }));
      const out = await api.composeProjectAction(project, action);
      toast.success(
        `Project ${action}d`,
        out.trim() ? `${project}: ${out.trim()}` : project
      );
      await loadContainers();
    } catch (e) {
      toast.error(`Compose ${action} failed`, String(e));
    } finally {
      setActionLoading((prev) => ({ ...prev, [`project:${project}`]: false }));
    }
  };

  return (
    <div className="p-8 space-y-6 max-w-7xl mx-auto h-full flex flex-col">
      {/* Action Bar */}
      <div className="flex flex-col sm:flex-row sm:flex-wrap sm:items-center justify-between gap-4">
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

      {/* Groups: bulk start/stop + engine auto-start */}
      <ContainerGroupsBar containers={containers} statsMap={statsMap} onChanged={loadContainers} />

      {/* Containers List / Cards */}
      <div className="flex-1 overflow-y-auto space-y-3 pb-8">
        <AnimatePresence>
          {entries.map((entry) => {
            if (entry.kind === "project") {
              const projectBusy = !!actionLoading[`project:${entry.project}`];
              return (
                <div
                  key={`project:${entry.project}`}
                  className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-blue-500/20 bg-blue-500/5 px-4 py-3"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <div className="w-8 h-8 shrink-0 rounded-lg bg-blue-500/10 border border-blue-500/20 flex items-center justify-center text-blue-400">
                      <FileCode2 className="w-4 h-4" />
                    </div>
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-semibold text-white truncate">
                          {entry.project}
                        </span>
                        <span className="text-[10px] uppercase tracking-wider text-blue-400/80">
                          compose
                        </span>
                      </div>
                      <p className="text-xs text-zinc-400">
                        {entry.running}/{entry.count} running
                      </p>
                    </div>
                  </div>
                  <div className="flex items-center gap-1.5 shrink-0">
                    <Button
                      variant="ghost"
                      size="sm"
                      className="gap-1.5 text-xs text-emerald-400 hover:text-emerald-300"
                      onClick={() => handleProjectAction(entry.project, "start")}
                      disabled={projectBusy}
                      title="Start all project containers"
                    >
                      <Play className="w-3.5 h-3.5 fill-current" />
                      Start
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="gap-1.5 text-xs text-rose-400 hover:text-rose-300"
                      onClick={() => handleProjectAction(entry.project, "stop")}
                      disabled={projectBusy}
                      title="Stop all project containers"
                    >
                      <Square className="w-3.5 h-3.5 fill-current" />
                      Stop
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="gap-1.5 text-xs text-amber-400 hover:text-amber-300"
                      onClick={() => handleProjectAction(entry.project, "restart")}
                      disabled={projectBusy}
                      title="Restart all project containers"
                    >
                      <RotateCw className="w-3.5 h-3.5" />
                      Restart
                    </Button>
                  </div>
                </div>
              );
            }
            const c = entry.c;
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
                <Card
                  className="flex flex-col lg:flex-row lg:items-center justify-between gap-5 p-4 hover:border-zinc-700/80 transition-all cursor-grab active:cursor-grabbing"
                  draggable
                  onDragStart={(e) => {
                    e.dataTransfer.setData("text/dm-container-id", c.id);
                    e.dataTransfer.effectAllowed = "copy";
                  }}
                >
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
                      <div className="flex items-center gap-1.5 mt-0.5">
                        <span className="text-zinc-200 truncate block font-mono">
                          {c.ports || "—"}
                        </span>
                        {c.ports && c.ports.includes("->") && (
                          <a
                            href={`http://localhost:${c.ports.split("->")[0].split(":").pop()}`}
                            target="_blank"
                            rel="noreferrer"
                            title="Open in Browser"
                            className="text-blue-400 hover:text-blue-300"
                            onClick={(e) => e.stopPropagation()}
                          >
                            <ExternalLink className="w-3 h-3 inline" />
                          </a>
                        )}
                      </div>
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
                          onClick={() => onOpenDetails(c.id, "terminal")}
                          title="Open Container Terminal / Shell"
                          className="text-emerald-400 hover:text-emerald-300 hover:bg-emerald-500/10"
                        >
                          <Terminal className="w-4 h-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => handleAction(c, "pause")}
                          disabled={isBusy}
                          title="Pause Container"
                          className="text-amber-400 hover:text-amber-300 hover:bg-amber-500/10"
                        >
                          <Pause className="w-4 h-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => handleAction(c, "stop")}
                          disabled={isBusy}
                          title="Stop Container"
                          className="text-rose-400 hover:text-rose-300 hover:bg-rose-500/10"
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
                      <>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => handleAction(c, c.state === "paused" ? "unpause" : "start")}
                          disabled={isBusy}
                          title={c.state === "paused" ? "Unpause Container" : "Start Container"}
                          className="text-emerald-400 hover:text-emerald-300 hover:bg-emerald-500/10"
                        >
                          <Play className="w-4 h-4 fill-current" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => onOpenDetails(c.id, "terminal", true)}
                          disabled={isBusy}
                          title="Debug — start the container and attach a shell"
                          className="text-amber-400 hover:text-amber-300 hover:bg-amber-500/10"
                        >
                          <Bug className="w-4 h-4" />
                        </Button>
                      </>
                    )}

                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => handleAction(c, "clone")}
                      disabled={isBusy}
                      title="Clone / Duplicate Container"
                      className="text-indigo-400 hover:text-indigo-300 hover:bg-indigo-500/10"
                    >
                      <Copy className="w-3.5 h-3.5" />
                    </Button>

                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => onOpenDetails(c.id, "inspect", false, true)}
                      title="Edit container limits & restart policy"
                      aria-label="Edit container limits"
                      className="text-zinc-400 hover:text-white"
                    >
                      <SlidersHorizontal className="w-4 h-4" />
                    </Button>

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
