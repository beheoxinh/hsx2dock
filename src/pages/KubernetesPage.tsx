import { useModal } from "../components/ui/ModalContext";
import { useToast } from "../components/ui/Toast";
import React, { useEffect, useRef, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Hexagon,
  Plus,
  Trash2,
  Search,
  X,
  Download,
  Copy,
  Loader2,
  RefreshCw,
  AlertTriangle,
  Rocket,
  Play,
  Square,
  Settings2,
  Activity,
  ArrowLeft,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusBadge } from "../components/ui/StatusBadge";
import { Switch } from "../components/ui/Switch";
import { ClusterMonitor } from "../components/k8s/ClusterMonitor";
import { api, disposeListener, events } from "../lib/api";
import { peekDashboardClusters } from "../lib/dashboardStore";
import { formatBytes, formatDuration, formatRelativeTime } from "../lib/utils";
import type {
  KindCluster,
  KindClusterPrefs,
  KindInstallStatus,
  KindProgressPayload,
} from "../types";
import type { DaemonState } from "../types";
import { EngineGate } from "../components/ui/EngineGate";

interface KubernetesPageProps {
  refreshKey?: number;
  /** Engine state — the page renders an EngineGate panel unless Running. */
  daemonState?: DaemonState;
  /** Cluster a tray row asked to open; null opens the cluster list. */
  openCluster?: string | null;
}

/** A single-series inline sparkline (change over time) drawn as a thin line. */
const Sparkline: React.FC<{
  data: number[];
  color: string;
  max?: number;
  className?: string;
}> = ({ data, color, max, className }) => {
  if (data.length < 2) {
    return <div className={`h-8 ${className ?? ""}`} aria-hidden="true" />;
  }
  const hi = max ?? Math.max(...data, 1);
  const lo = Math.min(...data, 0);
  const span = hi - lo || 1;
  const w = 100;
  const h = 32;
  const step = w / (data.length - 1);
  const pts = data.map((v, i) => {
    const x = i * step;
    const y = h - ((v - lo) / span) * h;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });
  return (
    <svg
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="none"
      className={`h-8 w-full ${className ?? ""}`}
      aria-hidden="true"
    >
      <polyline
        points={`0,${h} ${pts.join(" ")} ${w},${h}`}
        fill={color}
        fillOpacity="0.12"
        stroke="none"
      />
      <polyline
        points={pts.join(" ")}
        fill="none"
        stroke={color}
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
};

export const KubernetesPage: React.FC<KubernetesPageProps> = ({
  refreshKey,
  daemonState = "Running",
  openCluster,
}) => {
  const modal = useModal();
  const toast = useToast();
  const [status, setStatus] = useState<KindInstallStatus | null>(null);
  // Seeded from the dashboard cache so the cards paint at once; the first
  // pass below revalidates them.
  const [clusters, setClusters] = useState<KindCluster[]>(peekDashboardClusters);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  // kind binary install state (driven by kind-install-progress events).
  const [installing, setInstalling] = useState(false);
  const [installPct, setInstallPct] = useState<number | null>(null);
  const [installDetail, setInstallDetail] = useState<string | null>(null);

  // Create-cluster modal + form.
  const [createOpen, setCreateOpen] = useState(false);
  const [formName, setFormName] = useState("");
  const [formImage, setFormImage] = useState("");
  const [formNodes, setFormNodes] = useState(1);
  /** Defaults from Settings → Kubernetes, re-applied after every create. */
  const [formDefaults, setFormDefaults] = useState({ nodes: 1, image: "" });
  const [creating, setCreating] = useState(false);

  // One long-running kind operation at a time (create/delete/kubeconfig).
  const [busyOp, setBusyOp] = useState<string | null>(null);
  const [logLines, setLogLines] = useState<KindProgressPayload[]>([]);
  const logRef = useRef<HTMLDivElement>(null);

  // Per-cluster mini-monitor history for the card sparklines (capped ring).
  const [history, setHistory] = useState<
    Record<string, { pods: number[]; cpu: number[]; mem: number[] }>
  >({});

  // Full monitoring screen for the selected cluster (null = cluster list).
  const [detailFor, setDetailFor] = useState<string | null>(null);
  const detailCluster = clusters.find((c) => c.name === detailFor) ?? null;

  // A tray cluster row sets `openCluster`; open that monitor. Keeping the name
  // in app state also covers the page mounting after the event fired.
  useEffect(() => {
    if (openCluster) setDetailFor(openCluster);
  }, [openCluster]);

  // Per-cluster prefs (auto-start, per-node caps) mirrored from the config.
  const [prefsByName, setPrefsByName] = useState<Record<string, KindClusterPrefs>>({});
  const [prefsEdit, setPrefsEdit] = useState<
    (KindClusterPrefs & { name: string; isNew?: boolean }) | null
  >(null);
  const [prefsSaving, setPrefsSaving] = useState(false);
  // Master switch: start auto-start clusters together with the engine.
  const [engineAutoStart, setEngineAutoStart] = useState(false);
  const [autoStartBusy, setAutoStartBusy] = useState(false);
  // Toggles stay inert until the persisted prefs are read, so the first click
  // can never write back a stale `false` over an enabled cluster.
  const [prefsLoaded, setPrefsLoaded] = useState(false);
  const loadPrefs = async () => {
    try {
      const cfg = await api.getAppConfig();
      setPrefsByName(cfg.kind_cluster_prefs ?? {});
      setEngineAutoStart(cfg.kind_auto_start ?? false);
    } catch (e) {
      console.error(e);
    } finally {
      setPrefsLoaded(true);
    }
  };
  const savePrefs = async () => {
    if (!prefsEdit) return;
    const trimmed = {
      autoStart: prefsEdit.autoStart,
      nodeCpuM: Math.max(0, Math.round(prefsEdit.nodeCpuM || 0)),
      nodeMemoryMb: Math.max(0, Math.round(prefsEdit.nodeMemoryMb || 0)),
    };
    try {
      setPrefsSaving(true);
      await api.setClusterPrefs(prefsEdit.name, trimmed);
      setPrefsByName((p) => ({ ...p, [prefsEdit.name]: trimmed }));
      toast.success("Cluster prefs saved", prefsEdit.name);
      setPrefsEdit(null);
      await loadAll();
    } catch (e) {
      toast.error("Could not save prefs", String(e));
    } finally {
      setPrefsSaving(false);
    }
  };
  const prefsFor = (name: string): KindClusterPrefs =>
    prefsByName[name] ?? { autoStart: false, nodeCpuM: 0, nodeMemoryMb: 0 };

  /** Master auto-start switch: engine up → start the marked clusters. */
  const toggleEngineAutoStart = async (next: boolean) => {
    try {
      setAutoStartBusy(true);
      await api.patchAppConfig({ kind_auto_start: next });
      setEngineAutoStart(next);
      const marked = Object.values(prefsByName).filter((p) => p.autoStart).length;
      toast.success(
        next ? "Auto-start enabled" : "Auto-start disabled",
        next
          ? `${marked} cluster${marked === 1 ? "" : "s"} marked auto-start will start with the engine.`
          : "Clusters will wait for an explicit Start."
      );
    } catch (e) {
      toast.error("Could not update auto-start", String(e));
    } finally {
      setAutoStartBusy(false);
    }
  };

  /** Per-cluster switch on each card (honoured when the master is on). */
  const toggleClusterAutoStart = async (c: KindCluster) => {
    const current = prefsFor(c.name);
    const next = { ...current, autoStart: !current.autoStart };
    try {
      await api.setClusterPrefs(c.name, next);
      setPrefsByName((p) => ({ ...p, [c.name]: next }));
      toast.success(
        next.autoStart ? "Auto-start on" : "Auto-start off",
        next.autoStart
          ? engineAutoStart
            ? `${c.name} will start with the engine.`
            : `${c.name} is marked — turn on the master Auto-start switch too.`
          : `${c.name} will wait for an explicit Start.`
      );
    } catch (e) {
      toast.error("Could not update auto-start", String(e));
    }
  };

  // Each load gets a sequence number: a slow pass that started before a
  // newer one (refresh clicked twice, engine restarted) must not overwrite it.
  const loadSeq = useRef(0);
  const loadAll = async () => {
    const seq = ++loadSeq.current;
    setLoading(true);
    // Install status is a local file read; show it without waiting for the
    // cluster pass, which talks to Docker and every API server.
    api
      .kindStatus()
      .then((st) => seq === loadSeq.current && setStatus(st))
      .catch(() => {});
    try {
      const list = await api.listKindClusters();
      if (seq !== loadSeq.current) return;
      setClusters(list);
      setLoadError(null);
    } catch (e) {
      if (seq === loadSeq.current) setLoadError(String(e));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
    // Create/delete add or drop per-cluster prefs.
    loadPrefs();
  };

  useEffect(() => {
    loadAll();
  }, [refreshKey]);

  // Silent cluster refresh (no loading spinner) for the background card poll.
  const refreshInflight = useRef(false);
  const refreshClusters = async () => {
    // A slow kind/docker answer must not stack polls on top of each other.
    if (refreshInflight.current) return;
    refreshInflight.current = true;
    const seq = loadSeq.current;
    try {
      const list = await api.listKindClusters();
      // A full load started meanwhile owns the list now.
      if (seq !== loadSeq.current) return;
      setClusters(list);
      setLoadError(null);
    } catch {
      /* keep last-known list; cards simply stop updating until the next poll */
    } finally {
      refreshInflight.current = false;
    }
  };

  // Running-cluster cards need fresh uptime/pods/usage — poll gently so the
  // stats and sparklines stay alive without user interaction.
  const hasRunning = clusters.some((c) => c.running);
  useEffect(() => {
    if (!hasRunning) return;
    const t = window.setInterval(() => {
      // Nobody is looking at a hidden window: skip the docker round-trips.
      if (document.hidden) return;
      refreshClusters();
    }, 10_000);
    return () => window.clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasRunning]);

  // Track per-cluster samples for the card sparklines (capped ring buffer).
  useEffect(() => {
    setHistory((prev) => {
      // Rebuilt from the live list, so a deleted cluster's samples are dropped
      // instead of accumulating for the lifetime of the page.
      const next: typeof prev = {};
      for (const c of clusters) {
        if (prev[c.name]) next[c.name] = prev[c.name];
      }
      for (const c of clusters) {
        if (!c.running || c.pod_count == null) continue;
        const cur = next[c.name] ?? { pods: [], cpu: [], mem: [] };
        next[c.name] = {
          pods: [...cur.pods, c.running_pod_count ?? 0].slice(-40),
          cpu: [...cur.cpu, c.cpu_percent ?? 0].slice(-40),
          mem: [...cur.mem, c.mem_used_mb ?? 0].slice(-40),
        };
      }
      return next;
    });
  }, [clusters]);

  // Pre-fill the create form from Settings → Kubernetes defaults.
  useEffect(() => {
    api
      .getAppConfig()
      .then((cfg) => {
        const d = {
          nodes: Math.min(5, Math.max(1, cfg.kind_default_nodes ?? 1)),
          image: cfg.kind_default_image ?? "",
        };
        setFormDefaults(d);
        setFormNodes(d.nodes);
        setFormImage(d.image);
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    const unlistenProgress = disposeListener(events.onKindProgress((payload) => {
      setLogLines((prev) => [...prev.slice(-199), payload]);
    }));

    const unlistenInstall = disposeListener(events.onKindInstallProgress((payload) => {
      if (payload.error) {
        setInstalling(false);
        setInstallPct(null);
        setInstallDetail(null);
        toast.error("kind install failed", payload.error);
        return;
      }
      if (payload.fraction != null) {
        setInstallPct(Math.min(100, Math.round(payload.fraction * 100)));
      }
      if (payload.detail) {
        setInstallDetail(payload.detail);
      }
      if (payload.done) {
        setInstalling(false);
        setInstallPct(100);
        toast.success("kind installed", "The isolated kind binary is ready.");
        api.kindStatus().then(setStatus).catch(() => {});
      }
    }));

    return () => {
      unlistenProgress();
      unlistenInstall();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Keep the live log pinned to the bottom while kind streams output.
  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [logLines]);

  const handleInstall = async () => {
    try {
      setInstalling(true);
      setInstallPct(0);
      setInstallDetail(null);
      await api.installKind();
    } catch (e) {
      setInstalling(false);
      setInstallPct(null);
      setInstallDetail(null);
      toast.error("kind install failed", String(e));
    }
  };

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    const name = formName.trim();
    if (!name) return;
    if (busyOp) return;
    try {
      setCreating(true);
      setBusyOp("create");
      setLogLines([
        { action: "create", name, line: `Creating cluster ${name}…` },
      ]);
      await api.createKindCluster(
        name,
        formImage.trim() || undefined,
        formNodes
      );
      setCreateOpen(false);
      setFormName("");
      setFormImage(formDefaults.image);
      setFormNodes(formDefaults.nodes);
      toast.success("Cluster ready", `kind cluster '${name}' is up.`);
      await loadAll();
    } catch (err) {
      toast.error("Create failed", String(err));
    } finally {
      setCreating(false);
      setBusyOp(null);
    }
  };

  const handleDelete = async (name: string) => {
    const confirmed = await modal.showConfirm(
      `Delete kind cluster '${name}' and all of its node containers?`,
      "Delete Cluster",
      "danger"
    );
    if (!confirmed || busyOp) return;
    try {
      setBusyOp("delete");
      setLogLines([{ action: "delete", name, line: `Deleting cluster ${name}…` }]);
      await api.deleteKindCluster(name);
      setLogLines((prev) => prev.filter((l) => l.name !== name));
      toast.success("Cluster deleted", `kind cluster '${name}' removed.`);
      await loadAll();
    } catch (err) {
      toast.error("Delete failed", String(err));
    } finally {
      setBusyOp(null);
    }
  };

  const handleKubeconfig = async (name: string) => {
    if (busyOp) return;
    try {
      setBusyOp("kubeconfig");
      const path = await api.getKindKubeconfig(name);
      try {
        await navigator.clipboard.writeText(path);
        toast.success(
          "Kubeconfig path copied",
          `Use: KUBECONFIG=${path} kubectl get nodes`
        );
      } catch {
        toast.info("Kubeconfig ready", path);
      }
      await loadAll();
    } catch (err) {
      toast.error("Kubeconfig failed", String(err));
    } finally {
      setBusyOp(null);
    }
  };

  const handleTogglePower = async (c: KindCluster) => {
    if (busyOp) return;
    try {
      setBusyOp("power");
      setLogLines([
        {
          action: c.running ? "stop" : "start",
          name: c.name,
          line: `${c.running ? "Stopping" : "Starting"} cluster ${c.name}…`,
        },
      ]);
      if (c.running) {
        await api.stopKindCluster(c.name);
        toast.success("Cluster stopped", `'${c.name}' nodes are stopped — data is kept.`);
      } else {
        await api.startKindCluster(c.name);
        toast.success("Cluster started", `'${c.name}' is running again.`);
      }
      await loadAll();
    } catch (err) {
      toast.error("Cluster action failed", String(err));
    } finally {
      setBusyOp(null);
    }
  };

  const filtered = clusters.filter((c) =>
    c.name.toLowerCase().includes(search.trim().toLowerCase())
  );

  const commandPreview = (() => {
    const parts = ["kind", "create", "cluster", "--name", formName.trim() || "<name>"];
    if (formImage.trim()) parts.push("--image", formImage.trim());
    if (formNodes > 1) parts.push("--wait", "120s", "--nodes", String(formNodes));
    return parts.join(" ");
  })();

  const showLog = logLines.length > 0;
  // Kind output lines whose cluster has no card yet (being created).
  const orphanLogLines = logLines.filter(
    (l) => !clusters.some((c) => c.name === l.name)
  );

  // Engine off → clusters are unreachable by definition; show the designed
  // engine panel (with a start button) instead of empty lists and fetch errors.
  if (daemonState !== "Running") {
    return (
      <EngineGate state={daemonState} resource="Kubernetes clusters">
        {null}
      </EngineGate>
    );
  }

  return (
    <div className="p-8 space-y-6 max-w-7xl mx-auto h-full flex flex-col overflow-y-auto">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:flex-wrap sm:items-center justify-between gap-4">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">Kubernetes</h2>
          <p className="text-sm text-zinc-400 mt-1">
            kind clusters — Kubernetes IN Docker, isolated in-app binary, always
            running on this app's bundled engine
          </p>
        </div>

        <div className="flex items-center gap-3">
          <Switch
            checked={engineAutoStart}
            onChange={toggleEngineAutoStart}
            disabled={autoStartBusy || !prefsLoaded}
            label="Auto-start"
            title="Start clusters marked auto-start when the Docker engine comes up (staggered by the delay in Settings)"
          />

          {status?.installed && (
            <span
              title={status.path ?? undefined}
              className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-lg bg-emerald-500/10 border border-emerald-500/30 text-emerald-400 text-xs font-medium"
            >
              <Hexagon className="w-3.5 h-3.5" />
              kind v{status.version ?? status.latest}
            </span>
          )}

          <Button
            variant="ghost"
            size="icon"
            onClick={loadAll}
            title="Refresh"
            className="text-zinc-400 hover:text-white hover:bg-zinc-800"
          >
            <RefreshCw className={loading ? "w-4 h-4 animate-spin" : "w-4 h-4"} />
          </Button>

          {status?.installed && (
            <Button
              variant="primary"
              size="sm"
              onClick={() => setCreateOpen(true)}
              disabled={!!busyOp}
              className="gap-2"
            >
              <Plus className="w-4 h-4" />
              <span>Create Cluster</span>
            </Button>
          )}
        </div>
      </div>

      {/* Engine / load problems */}
      {loadError && (
        <div className="flex items-center gap-2.5 px-4 py-3 rounded-xl bg-amber-500/10 border border-amber-500/30 text-sm text-amber-300">
          <AlertTriangle className="w-4 h-4 shrink-0" />
          <span>{loadError}</span>
        </div>
      )}

      {/* kind binary not installed */}
      {status && !status.installed && (
        <Card className="p-6">
          <div className="flex flex-col sm:flex-row sm:items-center gap-4">
            <div className="flex-1">
              <h3 className="text-sm font-semibold text-white">
                {status.supported
                  ? "kind binary is not installed yet"
                  : `kind has no published binary for this platform`}
              </h3>
              <p className="text-xs text-zinc-400 mt-1">
                {status.supported
                  ? `Installs pinned v${status.latest} into this app's own data directory, checksum-verified before first run — your system PATH and ~/.kube are never touched.`
                  : "Clusters need the kind binary; supported platforms are Linux (amd64/arm64), macOS (amd64/arm64) and Windows (amd64)."}
              </p>
              {installing && (
                <div className="mt-3 max-w-md">
                  <div className="flex justify-between text-[11px] text-zinc-500 mb-1 font-mono">
                    <span>{installDetail ?? "Starting…"}</span>
                    <span>{installPct !== null ? `${installPct}%` : "…"}</span>
                  </div>
                  <div className="h-1.5 rounded-full bg-zinc-800 overflow-hidden">
                    <div
                      className="h-full bg-blue-500 transition-all duration-200"
                      style={{ width: `${installPct ?? 5}%` }}
                    />
                  </div>
                </div>
              )}
            </div>
            {status.supported && (
              <Button
                variant="primary"
                size="sm"
                onClick={handleInstall}
                disabled={installing}
                className="gap-2 self-start sm:self-center"
              >
                {installing ? (
                  <Loader2 className="w-4 h-4 animate-spin" />
                ) : (
                  <Download className="w-4 h-4" />
                )}
                <span>{installing ? "Installing…" : `Install kind v${status.latest}`}</span>
              </Button>
            )}
          </div>
        </Card>
      )}

      {/* Live kind output for each cluster lives inside its card. */}

      {/* Cluster list — hidden while the full monitoring screen is open. */}
      {!detailFor && (
        <>
          {/* Kind output for clusters with no card yet (still being created). */}
          {logLines.some((l) => !clusters.some((c) => c.name === l.name)) && (
            <div className="rounded-xl border border-zinc-800 bg-zinc-950 overflow-hidden">
              <div className="flex items-center justify-between px-3.5 py-2 border-b border-zinc-800/80 bg-zinc-900/50">
                <span className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
                  kind output —{" "}
                  {orphanLogLines[0]?.name ?? "cluster"}
                </span>
                <div className="flex items-center gap-3">
                  <button
                    onClick={async () => {
                      const text = orphanLogLines.map((l) => l.line).join("\n");
                      if (!text) return;
                      try {
                        await navigator.clipboard.writeText(text);
                        toast.success(
                          "Kind output copied",
                          `${orphanLogLines.length} line${orphanLogLines.length === 1 ? "" : "s"} on the clipboard.`
                        );
                      } catch (e) {
                        toast.error("Copy failed", String(e));
                      }
                    }}
                    className="text-[11px] text-zinc-500 hover:text-zinc-300"
                  >
                    Copy
                  </button>
                  <button
                    onClick={() =>
                      setLogLines((prev) =>
                        prev.filter((l) => clusters.some((c) => c.name === l.name))
                      )
                    }
                    className="text-[11px] text-zinc-500 hover:text-zinc-300"
                  >
                    Clear
                  </button>
                </div>
              </div>
              <div
                ref={logRef}
                className="px-3.5 py-2.5 max-h-44 overflow-y-auto font-mono text-xs leading-relaxed space-y-0.5 select-text"
              >
                {orphanLogLines.map((l, i) => (
                  <div
                    key={i}
                    className={
                      /error|failed|exit status/i.test(l.line)
                        ? "text-rose-400"
                        : "text-zinc-400"
                    }
                  >
                    {l.line}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Search */}
          {clusters.length > 0 && (
        <div className="relative w-64">
          <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
          <input
            type="text"
            placeholder="Search clusters..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full h-9 pl-9 pr-4 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white placeholder-zinc-400 focus:outline-none focus:border-blue-500"
          />
        </div>
      )}

      {/* Cluster grid */}
      <div className="grid grid-cols-1 gap-4">
        <AnimatePresence mode="popLayout">
          {filtered.map((c) => (
            <motion.div
              key={c.name}
              layout
              initial={{ opacity: 0, scale: 0.97 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.95 }}
              transition={{ duration: 0.16 }}
            >
              <Card className="p-5 hover:border-zinc-700 transition-colors">
                <div className="flex items-start justify-between gap-3">
                  <div className="flex items-center gap-2.5 min-w-0">
                    <div className="w-8 h-8 rounded-lg bg-blue-500/10 border border-blue-500/30 flex items-center justify-center shrink-0">
                      <Hexagon className="w-4 h-4 text-blue-400" />
                    </div>
                    <div className="min-w-0">
                      <p className="font-semibold text-white truncate" title={c.name}>
                        {c.name}
                      </p>
                      <div className="flex items-center gap-2 mt-1">
                        <StatusBadge
                          status={c.running ? "Running" : "Stopped"}
                          pulse={c.running}
                        />
                        {c.running && c.api_ready === false && (
                          <span
                            title="The API server is still starting — monitoring opens as soon as /readyz passes"
                            className="text-[10px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-300 border border-amber-500/30"
                          >
                            API starting…
                          </span>
                        )}
                        {c.has_kubeconfig && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-400 border border-zinc-700/60 font-mono">
                            kubeconfig
                          </span>
                        )}
                        {(prefsFor(c.name).nodeCpuM > 0 ||
                          prefsFor(c.name).nodeMemoryMb > 0) && (
                          <span
                            title="Per-node resource caps"
                            className="text-[10px] px-1.5 py-0.5 rounded bg-indigo-500/10 text-indigo-300 border border-indigo-500/30 font-mono"
                          >
                            {prefsFor(c.name).nodeCpuM > 0
                              ? `${(prefsFor(c.name).nodeCpuM / 1000).toFixed(1)} cpu`
                              : ""}
                            {prefsFor(c.name).nodeCpuM > 0 &&
                            prefsFor(c.name).nodeMemoryMb > 0
                              ? " · "
                              : ""}
                            {prefsFor(c.name).nodeMemoryMb > 0
                              ? `${prefsFor(c.name).nodeMemoryMb} MB`
                              : ""}
                          </span>
                        )}
                      </div>
                    </div>
                  </div>
                  <Switch
                    checked={prefsFor(c.name).autoStart}
                    onChange={() => toggleClusterAutoStart(c)}
                    disabled={!!busyOp || !prefsLoaded}
                    label="Auto-start"
                    title="Start this cluster when the Docker engine comes up (the master Auto-start switch above must also be on)"
                    className="mt-0.5 shrink-0"
                  />
                </div>

                <p className="mt-2 text-xs text-zinc-500">
                  {c.running ? "Uptime" : c.last_started_at ? "Last run" : "Age"}{" "}
                  <span className="font-mono text-zinc-300">
                    {c.running
                      ? formatDuration(c.uptime_seconds)
                      : c.last_started_at
                        ? formatRelativeTime(c.last_started_at)
                        : c.created
                          ? formatRelativeTime(c.created)
                          : "—"}
                  </span>
                  <span className="mx-2 text-zinc-700">·</span>
                  Nodes{" "}
                  <span className="font-mono text-zinc-300">
                    {c.running_nodes}/{c.node_count}
                  </span>
                  {c.disk_size_bytes != null && c.disk_size_bytes > 0 && (
                    <>
                      <span className="mx-2 text-zinc-700">·</span>
                      Disk{" "}
                      <span className="font-mono text-zinc-300">
                        {formatBytes(c.disk_size_bytes)}
                      </span>
                    </>
                  )}
                </p>

                {/* Forwarded / Published Ports */}
                {c.published_ports && c.published_ports.length > 0 && (
                  <div className="mt-2.5 flex flex-wrap items-center gap-1.5 text-[11px]">
                    <span className="text-zinc-500 font-medium">Forwarded:</span>
                    {c.published_ports.map((p, idx) => {
                      const hostPart = p.host_port ? `${p.host_ip ? `${p.host_ip}:` : ""}${p.host_port}` : "—";
                      const isHttp = p.protocol === "tcp" && (p.host_port === 80 || p.host_port === 443 || p.host_port >= 1024);
                      const url = isHttp
                        ? `http://${p.host_ip && p.host_ip !== "0.0.0.0" && p.host_ip !== "::" ? p.host_ip : "localhost"}:${p.host_port}`
                        : null;
                      return url ? (
                        <a
                          key={idx}
                          href={url}
                          target="_blank"
                          rel="noreferrer"
                          className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-sky-500/10 border border-sky-500/25 text-sky-300 font-mono text-[10px] hover:bg-sky-500/20 hover:border-sky-500/40 transition-colors"
                          title={`Open ${url} in browser (maps to node :${p.container_port}/${p.protocol})`}
                        >
                          <span>{hostPart}</span>
                          <span className="text-sky-400/60">→</span>
                          <span className="text-zinc-400">{p.container_port}/{p.protocol}</span>
                        </a>
                      ) : (
                        <span
                          key={idx}
                          className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md bg-zinc-800/80 border border-zinc-700/60 text-zinc-300 font-mono text-[10px]"
                          title={`Host port ${hostPart} forwards to node container :${p.container_port}/${p.protocol}`}
                        >
                          <span>{hostPart}</span>
                          <span className="text-zinc-500">→</span>
                          <span className="text-zinc-400">{p.container_port}/{p.protocol}</span>
                        </span>
                      );
                    })}
                  </div>
                )}

                {/* Live section — resource charts (left → right) + logs. */}
                {c.running && (
                  <div className="mt-3 space-y-2.5">
                    <div className="grid grid-cols-1 lg:grid-cols-3 gap-2.5">
                      <div className="rounded-xl border border-zinc-800/80 bg-zinc-950/40 px-3 py-2.5 min-w-0">
                        <p className="text-[11px] text-zinc-500 truncate">
                          Pods running
                          {c.pod_count != null
                            ? ` (${c.running_pod_count ?? 0}/${c.pod_count})`
                            : ""}
                        </p>
                        <p className="text-white text-sm font-mono font-medium">
                          {c.pod_count != null ? c.running_pod_count ?? 0 : "—"}
                        </p>
                        <Sparkline
                          data={history[c.name]?.pods ?? []}
                          color="#34d399"
                          max={Math.max(1, c.pod_count ?? 1)}
                        />
                      </div>
                      <div className="rounded-xl border border-zinc-800/80 bg-zinc-950/40 px-3 py-2.5 min-w-0">
                        <p className="text-[11px] text-zinc-500 truncate">
                          CPU {c.cpu_percent != null ? "% of host" : ""}
                        </p>
                        <p className="text-white text-sm font-mono font-medium">
                          {c.cpu_percent != null ? `${c.cpu_percent.toFixed(1)}%` : "—"}
                        </p>
                        <Sparkline data={history[c.name]?.cpu ?? []} color="#60a5fa" />
                      </div>
                      <div className="rounded-xl border border-zinc-800/80 bg-zinc-950/40 px-3 py-2.5 min-w-0">
                        <p className="text-[11px] text-zinc-500 truncate">
                          Memory
                          {c.mem_used_mb != null && c.mem_limit_mb
                            ? ` of ${Math.round(c.mem_limit_mb)} MiB cap`
                            : ""}
                        </p>
                        <p className="text-white text-sm font-mono font-medium">
                          {c.mem_used_mb != null ? `${Math.round(c.mem_used_mb)} MiB` : "—"}
                        </p>
                        <Sparkline data={history[c.name]?.mem ?? []} color="#a78bfa" />
                      </div>
                    </div>

                    {/* Per-cluster kind output — streams while an operation runs. */}
                    {logLines.some((l) => l.name === c.name) && (
                      <div className="rounded-xl border border-zinc-800 bg-zinc-950 overflow-hidden">
                        <div className="flex items-center justify-between px-3.5 py-2 border-b border-zinc-800/80 bg-zinc-900/50">
                          <span className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
                            kind output — {c.name}
                          </span>
                          <div className="flex items-center gap-3">
                            <button
                              onClick={async () => {
                                const text = logLines
                                  .filter((l) => l.name === c.name)
                                  .map((l) => l.line)
                                  .join("\n");
                                if (!text) return;
                                try {
                                  await navigator.clipboard.writeText(text);
                                  toast.success(
                                    "Kind output copied",
                                    `${logLines.filter((l) => l.name === c.name).length} line(s) on the clipboard.`
                                  );
                                } catch (e) {
                                  toast.error("Copy failed", String(e));
                                }
                              }}
                              className="text-[11px] text-zinc-500 hover:text-zinc-300"
                            >
                              Copy
                            </button>
                            <button
                              onClick={() =>
                                setLogLines((prev) => prev.filter((l) => l.name !== c.name))
                              }
                              className="text-[11px] text-zinc-500 hover:text-zinc-300"
                            >
                              Clear
                            </button>
                          </div>
                        </div>
                        <div
                          ref={logRef}
                          className="px-3.5 py-2.5 max-h-40 overflow-y-auto font-mono text-xs leading-relaxed space-y-0.5 select-text"
                        >
                          {logLines
                            .filter((l) => l.name === c.name)
                            .map((l, i) => (
                              <div
                                key={i}
                                className={
                                  /error|failed|exit status/i.test(l.line)
                                    ? "text-rose-400"
                                    : "text-zinc-400"
                                }
                              >
                                {l.line}
                              </div>
                            ))}
                        </div>
                      </div>
                    )}
                  </div>
                )}

                <div className="mt-4 flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setDetailFor(c.name)}
                    disabled={
                      !!busyOp || !c.running || c.api_ready === false
                    }
                    title={
                      !c.running
                        ? "Start the cluster to open monitoring"
                        : c.api_ready === false
                          ? "Waiting for the API server to become ready"
                          : "Open the full monitoring screen for this cluster"
                    }
                    className="gap-1.5"
                  >
                    <Activity className="w-3.5 h-3.5" />
                    <span>Details</span>
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleTogglePower(c)}
                    disabled={!!busyOp || !status?.installed}
                    title={
                      c.running
                        ? "Stop all node containers (data is kept)"
                        : "Start this cluster's node containers"
                    }
                    className="gap-1.5"
                  >
                    {busyOp === "power" ? (
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                    ) : c.running ? (
                      <Square className="w-3.5 h-3.5" />
                    ) : (
                      <Play className="w-3.5 h-3.5" />
                    )}
                    <span>{c.running ? "Stop" : "Start"}</span>
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleKubeconfig(c.name)}
                    disabled={!!busyOp || !status?.installed || !c.has_kubeconfig}
                    title="Copy KUBECONFIG path for this cluster"
                    className="gap-1.5"
                  >
                    <Copy className="w-3.5 h-3.5" />
                    <span>Kubeconfig</span>
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => {
                      const p = prefsFor(c.name);
                      setPrefsEdit({
                        name: c.name,
                        autoStart: p.autoStart,
                        nodeCpuM: p.nodeCpuM,
                        nodeMemoryMb: p.nodeMemoryMb,
                      });
                    }}
                    disabled={!!busyOp}
                    title="Cluster auto-start & per-node resource caps"
                    className="text-zinc-400 hover:text-white"
                  >
                    <Settings2 className="w-4 h-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => handleDelete(c.name)}
                    disabled={!!busyOp}
                    title="Delete Cluster"
                    className="text-zinc-400 hover:text-rose-400 hover:bg-rose-500/10"
                  >
                    <Trash2 className="w-4 h-4" />
                  </Button>
                </div>
              </Card>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>

      {/* Empty state */}
      {!loading && filtered.length === 0 && (
        <div className="py-16 text-center text-zinc-400">
          <Hexagon className="w-10 h-10 mx-auto text-zinc-400 mb-3" />
          <p className="font-medium text-white">
            {clusters.length === 0 ? "No kind clusters yet" : "No clusters match your search"}
          </p>
          <p className="text-xs text-zinc-500 mt-1">
            {clusters.length === 0
              ? status?.installed
                ? "Create your first local Kubernetes cluster — nodes run as containers in this app's engine."
                : "Install the kind binary above to get started."
              : "Try a different search term."}
          </p>
          {clusters.length === 0 && status?.installed && (
            <Button
              variant="primary"
              size="sm"
              onClick={() => setCreateOpen(true)}
              disabled={!!busyOp}
              className="mt-4 gap-2"
            >
              <Rocket className="w-4 h-4" />
              <span>Create your first cluster</span>
            </Button>
          )}
        </div>
      )}

        </>
      )}

      {/* Detail monitoring — full ClusterMonitor with a back link. */}
      {detailFor && detailCluster && (
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-3">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setDetailFor(null)}
              className="gap-1.5"
              title="Back to the cluster list"
            >
              <ArrowLeft className="w-4 h-4" />
              <span>Back to list</span>
            </Button>
            <StatusBadge
              status={detailCluster.running ? "Running" : "Stopped"}
              pulse={detailCluster.running}
            />
            <span className="text-white font-medium">{detailCluster.name}</span>
          </div>
          {detailCluster.api_ready === false ? (
            <div className="rounded-xl border border-amber-500/20 bg-amber-500/5 px-4 py-3 text-sm text-amber-300">
              The API server for <span className="font-mono">{detailCluster.name}</span> is
              not ready yet. Monitoring opens automatically as soon as <code>/readyz</code>{" "}
              passes — this refresh every ~10s.
            </div>
          ) : (
            <ClusterMonitor
              key={detailFor}
              cluster={detailFor}
              clusterStats={detailCluster}
            />
          )}
        </div>
      )}

      {/* Cluster prefs modal (auto-start + per-node caps) */}
      <AnimatePresence>
        {prefsEdit && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
          >
            <motion.div
              initial={{ scale: 0.95, y: 8 }}
              animate={{ scale: 1, y: 0 }}
              exit={{ scale: 0.95, y: 8 }}
              className="w-full max-w-md rounded-2xl border border-zinc-800 bg-zinc-900 shadow-2xl"
            >
              <div className="flex items-center justify-between px-5 py-4 border-b border-zinc-800">
                <div>
                  <span className="text-sm font-semibold text-white">
                    Cluster settings: {prefsEdit.name}
                  </span>
                  <p className="text-[11px] text-zinc-500 mt-0.5">
                    Auto-start & per-node caps keep clusters from starving the host.
                  </p>
                </div>
                <button
                  onClick={() => setPrefsEdit(null)}
                  className="text-zinc-400 hover:text-white"
                >
                  <X className="w-4 h-4" />
                </button>
              </div>
              <div className="px-5 py-4 space-y-4">
                <label className="flex items-center justify-between gap-3 text-sm text-zinc-200">
                  <span>
                    Auto-start with engine
                    <span className="block text-[11px] text-zinc-500">
                      Honoured when the master Auto-start switch on the page is on.
                    </span>
                  </span>
                  <input
                    type="checkbox"
                    checked={prefsEdit.autoStart}
                    onChange={(e) =>
                      setPrefsEdit({ ...prefsEdit, autoStart: e.target.checked })
                    }
                    className="rounded border-zinc-700 w-4 h-4"
                  />
                </label>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-xs text-zinc-400 mb-1">
                      CPU per node (cores, 0 = unlimited)
                    </label>
                    <input
                      type="number"
                      min={0}
                      step={0.5}
                      value={prefsEdit.nodeCpuM / 1000}
                      onChange={(e) =>
                        setPrefsEdit({
                          ...prefsEdit,
                          nodeCpuM: Math.max(0, Math.round((Number(e.target.value) || 0) * 1000)),
                        })
                      }
                      className="h-9 w-full px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-indigo-500"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-zinc-400 mb-1">
                      Memory per node (MB, 0 = unlimited)
                    </label>
                    <input
                      type="number"
                      min={0}
                      value={prefsEdit.nodeMemoryMb}
                      onChange={(e) =>
                        setPrefsEdit({
                          ...prefsEdit,
                          nodeMemoryMb: Math.max(0, Number(e.target.value) || 0),
                        })
                      }
                      className="h-9 w-full px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-indigo-500"
                    />
                  </div>
                </div>
                <p className="text-[11px] text-zinc-600">
                  Caps apply immediately (running nodes update via docker update) and
                  re-apply on every start. The auto-start master switch and stagger
                  delay live in Settings → Kubernetes.
                </p>
              </div>
              <div className="flex justify-end gap-2 px-5 py-4 border-t border-zinc-800">
                <Button variant="ghost" size="sm" onClick={() => setPrefsEdit(null)}>
                  Cancel
                </Button>
                <Button
                  variant="primary"
                  size="sm"
                  onClick={savePrefs}
                  disabled={prefsSaving}
                  className="gap-2"
                >
                  {prefsSaving && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                  Save
                </Button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Create cluster modal */}
      <AnimatePresence>
        {createOpen && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-6"
            onClick={() => !creating && setCreateOpen(false)}
          >
            <motion.div
              initial={{ opacity: 0, scale: 0.96, y: 8 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.97, y: 6 }}
              transition={{ duration: 0.18, ease: "easeOut" }}
              className="w-full max-w-lg bg-zinc-900 border border-zinc-800 rounded-2xl shadow-2xl"
              onClick={(e) => e.stopPropagation()}
            >
              <form onSubmit={handleCreate}>
                <div className="flex items-center justify-between px-5 py-4 border-b border-zinc-800">
                  <div className="flex items-center gap-2.5">
                    <div className="w-8 h-8 rounded-lg bg-blue-500/10 border border-blue-500/30 flex items-center justify-center">
                      <Hexagon className="w-4 h-4 text-blue-400" />
                    </div>
                    <h3 className="text-sm font-semibold text-white">Create kind cluster</h3>
                  </div>
                  <button
                    type="button"
                    onClick={() => !creating && setCreateOpen(false)}
                    className="text-zinc-400 hover:text-white"
                  >
                    <X className="w-4 h-4" />
                  </button>
                </div>

                <div className="px-5 py-4 space-y-4">
                  <div>
                    <label className="block text-xs font-medium text-zinc-400 mb-1.5">
                      Cluster name
                    </label>
                    <input
                      type="text"
                      autoFocus
                      value={formName}
                      onChange={(e) => setFormName(e.target.value)}
                      placeholder="my-cluster"
                      pattern="[A-Za-z0-9.\-]+"
                      title="Letters, digits, '-' and '.' only"
                      disabled={creating}
                      className="w-full h-9 px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white placeholder-zinc-500 focus:outline-none focus:border-blue-500 disabled:opacity-60"
                    />
                  </div>

                  <div className="grid grid-cols-2 gap-3">
                    <div>
                      <label className="block text-xs font-medium text-zinc-400 mb-1.5">
                        Nodes
                      </label>
                      <input
                        type="number"
                        min={1}
                        max={5}
                        value={formNodes}
                        onChange={(e) =>
                          setFormNodes(
                            Math.min(5, Math.max(1, Number(e.target.value) || 1))
                          )
                        }
                        disabled={creating}
                        className="w-full h-9 px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500 disabled:opacity-60"
                      />
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-zinc-400 mb-1.5">
                        Node image{" "}
                        <span className="text-zinc-600 font-normal">(optional)</span>
                      </label>
                      <input
                        type="text"
                        value={formImage}
                        onChange={(e) => setFormImage(e.target.value)}
                        placeholder="kind's bundled default"
                        disabled={creating}
                        className="w-full h-9 px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white placeholder-zinc-600 focus:outline-none focus:border-blue-500 disabled:opacity-60"
                      />
                    </div>
                  </div>

                  <div className="rounded-lg bg-zinc-950 border border-zinc-800/80 px-3 py-2.5">
                    <p className="text-[10px] uppercase tracking-wider text-zinc-500 mb-1">
                      Command preview
                    </p>
                    <p className="font-mono text-xs text-zinc-400 break-all">
                      {commandPreview}
                    </p>
                  </div>

                  {creating && showLog && (
                    <div className="rounded-lg bg-zinc-950 border border-zinc-800 px-3 py-2 max-h-32 overflow-y-auto font-mono text-[11px] space-y-0.5 select-text">
                      {logLines.slice(-12).map((l, i) => (
                        <div
                          key={i}
                          className={
                            /error|failed|exit status/i.test(l.line)
                              ? "text-rose-400"
                              : "text-zinc-400"
                          }
                        >
                          {l.line}
                        </div>
                      ))}
                    </div>
                  )}
                </div>

                <div className="flex items-center justify-end gap-2 px-5 py-4 border-t border-zinc-800">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => setCreateOpen(false)}
                    disabled={creating}
                  >
                    Cancel
                  </Button>
                  <Button
                    type="submit"
                    variant="primary"
                    size="sm"
                    disabled={creating || !formName.trim()}
                    className="gap-2"
                  >
                    {creating ? (
                      <Loader2 className="w-4 h-4 animate-spin" />
                    ) : (
                      <Rocket className="w-4 h-4" />
                    )}
                    <span>{creating ? "Creating…" : "Create Cluster"}</span>
                  </Button>
                </div>
              </form>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
};
