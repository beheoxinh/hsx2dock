import React, { useEffect, useMemo, useState } from "react";
import {
  Activity,
  AlertTriangle,
  Boxes,
  CheckCircle2,
  Download,
  Hexagon,
  Loader2,
  Package,
  RefreshCw,
  Search,
  Server,
} from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import { formatBytes, formatRelativeTime } from "../../lib/utils";
import type {
  K8sClusterSummary,
  K8sEventInfo,
  K8sMetrics,
  K8sNodeInfo,
  K8sPodInfo,
  K8sWorkloadInfo,
} from "../../types";

type MonitorTab = "overview" | "nodes" | "podlist" | "workloads" | "events";

const TABS: { id: MonitorTab; label: string; icon: React.ElementType }[] = [
  { id: "overview", label: "Overview", icon: Activity },
  { id: "nodes", label: "Nodes", icon: Server },
  { id: "podlist", label: "Pods", icon: Package },
  { id: "workloads", label: "Workloads", icon: Boxes },
  { id: "events", label: "Events", icon: AlertTriangle },
];

/** "1250m" → "1.25 cores"; "90m" → "90m". */
const fmtCpu = (m: number): string =>
  m >= 1000 ? `${(m / 1000).toFixed(2)} cores` : `${Math.round(m)}m`;

const phaseBadge: Record<string, string> = {
  Running: "bg-emerald-500/10 text-emerald-300 border-emerald-500/30",
  Pending: "bg-amber-500/10 text-amber-300 border-amber-500/30",
  Failed: "bg-rose-500/10 text-rose-300 border-rose-500/30",
  Succeeded: "bg-sky-500/10 text-sky-300 border-sky-500/30",
};

function UsageBar({ pct, danger }: { pct: number; danger?: boolean }) {
  const clamped = Math.max(0, Math.min(100, pct));
  const color =
    danger || clamped > 85 ? "bg-rose-500" : clamped > 65 ? "bg-amber-500" : "bg-emerald-500";
  return (
    <div className="h-1.5 w-full min-w-16 rounded-full bg-zinc-800">
      <div
        className={`h-full rounded-full ${color} transition-all duration-500`}
        style={{ width: `${clamped}%` }}
      />
    </div>
  );
}

function StatTile({
  label,
  value,
  sub,
  icon: Icon,
  tone = "zinc",
}: {
  label: string;
  value: React.ReactNode;
  sub?: string;
  icon: React.ElementType;
  tone?: "zinc" | "emerald" | "amber" | "rose" | "sky" | "indigo";
}) {
  const tones = {
    zinc: "text-zinc-300",
    emerald: "text-emerald-300",
    amber: "text-amber-300",
    rose: "text-rose-300",
    sky: "text-sky-300",
    indigo: "text-indigo-300",
  } as const;
  return (
    <div className="rounded-xl border border-zinc-800/80 bg-zinc-900/60 px-4 py-3">
      <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-zinc-500">
        <Icon className="h-3 w-3" />
        {label}
      </div>
      <div className={`mt-1 text-xl font-semibold tabular-nums ${tones[tone]}`}>{value}</div>
      {sub && <div className="mt-0.5 text-[11px] text-zinc-500">{sub}</div>}
    </div>
  );
}

/**
 * Live monitoring for one running kind cluster: summary, node & pod resource
 * usage (via metrics-server when present), workloads, and events. Data is
 * polled from kubectl inside the control-plane node — no host kubectl needed.
 */
export const ClusterMonitor: React.FC<{ cluster: string }> = ({ cluster }) => {
  const toast = useToast();
  const [tab, setTab] = useState<MonitorTab>("overview");
  const [summary, setSummary] = useState<K8sClusterSummary | null>(null);
  const [nodes, setNodes] = useState<K8sNodeInfo[]>([]);
  const [pods, setPods] = useState<K8sPodInfo[]>([]);
  const [events, setEvents] = useState<K8sEventInfo[]>([]);
  const [workloads, setWorkloads] = useState<K8sWorkloadInfo[]>([]);
  const [metrics, setMetrics] = useState<K8sMetrics | null>(null);
  const [monitorError, setMonitorError] = useState<string | null>(null);
  const [podNs, setPodNs] = useState("all");
  const [podSearch, setPodSearch] = useState("");
  const [installingMs, setInstallingMs] = useState(false);

  useEffect(() => {
    setTab("overview");
    setSummary(null);
    setNodes([]);
    setPods([]);
    setEvents([]);
    setWorkloads([]);
    setMetrics(null);
    setMonitorError(null);
    setPodNs("all");
    setPodSearch("");
  }, [cluster]);

  useEffect(() => {
    let alive = true;

    const fast = async () => {
      try {
        const [s, p, m] = await Promise.all([
          api.getClusterSummary(cluster),
          api.getClusterPods(cluster),
          api.getClusterMetrics(cluster),
        ]);
        if (!alive) return;
        setSummary(s);
        setPods(p);
        setMetrics(m);
        setMonitorError(null);
      } catch (e) {
        if (alive) setMonitorError(String(e));
      }
    };
    const slow = async () => {
      try {
        const [n, ev, w] = await Promise.all([
          api.getClusterNodes(cluster),
          api.getClusterEvents(cluster),
          api.getClusterWorkloads(cluster),
        ]);
        if (!alive) return;
        setNodes(n);
        setEvents(ev);
        setWorkloads(w);
      } catch {
        /* fast poll surfaces connectivity errors */
      }
    };

    fast();
    slow();
    const t1 = window.setInterval(fast, 6000);
    const t2 = window.setInterval(slow, 15000);
    return () => {
      alive = false;
      window.clearInterval(t1);
      window.clearInterval(t2);
    };
  }, [cluster]);

  const namespaces = useMemo(() => {
    const set = new Set(pods.map((p) => p.namespace));
    return Array.from(set).sort();
  }, [pods]);

  const nodeMetricsByName = useMemo(() => {
    const m = new Map(metrics?.nodes.map((n) => [n.name, n]) ?? []);
    return m;
  }, [metrics]);

  const podMetricsByKey = useMemo(() => {
    const m = new Map(metrics?.pods.map((p) => [`${p.namespace}/${p.name}`, p]) ?? []);
    return m;
  }, [metrics]);

  const filteredPods = useMemo(() => {
    const q = podSearch.trim().toLowerCase();
    return pods.filter((p) => {
      if (podNs !== "all" && p.namespace !== podNs) return false;
      if (!q) return true;
      return (
        p.name.toLowerCase().includes(q) ||
        p.namespace.toLowerCase().includes(q) ||
        p.node.toLowerCase().includes(q) ||
        p.images.some((i) => i.toLowerCase().includes(q))
      );
    });
  }, [pods, podNs, podSearch]);

  const totalCpuCap = useMemo(() => nodes.reduce((a, n) => a + n.cpuCapacityM, 0), [nodes]);
  const totalMemCap = useMemo(
    () => nodes.reduce((a, n) => a + n.memCapacityBytes, 0),
    [nodes]
  );
  const totalCpuUsed = useMemo(
    () => (metrics?.nodes ?? []).reduce((a, n) => a + n.cpuM, 0),
    [metrics]
  );
  const totalMemUsed = useMemo(
    () => (metrics?.nodes ?? []).reduce((a, n) => a + n.memBytes, 0),
    [metrics]
  );

  const warnings = useMemo(() => events.filter((e) => e.type === "Warning").slice(0, 6), [events]);

  const installMetricsServer = async () => {
    try {
      setInstallingMs(true);
      await api.installMetricsServer(cluster);
      toast.success(
        "metrics-server applied",
        "Pods schedule in ~1 min — usage pills fill in automatically."
      );
    } catch (e) {
      toast.error("Install failed", String(e));
    } finally {
      setInstallingMs(false);
    }
  };

  const cpuPct = totalCpuCap > 0 ? (totalCpuUsed / totalCpuCap) * 100 : 0;
  const memPct = totalMemCap > 0 ? (totalMemUsed / totalMemCap) * 100 : 0;
  const noMetrics = !!metrics?.error;

  return (
    <section className="rounded-2xl border border-zinc-800/80 bg-zinc-900/40 overflow-hidden">
      {/* Header: title + live chip + refresh info */}
      <div className="flex items-center gap-3 px-5 py-3 border-b border-zinc-800/80">
        <Hexagon className="h-4 w-4 text-indigo-400" />
        <span className="text-sm font-semibold text-white">Monitoring</span>
        <span className="text-xs text-zinc-500 font-mono">{cluster}</span>
        <div className="ml-auto flex items-center gap-3 text-[11px] text-zinc-500">
          {summary && (
            <span className="flex items-center gap-1.5">
              <span
                className={`h-1.5 w-1.5 rounded-full ${
                  summary.ready ? "bg-emerald-400 animate-pulse" : "bg-rose-400"
                }`}
              />
              {summary.ready ? "healthy" : "degraded"} · {summary.gitVersion || "?"}
            </span>
          )}
          <span>auto-refresh 6s</span>
        </div>
      </div>

      {monitorError && (
        <div className="mx-5 mt-3 rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-300">
          Cannot reach the cluster API: {monitorError}
        </div>
      )}

      {/* Tabs */}
      <div className="flex items-center gap-1 px-4 pt-2 border-b border-zinc-800/60">
        {TABS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className={`flex items-center gap-1.5 px-3 py-2 text-xs font-medium rounded-t-lg transition-colors ${
              tab === id
                ? "text-white border-b-2 border-indigo-400 bg-zinc-800/40"
                : "text-zinc-400 hover:text-zinc-200"
            }`}
          >
            <Icon className="h-3.5 w-3.5" />
            {label}
            {id === "events" && warnings.length > 0 && (
              <span className="ml-0.5 rounded-full bg-rose-500/20 px-1.5 text-[10px] text-rose-300">
                {events.filter((e) => e.type === "Warning").length}
              </span>
            )}
          </button>
        ))}
      </div>

      <div className="p-5">
        {/* ============ OVERVIEW ============ */}
        {tab === "overview" && (
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
              <StatTile
                label="API Server"
                icon={CheckCircle2}
                tone={summary?.ready ? "emerald" : "rose"}
                value={summary?.ready ? "Ready" : "Not ready"}
                sub={summary?.readyMessage ?? summary?.platform ?? undefined}
              />
              <StatTile
                label="Nodes"
                icon={Server}
                value={
                  summary ? `${summary.readyNodes}/${summary.nodeCount}` : "—"
                }
                sub="ready / total"
              />
              <StatTile
                label="Namespaces"
                icon={Boxes}
                value={summary?.namespaceCount ?? "—"}
              />
              <StatTile
                label="Pods"
                icon={Package}
                value={
                  summary
                    ? summary.podRunning + summary.podPending + summary.podFailed
                    : "—"
                }
                sub="active (excluding completed)"
              />
            </div>

            {/* Pod phase breakdown */}
            <div className="rounded-xl border border-zinc-800/80 bg-zinc-900/60 p-4">
              <div className="flex items-center justify-between mb-3">
                <span className="text-xs font-semibold uppercase tracking-wider text-zinc-400">
                  Pod phases
                </span>
                {noMetrics && (
                  <button
                    onClick={installMetricsServer}
                    disabled={installingMs}
                    className="flex items-center gap-1.5 rounded-lg border border-indigo-500/40 bg-indigo-500/10 px-2.5 py-1 text-[11px] font-medium text-indigo-300 hover:bg-indigo-500/20 disabled:opacity-60"
                  >
                    {installingMs ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      <Download className="h-3 w-3" />
                    )}
                    Install metrics-server
                  </button>
                )}
              </div>
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
                {(
                  [
                    ["Running", summary?.podRunning, "text-emerald-300"],
                    ["Pending", summary?.podPending, "text-amber-300"],
                    ["Failed", summary?.podFailed, "text-rose-300"],
                    ["Succeeded", summary?.podSucceeded, "text-sky-300"],
                  ] as const
                ).map(([label, count, tone]) => (
                  <div key={label} className="rounded-lg bg-zinc-950/60 border border-zinc-800/60 px-3 py-2">
                    <div className="text-[10px] uppercase tracking-wider text-zinc-500">
                      {label}
                    </div>
                    <div className={`text-lg font-semibold tabular-nums ${tone}`}>
                      {count ?? "—"}
                    </div>
                  </div>
                ))}
              </div>
            </div>

            {/* Cluster resource usage */}
            <div className="grid gap-3 lg:grid-cols-2">
              {(
                [
                  {
                    label: "Cluster CPU",
                    used: totalCpuUsed,
                    cap: totalCpuCap,
                    pct: cpuPct,
                    fmt: fmtCpu,
                    icon: Activity,
                  },
                  {
                    label: "Cluster memory",
                    used: totalMemUsed,
                    cap: totalMemCap,
                    pct: memPct,
                    fmt: formatBytes,
                    icon: Server,
                  },
                ] as const
              ).map(({ label, used, cap, pct, fmt, icon: Icon }) => (
                <div
                  key={label}
                  className="rounded-xl border border-zinc-800/80 bg-zinc-900/60 px-4 py-3"
                >
                  <div className="flex items-center justify-between text-xs">
                    <span className="flex items-center gap-1.5 font-semibold uppercase tracking-wider text-zinc-400">
                      <Icon className="h-3.5 w-3.5" />
                      {label}
                    </span>
                    <span className="font-mono text-zinc-300">
                      {noMetrics || cap === 0 ? "n/a" : `${fmt(used)} / ${fmt(cap)}`}
                    </span>
                  </div>
                  <div className="mt-2">
                    {noMetrics || cap === 0 ? (
                      <div className="flex items-center justify-between text-[11px] text-zinc-500">
                        <span>
                          metrics-server not installed — usage numbers unavailable
                        </span>
                      </div>
                    ) : (
                      <>
                        <UsageBar pct={pct} />
                        <div className="mt-1 text-right font-mono text-[11px] text-zinc-400">
                          {pct.toFixed(1)}%
                        </div>
                      </>
                    )}
                  </div>
                </div>
              ))}
            </div>

            {/* Recent warnings */}
            <div className="rounded-xl border border-zinc-800/80 bg-zinc-900/60 p-4">
              <span className="text-xs font-semibold uppercase tracking-wider text-zinc-400">
                Recent warnings
              </span>
              {warnings.length === 0 ? (
                <p className="mt-2 flex items-center gap-1.5 text-xs text-emerald-400">
                  <CheckCircle2 className="h-3.5 w-3.5" /> No warning events — all quiet.
                </p>
              ) : (
                <ul className="mt-2 space-y-1.5">
                  {warnings.map((e, i) => (
                    <li key={i} className="flex items-start gap-2 text-xs">
                      <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0 text-rose-400" />
                      <span className="shrink-0 font-mono text-rose-300">{e.reason}</span>
                      <span className="min-w-0 truncate text-zinc-400">
                        {e.object}: {e.message}
                      </span>
                      <span className="ml-auto shrink-0 text-[10px] text-zinc-600">
                        {e.lastTimestamp ? formatRelativeTime(e.lastTimestamp) : ""}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        )}

        {/* ============ NODES ============ */}
        {tab === "nodes" && (
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                  <th className="px-3 py-2">Node</th>
                  <th className="px-3 py-2">Status</th>
                  <th className="px-3 py-2">Roles</th>
                  <th className="px-3 py-2 min-w-36">CPU</th>
                  <th className="px-3 py-2 min-w-36">Memory</th>
                  <th className="px-3 py-2">Kubelet</th>
                  <th className="px-3 py-2">Runtime</th>
                  <th className="px-3 py-2">IP</th>
                  <th className="px-3 py-2">Age</th>
                </tr>
              </thead>
              <tbody>
                {nodes.map((n) => {
                  const m = nodeMetricsByName.get(n.name);
                  const cpuPctN = m && n.cpuCapacityM > 0 ? (m.cpuM / n.cpuCapacityM) * 100 : 0;
                  const memPctN =
                    m && n.memCapacityBytes > 0 ? (m.memBytes / n.memCapacityBytes) * 100 : 0;
                  return (
                    <tr key={n.name} className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20">
                      <td className="px-3 py-2 font-mono text-zinc-200">{n.name}</td>
                      <td className="px-3 py-2">
                        <span
                          className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-[10px] ${
                            n.ready
                              ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                              : "border-rose-500/30 bg-rose-500/10 text-rose-300"
                          }`}
                        >
                          {n.status}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-zinc-400">
                        {n.roles.length ? n.roles.join(", ") : "worker"}
                      </td>
                      <td className="px-3 py-2">
                        {m ? (
                          <div className="space-y-1">
                            <UsageBar pct={cpuPctN} />
                            <span className="font-mono text-[10px] text-zinc-400">
                              {fmtCpu(m.cpuM)} / {fmtCpu(n.cpuCapacityM)}
                            </span>
                          </div>
                        ) : (
                          <span className="font-mono text-[10px] text-zinc-600">
                            cap {fmtCpu(n.cpuCapacityM)}
                          </span>
                        )}
                      </td>
                      <td className="px-3 py-2">
                        {m ? (
                          <div className="space-y-1">
                            <UsageBar pct={memPctN} />
                            <span className="font-mono text-[10px] text-zinc-400">
                              {formatBytes(m.memBytes)} / {formatBytes(n.memCapacityBytes)}
                            </span>
                          </div>
                        ) : (
                          <span className="font-mono text-[10px] text-zinc-600">
                            cap {formatBytes(n.memCapacityBytes)}
                          </span>
                        )}
                      </td>
                      <td className="px-3 py-2 font-mono text-zinc-400">{n.kubeletVersion}</td>
                      <td className="px-3 py-2 text-zinc-500" title={n.containerRuntime}>
                        {n.containerRuntime.split("://")[0]}
                      </td>
                      <td className="px-3 py-2 font-mono text-zinc-400">{n.internalIp}</td>
                      <td className="px-3 py-2 text-zinc-500">
                        {n.created ? formatRelativeTime(n.created) : "-"}
                      </td>
                    </tr>
                  );
                })}
                {nodes.length === 0 && (
                  <tr>
                    <td colSpan={9} className="px-3 py-8 text-center text-zinc-500">
                      {monitorError ? "Cluster unreachable" : "Loading nodes…"}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        )}

        {/* ============ PODS ============ */}
        {tab === "podlist" && (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              <div className="relative">
                <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-zinc-500" />
                <input
                  value={podSearch}
                  onChange={(e) => setPodSearch(e.target.value)}
                  placeholder="Search pods, images, nodes…"
                  className="h-8 w-64 rounded-lg bg-zinc-950 border border-zinc-800 pl-8 pr-3 text-xs text-white placeholder-zinc-600 focus:outline-none focus:border-indigo-500"
                />
              </div>
              <select
                value={podNs}
                onChange={(e) => setPodNs(e.target.value)}
                className="h-8 rounded-lg bg-zinc-950 border border-zinc-800 px-2 text-xs text-zinc-200 focus:outline-none focus:border-indigo-500"
              >
                <option value="all">All namespaces ({pods.length})</option>
                {namespaces.map((ns) => (
                  <option key={ns} value={ns}>
                    {ns} ({pods.filter((p) => p.namespace === ns).length})
                  </option>
                ))}
              </select>
              <span className="ml-auto text-[11px] text-zinc-500">
                {filteredPods.length} pods
              </span>
            </div>

            <div className="overflow-x-auto rounded-xl border border-zinc-800/80">
              <table className="w-full text-xs">
                <thead>
                  <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                    <th className="px-3 py-2">Namespace</th>
                    <th className="px-3 py-2">Pod</th>
                    <th className="px-3 py-2">Phase</th>
                    <th className="px-3 py-2">Ready</th>
                    <th className="px-3 py-2">Restarts</th>
                    <th className="px-3 py-2">Node</th>
                    <th className="px-3 py-2 min-w-28">CPU</th>
                    <th className="px-3 py-2 min-w-28">Memory</th>
                    <th className="px-3 py-2">Age</th>
                  </tr>
                </thead>
                <tbody>
                  {filteredPods.map((p) => {
                    const m = podMetricsByKey.get(`${p.namespace}/${p.name}`);
                    return (
                      <tr
                        key={`${p.namespace}/${p.name}`}
                        className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20"
                      >
                        <td className="px-3 py-2 text-zinc-500">{p.namespace}</td>
                        <td
                          className="px-3 py-2 font-mono text-zinc-200 max-w-56 truncate"
                          title={p.images.join(", ")}
                        >
                          {p.name}
                        </td>
                        <td className="px-3 py-2">
                          <span
                            className={`inline-flex rounded-md border px-1.5 py-0.5 text-[10px] ${
                              phaseBadge[p.phase] ?? "border-zinc-700 text-zinc-400"
                            }`}
                          >
                            {p.phase}
                            {p.reason && p.phase !== "Running" ? ` (${p.reason})` : ""}
                          </span>
                        </td>
                        <td className="px-3 py-2 font-mono text-zinc-300">
                          {p.readyContainers}/{p.totalContainers}
                        </td>
                        <td
                          className={`px-3 py-2 font-mono ${
                            p.restarts > 0 ? "text-amber-300" : "text-zinc-400"
                          }`}
                        >
                          {p.restarts}
                        </td>
                        <td className="px-3 py-2 font-mono text-zinc-500">{p.node}</td>
                        <td className="px-3 py-2 font-mono text-zinc-300">
                          {m ? fmtCpu(m.cpuM) : "—"}
                        </td>
                        <td className="px-3 py-2 font-mono text-zinc-300">
                          {m ? formatBytes(m.memBytes) : "—"}
                        </td>
                        <td className="px-3 py-2 text-zinc-500">
                          {p.created ? formatRelativeTime(p.created) : "-"}
                        </td>
                      </tr>
                    );
                  })}
                  {filteredPods.length === 0 && (
                    <tr>
                      <td colSpan={9} className="px-3 py-8 text-center text-zinc-500">
                        {monitorError ? "Cluster unreachable" : "No pods match."}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>
        )}

        {/* ============ WORKLOADS ============ */}
        {tab === "workloads" && (
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                  <th className="px-3 py-2">Kind</th>
                  <th className="px-3 py-2">Namespace</th>
                  <th className="px-3 py-2">Name</th>
                  <th className="px-3 py-2">Ready / Desired</th>
                  <th className="px-3 py-2">Available</th>
                  <th className="px-3 py-2 min-w-32">Health</th>
                </tr>
              </thead>
              <tbody>
                {workloads.map((w) => {
                  const healthy = w.desired > 0 && w.ready >= w.desired;
                  return (
                    <tr
                      key={`${w.kind}/${w.namespace}/${w.name}`}
                      className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20"
                    >
                      <td className="px-3 py-2 text-zinc-400">{w.kind}</td>
                      <td className="px-3 py-2 text-zinc-500">{w.namespace}</td>
                      <td className="px-3 py-2 font-mono text-zinc-200">{w.name}</td>
                      <td className="px-3 py-2 font-mono text-zinc-300">
                        {w.ready}/{w.desired}
                      </td>
                      <td className="px-3 py-2 font-mono text-zinc-400">{w.available}</td>
                      <td className="px-3 py-2">
                        {w.desired === 0 ? (
                          <span className="text-[10px] text-zinc-500">scaled to zero</span>
                        ) : (
                          <UsageBar pct={(w.ready / w.desired) * 100} danger={!healthy} />
                        )}
                      </td>
                    </tr>
                  );
                })}
                {workloads.length === 0 && (
                  <tr>
                    <td colSpan={6} className="px-3 py-8 text-center text-zinc-500">
                      {monitorError ? "Cluster unreachable" : "No workloads yet."}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        )}

        {/* ============ EVENTS ============ */}
        {tab === "events" && (
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                  <th className="px-3 py-2">Type</th>
                  <th className="px-3 py-2">Reason</th>
                  <th className="px-3 py-2">Object</th>
                  <th className="px-3 py-2">Namespace</th>
                  <th className="px-3 py-2">Message</th>
                  <th className="px-3 py-2">Count</th>
                  <th className="px-3 py-2">Last seen</th>
                </tr>
              </thead>
              <tbody>
                {events.map((e, i) => (
                  <tr
                    key={i}
                    className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20"
                  >
                    <td className="px-3 py-2">
                      <span
                        className={`inline-flex rounded-md border px-1.5 py-0.5 text-[10px] ${
                          e.type === "Warning"
                            ? "border-rose-500/30 bg-rose-500/10 text-rose-300"
                            : "border-zinc-700 text-zinc-400"
                        }`}
                      >
                        {e.type}
                      </span>
                    </td>
                    <td className="px-3 py-2 font-mono text-zinc-300">{e.reason}</td>
                    <td className="px-3 py-2 font-mono text-zinc-400">{e.object}</td>
                    <td className="px-3 py-2 text-zinc-500">{e.namespace}</td>
                    <td className="px-3 py-2 max-w-96 truncate text-zinc-400" title={e.message}>
                      {e.message}
                    </td>
                    <td className="px-3 py-2 font-mono text-zinc-400">{e.count}</td>
                    <td className="px-3 py-2 text-zinc-500">
                      {e.lastTimestamp ? formatRelativeTime(e.lastTimestamp) : "-"}
                    </td>
                  </tr>
                ))}
                {events.length === 0 && (
                  <tr>
                    <td colSpan={7} className="px-3 py-8 text-center text-zinc-500">
                      {monitorError ? "Cluster unreachable" : "No events."}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Footer: manual refresh */}
      <div className="flex items-center justify-end gap-2 border-t border-zinc-800/80 px-5 py-2.5">
        <Button
          variant="ghost"
          size="sm"
          className="gap-1.5 text-xs text-zinc-400 hover:text-white"
          onClick={() => {
            api
              .getClusterSummary(cluster)
              .then(setSummary)
              .catch((e) => setMonitorError(String(e)));
            api.getClusterNodes(cluster).then(setNodes).catch(() => {});
            api.getClusterPods(cluster).then(setPods).catch(() => {});
            api.getClusterEvents(cluster).then(setEvents).catch(() => {});
            api.getClusterWorkloads(cluster).then(setWorkloads).catch(() => {});
            api.getClusterMetrics(cluster).then(setMetrics).catch(() => {});
          }}
        >
          <RefreshCw className="h-3.5 w-3.5" />
          Refresh now
        </Button>
      </div>
    </section>
  );
};
