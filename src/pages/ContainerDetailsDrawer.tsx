import React, { useEffect, useState, useRef } from "react";
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
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { api } from "../lib/api";
import { formatBytes } from "../lib/utils";
import type { ContainerDetails, ContainerStats } from "../types";

interface ContainerDetailsDrawerProps {
  containerId: string | null;
  initialTab?: "logs" | "inspect" | "stats";
  onClose: () => void;
}

export const ContainerDetailsDrawer: React.FC<ContainerDetailsDrawerProps> = ({
  containerId,
  initialTab = "logs",
  onClose,
}) => {
  const [activeTab, setActiveTab] = useState<"logs" | "inspect" | "stats">(initialTab);
  const [details, setDetails] = useState<ContainerDetails | null>(null);
  const [stats, setStats] = useState<ContainerStats | null>(null);
  const [logs, setLogs] = useState<string>("");
  const [, setLoading] = useState(false);
  const [copied, setCopied] = useState(false);
  const [logFilter, setLogFilter] = useState("");
  const logContainerRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab]);

  const loadAll = async () => {
    if (!containerId) return;
    try {
      setLoading(true);
      const [det, log, st] = await Promise.allSettled([
        api.inspectContainer(containerId),
        api.getContainerLogs(containerId, "300"),
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
              <span className="text-xs px-2 py-0.5 rounded-full bg-zinc-800 text-zinc-400 font-mono">
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
          <div className="flex items-center gap-4 px-6 border-b border-zinc-800 text-sm font-medium">
            <button
              onClick={() => setActiveTab("logs")}
              className={`py-3 flex items-center gap-2 border-b-2 transition-all cursor-pointer ${
                activeTab === "logs"
                  ? "border-blue-500 text-blue-400 font-semibold"
                  : "border-transparent text-zinc-400 hover:text-zinc-200"
              }`}
            >
              <FileText className="w-4 h-4" />
              Logs
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
              Live Stats
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
                      placeholder="Filter logs..."
                      value={logFilter}
                      onChange={(e) => setLogFilter(e.target.value)}
                      className="w-full h-8 pl-8 pr-3 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white placeholder-zinc-500 focus:outline-none focus:border-blue-500"
                    />
                  </div>
                  <span className="text-xs text-zinc-500">Showing last 300 lines</span>
                </div>

                <div className="flex-1 bg-zinc-900/90 border border-zinc-800 rounded-xl p-4 overflow-hidden flex flex-col">
                  <pre
                    ref={logContainerRef}
                    className="flex-1 font-mono text-xs text-zinc-300 overflow-y-auto whitespace-pre-wrap select-text leading-relaxed"
                  >
                    {filteredLogs || "No logs available."}
                  </pre>
                </div>
              </div>
            )}

            {activeTab === "stats" && (
              <div className="space-y-6">
                <div className="grid grid-cols-2 gap-4">
                  <div className="bg-zinc-900/60 border border-zinc-800 rounded-xl p-4">
                    <span className="text-xs font-semibold text-zinc-500 uppercase">
                      CPU Usage
                    </span>
                    <div className="text-2xl font-bold text-white mt-1">
                      {stats ? `${stats.cpu_percent.toFixed(2)}%` : "0%"}
                    </div>
                  </div>
                  <div className="bg-zinc-900/60 border border-zinc-800 rounded-xl p-4">
                    <span className="text-xs font-semibold text-zinc-500 uppercase">
                      Memory Usage
                    </span>
                    <div className="text-2xl font-bold text-white mt-1">
                      {stats ? formatBytes(stats.mem_usage) : "0 B"}
                    </div>
                    <span className="text-xs text-zinc-500 mt-1 block">
                      Limit: {stats ? formatBytes(stats.mem_limit) : "0 B"}
                    </span>
                  </div>
                  <div className="bg-zinc-900/60 border border-zinc-800 rounded-xl p-4">
                    <span className="text-xs font-semibold text-zinc-500 uppercase">
                      Network RX
                    </span>
                    <div className="text-2xl font-bold text-white mt-1">
                      {stats ? formatBytes(stats.net_rx) : "0 B"}
                    </div>
                  </div>
                  <div className="bg-zinc-900/60 border border-zinc-800 rounded-xl p-4">
                    <span className="text-xs font-semibold text-zinc-500 uppercase">
                      Network TX
                    </span>
                    <div className="text-2xl font-bold text-white mt-1">
                      {stats ? formatBytes(stats.net_tx) : "0 B"}
                    </div>
                  </div>
                </div>

                <div className="bg-zinc-900/40 border border-zinc-800 rounded-xl p-5 space-y-3 text-xs">
                  <h4 className="font-semibold text-white">Network & Mounts</h4>
                  <div className="flex justify-between py-1 border-b border-zinc-800/40">
                    <span className="text-zinc-500">IP Address</span>
                    <span className="font-mono text-zinc-200">
                      {details?.ip_address || "None"}
                    </span>
                  </div>
                  <div className="py-1">
                    <span className="text-zinc-500 block mb-1">Mounted Volumes:</span>
                    {details?.mounts && details.mounts.length > 0 ? (
                      <ul className="list-disc list-inside space-y-1 font-mono text-zinc-300">
                        {details.mounts.map((m, i) => (
                          <li key={i}>{m}</li>
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
                    Complete Docker Inspect Result
                  </span>
                  <Button variant="outline" size="sm" onClick={handleCopyInspect} className="gap-2">
                    {copied ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
                    <span>{copied ? "Copied JSON" : "Copy JSON"}</span>
                  </Button>
                </div>
                <div className="flex-1 bg-zinc-900/90 border border-zinc-800 rounded-xl p-4 overflow-hidden">
                  <pre className="h-full font-mono text-xs text-blue-300/90 overflow-y-auto whitespace-pre-wrap select-text leading-relaxed">
                    {details?.raw_json || "Loading inspect JSON..."}
                  </pre>
                </div>
              </div>
            )}
          </div>
        </motion.div>
      </div>
    </AnimatePresence>
  );
};
