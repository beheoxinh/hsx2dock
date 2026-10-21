import React, { useEffect, useMemo, useState } from "react";
import {
  Activity,
  AlertTriangle,
  Bell,
  Boxes,
  CheckCircle2,
  Copy,
  Download,
  FileCode2,
  FileText,
  Folder,
  Hexagon,
  Loader2,
  Package,
  RefreshCw,
  RotateCw,
  Search,
  Server,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "../ui/Button";
import { useModal } from "../ui/ModalContext";
import { useToast } from "../ui/Toast";
import { InteractiveTerminal } from "../terminal/InteractiveTerminal";
import { api } from "../../lib/api";
import { formatBytes, formatRelativeTime } from "../../lib/utils";
import type {
  K8sClusterSummary,
  K8sConfigInfo,
  K8sEventInfo,
  K8sMetrics,
  K8sNodeInfo,
  K8sPodInfo,
  K8sPvcInfo,
  K8sServiceInfo,
  K8sWorkloadInfo,
  MetricsServerStatus,
} from "../../types";

type MonitorTab =
  | "overview"
  | "nodes"
  | "podlist"
  | "workloads"
  | "services"
  | "config"
  | "pvcs"
  | "events";

const TABS: { id: MonitorTab; label: string; icon: React.ElementType }[] = [
  { id: "overview", label: "Overview", icon: Activity },
  { id: "nodes", label: "Nodes", icon: Server },
  { id: "podlist", label: "Pods", icon: Package },
  { id: "workloads", label: "Workloads", icon: Boxes },
  { id: "services", label: "Services", icon: Folder },
  { id: "config", label: "Config/Secrets", icon: FileText },
  { id: "pvcs", label: "PVCs", icon: Package },
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
  // Real metrics-server state (deployment/pods/`kubectl top`), polled with the
  // slow tick — keeps "installed but warming up" distinct from "not installed".
  const [msStatus, setMsStatus] = useState<MetricsServerStatus | null>(null);
  const modal = useModal();

  // Extra resource listings (tab-scoped, polled with the slow tick).
  const [services, setServices] = useState<K8sServiceInfo[]>([]);
  const [configs, setConfigs] = useState<K8sConfigInfo[]>([]);
  const [pvcs, setPvcs] = useState<K8sPvcInfo[]>([]);

  // Pod / workload logs viewer (workloadKind set → aggregated workload logs).
  const [logsFor, setLogsFor] = useState<
    { ns: string; name: string; workloadKind?: string } | null
  >(null);
  const [logsText, setLogsText] = useState<string>("");
  const [logsLoading, setLogsLoading] = useState(false);
  const [logsError, setLogsError] = useState<string | null>(null);

  // Workload scale prompt.
  const [scaleFor, setScaleFor] = useState<
    { kind: string; ns: string; name: string; current: number } | null
  >(null);
  const [scaleValue, setScaleValue] = useState(1);

  // Resource YAML / events viewers + pod terminal.
  const [yamlFor, setYamlFor] = useState<
    { kind: string; ns: string; name: string } | null
  >(null);
  const [yamlText, setYamlText] = useState("");
  const [yamlLoading, setYamlLoading] = useState(false);
  const [yamlError, setYamlError] = useState<string | null>(null);
  const [eventsFor, setEventsFor] = useState<
    { kind: string; ns: string; name: string } | null
  >(null);
  const [eventsList, setEventsList] = useState<K8sEventInfo[]>([]);
  const [eventsLoading, setEventsLoading] = useState(false);
  const [eventsError, setEventsError] = useState<string | null>(null);
  const [terminalFor, setTerminalFor] = useState<
    { ns: string; name: string; shell: string; mode: "shell" | "debug" } | null
  >(null);
  // "ns/name" while a shell probe is in flight (prevents double-clicks).
  const [terminalBusy, setTerminalBusy] = useState<string | null>(null);

  useEffect(() => {
    setTab("overview");
    setSummary(null);
    setNodes([]);
    setPods([]);
    setEvents([]);
    setWorkloads([]);
    setServices([]);
    setConfigs([]);
    setPvcs([]);
    setLogsFor(null);
    setLogsText("");
    setScaleFor(null);
    setMetrics(null);
    setMonitorError(null);
    setPodNs("all");
    setPodSearch("");
    setYamlFor(null);
    setEventsFor(null);
    setTerminalFor(null);
    setMsStatus(null);
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
        const [n, ev, w, sv, cf, pv] = await Promise.all([
          api.getClusterNodes(cluster),
          api.getClusterEvents(cluster),
          api.getClusterWorkloads(cluster),
          api.getClusterServices(cluster),
          api.getClusterConfigResources(cluster),
          api.getClusterPvcs(cluster),
        ]);
        if (!alive) return;
        setNodes(n);
        setEvents(ev);
        setWorkloads(w);
        setServices(sv);
        setConfigs(cf);
        setPvcs(pv);
      } catch {
        /* fast poll surfaces connectivity errors */
      }
    };
    const ms = () => {
      api
        .metricsServerStatus(cluster)
        .then((s) => {
          if (alive) setMsStatus(s);
        })
        .catch(() => {
          /* unreachable cluster is reported by the fast poll */
        });
    };

    fast();
    slow();
    ms();
    const t1 = window.setInterval(fast, 6000);
    const t2 = window.setInterval(slow, 15000);
    const t3 = window.setInterval(ms, 20000);
    return () => {
      alive = false;
      window.clearInterval(t1);
      window.clearInterval(t2);
      window.clearInterval(t3);
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

  // Live counts from the polled lists — used instead of the summary's numbers
  // whenever the lists loaded, so one flaky sub-query can't zero out the tiles.
  const phaseCounts = useMemo(() => {
    if (pods.length === 0) return null;
    const c = { Running: 0, Pending: 0, Failed: 0, Succeeded: 0 };
    for (const p of pods) {
      if (p.phase in c) c[p.phase as keyof typeof c] += 1;
    }
    return c;
  }, [pods]);
  const liveReadyNodes = useMemo(() => {
    if (nodes.length === 0) return null;
    return `${nodes.filter((n) => n.ready).length}/${nodes.length}`;
  }, [nodes]);

  const installMetricsServer = async () => {
    try {
      setInstallingMs(true);
      // The command applies the manifest, patches --kubelet-insecure-tls,
      // waits for the rollout and for `kubectl top` — up to ~4 minutes.
      const status = await api.installMetricsServer(cluster);
      setMsStatus(status);
      // Fresh usage numbers as soon as the API answers.
      api.getClusterMetrics(cluster).then(setMetrics).catch(() => {});
      if (status.top_works) {
        toast.success(
          "metrics-server ready",
          "Pod CPU/memory pills will fill in on the next poll."
        );
      } else if (status.installed) {
        toast.info(
          "metrics-server installed",
          status.message ??
            "Usage data usually shows up within a minute — this tab re-checks automatically."
        );
      } else {
        toast.error("Install incomplete", status.message ?? "Check the pod list in kube-system.");
      }
    } catch (e) {
      toast.error("Install failed", String(e));
    } finally {
      setInstallingMs(false);
    }
  };

  // ---- Pod / workload / resource actions ----

  const loadLogs = async (
    ns: string,
    name: string,
    tail = 300,
    workloadKind?: string
  ) => {
    setLogsLoading(true);
    setLogsError(null);
    try {
      setLogsText(
        workloadKind
          ? await api.getWorkloadLogs(cluster, workloadKind, ns, name, tail)
          : await api.getPodLogs(cluster, ns, name, tail)
      );
    } catch (e) {
      setLogsText("");
      setLogsError(String(e));
    } finally {
      setLogsLoading(false);
    }
  };

  useEffect(() => {
    if (logsFor) loadLogs(logsFor.ns, logsFor.name, 300, logsFor.workloadKind);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [logsFor?.ns, logsFor?.name, logsFor?.workloadKind, cluster]);

  /** Opens the YAML or events viewer for a resource. */
  const openResourceView = (
    kind: string,
    ns: string,
    name: string,
    mode: "yaml" | "events" = "yaml"
  ) => {
    if (mode === "events") setEventsFor({ kind, ns, name });
    else setYamlFor({ kind, ns, name });
  };

  /** Streams aggregated workload logs through the shared logs modal. */
  const openWorkloadLogs = (kind: string, ns: string, name: string) => {
    setLogsFor({ ns, name, workloadKind: kind });
  };

  /**
   * Opens a terminal in a pod. The image may have no shell at all (distroless
   * — `exec: "sh": executable file not found in $PATH`), so the shell is probed
   * first and, when none exists, the user can inject a busybox debug container.
   */
  const openPodTerminal = async (ns: string, name: string) => {
    if (terminalBusy) return;
    try {
      setTerminalBusy(`${ns}/${name}`);
      const shell = await api.detectPodShell(cluster, ns, name);
      if (shell) {
        setTerminalFor({ ns, name, shell, mode: "shell" });
        return;
      }
      const useDebug = await modal.showConfirm(
        `No shell was found in '${name}' — its image ships no /bin/sh (distroless/scratch), so a normal exec cannot start. Open a debug container instead? A busybox ephemeral container is injected into the pod and removed when the pod is deleted.`,
        "No shell in this image",
        "info"
      );
      if (useDebug) setTerminalFor({ ns, name, shell: "/bin/sh", mode: "debug" });
    } catch (e) {
      toast.error("Terminal unavailable", String(e));
    } finally {
      setTerminalBusy(null);
    }
  };

  useEffect(() => {
    if (!yamlFor) return;
    let alive = true;
    setYamlLoading(true);
    setYamlError(null);
    setYamlText("");
    api
      .getResourceYaml(cluster, yamlFor.kind, yamlFor.ns, yamlFor.name)
      .then((y) => {
        if (alive) setYamlText(y);
      })
      .catch((e) => {
        if (alive) setYamlError(String(e));
      })
      .finally(() => {
        if (alive) setYamlLoading(false);
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [yamlFor?.kind, yamlFor?.ns, yamlFor?.name, cluster]);

  useEffect(() => {
    if (!eventsFor) return;
    let alive = true;
    setEventsLoading(true);
    setEventsError(null);
    setEventsList([]);
    api
      .getResourceEvents(cluster, eventsFor.ns, eventsFor.name)
      .then((ev) => {
        if (alive) setEventsList(ev);
      })
      .catch((e) => {
        if (alive) setEventsError(String(e));
      })
      .finally(() => {
        if (alive) setEventsLoading(false);
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eventsFor?.ns, eventsFor?.name, cluster]);

  const handleDeletePod = async (p: K8sPodInfo) => {
    const ok = await modal.showConfirm(
      `Delete pod '${p.name}' in ${p.namespace}? Its controller will recreate it.`,
      "Delete Pod",
      "danger"
    );
    if (!ok) return;
    try {
      await api.deletePod(cluster, p.namespace, p.name);
      toast.success("Pod deleted", `${p.namespace}/${p.name}`);
      api.getClusterPods(cluster).then(setPods).catch(() => {});
    } catch (e) {
      toast.error("Delete failed", String(e));
    }
  };

  const handleDeleteResource = async (
    kind: string,
    ns: string,
    name: string,
    label: string
  ) => {
    const ok = await modal.showConfirm(
      `Delete ${label} '${name}' in ${ns}?`,
      `Delete ${label}`,
      "danger"
    );
    if (!ok) return;
    try {
      await api.deleteK8sResource(cluster, kind, ns, name);
      toast.success(`${label} deleted`, `${ns}/${name}`);
      slowRefresh();
    } catch (e) {
      toast.error("Delete failed", String(e));
    }
  };

  const handleRestart = async (w: K8sWorkloadInfo) => {
    try {
      await api.restartWorkload(cluster, w.kind.toLowerCase(), w.namespace, w.name);
      toast.success("Rolling restart started", `${w.kind} ${w.namespace}/${w.name}`);
    } catch (e) {
      toast.error("Restart failed", String(e));
    }
  };

  const handleScaleSubmit = async () => {
    if (!scaleFor) return;
    try {
      await api.scaleWorkload(
        cluster,
        scaleFor.kind.toLowerCase(),
        scaleFor.ns,
        scaleFor.name,
        Math.max(0, scaleValue)
      );
      toast.success(
        "Scaled",
        `${scaleFor.name} → ${Math.max(0, scaleValue)} replica(s)`
      );
      setScaleFor(null);
      api.getClusterWorkloads(cluster).then(setWorkloads).catch(() => {});
    } catch (e) {
      toast.error("Scale failed", String(e));
    }
  };

  const slowRefresh = () => {
    api.getClusterNodes(cluster).then(setNodes).catch(() => {});
    api.getClusterEvents(cluster).then(setEvents).catch(() => {});
    api.getClusterWorkloads(cluster).then(setWorkloads).catch(() => {});
    api.getClusterServices(cluster).then(setServices).catch(() => {});
    api.getClusterConfigResources(cluster).then(setConfigs).catch(() => {});
    api.getClusterPvcs(cluster).then(setPvcs).catch(() => {});
    api.getClusterPods(cluster).then(setPods).catch(() => {});
    api.getClusterSummary(cluster).then(setSummary).catch(() => {});
  };

  const cpuPct = totalCpuCap > 0 ? (totalCpuUsed / totalCpuCap) * 100 : 0;
  const memPct = totalMemCap > 0 ? (totalMemUsed / totalMemCap) * 100 : 0;
  // Usage is missing only when neither source produced data — one failing
  // query must not blank the other's numbers.
  const noMetrics =
    !metrics?.nodes.length &&
    !metrics?.pods.length &&
    (!!metrics?.error || (msStatus?.installed === true && !msStatus.top_works));

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

      {summary?.errors && summary.errors.length > 0 && (
        <div className="mx-5 mt-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-300">
          <span className="font-semibold">Partial data:</span>{" "}
          {summary.errors.join(" · ")}
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
                  liveReadyNodes ??
                  (summary ? `${summary.readyNodes}/${summary.nodeCount}` : "—")
                }
                sub="ready / total"
              />
              <StatTile
                label="Namespaces"
                icon={Boxes}
                value={namespaces.length || summary?.namespaceCount || "—"}
              />
              <StatTile
                label="Pods"
                icon={Package}
                value={
                  pods.length ||
                  (summary
                    ? summary.podRunning + summary.podPending + summary.podFailed
                    : "—")
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
                    ["Running", phaseCounts?.Running ?? summary?.podRunning, "text-emerald-300"],
                    ["Pending", phaseCounts?.Pending ?? summary?.podPending, "text-amber-300"],
                    ["Failed", phaseCounts?.Failed ?? summary?.podFailed, "text-rose-300"],
                    ["Succeeded", phaseCounts?.Succeeded ?? summary?.podSucceeded, "text-sky-300"],
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
                      <div className="flex flex-wrap items-center gap-2 text-[11px] text-zinc-500">
                        <span>
                          {msStatus?.installed
                            ? msStatus.message ??
                              "metrics-server installed — usage data appears within a minute."
                            : msStatus?.message ?? "metrics-server not installed."}
                        </span>
                        {msStatus?.pods.length ? (
                          <span className="font-mono text-zinc-400">
                            {msStatus.pods
                              .map(
                                (p) =>
                                  `${p.name}:${p.phase}${p.ready ? "" : "!"}${p.reason ? `(${p.reason})` : ""}`
                              )
                              .join(" | ")}
                          </span>
                        ) : null}
                        {msStatus && !msStatus.installed && (
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={installMetricsServer}
                            disabled={installingMs}
                            className="gap-1.5 ml-auto"
                          >
                            {installingMs && (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            )}
                            Install metrics-server
                          </Button>
                        )}
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
            {/* Pod usage comes from metrics-server — offer the install inline. */}
            {noMetrics && msStatus && (
              <div className="flex flex-wrap items-center gap-3 rounded-xl border border-amber-500/20 bg-amber-500/5 px-4 py-2.5 text-xs text-amber-300">
                {msStatus.installed ? (
                  <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin" />
                ) : (
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                )}
                <span>
                  {msStatus.installed
                    ? `metrics-server installed (${
                        msStatus.ready ? "ready" : "starting"
                      } ${msStatus.available_replicas}/${msStatus.desired_replicas}) — ${
                      msStatus.message ?? "usage columns fill in automatically."
                    }`
                    : "metrics-server is not installed — pod CPU/memory columns stay empty until it is."}
                </span>
                {msStatus.pods.length ? (
                  <span className="font-mono text-amber-200/70">
                    {msStatus.pods
                      .map(
                        (p) =>
                          `${p.name}:${p.phase}${p.ready ? "" : "!"}${p.reason ? `(${p.reason})` : ""}`
                      )
                      .join(" | ")}
                  </span>
                ) : null}
                <Button
                  variant="outline"
                  size="sm"
                  onClick={installMetricsServer}
                  disabled={installingMs}
                  className="gap-1.5 ml-auto"
                >
                  {installingMs && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                  {msStatus.installed ? "Repair & restart" : "Install metrics-server"}
                </Button>
              </div>
            )}
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
                    <th className="px-3 py-2 text-right">Actions</th>
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
                        <td className="px-3 py-2">
                          <div className="flex items-center justify-end gap-1">
                            <Button
                              variant="ghost"
                              size="icon"
                              title="View logs"
                              aria-label="View pod logs"
                              onClick={() => setLogsFor({ ns: p.namespace, name: p.name })}
                              className="h-6 w-6 text-zinc-400 hover:text-white"
                            >
                              <FileText className="h-3.5 w-3.5" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              title="Open an interactive shell in this pod"
                              aria-label="Open pod terminal"
                              disabled={p.phase !== "Running" || terminalBusy !== null}
                              onClick={() => openPodTerminal(p.namespace, p.name)}
                              className="h-6 w-6 text-zinc-400 hover:text-emerald-300 disabled:opacity-40"
                            >
                              {terminalBusy === `${p.namespace}/${p.name}` ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                              ) : (
                                <Terminal className="h-3.5 w-3.5" />
                              )}
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              title="View pod YAML"
                              aria-label="View pod YAML"
                              onClick={() =>
                                openResourceView("pod", p.namespace, p.name)
                              }
                              className="h-6 w-6 text-zinc-400 hover:text-sky-300"
                            >
                              <FileCode2 className="h-3.5 w-3.5" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              title="View pod events"
                              aria-label="View pod events"
                              onClick={() =>
                                openResourceView("pod", p.namespace, p.name, "events")
                              }
                              className="h-6 w-6 text-zinc-400 hover:text-amber-300"
                            >
                              <Bell className="h-3.5 w-3.5" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              title="Delete pod"
                              aria-label="Delete pod"
                              onClick={() => handleDeletePod(p)}
                              className="h-6 w-6 text-zinc-400 hover:text-rose-400"
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </Button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                  {filteredPods.length === 0 && (
                    <tr>
                      <td colSpan={10} className="px-3 py-8 text-center text-zinc-500">
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
                  <th className="px-3 py-2 text-right">Actions</th>
                </tr>
              </thead>
              <tbody>
                {workloads.map((w) => {
                  const healthy = w.desired > 0 && w.ready >= w.desired;
                  const wk = w.kind.toLowerCase();
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
                      <td className="px-3 py-2">
                        <div className="flex items-center justify-end gap-1">
                          {(wk === "deployment" || wk === "statefulset") && (
                            <Button
                              variant="ghost"
                              size="icon"
                              title="Scale"
                              onClick={() =>
                                setScaleFor({
                                  kind: w.kind,
                                  ns: w.namespace,
                                  name: w.name,
                                  current: w.desired,
                                })
                              }
                              className="h-6 w-6 text-zinc-400 hover:text-white"
                            >
                              <Activity className="h-3.5 w-3.5" />
                            </Button>
                          )}
                          <Button
                            variant="ghost"
                            size="icon"
                            title="Rolling restart"
                            onClick={() => handleRestart(w)}
                            className="h-6 w-6 text-zinc-400 hover:text-white"
                          >
                            <RotateCw className="h-3.5 w-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="Stream workload logs"
                            aria-label="Stream workload logs"
                            onClick={() =>
                              openWorkloadLogs(w.kind.toLowerCase(), w.namespace, w.name)
                            }
                            className="h-6 w-6 text-zinc-400 hover:text-white"
                          >
                            <FileText className="h-3.5 w-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="View YAML"
                            aria-label="View workload YAML"
                            onClick={() =>
                              openResourceView(w.kind.toLowerCase(), w.namespace, w.name)
                            }
                            className="h-6 w-6 text-zinc-400 hover:text-sky-300"
                          >
                            <FileCode2 className="h-3.5 w-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="View events"
                            aria-label="View workload events"
                            onClick={() =>
                              openResourceView(
                                w.kind.toLowerCase(),
                                w.namespace,
                                w.name,
                                "events"
                              )
                            }
                            className="h-6 w-6 text-zinc-400 hover:text-amber-300"
                          >
                            <Bell className="h-3.5 w-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="Delete workload"
                            onClick={() =>
                              handleDeleteResource(wk, w.namespace, w.name, w.kind)
                            }
                            className="h-6 w-6 text-zinc-400 hover:text-rose-400"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
                {workloads.length === 0 && (
                  <tr>
                    <td colSpan={7} className="px-3 py-8 text-center text-zinc-500">
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

        {/* ============ SERVICES ============ */}
        {tab === "services" && (
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                  <th className="px-3 py-2">Namespace</th>
                  <th className="px-3 py-2">Name</th>
                  <th className="px-3 py-2">Type</th>
                  <th className="px-3 py-2">Cluster IP</th>
                  <th className="px-3 py-2">Ports</th>
                  <th className="px-3 py-2">Age</th>
                  <th className="px-3 py-2 text-right">Actions</th>
                </tr>
              </thead>
              <tbody>
                {services.map((s) => (
                  <tr
                    key={`${s.namespace}/${s.name}`}
                    className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20"
                  >
                    <td className="px-3 py-2 text-zinc-500">{s.namespace}</td>
                    <td className="px-3 py-2 font-mono text-zinc-200">{s.name}</td>
                    <td className="px-3 py-2 text-zinc-400">{s.type}</td>
                    <td className="px-3 py-2 font-mono text-zinc-400">{s.clusterIp}</td>
                    <td className="px-3 py-2 font-mono text-zinc-400">{s.ports}</td>
                    <td className="px-3 py-2 text-zinc-500">
                      {s.created ? formatRelativeTime(s.created) : "-"}
                    </td>
                    <td className="px-3 py-2 text-right">
                      <Button
                        variant="ghost"
                        size="icon"
                        title="View YAML"
                        aria-label="View service YAML"
                        onClick={() => openResourceView("service", s.namespace, s.name)}
                        className="h-6 w-6 text-zinc-400 hover:text-sky-300"
                      >
                        <FileCode2 className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        title="View events"
                        aria-label="View service events"
                        onClick={() =>
                          openResourceView("service", s.namespace, s.name, "events")
                        }
                        className="h-6 w-6 text-zinc-400 hover:text-amber-300"
                      >
                        <Bell className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        title="Delete service"
                        onClick={() => handleDeleteResource("service", s.namespace, s.name, "Service")}
                        className="h-6 w-6 text-zinc-400 hover:text-rose-400"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </td>
                  </tr>
                ))}
                {services.length === 0 && (
                  <tr>
                    <td colSpan={7} className="px-3 py-8 text-center text-zinc-500">
                      {monitorError ? "Cluster unreachable" : "No services."}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        )}

        {/* ============ CONFIG MAPS & SECRETS ============ */}
        {tab === "config" && (
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                  <th className="px-3 py-2">Namespace</th>
                  <th className="px-3 py-2">Name</th>
                  <th className="px-3 py-2">Kind</th>
                  <th className="px-3 py-2">Keys</th>
                  <th className="px-3 py-2">Age</th>
                  <th className="px-3 py-2 text-right">Actions</th>
                </tr>
              </thead>
              <tbody>
                {configs.map((c) => (
                  <tr
                    key={`${c.resourceKind}/${c.namespace}/${c.name}`}
                    className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20"
                  >
                    <td className="px-3 py-2 text-zinc-500">{c.namespace}</td>
                    <td className="px-3 py-2 font-mono text-zinc-200">{c.name}</td>
                    <td className="px-3 py-2">
                      <span
                        className={`inline-flex rounded-md border px-1.5 py-0.5 text-[10px] ${
                          c.resourceKind === "Secret"
                            ? "border-amber-500/30 bg-amber-500/10 text-amber-300"
                            : "border-zinc-700 text-zinc-400"
                        }`}
                      >
                        {c.resourceKind}
                      </span>
                    </td>
                    <td
                      className="px-3 py-2 font-mono text-zinc-400 max-w-64 truncate"
                      title={c.keys.join(", ")}
                    >
                      {c.keys.length ? c.keys.join(", ") : "—"}
                    </td>
                    <td className="px-3 py-2 text-zinc-500">
                      {c.created ? formatRelativeTime(c.created) : "-"}
                    </td>
                    <td className="px-3 py-2 text-right">
                      <Button
                        variant="ghost"
                        size="icon"
                        title="View YAML"
                        aria-label="View resource YAML"
                        onClick={() =>
                          openResourceView(
                            c.resourceKind === "Secret" ? "secret" : "configmap",
                            c.namespace,
                            c.name
                          )
                        }
                        className="h-6 w-6 text-zinc-400 hover:text-sky-300"
                      >
                        <FileCode2 className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        title="View events"
                        aria-label="View resource events"
                        onClick={() =>
                          openResourceView(
                            c.resourceKind === "Secret" ? "secret" : "configmap",
                            c.namespace,
                            c.name,
                            "events"
                          )
                        }
                        className="h-6 w-6 text-zinc-400 hover:text-amber-300"
                      >
                        <Bell className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        title={`Delete ${c.resourceKind.toLowerCase()}`}
                        onClick={() =>
                          handleDeleteResource(
                            c.resourceKind === "Secret" ? "secret" : "configmap",
                            c.namespace,
                            c.name,
                            c.resourceKind
                          )
                        }
                        className="h-6 w-6 text-zinc-400 hover:text-rose-400"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </td>
                  </tr>
                ))}
                {configs.length === 0 && (
                  <tr>
                    <td colSpan={6} className="px-3 py-8 text-center text-zinc-500">
                      {monitorError ? "Cluster unreachable" : "No config maps or secrets."}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        )}

        {/* ============ PVCs ============ */}
        {tab === "pvcs" && (
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80">
            <table className="w-full text-xs">
              <thead>
                <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                  <th className="px-3 py-2">Namespace</th>
                  <th className="px-3 py-2">Name</th>
                  <th className="px-3 py-2">Status</th>
                  <th className="px-3 py-2">Capacity</th>
                  <th className="px-3 py-2">Access</th>
                  <th className="px-3 py-2">Class</th>
                  <th className="px-3 py-2">Age</th>
                  <th className="px-3 py-2 text-right">Actions</th>
                </tr>
              </thead>
              <tbody>
                {pvcs.map((p) => (
                  <tr
                    key={`${p.namespace}/${p.name}`}
                    className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20"
                  >
                    <td className="px-3 py-2 text-zinc-500">{p.namespace}</td>
                    <td className="px-3 py-2 font-mono text-zinc-200">{p.name}</td>
                    <td className="px-3 py-2">
                      <span
                        className={`inline-flex rounded-md border px-1.5 py-0.5 text-[10px] ${
                          p.status === "Bound"
                            ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                            : p.status === "Pending"
                              ? "border-amber-500/30 bg-amber-500/10 text-amber-300"
                              : "border-zinc-700 text-zinc-400"
                        }`}
                      >
                        {p.status}
                      </span>
                    </td>
                    <td className="px-3 py-2 font-mono text-zinc-300">{p.capacity || "—"}</td>
                    <td className="px-3 py-2 text-zinc-500">{p.accessModes}</td>
                    <td className="px-3 py-2 text-zinc-500">{p.storageClass || "—"}</td>
                    <td className="px-3 py-2 text-zinc-500">
                      {p.created ? formatRelativeTime(p.created) : "-"}
                    </td>
                    <td className="px-3 py-2 text-right">
                      <Button
                        variant="ghost"
                        size="icon"
                        title="View YAML"
                        aria-label="View PVC YAML"
                        onClick={() =>
                          openResourceView("persistentvolumeclaim", p.namespace, p.name)
                        }
                        className="h-6 w-6 text-zinc-400 hover:text-sky-300"
                      >
                        <FileCode2 className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        title="Delete PVC"
                        onClick={() =>
                          handleDeleteResource(
                            "persistentvolumeclaim",
                            p.namespace,
                            p.name,
                            "PVC"
                          )
                        }
                        className="h-6 w-6 text-zinc-400 hover:text-rose-400"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </td>
                  </tr>
                ))}
                {pvcs.length === 0 && (
                  <tr>
                    <td colSpan={8} className="px-3 py-8 text-center text-zinc-500">
                      {monitorError ? "Cluster unreachable" : "No persistent volume claims."}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        )}
      {/* Pod logs modal */}
      {logsFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
          <div className="flex w-full max-w-3xl flex-col rounded-2xl border border-zinc-800 bg-zinc-900 shadow-2xl max-h-[80vh]">
            <div className="flex items-center gap-2 border-b border-zinc-800 px-4 py-3">
              <FileText className="h-4 w-4 text-zinc-400" />
              <span className="text-sm font-semibold text-white">
                {logsFor.workloadKind ? `${logsFor.workloadKind}/` : ""}
                {logsFor.ns}/{logsFor.name}
              </span>
              <span className="text-[11px] text-zinc-500">logs (last 300 lines)</span>
              <div className="ml-auto flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="icon"
                  title="Reload logs"
                  onClick={() =>
                    loadLogs(logsFor.ns, logsFor.name, 300, logsFor.workloadKind)
                  }
                  className="h-7 w-7 text-zinc-400 hover:text-white"
                >
                  <RefreshCw className={`h-3.5 w-3.5 ${logsLoading ? "animate-spin" : ""}`} />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  title="Copy logs"
                  onClick={() => navigator.clipboard.writeText(logsText).catch(() => {})}
                  className="h-7 w-7 text-zinc-400 hover:text-white"
                >
                  <Copy className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  title="Close"
                  onClick={() => setLogsFor(null)}
                  className="h-7 w-7 text-zinc-400 hover:text-white"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
            </div>
            <div className="flex-1 overflow-auto bg-zinc-950/60 px-4 py-3">
              {logsLoading ? (
                <p className="text-xs text-zinc-500">Loading logs…</p>
              ) : logsError ? (
                <p className="text-xs text-rose-400">{logsError}</p>
              ) : (
                <pre className="whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed text-zinc-300">
                  {logsText || "(empty)"}
                </pre>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Resource YAML modal */}
      {yamlFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
          <div className="w-full max-w-3xl h-[70vh] flex flex-col rounded-xl border border-zinc-800 bg-zinc-900 shadow-2xl">
            <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
              <div className="min-w-0">
                <p className="text-sm font-semibold text-white truncate">
                  {yamlFor.kind} · {yamlFor.name}
                </p>
                <p className="text-[11px] text-zinc-500">namespace {yamlFor.ns}</p>
              </div>
              <div className="flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="icon"
                  title="Copy YAML"
                  aria-label="Copy YAML"
                  disabled={!yamlText}
                  onClick={async () => {
                    try {
                      await navigator.clipboard.writeText(yamlText);
                      toast.success("YAML copied");
                    } catch (e) {
                      toast.error("Copy failed", String(e));
                    }
                  }}
                  className="h-7 w-7 text-zinc-400"
                >
                  <Copy className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  title="Close"
                  aria-label="Close YAML viewer"
                  onClick={() => setYamlFor(null)}
                  className="h-7 w-7 text-zinc-400"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
            </div>
            <div className="flex-1 min-h-0 overflow-auto px-4 py-3">
              {yamlLoading ? (
                <div className="flex items-center gap-2 text-xs text-zinc-500">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading manifest…
                </div>
              ) : yamlError ? (
                <p className="text-xs text-rose-400">{yamlError}</p>
              ) : (
                <pre className="whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed text-zinc-300">
                  {yamlText}
                </pre>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Resource events modal */}
      {eventsFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
          <div className="w-full max-w-3xl h-[70vh] flex flex-col rounded-xl border border-zinc-800 bg-zinc-900 shadow-2xl">
            <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
              <div className="min-w-0">
                <p className="text-sm font-semibold text-white truncate">
                  Events · {eventsFor.kind} {eventsFor.name}
                </p>
                <p className="text-[11px] text-zinc-500">namespace {eventsFor.ns}</p>
              </div>
              <Button
                variant="ghost"
                size="icon"
                title="Close"
                aria-label="Close events viewer"
                onClick={() => setEventsFor(null)}
                className="h-7 w-7 text-zinc-400"
              >
                <X className="h-4 w-4" />
              </Button>
            </div>
            <div className="flex-1 min-h-0 overflow-auto">
              {eventsLoading ? (
                <div className="flex items-center gap-2 px-4 py-3 text-xs text-zinc-500">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading events…
                </div>
              ) : eventsError ? (
                <p className="px-4 py-3 text-xs text-rose-400">{eventsError}</p>
              ) : eventsList.length === 0 ? (
                <p className="px-4 py-6 text-center text-xs text-zinc-500">
                  No events recorded for this resource.
                </p>
              ) : (
                <table className="w-full text-xs">
                  <thead>
                    <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                      <th className="px-3 py-2 font-medium">Type</th>
                      <th className="px-3 py-2 font-medium">Reason</th>
                      <th className="px-3 py-2 font-medium">Object</th>
                      <th className="px-3 py-2 font-medium">Message</th>
                      <th className="px-3 py-2 font-medium">Count</th>
                      <th className="px-3 py-2 font-medium">Last seen</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-zinc-800/60">
                    {eventsList.map((ev, i) => (
                      <tr key={i} className="hover:bg-zinc-800/30">
                        <td className="px-3 py-2">
                          <span
                            className={`text-[10px] px-1.5 py-0.5 rounded border ${
                              ev.type === "Warning"
                                ? "bg-rose-500/10 text-rose-300 border-rose-500/30"
                                : "bg-zinc-800 text-zinc-400 border-zinc-700"
                            }`}
                          >
                            {ev.type}
                          </span>
                        </td>
                        <td className="px-3 py-2 font-mono text-zinc-300">{ev.reason}</td>
                        <td className="px-3 py-2 font-mono text-zinc-500">{ev.object}</td>
                        <td
                          className="px-3 py-2 text-zinc-400 max-w-md truncate"
                          title={ev.message}
                        >
                          {ev.message}
                        </td>
                        <td className="px-3 py-2 font-mono text-zinc-400">{ev.count}</td>
                        <td className="px-3 py-2 text-zinc-500">
                          {ev.lastTimestamp ? formatRelativeTime(ev.lastTimestamp) : "—"}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Pod terminal modal */}
      {terminalFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
          <div className="w-full max-w-4xl h-[70vh] flex flex-col rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl">
            <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
              <div className="min-w-0">
                <p className="text-sm font-semibold text-white truncate">
                  Terminal · {terminalFor.name}
                </p>
                <p className="text-[11px] text-zinc-500">
                  {cluster} · namespace {terminalFor.ns} ·{" "}
                  {terminalFor.mode === "debug"
                    ? "busybox debug container"
                    : terminalFor.shell}
                </p>
              </div>
              <Button
                variant="ghost"
                size="icon"
                title="Close terminal"
                aria-label="Close terminal"
                onClick={() => setTerminalFor(null)}
                className="h-7 w-7 text-zinc-400"
              >
                <X className="h-4 w-4" />
              </Button>
            </div>
            <div className="flex-1 min-h-0">
              <InteractiveTerminal
                containerId={`${cluster}-control-plane`}
                cmd={
                  terminalFor.mode === "debug"
                    ? [
                        // Ephemeral busybox container for images with no shell.
                        "kubectl",
                        "--kubeconfig=/etc/kubernetes/admin.conf",
                        "debug",
                        "-it",
                        terminalFor.name,
                        "-n",
                        terminalFor.ns,
                        "--image=busybox:1.36",
                        "--",
                        "/bin/sh",
                      ]
                    : [
                        "kubectl",
                        "--kubeconfig=/etc/kubernetes/admin.conf",
                        "exec",
                        "-n",
                        terminalFor.ns,
                        terminalFor.name,
                        "-i",
                        "-t",
                        "--",
                        terminalFor.shell,
                      ]
                }
              />
            </div>
          </div>
        </div>
      )}

      {/* Scale modal */}
      {scaleFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
          <div className="w-full max-w-sm rounded-2xl border border-zinc-800 bg-zinc-900 shadow-2xl">
            <div className="border-b border-zinc-800 px-4 py-3">
              <span className="text-sm font-semibold text-white">
                Scale {scaleFor.kind}: {scaleFor.name}
              </span>
              <p className="text-[11px] text-zinc-500 mt-0.5">
                {scaleFor.ns} · currently {scaleFor.current} replica(s)
              </p>
            </div>
            <div className="px-4 py-4">
              <label className="block text-xs text-zinc-400 mb-1">Replicas</label>
              <input
                type="number"
                min={0}
                value={scaleValue}
                onChange={(e) => setScaleValue(Math.max(0, Number(e.target.value) || 0))}
                className="h-9 w-full px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-indigo-500"
                autoFocus
                onKeyDown={(e) => {
                  if (e.key === "Enter") handleScaleSubmit();
                  if (e.key === "Escape") setScaleFor(null);
                }}
              />
            </div>
            <div className="flex justify-end gap-2 border-t border-zinc-800 px-4 py-3">
              <Button variant="ghost" size="sm" onClick={() => setScaleFor(null)}>
                Cancel
              </Button>
              <Button variant="primary" size="sm" onClick={handleScaleSubmit}>
                Scale
              </Button>
            </div>
          </div>
        </div>
      )}

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
