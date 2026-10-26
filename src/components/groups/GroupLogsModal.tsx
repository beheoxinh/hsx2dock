import React, { useState, useEffect, useRef, useMemo, useCallback } from "react";
import {
  X,
  FileText,
  Search,
  Copy,
  Check,
  RefreshCw,
  Pause,
  Play,
  Filter,
  Layers,
  ArrowDown,
} from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import type { ContainerItem } from "../../types";

interface GroupLogsModalProps {
  isOpen: boolean;
  onClose: () => void;
  groupName: string;
  containers: ContainerItem[];
}

interface LogLine {
  id: string;
  containerId: string;
  containerName: string;
  color: string;
  timestamp: string; // parsed ISO timestamp or raw line timestamp
  rawTime?: number;
  message: string;
}

// Deterministic distinct badge colors for container tags & log line prefixes
const TAG_COLORS = [
  {
    bg: "bg-sky-500/15",
    text: "text-sky-300",
    border: "border-sky-500/30",
    badge: "bg-sky-500 text-slate-950 font-bold",
    hex: "#38bdf8",
  },
  {
    bg: "bg-emerald-500/15",
    text: "text-emerald-300",
    border: "border-emerald-500/30",
    badge: "bg-emerald-500 text-slate-950 font-bold",
    hex: "#34d399",
  },
  {
    bg: "bg-violet-500/15",
    text: "text-violet-300",
    border: "border-violet-500/30",
    badge: "bg-violet-500 text-slate-950 font-bold",
    hex: "#a78bfa",
  },
  {
    bg: "bg-amber-500/15",
    text: "text-amber-300",
    border: "border-amber-500/30",
    badge: "bg-amber-500 text-slate-950 font-bold",
    hex: "#fbbf24",
  },
  {
    bg: "bg-rose-500/15",
    text: "text-rose-300",
    border: "border-rose-500/30",
    badge: "bg-rose-500 text-slate-950 font-bold",
    hex: "#fb7185",
  },
  {
    bg: "bg-teal-500/15",
    text: "text-teal-300",
    border: "border-teal-500/30",
    badge: "bg-teal-500 text-slate-950 font-bold",
    hex: "#2dd4bf",
  },
  {
    bg: "bg-fuchsia-500/15",
    text: "text-fuchsia-300",
    border: "border-fuchsia-500/30",
    badge: "bg-fuchsia-500 text-slate-950 font-bold",
    hex: "#e879f9",
  },
  {
    bg: "bg-indigo-500/15",
    text: "text-indigo-300",
    border: "border-indigo-500/30",
    badge: "bg-indigo-500 text-slate-950 font-bold",
    hex: "#818cf8",
  },
  {
    bg: "bg-orange-500/15",
    text: "text-orange-300",
    border: "border-orange-500/30",
    badge: "bg-orange-500 text-slate-950 font-bold",
    hex: "#fb923c",
  },
];

export const GroupLogsModal: React.FC<GroupLogsModalProps> = ({
  isOpen,
  onClose,
  groupName,
  containers,
}) => {
  const toast = useToast();
  // Filter chips: selected container IDs. Empty Set = "All"
  const [selectedContainerIds, setSelectedContainerIds] = useState<Set<string>>(new Set());
  const [tail, setTail] = useState<string>("300");
  const [searchTerm, setSearchTerm] = useState("");
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [loading, setLoading] = useState(false);
  const [copied, setCopied] = useState(false);
  const [logsByContainer, setLogsByContainer] = useState<Record<string, string>>({});
  const [autoScroll, setAutoScroll] = useState(true);

  const scrollRef = useRef<HTMLDivElement>(null);

  // Map each container to a fixed color palette
  const containerColors = useMemo(() => {
    const map: Record<string, (typeof TAG_COLORS)[number]> = {};
    containers.forEach((c, idx) => {
      map[c.id] = TAG_COLORS[idx % TAG_COLORS.length];
    });
    return map;
  }, [containers]);

  // Fetch logs for all containers in the group
  const fetchAllLogs = useCallback(async () => {
    if (containers.length === 0) return;
    try {
      setLoading(true);
      const results = await Promise.allSettled(
        containers.map((c) => api.getContainerLogs(c.id, tail, true))
      );
      const next: Record<string, string> = {};
      results.forEach((res, idx) => {
        const c = containers[idx];
        if (res.status === "fulfilled") {
          next[c.id] = res.value;
        } else {
          next[c.id] = `[Error loading logs: ${String(res.reason)}]`;
        }
      });
      setLogsByContainer(next);
    } catch (err) {
      console.error("Failed to load group logs", err);
    } finally {
      setLoading(false);
    }
  }, [containers, tail]);

  // Initial fetch and auto-refresh timer
  useEffect(() => {
    if (!isOpen) return;
    fetchAllLogs();
  }, [isOpen, fetchAllLogs]);

  useEffect(() => {
    if (!isOpen || !autoRefresh) return;
    const interval = setInterval(fetchAllLogs, 4000);
    return () => clearInterval(interval);
  }, [isOpen, autoRefresh, fetchAllLogs]);

  // Parse and aggregate logs from each container
  const aggregatedLines = useMemo(() => {
    const lines: LogLine[] = [];

    containers.forEach((c) => {
      // If user selected specific tags and this container is not selected, skip
      if (selectedContainerIds.size > 0 && !selectedContainerIds.has(c.id)) {
        return;
      }

      const raw = logsByContainer[c.id];
      if (!raw) return;

      const cName = c.name.replace(/^\//, "");
      const color = containerColors[c.id]?.hex || "#94a3b8";

      const rawLines = raw.split("\n");
      rawLines.forEach((line, idx) => {
        if (!line.trim()) return;

        // Docker timestamp format: 2026-03-29T10:15:30.123456789Z message
        let timestamp = "";
        let message = line;
        let rawTime: number | undefined;

        const spaceIdx = line.indexOf(" ");
        if (spaceIdx > 19 && line.slice(0, 4).match(/^\d{4}$/)) {
          const possibleDate = line.slice(0, spaceIdx);
          const t = Date.parse(possibleDate);
          if (!isNaN(t)) {
            rawTime = t;
            // Format nice time string: HH:mm:ss.SSS
            const d = new Date(t);
            timestamp = d.toTimeString().split(" ")[0] + "." + String(d.getMilliseconds()).padStart(3, "0");
            message = line.slice(spaceIdx + 1);
          }
        }

        lines.push({
          id: `${c.id}-${idx}`,
          containerId: c.id,
          containerName: cName,
          color,
          timestamp,
          rawTime,
          message,
        });
      });
    });

    // Sort lines by raw timestamp if available so logs interleave chronologically
    lines.sort((a, b) => {
      if (a.rawTime !== undefined && b.rawTime !== undefined) {
        return a.rawTime - b.rawTime;
      }
      return 0;
    });

    return lines;
  }, [containers, logsByContainer, selectedContainerIds, containerColors]);

  // Filter lines by search term
  const filteredLines = useMemo(() => {
    if (!searchTerm.trim()) return aggregatedLines;
    const q = searchTerm.toLowerCase();
    return aggregatedLines.filter(
      (l) =>
        l.message.toLowerCase().includes(q) ||
        l.containerName.toLowerCase().includes(q) ||
        l.timestamp.toLowerCase().includes(q)
    );
  }, [aggregatedLines, searchTerm]);

  // Auto-scroll to bottom when new logs arrive
  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [filteredLines, autoScroll]);

  // Toggle filter tags
  const toggleContainerTag = (id: string) => {
    setSelectedContainerIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const selectAll = () => {
    setSelectedContainerIds(new Set());
  };

  const handleCopy = () => {
    const text = filteredLines
      .map(
        (l) =>
          `[${l.containerName}] ${l.timestamp ? `${l.timestamp} ` : ""}${l.message}`
      )
      .join("\n");
    if (!text) return;
    navigator.clipboard.writeText(text);
    setCopied(true);
    toast.success("Group logs copied to clipboard");
    setTimeout(() => setCopied(false), 2000);
  };

  if (!isOpen) return null;

  const isAllSelected = selectedContainerIds.size === 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-3 sm:p-6 bg-black/80 backdrop-blur-md animate-in fade-in duration-200">
      <div className="w-full max-w-6xl h-[88vh] flex flex-col bg-zinc-950 border border-zinc-800 rounded-2xl shadow-2xl overflow-hidden">
        {/* Header */}
        <div className="px-6 py-4 border-b border-zinc-800/80 bg-zinc-900/60 backdrop-blur flex items-center justify-between gap-4 shrink-0">
          <div className="flex items-center gap-3 min-w-0">
            <div className="w-10 h-10 rounded-xl bg-indigo-500/10 border border-indigo-500/20 flex items-center justify-center text-indigo-400 shrink-0">
              <Layers className="w-5 h-5" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h2 className="text-base font-semibold text-white truncate tracking-tight">
                  {groupName}
                </h2>
                <span className="px-2 py-0.5 rounded-full text-[11px] font-medium bg-zinc-800 text-zinc-300 border border-zinc-700/50">
                  {containers.length} {containers.length === 1 ? "container" : "containers"}
                </span>
              </div>
              <p className="text-xs text-zinc-400 truncate">
                Aggregated live log stream with interactive container filtering
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            {/* Auto refresh button */}
            <Button
              variant={autoRefresh ? "secondary" : "outline"}
              size="sm"
              onClick={() => setAutoRefresh(!autoRefresh)}
              className="text-xs h-8 gap-1.5"
              title={autoRefresh ? "Pause auto-refresh (4s)" : "Resume auto-refresh"}
            >
              {autoRefresh ? (
                <>
                  <Pause className="w-3.5 h-3.5 text-emerald-400" />
                  <span className="text-emerald-300 font-medium">Live</span>
                </>
              ) : (
                <>
                  <Play className="w-3.5 h-3.5 text-zinc-400" />
                  <span>Paused</span>
                </>
              )}
            </Button>

            {/* Reload button */}
            <Button
              variant="outline"
              size="sm"
              onClick={() => fetchAllLogs()}
              disabled={loading}
              className="text-xs h-8 gap-1.5"
              title="Refresh logs now"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin text-indigo-400" : ""}`} />
              <span className="hidden sm:inline">Refresh</span>
            </Button>

            {/* Copy button */}
            <Button
              variant="outline"
              size="sm"
              onClick={handleCopy}
              className="text-xs h-8 gap-1.5"
              title="Copy visible logs"
            >
              {copied ? (
                <>
                  <Check className="w-3.5 h-3.5 text-emerald-400" />
                  <span className="text-emerald-300">Copied</span>
                </>
              ) : (
                <>
                  <Copy className="w-3.5 h-3.5" />
                  <span className="hidden sm:inline">Copy</span>
                </>
              )}
            </Button>

            {/* Close button */}
            <button
              type="button"
              onClick={onClose}
              className="p-1.5 rounded-lg text-zinc-400 hover:text-white hover:bg-zinc-800 transition-colors ml-1 cursor-pointer"
              aria-label="Close group logs modal"
            >
              <X className="w-5 h-5" />
            </button>
          </div>
        </div>

        {/* Toolbar & Filter Chips */}
        <div className="px-6 py-3 border-b border-zinc-800/60 bg-zinc-900/30 flex flex-col gap-2.5 shrink-0">
          <div className="flex flex-wrap items-center justify-between gap-3">
            {/* Search Input */}
            <div className="relative flex-1 min-w-[200px] max-w-md">
              <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400 pointer-events-none" />
              <input
                type="text"
                placeholder="Filter logs or search regex..."
                value={searchTerm}
                onChange={(e) => setSearchTerm(e.target.value)}
                className="w-full bg-zinc-950/80 border border-zinc-800 rounded-lg pl-8.5 pr-8 py-1.5 text-xs text-zinc-200 placeholder-zinc-500 focus:outline-none focus:border-indigo-500/80 focus:ring-1 focus:ring-indigo-500/30"
              />
              {searchTerm && (
                <button
                  type="button"
                  onClick={() => setSearchTerm("")}
                  className="absolute right-2.5 top-1/2 -translate-y-1/2 text-zinc-500 hover:text-zinc-300"
                >
                  <X className="w-3 h-3" />
                </button>
              )}
            </div>

            {/* Tail lines selector & Auto-scroll toggle */}
            <div className="flex items-center gap-3">
              <div className="flex items-center gap-1.5 text-xs text-zinc-400">
                <span className="text-zinc-500">Lines:</span>
                <select
                  value={tail}
                  onChange={(e) => setTail(e.target.value)}
                  className="bg-zinc-900 border border-zinc-800 text-zinc-200 rounded-lg px-2 py-1 text-xs focus:outline-none focus:border-indigo-500"
                >
                  <option value="100">100</option>
                  <option value="200">200</option>
                  <option value="300">300</option>
                  <option value="500">500</option>
                  <option value="1000">1000</option>
                </select>
              </div>

              <button
                type="button"
                onClick={() => setAutoScroll(!autoScroll)}
                className={`flex items-center gap-1 px-2.5 py-1 rounded-lg border text-xs font-medium transition-colors cursor-pointer ${
                  autoScroll
                    ? "bg-indigo-500/15 border-indigo-500/30 text-indigo-300"
                    : "bg-zinc-900 border-zinc-800 text-zinc-400 hover:text-zinc-200"
                }`}
                title="Automatically scroll down as new log entries arrive"
              >
                <ArrowDown className="w-3 h-3" />
                <span>Auto-scroll</span>
              </button>
            </div>
          </div>

          {/* Container Tag Chips Bar */}
          <div className="flex items-center gap-2 overflow-x-auto py-1 scrollbar-thin">
            <div className="flex items-center gap-1 text-xs text-zinc-500 font-medium mr-1 shrink-0">
              <Filter className="w-3 h-3" />
              <span>Filter:</span>
            </div>

            {/* "All" Tag Chip */}
            <button
              type="button"
              onClick={selectAll}
              className={`px-3 py-1 rounded-lg text-xs font-semibold shrink-0 transition-all border cursor-pointer ${
                isAllSelected
                  ? "bg-indigo-600 text-white border-indigo-500 shadow-sm shadow-indigo-500/30"
                  : "bg-zinc-900/90 text-zinc-400 border-zinc-800 hover:text-zinc-200 hover:border-zinc-700"
              }`}
            >
              All Containers ({containers.length})
            </button>

            {/* Individual Container Tags */}
            {containers.map((c) => {
              const isSelected = selectedContainerIds.has(c.id);
              const palette = containerColors[c.id] || TAG_COLORS[0];
              const cName = c.name.replace(/^\//, "");
              const isRunning = c.state === "running";

              return (
                <button
                  key={c.id}
                  type="button"
                  onClick={() => toggleContainerTag(c.id)}
                  className={`group px-2.5 py-1 rounded-lg text-xs font-medium shrink-0 flex items-center gap-2 border transition-all cursor-pointer ${
                    isSelected
                      ? `${palette.bg} ${palette.text} ${palette.border} ring-1 ring-inset ${palette.border}`
                      : "bg-zinc-900/80 text-zinc-400 border-zinc-800 hover:text-zinc-200 hover:border-zinc-700"
                  }`}
                >
                  <span
                    className={`w-1.5 h-1.5 rounded-full ${
                      isRunning ? "bg-emerald-400" : "bg-zinc-500"
                    }`}
                  />
                  <span className="font-mono text-[11px] truncate max-w-[140px]">
                    {cName}
                  </span>
                </button>
              );
            })}
          </div>
        </div>

        {/* Log Viewer Content */}
        <div
          ref={scrollRef}
          className="flex-1 overflow-y-auto bg-zinc-950 p-4 font-mono text-xs leading-relaxed select-text"
        >
          {filteredLines.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center text-zinc-500 gap-2">
              <FileText className="w-8 h-8 opacity-40" />
              <p className="text-sm">
                {loading
                  ? "Fetching logs from containers..."
                  : searchTerm
                  ? "No log entries match your filter query."
                  : "No log output recorded for selected containers."}
              </p>
            </div>
          ) : (
            <div className="space-y-1">
              {filteredLines.map((line) => (
                <div
                  key={line.id}
                  className="flex items-start gap-2.5 hover:bg-zinc-900/50 px-2 py-0.5 rounded transition-colors group"
                >
                  {/* Container badge */}
                  <span
                    className="shrink-0 px-2 py-0.2 rounded text-[10px] font-semibold tracking-wide border cursor-default select-none transition-opacity"
                    style={{
                      color: line.color,
                      borderColor: `${line.color}40`,
                      backgroundColor: `${line.color}15`,
                    }}
                    title={`Container: ${line.containerName}`}
                  >
                    {line.containerName}
                  </span>

                  {/* Timestamp if available */}
                  {line.timestamp && (
                    <span className="shrink-0 text-zinc-500 text-[11px] select-none font-mono">
                      {line.timestamp}
                    </span>
                  )}

                  {/* Log message */}
                  <span className="text-zinc-200 break-all whitespace-pre-wrap flex-1">
                    {line.message}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Footer info bar */}
        <div className="px-6 py-2.5 border-t border-zinc-800/80 bg-zinc-950 flex items-center justify-between text-[11px] text-zinc-400 shrink-0">
          <div>
            Showing <span className="font-semibold text-zinc-200">{filteredLines.length}</span> lines
            {selectedContainerIds.size > 0 && (
              <span>
                {" "}
                across <span className="font-semibold text-zinc-200">{selectedContainerIds.size}</span> filtered containers
              </span>
            )}
          </div>
          <div className="flex items-center gap-3">
            <span>Press Esc to close</span>
          </div>
        </div>
      </div>
    </div>
  );
};
