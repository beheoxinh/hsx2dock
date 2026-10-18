import React, { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  X,
  FileText,
  Activity,
  Terminal as TerminalIcon,
  Search,
  Copy,
  Check,
  RotateCw,
  Pencil,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { TerminalViewer } from "../components/ui/TerminalViewer";
import { InteractiveTerminal } from "../components/terminal/InteractiveTerminal";
import { useTerminalFontSize } from "../lib/terminalSettings";
import { useToast } from "../components/ui/Toast";
import { api } from "../lib/api";
import { formatBytes } from "../lib/utils";
import type { ContainerDetails, ContainerStats, ResourceLimits } from "../types";

interface ContainerDetailsDrawerProps {
  containerId: string | null;
  initialTab?: "logs" | "inspect" | "stats" | "terminal";
  /** true = the Debug flow: start the container if needed, then attach a shell. */
  debug?: boolean;
  onClose: () => void;
}

export const ContainerDetailsDrawer: React.FC<ContainerDetailsDrawerProps> = ({
  containerId,
  initialTab = "logs",
  debug = false,
  onClose,
}) => {
  const [activeTab, setActiveTab] = useState<"logs" | "inspect" | "stats" | "terminal">(initialTab);
  const [details, setDetails] = useState<ContainerDetails | null>(null);
  const [stats, setStats] = useState<ContainerStats | null>(null);
  const [logs, setLogs] = useState<string>("");
  const [, setLoading] = useState(false);
  const [copied, setCopied] = useState(false);
  const [logFilter, setLogFilter] = useState("");
  const [limitsDraft, setLimitsDraft] = useState<ResourceLimits | null>(null);
  const [savingLimits, setSavingLimits] = useState(false);
  const toast = useToast();
  const terminalFontSize = useTerminalFontSize();
  const isRunning = details?.state === "running";

  const handleSaveLimits = async () => {
    if (!containerId || !limitsDraft) return;
    try {
      setSavingLimits(true);
      await api.updateContainerResources(containerId, {
        memory_mb: limitsDraft.memory_mb?.trim() || "",
        cpus: limitsDraft.cpus?.trim() || "",
        cpuset_cpus: limitsDraft.cpuset_cpus?.trim() || "",
        cpu_shares: limitsDraft.cpu_shares?.trim() || "",
        pids_limit: limitsDraft.pids_limit?.trim() || "",
      });
      toast.success("Resource limits updated");
      setLimitsDraft(null);
      await loadAll();
    } catch (err) {
      toast.error("Could not update limits", String(err));
    } finally {
      setSavingLimits(false);
    }
  };

  useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab]);

  const loadAll = async () => {
    if (!containerId) return;
    try {
      setLoading(true);
      const [det, log, st] = await Promise.allSettled([
        api.inspectContainer(containerId),
        api.getContainerLogs(containerId, "400"),
        api.getContainerStats(containerId),
      ]);

      if (det.status === "fulfilled") setDetails(det.value);
      if (log.status === "fulfilled") setLogs(log.value);
      if (st.status === "fulfilled") setStats(st.value);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (containerId) {
      loadAll();
    }
  }, [containerId]);

  const handleCopyInspect = () => {
    if (!details?.raw_json) return;
    navigator.clipboard.writeText(details.raw_json);
    setCopied(true);
    toast.success("Copied to clipboard", "Raw container inspect JSON copied.");
    setTimeout(() => setCopied(false), 2000);
  };

  if (!containerId) return null;

  const filteredLogs = logFilter
    ? logs
        .split("\n")
        .filter((l) => l.toLowerCase().includes(logFilter.toLowerCase()))
        .join("\n")
    : logs;

  return (
    <AnimatePresence>
      <div className="fixed inset-0 z-50 flex justify-end">
        {/* Backdrop */}
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          className="fixed inset-0 bg-black/60 backdrop-blur-sm"
          onClick={onClose}
        />

        {/* Slide-over Drawer */}
        <motion.div
          initial={{ x: "100%" }}
          animate={{ x: 0 }}
          exit={{ x: "100%" }}
          transition={{ type: "spring", damping: 28, stiffness: 260 }}
          className="relative w-full max-w-3xl bg-zinc-950 border-l border-zinc-800/80 h-full flex flex-col shadow-2xl z-10"
        >
          {/* Header */}
          <div className="h-16 px-6 border-b border-zinc-800 flex items-center justify-between">
            <div className="flex items-center gap-3">
              <span className="font-semibold text-white tracking-tight">
                {details?.name.replace(/^\//, "") || containerId.slice(0, 12)}
              </span>
              <span className="text-xs px-2.5 py-1 rounded-full bg-zinc-800/80 text-zinc-400 font-mono border border-zinc-700/60">
                {containerId.slice(0, 12)}
              </span>
            </div>

            <div className="flex items-center gap-2">
              <Button variant="ghost" size="icon" onClick={loadAll} title="Reload Data">
                <RotateCw className="w-4 h-4" />
              </Button>
              <Button variant="ghost" size="icon" onClick={onClose}>
                <X className="w-5 h-5" />
              </Button>
            </div>
          </div>

          {/* Tab Navigation */}
          <div className="flex items-center gap-6 px-6 border-b border-zinc-800 text-sm font-medium">
            <button
              onClick={() => setActiveTab("logs")}
              className={`py-3 flex items-center gap-2 border-b-2 transition-all cursor-pointer ${
                activeTab === "logs"
                  ? "border-blue-500 text-blue-400 font-semibold"
                  : "border-transparent text-zinc-400 hover:text-zinc-200"
              }`}
            >
              <FileText className="w-4 h-4" />
              Logs Stream
            </button>
            <button
              onClick={() => setActiveTab("stats")}
              className={`py-3 flex items-center gap-2 border-b-2 transition-all cursor-pointer ${
                activeTab === "stats"
                  ? "border-blue-500 text-blue-400 font-semibold"
                  : "border-transparent text-zinc-400 hover:text-zinc-200"
              }`}
            >
              <Activity className="w-4 h-4" />
              Live Telemetry
            </button>
            <button
              onClick={() => setActiveTab("inspect")}
              className={`py-3 flex items-center gap-2 border-b-2 transition-all cursor-pointer ${
                activeTab === "inspect"
                  ? "border-blue-500 text-blue-400 font-semibold"
                  : "border-transparent text-zinc-400 hover:text-zinc-200"
              }`}
            >
              <TerminalIcon className="w-4 h-4" />
              Inspect JSON
            </button>
            <button
              onClick={() => setActiveTab("terminal")}
              className={`py-3 flex items-center gap-2 border-b-2 transition-all cursor-pointer ${
                activeTab === "terminal"
                  ? "border-emerald-500 text-emerald-400 font-semibold"
                  : "border-transparent text-zinc-400 hover:text-zinc-200"
              }`}
            >
              <TerminalIcon className="w-4 h-4 text-emerald-400" />
              Exec Terminal
            </button>
          </div>

          {/* Drawer Body */}
          <div className="flex-1 overflow-y-auto p-6">
            {activeTab === "logs" && (
              <div className="h-full flex flex-col space-y-3">
                <div className="flex items-center justify-between">
                  <div className="relative w-64">
                    <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-500" />
                    <input
                      type="text"
                      placeholder="Search log lines..."
                      value={logFilter}
                      onChange={(e) => setLogFilter(e.target.value)}
                      className="w-full h-8 pl-8 pr-3 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white placeholder-zinc-500 focus:outline-none focus:border-blue-500"
                    />
                  </div>
                  <span className="text-xs text-zinc-500 font-mono">ANSI Interactive Shell</span>
                </div>

                <div className="flex-1 bg-zinc-950 border border-zinc-800 rounded-2xl p-3 overflow-hidden shadow-inner flex flex-col">
                  <TerminalViewer logs={filteredLogs || "No logs output recorded."} />
                </div>
              </div>
            )}

            {activeTab === "stats" && (
              <div className="space-y-6">
                <div className="grid grid-cols-2 gap-4">
                  <div className="bg-zinc-900/60 border border-zinc-800 rounded-2xl p-5 shadow-lg">
                    <span className="text-xs font-semibold text-zinc-500 uppercase tracking-wider">
                      CPU Compute
                    </span>
                    <div className="text-3xl font-extrabold text-blue-400 mt-2 font-mono">
                      {stats ? `${stats.cpu_percent.toFixed(2)}%` : "0%"}
                    </div>
                  </div>
                  <div className="bg-zinc-900/60 border border-zinc-800 rounded-2xl p-5 shadow-lg">
                    <span className="text-xs font-semibold text-zinc-500 uppercase tracking-wider">
                      Memory Allocated
                    </span>
                    <div className="text-3xl font-extrabold text-emerald-400 mt-2 font-mono">
                      {stats ? formatBytes(stats.mem_usage) : "0 B"}
                    </div>
                    <span className="text-xs text-zinc-500 mt-1 block">
                      Limit: {stats ? formatBytes(stats.mem_limit) : "Unlimited"}
                    </span>
                  </div>
                  <div className="bg-zinc-900/60 border border-zinc-800 rounded-2xl p-5 shadow-lg">
                    <span className="text-xs font-semibold text-zinc-500 uppercase tracking-wider">
                      Network Inbound (RX)
                    </span>
                    <div className="text-3xl font-extrabold text-zinc-200 mt-2 font-mono">
                      {stats ? formatBytes(stats.net_rx) : "0 B"}
                    </div>
                  </div>
                  <div className="bg-zinc-900/60 border border-zinc-800 rounded-2xl p-5 shadow-lg">
                    <span className="text-xs font-semibold text-zinc-500 uppercase tracking-wider">
                      Network Outbound (TX)
                    </span>
                    <div className="text-3xl font-extrabold text-zinc-200 mt-2 font-mono">
                      {stats ? formatBytes(stats.net_tx) : "0 B"}
                    </div>
                  </div>
                </div>

                <div className="bg-zinc-900/40 border border-zinc-800 rounded-2xl p-6 space-y-4 text-xs shadow-lg">
                  <h4 className="font-semibold text-white text-sm">Network Configuration & Mounts</h4>
                  <div className="flex justify-between py-2 border-b border-zinc-800/40">
                    <span className="text-zinc-500">Internal IPv4</span>
                    <span className="font-mono text-zinc-200">
                      {details?.ip_address || "None"}
                    </span>
                  </div>
                  <div className="py-2">
                    <span className="text-zinc-500 block mb-2 font-medium">Mountpoints:</span>
                    {details?.mounts && details.mounts.length > 0 ? (
                      <ul className="space-y-1.5 font-mono text-zinc-300">
                        {details.mounts.map((m, i) => (
                          <li
                            key={i}
                            className="bg-zinc-950/60 px-3 py-1.5 rounded-lg border border-zinc-800/50"
                          >
                            {m}
                          </li>
                        ))}
                      </ul>
                    ) : (
                      <span className="text-zinc-600">No mounted volumes</span>
                    )}
                  </div>
                </div>
              </div>
            )}

            {activeTab === "inspect" && (
              <div className="h-full flex flex-col space-y-3">
                <div className="flex items-center justify-between">
                  <span className="text-xs text-zinc-500 font-mono">
                    Complete Docker Container Inspection
                  </span>
                  <Button variant="outline" size="sm" onClick={handleCopyInspect} className="gap-2">
                    {copied ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
                    <span>{copied ? "Copied" : "Copy JSON"}</span>
                  </Button>
                </div>

                {/* Resource limits snapshot + inline editor */}
                <div className="rounded-xl border border-zinc-800 bg-zinc-900/50 p-3">
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-semibold text-zinc-300">
                      Resource limits
                    </span>
                    {!limitsDraft && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() =>
                          setLimitsDraft(
                            details?.limits ?? {
                              memory_mb: "",
                              cpus: "",
                              cpuset_cpus: "",
                              cpu_shares: "",
                              pids_limit: "",
                            }
                          )
                        }
                        className="gap-1.5 text-xs text-zinc-400 hover:text-white"
                      >
                        <Pencil className="h-3.5 w-3.5" />
                        Edit
                      </Button>
                    )}
                  </div>

                  {!limitsDraft && (
                    <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1.5 font-mono text-[11px] sm:grid-cols-3">
                      <div className="flex justify-between gap-2">
                        <dt className="text-zinc-500">Memory</dt>
                        <dd className="text-zinc-200">
                          {details?.limits?.memory_mb
                            ? `${details.limits.memory_mb} MiB`
                            : "unlimited"}
                        </dd>
                      </div>
                      <div className="flex justify-between gap-2">
                        <dt className="text-zinc-500">CPU</dt>
                        <dd className="text-zinc-200">
                          {details?.limits?.cpus
                            ? `${details.limits.cpus} cores`
                            : "unlimited"}
                        </dd>
                      </div>
                      <div className="flex justify-between gap-2">
                        <dt className="text-zinc-500">CPU set</dt>
                        <dd className="text-zinc-200">
                          {details?.limits?.cpuset_cpus || "all"}
                        </dd>
                      </div>
                      <div className="flex justify-between gap-2">
                        <dt className="text-zinc-500">Shares</dt>
                        <dd className="text-zinc-200">
                          {details?.limits?.cpu_shares || "default"}
                        </dd>
                      </div>
                      <div className="flex justify-between gap-2">
                        <dt className="text-zinc-500">PIDs</dt>
                        <dd className="text-zinc-200">
                          {details?.limits?.pids_limit || "unlimited"}
                        </dd>
                      </div>
                    </dl>
                  )}

                  {limitsDraft && (
                    <div className="mt-2 space-y-2">
                      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                        <label className="space-y-1">
                          <span className="text-[11px] text-zinc-500">Memory (MiB)</span>
                          <input
                            type="text"
                            value={limitsDraft.memory_mb ?? ""}
                            onChange={(e) =>
                              setLimitsDraft({ ...limitsDraft, memory_mb: e.target.value })
                            }
                            placeholder="512"
                            className="h-8 w-full rounded-md border border-zinc-800 bg-zinc-950/60 px-2 font-mono text-xs text-white outline-none focus:border-zinc-600 placeholder:text-zinc-700"
                          />
                        </label>
                        <label className="space-y-1">
                          <span className="text-[11px] text-zinc-500">CPU (cores)</span>
                          <input
                            type="text"
                            value={limitsDraft.cpus ?? ""}
                            onChange={(e) =>
                              setLimitsDraft({ ...limitsDraft, cpus: e.target.value })
                            }
                            placeholder="1.5"
                            className="h-8 w-full rounded-md border border-zinc-800 bg-zinc-950/60 px-2 font-mono text-xs text-white outline-none focus:border-zinc-600 placeholder:text-zinc-700"
                          />
                        </label>
                        <label className="space-y-1">
                          <span className="text-[11px] text-zinc-500">CPU set</span>
                          <input
                            type="text"
                            value={limitsDraft.cpuset_cpus ?? ""}
                            onChange={(e) =>
                              setLimitsDraft({ ...limitsDraft, cpuset_cpus: e.target.value })
                            }
                            placeholder="0-2,4"
                            className="h-8 w-full rounded-md border border-zinc-800 bg-zinc-950/60 px-2 font-mono text-xs text-white outline-none focus:border-zinc-600 placeholder:text-zinc-700"
                          />
                        </label>
                        <label className="space-y-1">
                          <span className="text-[11px] text-zinc-500">CPU shares</span>
                          <input
                            type="text"
                            value={limitsDraft.cpu_shares ?? ""}
                            onChange={(e) =>
                              setLimitsDraft({ ...limitsDraft, cpu_shares: e.target.value })
                            }
                            placeholder="1024"
                            className="h-8 w-full rounded-md border border-zinc-800 bg-zinc-950/60 px-2 font-mono text-xs text-white outline-none focus:border-zinc-600 placeholder:text-zinc-700"
                          />
                        </label>
                        <label className="space-y-1">
                          <span className="text-[11px] text-zinc-500">PIDs limit</span>
                          <input
                            type="text"
                            value={limitsDraft.pids_limit ?? ""}
                            onChange={(e) =>
                              setLimitsDraft({ ...limitsDraft, pids_limit: e.target.value })
                            }
                            placeholder="256"
                            className="h-8 w-full rounded-md border border-zinc-800 bg-zinc-950/60 px-2 font-mono text-xs text-white outline-none focus:border-zinc-600 placeholder:text-zinc-700"
                          />
                        </label>
                      </div>
                      <p className="text-[11px] text-zinc-600">
                        Leaving a field empty keeps its current value — the engine can
                        change a limit but cannot remove one.
                      </p>
                      <div className="flex justify-end gap-2">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setLimitsDraft(null)}
                          disabled={savingLimits}
                        >
                          Cancel
                        </Button>
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={handleSaveLimits}
                          disabled={savingLimits}
                        >
                          {savingLimits ? "Saving…" : "Save"}
                        </Button>
                      </div>
                    </div>
                  )}
                </div>

                <div className="flex-1 bg-zinc-950 border border-zinc-800 rounded-2xl p-4 overflow-hidden shadow-inner">
                  <pre className="h-full font-mono text-xs text-blue-300/90 overflow-y-auto whitespace-pre-wrap select-text leading-relaxed">
                    {details?.raw_json || "Loading inspect JSON..."}
                  </pre>
                </div>
              </div>
            )}

            {activeTab === "terminal" && (
              <div className="h-full flex flex-col space-y-3 min-h-0">
                <div className="flex items-center justify-between">
                  <span className="text-xs text-zinc-500 font-mono">
                    {isRunning
                      ? "Interactive shell — stdin is live"
                      : debug
                        ? "Debug — starting container & attaching…"
                        : "Container is not running"}
                  </span>
                  {isRunning && (
                    <span className="text-[10px] text-zinc-600 font-mono">
                      type to interact · exit ends the session
                    </span>
                  )}
                </div>

                <div className="flex-1 bg-zinc-950 border border-zinc-800 rounded-2xl p-3 overflow-hidden shadow-inner flex flex-col min-h-0">
                  {isRunning || debug ? (
                    <InteractiveTerminal
                      key={containerId + (debug ? ":debug" : "")}
                      containerId={containerId}
                      debug={debug}
                      fontSize={terminalFontSize}
                      onSessionLive={() => {
                        // the Debug flow may have just started the container
                        void loadAll();
                      }}
                    />
                  ) : (
                    <div className="h-full flex flex-col items-center justify-center gap-3 text-center px-6">
                      <p className="text-sm text-zinc-400">
                        This container is{" "}
                        <span className="font-semibold">
                          {details?.state || "not running"}
                        </span>
                        .
                      </p>
                      <p className="text-xs text-zinc-600 max-w-sm">
                        A shell needs a running container. Use the Debug action on
                        the container row to start it and attach a shell
                        automatically.
                      </p>
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>
        </motion.div>
      </div>
    </AnimatePresence>
  );
};
