import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  AlertTriangle,
  ArrowDownToLine,
  Ban,
  Bell,
  Boxes,
  Cable,
  CheckCircle2,
  Copy,
  Download,
  FileCode2,
  FileText,
  Folder,
  Globe,
  Hexagon,
  History,
  Layers,
  Loader2,
  Package,
  Plus,
  Puzzle,
  RefreshCw,
  RotateCw,
  ScrollText,
  Search,
  Server,
  Ship,
  Terminal,
  Trash2,
  X,
} from "lucide-react";
import { Button } from "../ui/Button";
import { useModal } from "../ui/ModalContext";
import { useToast } from "../ui/Toast";
import { InteractiveTerminal } from "../terminal/InteractiveTerminal";
import { YamlApplyModal } from "./YamlApplyModal";
import { SelectorLogsModal } from "./SelectorLogsModal";
import { KubeShellModal } from "./KubeShellModal";
import { AddonsPanel } from "./AddonsPanel";
import { IngressPanel } from "./IngressPanel";
import { HelmPanel } from "./HelmPanel";
import { PortForwardPanel } from "./PortForwardPanel";
import { PortForwardDialog } from "./PortForwardDialog";
import { ServiceLinksPanel } from "./ServiceLinksPanel";
import { HistoryPanel } from "./HistoryPanel";
import { AlertsPanel } from "./AlertsPanel";
import { parsePodContainerPorts, type ContainerPort } from "../../lib/portForward";
import { api, disposeListener, events as apiEvents } from "../../lib/api";
import { formatBytes, formatRelativeTime } from "../../lib/utils";
import { stripServerFields } from "../../lib/k8sYaml";
import type {
  K8sClusterSummary,
  K8sConfigInfo,
  K8sEventInfo,
  K8sMetrics,
  K8sNodeInfo,
  K8sPodInfo,
  K8sPvcInfo,
  K8sNamespaceInfo,
  K8sIngressInfo,
  K8sPvInfo,
  K8sServiceInfo,
  K8sWorkloadInfo,
  KindCluster,
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
  | "namespaces"
  | "events"
  | "addons"
  | "ingress"
  | "helm"
  | "forwards"
  | "history"
  | "alerts";

const TABS: { id: MonitorTab; label: string; icon: React.ElementType }[] = [
  { id: "overview", label: "Overview", icon: Activity },
  { id: "nodes", label: "Nodes", icon: Server },
  { id: "podlist", label: "Pods", icon: Package },
  { id: "workloads", label: "Workloads", icon: Boxes },
  { id: "services", label: "Network", icon: Folder },
  { id: "config", label: "Config/Secrets", icon: FileText },
  { id: "pvcs", label: "Storage", icon: Package },
  { id: "namespaces", label: "Namespaces", icon: Hexagon },
  { id: "events", label: "Events", icon: ScrollText },
  { id: "addons", label: "Addons", icon: Puzzle },
  { id: "ingress", label: "Ingress", icon: Globe },
  { id: "helm", label: "Helm", icon: Ship },
  { id: "forwards", label: "Port-forwards", icon: Cable },
  { id: "history", label: "History", icon: History },
  { id: "alerts", label: "Alerts", icon: Bell },
];

/**
 * Remote ports of a service, parsed from the `port[:nodePort]/PROTO, ...`
 * string kind.rs builds in parse_services. Returns distinct port numbers.
 */
function servicePorts(ports: string): number[] {
  const out: number[] = [];
  for (const part of ports.split(",")) {
    const m = /^\s*(\d+)(?::\d+)?\/[A-Za-z]+\s*$/.exec(part);
    if (!m) continue;
    const n = Number(m[1]);
    if (n >= 1 && n <= 65535 && !out.includes(n)) out.push(n);
  }
  return out;
}

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
    <div className="h-1.5 w-full min-w-16 rounded-none bg-zinc-800">
      <div
        className={`h-full rounded-none ${color} transition-all duration-500`}
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
export const ClusterMonitor: React.FC<{
  cluster: string;
  /**
   * Per-cluster numbers already shown on the cluster card (docker stats of the
   * node containers). Used as the usage fallback so Detail never goes blank
   * just because metrics-server has not answered yet.
   */
  clusterStats?: KindCluster | null;
}> = ({ cluster, clusterStats }) => {
  const toast = useToast();
  // Ad-hoc refreshes (after an action, "Refresh now") resolve whenever kubectl
  // answers; by then the user may have switched clusters. `forCluster` drops a
  // result that belongs to a cluster no longer on screen.
  const clusterRef = useRef(cluster);
  clusterRef.current = cluster;
  const forCluster =
    <T,>(apply: (v: T) => void) => {
      const owner = cluster;
      return (v: T) => {
        if (clusterRef.current === owner) apply(v);
      };
    };
  const logsSeq = useRef(0);
  const [tab, setTab] = useState<MonitorTab>("overview");
  const [summary, setSummary] = useState<K8sClusterSummary | null>(null);
  const [nodes, setNodes] = useState<K8sNodeInfo[]>([]);
  const [pods, setPods] = useState<K8sPodInfo[]>([]);
  const [events, setEvents] = useState<K8sEventInfo[]>([]);
  const [workloads, setWorkloads] = useState<K8sWorkloadInfo[]>([]);
  const [metrics, setMetrics] = useState<K8sMetrics | null>(null);
  const [monitorError, setMonitorError] = useState<string | null>(null);
  // One namespace filter for every tab (Lens-style); pods keeps its own
  // search box on top of it.
  const [nsFilter, setNsFilter] = useState("all");
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
  const [ingresses, setIngresses] = useState<K8sIngressInfo[]>([]);
  const [pvs, setPvs] = useState<K8sPvInfo[]>([]);
  const [namespaceList, setNamespaceList] = useState<K8sNamespaceInfo[]>([]);
  // Node name while a cordon/uncordon/drain/remove request is in flight.
  const [nodeBusy, setNodeBusy] = useState<string | null>(null);
  // Adding a worker takes about a minute; its kind output streams into the toolbar.
  const [addingWorker, setAddingWorker] = useState(false);
  const [workerLine, setWorkerLine] = useState<string | null>(null);
  // Workloads with a scale or rolling restart in flight. The ref is the guard
  // (a double click lands before React re-renders); the state disables buttons.
  const workloadOpsRef = useRef<Set<string>>(new Set());
  const [workloadOps, setWorkloadOps] = useState<Set<string>>(() => new Set());
  const workloadKey = (kind: string, ns: string, name: string) =>
    `${kind.toLowerCase()}/${ns}/${name}`;
  const claimWorkload = (key: string) => {
    if (workloadOpsRef.current.has(key)) return false;
    workloadOpsRef.current.add(key);
    setWorkloadOps(new Set(workloadOpsRef.current));
    return true;
  };
  const releaseWorkload = (key: string) => {
    workloadOpsRef.current.delete(key);
    setWorkloadOps(new Set(workloadOpsRef.current));
  };

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
    { kind: string; ns: string | null; name: string } | null
  >(null);
  const [yamlText, setYamlText] = useState("");
  const [yamlLoading, setYamlLoading] = useState(false);
  const [yamlError, setYamlError] = useState<string | null>(null);
  const [eventsFor, setEventsFor] = useState<
    { kind: string; ns: string | null; name: string } | null
  >(null);
  const [eventsList, setEventsList] = useState<K8sEventInfo[]>([]);
  const [eventsLoading, setEventsLoading] = useState(false);
  const [eventsError, setEventsError] = useState<string | null>(null);
  const [terminalFor, setTerminalFor] = useState<
    { ns: string; name: string; shell: string; mode: "shell" | "debug" } | null
  >(null);
  // "ns/name" while a shell probe is in flight (prevents double-clicks).
  const [terminalBusy, setTerminalBusy] = useState<string | null>(null);
  // Apply YAML / quick deploy, selector logs, and the kubectl shell.
  /** Bumped after an apply so an open YAML viewer shows the new state. */
  const [yamlReload, setYamlReload] = useState(0);
  const [applyFor, setApplyFor] = useState<
    { tab: "yaml" | "quick"; yaml?: string; title?: string } | null
  >(null);
  const [selectorLogsFor, setSelectorLogsFor] = useState<
    {
      ns: string;
      title: string;
      selector?: string;
      workloadKind?: string;
      workloadName?: string;
    } | null
  >(null);
  const [kubeShellOpen, setKubeShellOpen] = useState(false);
  // Port-forward dialog for one service or pod port; pod YAML fetch in flight.
  const [forwardFor, setForwardFor] = useState<
    { kind: "svc" | "pod"; ns: string; name: string; port: number } | null
  >(null);
  const [podForwardBusy, setPodForwardBusy] = useState<string | null>(null);

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
    setIngresses([]);
    setPvs([]);
    setNamespaceList([]);
    setLogsFor(null);
    setLogsText("");
    setScaleFor(null);
    setMetrics(null);
    setMonitorError(null);
    setNsFilter("all");
    setPodSearch("");
    setYamlFor(null);
    setEventsFor(null);
    setTerminalFor(null);
    setMsStatus(null);
    setNodeBusy(null);
    setApplyFor(null);
    setSelectorLogsFor(null);
    setKubeShellOpen(false);
    setForwardFor(null);
    setPodForwardBusy(null);
  }, [cluster]);

  useEffect(() => {
    let alive = true;
    // Each tick is three `docker exec kubectl` calls; a slow API server must
    // not let ticks pile up behind it.
    let inflight = false;

    const fast = async () => {
      if (inflight || document.hidden) return;
      inflight = true;
      // Each query settles on its own: without metrics-server only the
      // metrics call fails, and it must not blank the pods/summary panes.
      const [s, p, m] = await Promise.allSettled([
        api.getClusterSummary(cluster),
        api.getClusterPods(cluster),
        api.getClusterMetrics(cluster),
      ]).finally(() => {
        inflight = false;
      });
      if (!alive) return;
      if (s.status === "fulfilled") setSummary(s.value);
      if (p.status === "fulfilled") setPods(p.value);
      if (m.status === "fulfilled") setMetrics(m.value);
      const failed = [s, p, m].find((r) => r.status === "rejected");
      setMonitorError(
        failed && failed.status === "rejected" ? String(failed.reason) : null
      );
    };
    fast();
    // A window hidden to the tray polls nothing; the next visible tick catches up.
    // 12s: each tick is three kubectl runs in the node container. The inflight flag above skips a tick while one is still running.
    const t1 = window.setInterval(() => {
      if (!document.hidden) fast();
    }, 12000);
    return () => {
      alive = false;
      window.clearInterval(t1);
    };
  }, [cluster]);

  // Tab-scoped slow poll (15s): only query the kubectl resources needed by the
  // active tab instead of spawning 9 concurrent `docker exec kubectl` calls.
  useEffect(() => {
    let alive = true;

    // A slow API server (or a cluster still booting) made 15s ticks stack up
    // behind each other, each spawning more `docker exec kubectl` processes.
    let inflight = false;
    const settle = <T,>(p: Promise<T>, apply: (v: T) => void) =>
      p.then((v) => {
        if (alive) apply(v);
      }).catch(() => {});

    const fetchTab = async () => {
      if (inflight || document.hidden) return;
      inflight = true;
      try {
        if (tab === "overview") {
          await Promise.all([
            settle(api.getClusterNodes(cluster), setNodes),
            settle(api.getClusterEvents(cluster), setEvents),
          ]);
        } else if (tab === "nodes") {
          await settle(api.getClusterNodes(cluster), setNodes);
        } else if (tab === "workloads") {
          await settle(api.getClusterWorkloads(cluster), setWorkloads);
        } else if (tab === "services") {
          await Promise.all([
            settle(api.getClusterServices(cluster), setServices),
            settle(api.getClusterIngresses(cluster), setIngresses),
          ]);
        } else if (tab === "config") {
          await settle(api.getClusterConfigResources(cluster), setConfigs);
        } else if (tab === "pvcs") {
          await Promise.all([
            settle(api.getClusterPvcs(cluster), setPvcs),
            settle(api.getClusterPvs(cluster), setPvs),
          ]);
        } else if (tab === "namespaces") {
          await settle(api.getClusterNamespaces(cluster), setNamespaceList);
        } else if (tab === "events") {
          await settle(api.getClusterEvents(cluster), setEvents);
        }
      } finally {
        inflight = false;
      }
    };

    fetchTab();
    const t2 = window.setInterval(() => {
      if (!document.hidden) fetchTab();
    }, 15000);
    return () => {
      alive = false;
      window.clearInterval(t2);
    };
  }, [cluster, tab]);

  // Metrics-server is a *post-start* check. A freshly started cluster needs
  // roughly a minute before the addon has had a chance to become ready; probing
  // sooner turns a warming component into a "install metrics-server" prompt.
  // Once the cluster is up we wait out that first minute, then re-check every
  // 60s so a genuine failure still gets surfaced.
  useEffect(() => {
    let alive = true;
    let timeout: number | undefined;
    let interval: number | undefined;

    if (clusterStats?.api_ready !== true) {
      // Stopped (or still booting): nothing to probe and no stale prompt.
      setMsStatus(null);
      return;
    }

    const startedAtMs = clusterStats?.last_started_at
      ? clusterStats.last_started_at * 1000
      : null;
    const waitMs = startedAtMs ? Math.max(0, 60_000 - (Date.now() - startedAtMs)) : 0;

    const probe = () => {
      if (document.hidden) return;
      api
        .metricsServerStatus(cluster)
        .then((s) => {
          if (alive) setMsStatus(s);
        })
        .catch(() => {
          /* an unreachable cluster is already reported by the fast poll */
        });
    };

    timeout = window.setTimeout(() => {
      if (!alive) return;
      probe();
      interval = window.setInterval(probe, 60_000);
    }, waitMs);

    return () => {
      alive = false;
      if (timeout) window.clearTimeout(timeout);
      if (interval) window.clearInterval(interval);
    };
  }, [cluster, clusterStats?.api_ready, clusterStats?.last_started_at]);

  const namespaces = useMemo(() => {
    // Pod namespaces alone would hide kube-system from the filter until a pod
    // exists there; the namespace list keeps the picker complete.
    const set = new Set<string>(namespaceList.map((n) => n.name));
    for (const p of pods) set.add(p.namespace);
    return Array.from(set).sort();
  }, [pods, namespaceList]);

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
      if (nsFilter !== "all" && p.namespace !== nsFilter) return false;
      if (!q) return true;
      return (
        p.name.toLowerCase().includes(q) ||
        p.namespace.toLowerCase().includes(q) ||
        p.node.toLowerCase().includes(q) ||
        p.images.some((i) => i.toLowerCase().includes(q))
      );
    });
  }, [pods, nsFilter, podSearch]);

  // Lens-style namespace scope shared by every namespaced tab. Lists stay raw
  // in state and are filtered at render time, so switching the selector never
  // costs a round trip.
  const inScope = (ns: string) => nsFilter === "all" || ns === nsFilter;
  const filteredWorkloads = useMemo(
    () => workloads.filter((w) => inScope(w.namespace)),
    [workloads, nsFilter]
  );
  const filteredEvents = useMemo(
    () => events.filter((e) => inScope(e.namespace)),
    [events, nsFilter]
  );
  const filteredServices = useMemo(
    () => services.filter((s) => inScope(s.namespace)),
    [services, nsFilter]
  );
  const filteredConfigs = useMemo(
    () => configs.filter((c) => inScope(c.namespace)),
    [configs, nsFilter]
  );
  const filteredPvcs = useMemo(
    () => pvcs.filter((p) => inScope(p.namespace)),
    [pvcs, nsFilter]
  );
  const filteredIngresses = useMemo(
    () => ingresses.filter((i) => inScope(i.namespace)),
    [ingresses, nsFilter]
  );

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
      api.getClusterMetrics(cluster).then(forCluster(setMetrics)).catch(() => {});
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
    // Only the newest request may write: switching pods (or clusters) while
    // an older, slower fetch is in flight used to show the wrong pod's logs.
    const seq = ++logsSeq.current;
    setLogsLoading(true);
    setLogsError(null);
    try {
      const text = workloadKind
        ? await api.getWorkloadLogs(cluster, workloadKind, ns, name, tail)
        : await api.getPodLogs(cluster, ns, name, tail);
      if (seq === logsSeq.current) setLogsText(text);
    } catch (e) {
      if (seq === logsSeq.current) {
        setLogsText("");
        setLogsError(String(e));
      }
    } finally {
      if (seq === logsSeq.current) setLogsLoading(false);
    }
  };

  useEffect(() => {
    if (logsFor) loadLogs(logsFor.ns, logsFor.name, 300, logsFor.workloadKind);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [logsFor?.ns, logsFor?.name, logsFor?.workloadKind, cluster]);

  /** Opens the YAML or events viewer for a resource. */
  const openResourceView = (
    kind: string,
    ns: string | null,
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
  }, [yamlFor?.kind, yamlFor?.ns, yamlFor?.name, cluster, yamlReload]);

  useEffect(() => {
    if (!eventsFor) return;
    let alive = true;
    setEventsLoading(true);
    setEventsError(null);
    setEventsList([]);
    api
      .getResourceEvents(cluster, eventsFor.ns ?? "", eventsFor.name)
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

  /** Opens a port-forward on one service port; asks first when there are several. */
  const forwardService = async (s: K8sServiceInfo) => {
    const ports = servicePorts(s.ports);
    if (ports.length === 0) {
      toast.info("No ports", `${s.namespace}/${s.name} exposes no ports.`);
      return;
    }
    let port = ports[0];
    if (ports.length > 1) {
      const pick = await modal.showChoice(
        `Forward which port of service '${s.name}'?`,
        ports.map((p) => ({ value: String(p), label: `Port ${p}` })),
        "Forward service",
        "info"
      );
      if (pick === null) return;
      port = Number(pick);
    }
    setForwardFor({ kind: "svc", ns: s.namespace, name: s.name, port });
  };

  /** Reads the pod's container ports from its YAML, then opens a port-forward. */
  const forwardPod = async (p: K8sPodInfo) => {
    if (podForwardBusy) return;
    setPodForwardBusy(`${p.namespace}/${p.name}`);
    let ports: ContainerPort[] = [];
    try {
      const yaml = await api.getResourceYaml(cluster, "pod", p.namespace, p.name);
      ports = parsePodContainerPorts(yaml);
    } catch (e) {
      toast.error("Forward failed", String(e));
      return;
    } finally {
      setPodForwardBusy(null);
    }
    if (ports.length === 0) {
      toast.info("No container ports", `${p.name} declares no containerPort.`);
      return;
    }
    let chosen = ports[0];
    if (ports.length > 1) {
      const pick = await modal.showChoice(
        `Forward which container port of pod '${p.name}'?`,
        ports.map((cp, i) => ({
          value: String(i),
          label: `${cp.container} ${cp.port}/${cp.protocol}`,
        })),
        "Forward pod",
        "info"
      );
      if (pick === null) return;
      chosen = ports[Number(pick)];
    }
    setForwardFor({ kind: "pod", ns: p.namespace, name: p.name, port: chosen.port });
  };

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
      api.getClusterPods(cluster).then(forCluster(setPods)).catch(() => {});
    } catch (e) {
      toast.error("Delete failed", String(e));
    }
  };

  const handleDeleteResource = async (
    kind: string,
    ns: string | null,
    name: string,
    label: string
  ) => {
    const target = ns ? `${ns}/${name}` : name;
    const ok = await modal.showConfirm(
      `Delete ${label} '${target}'?`,
      `Delete ${label}`,
      "danger"
    );
    if (!ok) return;
    try {
      await api.deleteK8sResource(cluster, kind, ns, name);
      toast.success(`${label} deleted`, target);
      slowRefresh();
    } catch (e) {
      toast.error("Delete failed", String(e));
    }
  };

  const handleRestart = async (w: K8sWorkloadInfo) => {
    const key = workloadKey(w.kind, w.namespace, w.name);
    if (!claimWorkload(key)) return;
    try {
      await api.restartWorkload(cluster, w.kind.toLowerCase(), w.namespace, w.name);
      toast.success("Rolling restart started", `${w.kind} ${w.namespace}/${w.name}`);
    } catch (e) {
      toast.error("Restart failed", String(e));
    } finally {
      releaseWorkload(key);
    }
  };

  const handleScaleSubmit = async () => {
    if (!scaleFor) return;
    const key = workloadKey(scaleFor.kind, scaleFor.ns, scaleFor.name);
    if (!claimWorkload(key)) return;
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
      api.getClusterWorkloads(cluster).then(forCluster(setWorkloads)).catch(() => {});
    } catch (e) {
      toast.error("Scale failed", String(e));
    } finally {
      releaseWorkload(key);
    }
  };

  // Kubernetes spells this `spec.unschedulable`; the UI uses the familiar
  // Cordon/Uncordon wording so it matches what `kubectl cordon` reports.
  const handleToggleCordon = async (n: K8sNodeInfo) => {
    if (nodeBusy) return;
    const cordon = !n.unschedulable;
    setNodeBusy(n.name);
    try {
      await api.setNodeSchedulable(cluster, n.name, !cordon);
      toast.success(cordon ? "Node cordoned" : "Node uncordoned", n.name);
      api.getClusterNodes(cluster).then(forCluster(setNodes)).catch(() => {});
    } catch (e) {
      toast.error(cordon ? "Cordon failed" : "Uncordon failed", String(e));
    } finally {
      setNodeBusy(null);
    }
  };

  const handleDrain = async (n: K8sNodeInfo) => {
    if (nodeBusy) return;
    const ok = await modal.showConfirm(
      `Drain '${n.name}'? Its pods are evicted, then the node is cordoned so nothing new is scheduled.`,
      "Drain Node",
      "danger"
    );
    if (!ok) return;
    setNodeBusy(n.name);
    try {
      const out = await api.drainNode(cluster, n.name);
      toast.success("Node drained", n.name);
      // kubectl warns about unmanaged pods on stderr; keep that visible in the
      // dev console without turning a completed drain into an error toast.
      if (out.trim()) console.info("drain:", out.trim());
      api.getClusterNodes(cluster).then(forCluster(setNodes)).catch(() => {});
      api.getClusterPods(cluster).then(forCluster(setPods)).catch(() => {});
    } catch (e) {
      toast.error("Drain failed", String(e));
    } finally {
      setNodeBusy(null);
    }
  };

  // Worker-add output (kind-progress, action "node") for this cluster only.
  useEffect(() => {
    return disposeListener(
      apiEvents.onKindProgress((p) => {
        if (p.action === "node" && p.name === cluster) setWorkerLine(p.line);
      })
    );
  }, [cluster]);

  // Control planes are never offered for removal.
  const isControlPlane = (n: K8sNodeInfo) =>
    n.roles.some((r) => r === "control-plane" || r === "master");

  const refreshNodes = () => api.getClusterNodes(cluster).then(forCluster(setNodes)).catch(() => {});

  const handleAddWorker = async () => {
    if (addingWorker || nodeBusy) return;
    setAddingWorker(true);
    setWorkerLine(`Adding a worker to ${cluster}…`);
    try {
      const node = await api.addClusterWorker(cluster);
      toast.success("Worker added", node);
      refreshNodes();
    } catch (e) {
      toast.error("Add worker failed", String(e));
    } finally {
      setAddingWorker(false);
      setWorkerLine(null);
    }
  };

  const handleRemoveNode = async (n: K8sNodeInfo) => {
    if (nodeBusy || addingWorker) return;
    const ok = await modal.showConfirm(
      `Remove worker '${n.name}'? Its pods are evicted and rescheduled onto the other nodes, then the node container is deleted.`,
      "Remove Worker Node",
      "danger"
    );
    if (!ok) return;
    setNodeBusy(n.name);
    try {
      await api.removeClusterNode(cluster, n.name);
      toast.success("Worker removed", n.name);
      refreshNodes();
      api.getClusterPods(cluster).then(forCluster(setPods)).catch(() => {});
    } catch (e) {
      toast.error("Remove failed", String(e));
    } finally {
      setNodeBusy(null);
    }
  };

  const handleCreateNamespace = async () => {
    const input = await modal.showPrompt(
      "Namespace name (DNS-1123 label: lowercase letters, digits and dashes)",
      "",
      "Create Namespace"
    );
    if (input === null) return;
    const name = input.trim();
    if (!/^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/.test(name) || name.length > 63) {
      toast.error("Invalid name", "Use lowercase letters, digits and dashes.");
      return;
    }
    try {
      await api.createNamespace(cluster, name);
      toast.success("Namespace created", name);
      api.getClusterNamespaces(cluster).then(forCluster(setNamespaceList)).catch(() => {});
    } catch (e) {
      toast.error("Create failed", String(e));
    }
  };

  const handleDeleteNamespace = async (n: K8sNamespaceInfo) => {
    // Deleting a namespace cascades to everything inside it, so this prompt
    // spells that out instead of reusing the generic resource wording.
    const ok = await modal.showConfirm(
      `Delete namespace '${n.name}'? Every resource inside it is deleted.`,
      "Delete Namespace",
      "danger"
    );
    if (!ok) return;
    try {
      await api.deleteK8sResource(cluster, "namespace", null, n.name);
      toast.success("Namespace deleted", n.name);
      slowRefresh();
    } catch (e) {
      toast.error("Delete failed", String(e));
    }
  };

  const slowRefresh = () => {
    api.getClusterNodes(cluster).then(forCluster(setNodes)).catch(() => {});
    api.getClusterEvents(cluster).then(forCluster(setEvents)).catch(() => {});
    api.getClusterWorkloads(cluster).then(forCluster(setWorkloads)).catch(() => {});
    api.getClusterServices(cluster).then(forCluster(setServices)).catch(() => {});
    api.getClusterConfigResources(cluster).then(forCluster(setConfigs)).catch(() => {});
    api.getClusterPvcs(cluster).then(forCluster(setPvcs)).catch(() => {});
    api.getClusterIngresses(cluster).then(forCluster(setIngresses)).catch(() => {});
    api.getClusterPvs(cluster).then(forCluster(setPvs)).catch(() => {});
    api.getClusterNamespaces(cluster).then(forCluster(setNamespaceList)).catch(() => {});
    api.getClusterPods(cluster).then(forCluster(setPods)).catch(() => {});
    api.getClusterSummary(cluster).then(forCluster(setSummary)).catch(() => {});
  };

  const cpuPct = totalCpuCap > 0 ? (totalCpuUsed / totalCpuCap) * 100 : 0;
  const memPct = totalMemCap > 0 ? (totalMemUsed / totalMemCap) * 100 : 0;
  // Usage is missing only when neither source produced data — one failing
  // query must not blank the other's numbers.
  const noMetrics =
    !metrics?.nodes.length &&
    !metrics?.pods.length &&
    (!!metrics?.error || (msStatus?.installed === true && !msStatus.top_works));

  // Fallback numbers from the cluster card (docker stats over the node
  // containers). The Metrics API needs metrics-server; docker stats always
  // works, so Detail shows live usage even before the API catches up.
  const hostCpuPct = clusterStats?.cpu_percent ?? null;
  const fallbackMemBytes =
    clusterStats?.mem_used_mb != null
      ? clusterStats.mem_used_mb * 1024 * 1024
      : null;
  // Same denominator as the CPU tile's "% of host": the machine's RAM, not
  // the node cgroups. Falls back to the summed node capacity when unknown.
  const hostMemTotalBytes =
    clusterStats?.host_mem_total_mb != null && clusterStats.host_mem_total_mb > 0
      ? clusterStats.host_mem_total_mb * 1024 * 1024
      : null;

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
                  summary.ready ? "bg-emerald-400" : "bg-rose-400"
                }`}
              />
              {summary.ready ? "healthy" : "degraded"} · {summary.gitVersion || "?"}
            </span>
          )}
          <div className="flex items-center gap-1.5">
            <Button
              variant="outline"
              size="sm"
              title="Apply a YAML manifest to this cluster"
              className="h-7 gap-1.5 px-2.5 text-[11px]"
              onClick={() => setApplyFor({ tab: "yaml" })}
            >
              <FileCode2 className="h-3.5 w-3.5" />
              Apply YAML
            </Button>
            <Button
              variant="outline"
              size="sm"
              title="Generate a Deployment and Service and apply them"
              className="h-7 gap-1.5 px-2.5 text-[11px]"
              onClick={() => setApplyFor({ tab: "quick" })}
            >
              <Plus className="h-3.5 w-3.5" />
              Quick deploy
            </Button>
            <Button
              variant="outline"
              size="sm"
              title="Open a shell in the control-plane node with kubectl ready"
              className="h-7 gap-1.5 px-2.5 text-[11px]"
              onClick={() => setKubeShellOpen(true)}
            >
              <Terminal className="h-3.5 w-3.5" />
              kubectl shell
            </Button>
          </div>
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
      <div className="flex items-center gap-1 px-4 pt-2 border-b border-zinc-800/60 overflow-x-auto">
        {TABS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className={`flex shrink-0 whitespace-nowrap items-center gap-1.5 px-3 py-2 text-xs font-medium rounded-t-lg transition-colors ${
              tab === id
                ? "text-white border-b-2 border-indigo-400 bg-zinc-800/40"
                : "text-zinc-400 hover:text-zinc-200"
            }`}
          >
            <Icon className="h-3.5 w-3.5" />
            {label}
            {id === "events" && warnings.length > 0 && (
              <span className="ml-0.5 rounded-none bg-rose-500/20 px-1.5 text-[10px] text-rose-300">
                {events.filter((e) => e.type === "Warning").length}
              </span>
            )}
          </button>
        ))}
        {(tab === "podlist" ||
          tab === "workloads" ||
          tab === "services" ||
          tab === "config" ||
          tab === "pvcs" ||
          tab === "events") && (
          <select
            value={nsFilter}
            onChange={(e) => setNsFilter(e.target.value)}
            title="Filter by namespace"
            aria-label="Filter by namespace"
            className="ml-auto mb-1 h-7 max-w-52 rounded-lg bg-zinc-950 border border-zinc-800 px-2 text-[11px] text-zinc-200 focus:outline-none focus:border-indigo-500"
          >
            <option value="all">All namespaces ({pods.length} pods)</option>
            {namespaces.map((ns) => (
              <option key={ns} value={ns}>
                {ns} ({pods.filter((p) => p.namespace === ns).length})
              </option>
            ))}
          </select>
        )}
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
                {noMetrics && msStatus && (
                  <div className="flex flex-wrap items-center justify-end gap-2">
                    <span
                      className={`flex items-center gap-1.5 text-[11px] ${
                        msStatus.installed ? "text-amber-300" : "text-zinc-500"
                      }`}
                    >
                      {msStatus.installed ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : (
                        <AlertTriangle className="h-3 w-3" />
                      )}
                      {msStatus.installed
                        ? `metrics-server installed — ${
                            msStatus.top_works
                              ? "warming up"
                              : msStatus.ready
                                ? "waiting for usage data"
                                : "not ready yet"
                          }`
                        : "metrics-server not installed"}
                    </span>
                    <button
                      onClick={installMetricsServer}
                      disabled={installingMs}
                      title={msStatus.message ?? undefined}
                      className="flex items-center gap-1.5 rounded-lg border border-indigo-500/40 bg-indigo-500/10 px-2.5 py-1 text-[11px] font-medium text-indigo-300 hover:bg-indigo-500/20 disabled:opacity-60"
                    >
                      {installingMs ? (
                        <Loader2 className="h-3 w-3 animate-spin" />
                      ) : msStatus.installed ? (
                        <RotateCw className="h-3 w-3" />
                      ) : (
                        <Download className="h-3 w-3" />
                      )}
                      {installingMs
                        ? "Repairing…"
                        : msStatus.installed
                          ? "Repair & restart"
                          : "Install metrics-server"}
                    </button>
                  </div>
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
                    capSuffix: "",
                    fallbackText:
                      hostCpuPct != null ? `${hostCpuPct.toFixed(1)}% of host` : null,
                    fallbackPct: hostCpuPct,
                  },
                  {
                    label: "Cluster memory",
                    used: totalMemUsed,
                    // Report memory against the host's RAM, matching the CPU
                    // tile; node capacity is only a fallback.
                    cap: hostMemTotalBytes ?? totalMemCap,
                    pct:
                      hostMemTotalBytes != null
                        ? (totalMemUsed / hostMemTotalBytes) * 100
                        : memPct,
                    fmt: formatBytes,
                    icon: Server,
                    capSuffix: hostMemTotalBytes != null ? " of host" : "",
                    fallbackText:
                      fallbackMemBytes != null
                        ? `${formatBytes(fallbackMemBytes)} / ${
                            hostMemTotalBytes != null
                              ? `${formatBytes(hostMemTotalBytes)} of host`
                              : totalMemCap > 0
                                ? formatBytes(totalMemCap)
                                : "?"
                          }`
                        : null,
                    fallbackPct:
                      fallbackMemBytes != null && (hostMemTotalBytes ?? totalMemCap) > 0
                        ? (fallbackMemBytes / (hostMemTotalBytes ?? totalMemCap)) * 100
                        : null,
                  },
                ] as const
              ).map(
                ({
                  label,
                  used,
                  cap,
                  pct,
                  fmt,
                  icon: Icon,
                  capSuffix,
                  fallbackText,
                  fallbackPct,
                }) => {
                  // Metrics API numbers win; docker-stats numbers fill in
                  // while metrics-server is warming up or absent.
                  const live = (metrics?.nodes.length ?? 0) > 0 && cap > 0;
                  const showPct = live ? pct : fallbackPct;
                  return (
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
                      {live
                        ? `${fmt(used)} / ${fmt(cap)}${capSuffix}`
                        : fallbackText ?? "n/a"}
                    </span>
                  </div>
                  <div className="mt-2">
                    {live || fallbackText != null ? (
                      <>
                        {showPct != null && <UsageBar pct={showPct} />}
                        {showPct != null && (
                          <div className="mt-1 text-right font-mono text-[11px] text-zinc-400">
                            {showPct.toFixed(1)}%
                          </div>
                        )}
                      </>
                    ) : (
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
                    )}
                  </div>
                </div>
                  );
                }
              )}
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
          <div className="space-y-3">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <span className="min-w-0 truncate font-mono text-[11px] text-zinc-500">
              {addingWorker ? workerLine : "Workers can be added or removed; control planes stay."}
            </span>
            <Button
              variant="outline"
              size="sm"
              className="gap-1.5 rounded-none"
              title="Add a worker node to this cluster (takes about a minute)"
              disabled={addingWorker || nodeBusy !== null || !!monitorError}
              isLoading={addingWorker}
              onClick={() => void handleAddWorker()}
            >
              <Plus className="h-3.5 w-3.5" />
              Add worker
            </Button>
          </div>
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
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
                  <th className="px-3 py-2 text-right">Actions</th>
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
                      <td className="px-3 py-2">
                        <div className="flex items-center justify-end gap-1">
                          {n.unschedulable && (
                            <span className="rounded border border-amber-500/30 bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-300">
                              Cordoned
                            </span>
                          )}
                          <Button
                            variant="ghost"
                            size="icon"
                            title={n.unschedulable ? "Uncordon node" : "Cordon node"}
                            disabled={nodeBusy === n.name}
                            onClick={() => void handleToggleCordon(n)}
                          >
                            {nodeBusy === n.name ? (
                              <Loader2 className="h-3.5 w-3.5 animate-spin" />
                            ) : (
                              <Ban className="h-3.5 w-3.5" />
                            )}
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="Drain node"
                            disabled={nodeBusy === n.name}
                            onClick={() => void handleDrain(n)}
                          >
                            <ArrowDownToLine className="h-3.5 w-3.5" />
                          </Button>
                          {!isControlPlane(n) && (
                            <Button
                              variant="ghost"
                              size="icon"
                              title="Remove worker node (its pods are evicted first)"
                              disabled={nodeBusy === n.name || addingWorker}
                              onClick={() => void handleRemoveNode(n)}
                              className="hover:text-rose-400"
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </Button>
                          )}
                        </div>
                      </td>
                    </tr>
                  );
                })}
                {nodes.length === 0 && (
                  <tr>
                    <td colSpan={10} className="px-3 py-8 text-center text-zinc-500">
                      {monitorError ? "Cluster unreachable" : "Loading nodes…"}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
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
              <span className="ml-auto text-[11px] text-zinc-500">
                {filteredPods.length} pods
              </span>
            </div>

            <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
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
                              title="Forward a container port of this pod to localhost"
                              aria-label="Forward pod port"
                              disabled={podForwardBusy !== null}
                              onClick={() => void forwardPod(p)}
                              className="h-6 w-6 text-zinc-400 hover:text-emerald-300 disabled:opacity-40"
                            >
                              {podForwardBusy === `${p.namespace}/${p.name}` ? (
                                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                              ) : (
                                <Cable className="h-3.5 w-3.5" />
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
                              <ScrollText className="h-3.5 w-3.5" />
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
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
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
                {filteredWorkloads.map((w) => {
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
                            disabled={workloadOps.has(workloadKey(w.kind, w.namespace, w.name))}
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
                          {(wk === "deployment" || wk === "statefulset" || wk === "daemonset") && (
                            <Button
                              variant="ghost"
                              size="icon"
                              title="Logs (all pods)"
                              aria-label="Logs for all pods of workload"
                              onClick={() =>
                                setSelectorLogsFor({
                                  ns: w.namespace,
                                  title: `${w.kind}/${w.namespace}/${w.name} · all pods`,
                                  workloadKind: wk,
                                  workloadName: w.name,
                                })
                              }
                              className="h-6 w-6 text-zinc-400 hover:text-emerald-300"
                            >
                              <Layers className="h-3.5 w-3.5" />
                            </Button>
                          )}
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
                            <ScrollText className="h-3.5 w-3.5" />
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
                {filteredWorkloads.length === 0 && (
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
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
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
                {filteredEvents.map((e, i) => (
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
                {filteredEvents.length === 0 && (
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
        {/* ============ ADDONS ============ */}
        {tab === "addons" && (
          <AddonsPanel cluster={cluster} />
        )}

        {/* ============ INGRESS CONTROLLERS ============ */}
        {tab === "ingress" && (
          <IngressPanel cluster={cluster} onOpenAddons={() => setTab("addons")} />
        )}

        {/* ============ HELM ============ */}
        {tab === "helm" && (
          <HelmPanel key={cluster} cluster={cluster} />
        )}

        {/* ============ PORT-FORWARDS ============ */}
        {tab === "forwards" && (
          <PortForwardPanel cluster={cluster} />
        )}

        {/* ============ HISTORY ============ */}
        {tab === "history" && (
          <HistoryPanel cluster={cluster} />
        )}

        {/* ============ ALERTS ============ */}
        {tab === "alerts" && (
          <AlertsPanel cluster={cluster} />
        )}
      </div>

        {/* ============ SERVICES ============ */}
        {tab === "services" && (
          <div className="space-y-5">
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
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
                {filteredServices.map((s) => (
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
                        title={
                          s.selector
                            ? `Logs of all pods behind this service (${s.selector})`
                            : "This service has no pod selector"
                        }
                        aria-label="View logs of the service's pods"
                        disabled={!s.selector}
                        onClick={() =>
                          setSelectorLogsFor({
                            ns: s.namespace,
                            selector: s.selector,
                            title: `Service ${s.name}`,
                          })
                        }
                        className="h-6 w-6 text-zinc-400 hover:text-emerald-300 disabled:opacity-30"
                      >
                        <Layers className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        title="Forward a port of this service to localhost"
                        aria-label="Forward service port"
                        disabled={servicePorts(s.ports).length === 0}
                        onClick={() => void forwardService(s)}
                        className="h-6 w-6 text-zinc-400 hover:text-emerald-300 disabled:opacity-30"
                      >
                        <Cable className="h-3.5 w-3.5" />
                      </Button>
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
                        <ScrollText className="h-3.5 w-3.5" />
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
                {filteredServices.length === 0 && (
                  <tr>
                    <td colSpan={7} className="px-3 py-8 text-center text-zinc-500">
                      {monitorError ? "Cluster unreachable" : "No services."}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>

          <div>
            <div className="mb-2 flex items-center gap-2 px-1">
              <Folder className="h-3.5 w-3.5 text-zinc-500" />
              <h3 className="text-xs font-semibold text-zinc-300">Ingresses</h3>
              <span className="text-[11px] text-zinc-500">{filteredIngresses.length}</span>
            </div>
            <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
              <table className="w-full text-xs">
                <thead>
                  <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                    <th className="px-3 py-2">Namespace</th>
                    <th className="px-3 py-2">Name</th>
                    <th className="px-3 py-2">Class</th>
                    <th className="px-3 py-2">Hosts</th>
                    <th className="px-3 py-2">Address</th>
                    <th className="px-3 py-2">TLS</th>
                    <th className="px-3 py-2">Age</th>
                    <th className="px-3 py-2 text-right">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {filteredIngresses.map((i) => (
                    <tr
                      key={`${i.namespace}/${i.name}`}
                      className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20"
                    >
                      <td className="px-3 py-2 text-zinc-500">{i.namespace}</td>
                      <td className="px-3 py-2 font-mono text-zinc-200">{i.name}</td>
                      <td className="px-3 py-2 text-zinc-400">{i.className || "—"}</td>
                      <td
                        className="px-3 py-2 font-mono text-zinc-300 max-w-64 truncate"
                        title={i.hosts}
                      >
                        {i.hosts || "—"}
                      </td>
                      <td className="px-3 py-2 font-mono text-zinc-400">{i.address || "—"}</td>
                      <td className="px-3 py-2">
                        <span
                          className={`inline-flex rounded-md border px-1.5 py-0.5 text-[10px] ${
                            i.tls
                              ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                              : "border-zinc-700 text-zinc-500"
                          }`}
                        >
                          {i.tls ? "TLS" : "—"}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-zinc-500">
                        {i.created ? formatRelativeTime(i.created) : "-"}
                      </td>
                      <td className="px-3 py-2">
                        <div className="flex items-center justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="icon"
                            title="View YAML"
                            aria-label="View ingress YAML"
                            onClick={() => openResourceView("ingress", i.namespace, i.name)}
                            className="h-6 w-6 text-zinc-400 hover:text-sky-300"
                          >
                            <FileCode2 className="h-3.5 w-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="Delete ingress"
                            aria-label="Delete ingress"
                            onClick={() =>
                              handleDeleteResource("ingress", i.namespace, i.name, "Ingress")
                            }
                            className="h-6 w-6 text-zinc-400 hover:text-rose-400"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  ))}
                  {filteredIngresses.length === 0 && (
                    <tr>
                      <td colSpan={8} className="px-3 py-8 text-center text-zinc-500">
                        {monitorError ? "Cluster unreachable" : "No ingresses."}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>

          <div>
            <h3 className="mb-2 px-1 text-xs font-semibold text-zinc-300">Links</h3>
            <ServiceLinksPanel cluster={cluster} />
          </div>
          </div>
        )}
        {tab === "config" && (
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
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
                {filteredConfigs.map((c) => (
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
                        <ScrollText className="h-3.5 w-3.5" />
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
                {filteredConfigs.length === 0 && (
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
          <div className="space-y-5">
          <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
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
                {filteredPvcs.map((p) => (
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
                {filteredPvcs.length === 0 && (
                  <tr>
                    <td colSpan={8} className="px-3 py-8 text-center text-zinc-500">
                      {monitorError ? "Cluster unreachable" : "No persistent volume claims."}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>

          <div>
            <div className="mb-2 flex items-center gap-2 px-1">
              <Package className="h-3.5 w-3.5 text-zinc-500" />
              <h3 className="text-xs font-semibold text-zinc-300">Persistent Volumes</h3>
              <span className="text-[11px] text-zinc-500">{pvs.length}</span>
              <span className="text-[10px] text-zinc-600">cluster-scoped</span>
            </div>
            <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
              <table className="w-full text-xs">
                <thead>
                  <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                    <th className="px-3 py-2">Name</th>
                    <th className="px-3 py-2">Capacity</th>
                    <th className="px-3 py-2">Access</th>
                    <th className="px-3 py-2">Reclaim</th>
                    <th className="px-3 py-2">Status</th>
                    <th className="px-3 py-2">Class</th>
                    <th className="px-3 py-2">Claim</th>
                    <th className="px-3 py-2">Age</th>
                    <th className="px-3 py-2 text-right">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {pvs.map((v) => (
                    <tr
                      key={v.name}
                      className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20"
                    >
                      <td className="px-3 py-2 font-mono text-zinc-200">{v.name}</td>
                      <td className="px-3 py-2 font-mono text-zinc-300">{v.capacity || "—"}</td>
                      <td className="px-3 py-2 text-zinc-500">{v.accessModes}</td>
                      <td className="px-3 py-2 text-zinc-400">{v.reclaimPolicy}</td>
                      <td className="px-3 py-2">
                        <span
                          className={`inline-flex rounded-md border px-1.5 py-0.5 text-[10px] ${
                            v.status === "Bound"
                              ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                              : v.status === "Available"
                                ? "border-sky-500/30 bg-sky-500/10 text-sky-300"
                                : "border-zinc-700 text-zinc-400"
                          }`}
                        >
                          {v.status}
                        </span>
                      </td>
                      <td className="px-3 py-2 text-zinc-500">{v.storageClass || "—"}</td>
                      <td className="px-3 py-2 font-mono text-zinc-400">{v.claim || "—"}</td>
                      <td className="px-3 py-2 text-zinc-500">
                        {v.created ? formatRelativeTime(v.created) : "-"}
                      </td>
                      <td className="px-3 py-2">
                        <div className="flex items-center justify-end gap-1">
                          <Button
                            variant="ghost"
                            size="icon"
                            title="View YAML"
                            aria-label="View persistent volume YAML"
                            onClick={() => openResourceView("persistentvolume", null, v.name)}
                            className="h-6 w-6 text-zinc-400 hover:text-sky-300"
                          >
                            <FileCode2 className="h-3.5 w-3.5" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            title="Delete persistent volume"
                            aria-label="Delete persistent volume"
                            onClick={() =>
                              handleDeleteResource("persistentvolume", null, v.name, "PersistentVolume")
                            }
                            className="h-6 w-6 text-zinc-400 hover:text-rose-400"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        </div>
                      </td>
                    </tr>
                  ))}
                  {pvs.length === 0 && (
                    <tr>
                      <td colSpan={9} className="px-3 py-8 text-center text-zinc-500">
                        {monitorError ? "Cluster unreachable" : "No persistent volumes."}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>
          </div>
        )}

        {/* ============ NAMESPACES ============ */}
        {tab === "namespaces" && (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-[11px] text-zinc-500">
                {namespaceList.length} namespaces
              </span>
              <Button
                variant="outline"
                size="sm"
                onClick={() => void handleCreateNamespace()}
                className="ml-auto gap-1.5"
              >
                <Plus className="h-3.5 w-3.5" />
                Create namespace
              </Button>
            </div>
            <div className="overflow-x-auto rounded-xl border border-zinc-800/80 select-text">
              <table className="w-full text-xs">
                <thead>
                  <tr className="border-b border-zinc-800 bg-zinc-900/80 text-left text-[10px] uppercase tracking-wider text-zinc-500">
                    <th className="px-3 py-2">Name</th>
                    <th className="px-3 py-2">Status</th>
                    <th className="px-3 py-2">Age</th>
                    <th className="px-3 py-2 text-right">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {namespaceList.map((n) => {
                    const system = ["default", "kube-system", "kube-public", "kube-node-lease"].includes(
                      n.name
                    );
                    return (
                      <tr
                        key={n.name}
                        className="border-b border-zinc-800/50 last:border-0 hover:bg-zinc-800/20"
                      >
                        <td className="px-3 py-2 font-mono text-zinc-200">{n.name}</td>
                        <td className="px-3 py-2">
                          <span
                            className={`inline-flex rounded-md border px-1.5 py-0.5 text-[10px] ${
                              n.status === "Active"
                                ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                                : "border-amber-500/30 bg-amber-500/10 text-amber-300"
                            }`}
                          >
                            {n.status}
                          </span>
                        </td>
                        <td className="px-3 py-2 text-zinc-500">
                          {n.created ? formatRelativeTime(n.created) : "-"}
                        </td>
                        <td className="px-3 py-2">
                          <div className="flex items-center justify-end gap-1">
                            {system && (
                              <span className="mr-1 text-[10px] text-zinc-600">system</span>
                            )}
                            <Button
                              variant="ghost"
                              size="icon"
                              title="View YAML"
                              aria-label="View namespace YAML"
                              onClick={() => openResourceView("namespace", null, n.name)}
                              className="h-6 w-6 text-zinc-400 hover:text-sky-300"
                            >
                              <FileCode2 className="h-3.5 w-3.5" />
                            </Button>
                            <Button
                              variant="ghost"
                              size="icon"
                              title={system ? "System namespaces cannot be deleted here" : "Delete namespace"}
                              aria-label="Delete namespace"
                              disabled={system}
                              onClick={() => void handleDeleteNamespace(n)}
                              className="h-6 w-6 text-zinc-400 hover:text-rose-400 disabled:opacity-30"
                            >
                              <Trash2 className="h-3.5 w-3.5" />
                            </Button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                  {namespaceList.length === 0 && (
                    <tr>
                      <td colSpan={4} className="px-3 py-8 text-center text-zinc-500">
                        {monitorError ? "Cluster unreachable" : "No namespaces."}
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </div>
        )}
      {/* Pod logs modal */}
      {logsFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
          <div className="flex w-[80vw] h-[90vh] flex-col rounded-2xl border border-zinc-800 bg-zinc-900 shadow-2xl">
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
            <div className="flex-1 overflow-auto bg-zinc-950/60 px-4 py-3 select-text">
              {logsLoading ? (
                <p className="text-xs text-zinc-500">Loading logs…</p>
              ) : logsError ? (
                <p className="text-xs text-rose-400">{logsError}</p>
              ) : (
                <pre className="whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed text-zinc-300 select-text">
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
          <div className="w-[80vw] h-[90vh] flex flex-col rounded-xl border border-zinc-800 bg-zinc-900 shadow-2xl">
            <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
              <div className="min-w-0">
                <p className="text-sm font-semibold text-white truncate">
                  {yamlFor.kind} · {yamlFor.name}
                </p>
                <p className="text-[11px] text-zinc-500">
                  {yamlFor.ns ? `namespace ${yamlFor.ns}` : "cluster-scoped"}
                </p>
              </div>
              <div className="flex items-center gap-1">
                <Button
                  variant="outline"
                  size="sm"
                  title="Edit this manifest and apply it"
                  disabled={!yamlText || yamlLoading}
                  onClick={() =>
                    setApplyFor({
                      tab: "yaml",
                      yaml: stripServerFields(yamlText),
                      title: `Edit ${yamlFor.kind}/${yamlFor.name}`,
                    })
                  }
                  className="mr-1 h-7 px-2.5 text-[11px]"
                >
                  Edit
                </Button>
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
            <div className="flex-1 min-h-0 overflow-auto px-4 py-3 select-text">
              {yamlLoading ? (
                <div className="flex items-center gap-2 text-xs text-zinc-500">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" /> Loading manifest…
                </div>
              ) : yamlError ? (
                <p className="text-xs text-rose-400">{yamlError}</p>
              ) : (
                <pre className="whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed text-zinc-300 select-text">
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
          <div className="w-[80vw] h-[90vh] flex flex-col rounded-xl border border-zinc-800 bg-zinc-900 shadow-2xl">
            <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
              <div className="min-w-0">
                <p className="text-sm font-semibold text-white truncate">
                  Events · {eventsFor.kind} {eventsFor.name}
                </p>
                <p className="text-[11px] text-zinc-500">
                  {eventsFor.ns ? `namespace ${eventsFor.ns}` : "cluster-scoped"}
                </p>
              </div>
              <div className="flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="icon"
                  title="Copy events"
                  aria-label="Copy events"
                  disabled={eventsList.length === 0}
                  onClick={async () => {
                    const text = eventsList
                      .map(
                        (e) =>
                          `[${e.type}] ${e.reason} on ${e.object}: ${e.message} (count: ${e.count})`
                      )
                      .join("\n");
                    try {
                      await navigator.clipboard.writeText(text);
                      toast.success("Events copied", `${eventsList.length} event(s) copied.`);
                    } catch (err) {
                      toast.error("Copy failed", String(err));
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
                  aria-label="Close events viewer"
                  onClick={() => setEventsFor(null)}
                  className="h-7 w-7 text-zinc-400"
                >
                  <X className="h-4 w-4" />
                </Button>
              </div>
            </div>
            <div className="flex-1 min-h-0 overflow-auto select-text">
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

      {/* YAML apply / quick deploy */}
      {applyFor && (
        <YamlApplyModal
          cluster={cluster}
          initialYaml={applyFor.yaml}
          initialTab={applyFor.tab}
          title={applyFor.title}
          onClose={() => setApplyFor(null)}
          onApplied={() => {
            slowRefresh();
            setYamlReload((n) => n + 1);
          }}
        />
      )}

      {/* Selector logs (all pods of a workload or a label selector) */}
      {selectorLogsFor && (
        <SelectorLogsModal
          cluster={cluster}
          namespace={selectorLogsFor.ns}
          selector={selectorLogsFor.selector}
          workloadKind={selectorLogsFor.workloadKind}
          workloadName={selectorLogsFor.workloadName}
          title={selectorLogsFor.title}
          onClose={() => setSelectorLogsFor(null)}
        />
      )}

      {kubeShellOpen && (
        <KubeShellModal cluster={cluster} onClose={() => setKubeShellOpen(false)} />
      )}

      {/* Pod terminal modal */}
      {terminalFor && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
          <div className="w-[80vw] h-[90vh] flex flex-col rounded-xl border border-zinc-800 bg-zinc-950 shadow-2xl">
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
              <Button
                variant="primary"
                size="sm"
                onClick={handleScaleSubmit}
                disabled={
                  !!scaleFor &&
                  workloadOps.has(workloadKey(scaleFor.kind, scaleFor.ns, scaleFor.name))
                }
              >
                Scale
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* Port-forward dialog for a service or pod port */}
      {forwardFor && (
        <PortForwardDialog
          cluster={cluster}
          namespace={forwardFor.ns}
          kind={forwardFor.kind}
          name={forwardFor.name}
          remotePort={forwardFor.port}
          onClose={() => setForwardFor(null)}
          onStarted={() => setForwardFor(null)}
        />
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
              .then(forCluster(setSummary))
              .catch((e) => setMonitorError(String(e)));
            api.getClusterNodes(cluster).then(forCluster(setNodes)).catch(() => {});
            api.getClusterPods(cluster).then(forCluster(setPods)).catch(() => {});
            api.getClusterEvents(cluster).then(forCluster(setEvents)).catch(() => {});
            api.getClusterWorkloads(cluster).then(forCluster(setWorkloads)).catch(() => {});
            api.getClusterMetrics(cluster).then(forCluster(setMetrics)).catch(() => {});
          }}
        >
          <RefreshCw className="h-3.5 w-3.5" />
          Refresh now
        </Button>
      </div>
    </section>
  );
};
