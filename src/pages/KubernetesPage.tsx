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
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusBadge } from "../components/ui/StatusBadge";
import { ClusterMonitor } from "../components/k8s/ClusterMonitor";
import { api, events } from "../lib/api";
import { formatBytes, formatRelativeTime } from "../lib/utils";
import type {
  KindCluster,
  KindClusterPrefs,
  KindInstallStatus,
  KindProgressPayload,
} from "../types";

interface KubernetesPageProps {
  refreshKey?: number;
}

export const KubernetesPage: React.FC<KubernetesPageProps> = ({ refreshKey }) => {
  const modal = useModal();
  const toast = useToast();
  const [status, setStatus] = useState<KindInstallStatus | null>(null);
  const [clusters, setClusters] = useState<KindCluster[]>([]);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  // kind binary install state (driven by kind-install-progress events).
  const [installing, setInstalling] = useState(false);
  const [installPct, setInstallPct] = useState<number | null>(null);
  const [installBytes, setInstallBytes] = useState<{ done: number; total: number | null } | null>(null);

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

  // Which running cluster the live monitoring panel watches (auto-picks the
  // first running one; selector appears when several are up).
  const [monitorFor, setMonitorFor] = useState<string | null>(null);
  const runningClusters = clusters.filter((c) => c.running);
  useEffect(() => {
    if (monitorFor && clusters.some((c) => c.name === monitorFor && c.running)) return;
    setMonitorFor(clusters.find((c) => c.running)?.name ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clusters]);

  // Page-level split: manage clusters vs watch/manage their internals.
  const [pageTab, setPageTab] = useState<"clusters" | "monitor">("clusters");

  // Per-cluster prefs (auto-start, per-node caps) mirrored from the config.
  const [prefsByName, setPrefsByName] = useState<Record<string, KindClusterPrefs>>({});
  const [prefsEdit, setPrefsEdit] = useState<
    (KindClusterPrefs & { name: string; isNew?: boolean }) | null
  >(null);
  const [prefsSaving, setPrefsSaving] = useState(false);
  const loadPrefs = async () => {
    try {
      const cfg = await api.getAppConfig();
      setPrefsByName(cfg.kind_cluster_prefs ?? {});
    } catch (e) {
      console.error(e);
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

  const loadAll = async () => {
    try {
      setLoading(true);
      const [st, list] = await Promise.all([
        api.kindStatus(),
        api.listKindClusters(),
      ]);
      setStatus(st);
      setClusters(list);
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    } finally {
      setLoading(false);
    }
    loadPrefs();
  };

  useEffect(() => {
    loadAll();
    loadPrefs();
  }, [refreshKey]);

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
    let unlistenProgress: (() => void) | undefined;
    let unlistenInstall: (() => void) | undefined;

    events.onKindProgress((payload) => {
      setLogLines((prev) => [...prev.slice(-199), payload]);
    }).then((u) => (unlistenProgress = u));

    events.onKindInstallProgress((payload) => {
      if (payload.error) {
        setInstalling(false);
        setInstallPct(null);
        toast.error("kind install failed", payload.error);
        return;
      }
      if (payload.finished) {
        setInstalling(false);
        setInstallPct(100);
        toast.success("kind installed", "The isolated kind binary is ready.");
        api.kindStatus().then(setStatus).catch(() => {});
        return;
      }
      if (typeof payload.done === "number") {
        const total = typeof payload.total === "number" && payload.total > 0 ? payload.total : null;
        setInstallBytes({ done: payload.done, total });
        setInstallPct(total ? Math.min(100, Math.round((payload.done / total) * 100)) : null);
      }
    }).then((u) => (unlistenInstall = u));

    return () => {
      unlistenProgress?.();
      unlistenInstall?.();
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
      setInstallBytes(null);
      await api.installKind();
    } catch (e) {
      setInstalling(false);
      setInstallPct(null);
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
              {installing && installBytes && (
                <div className="mt-3 max-w-md">
                  <div className="flex justify-between text-[11px] text-zinc-500 mb-1 font-mono">
                    <span>
                      {formatBytes(installBytes.done)}
                      {installBytes.total ? ` / ${formatBytes(installBytes.total)}` : ""}
                    </span>
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

      {/* Live kind output */}
      {showLog && (
        <div className="rounded-xl border border-zinc-800 bg-zinc-950 overflow-hidden">
          <div className="flex items-center justify-between px-3.5 py-2 border-b border-zinc-800/80 bg-zinc-900/50">
            <span className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
              kind output
            </span>
            <div className="flex items-center gap-3">
              <button
                onClick={async () => {
                  const text = logLines.map((l) => l.line).join("\n");
                  if (!text) return;
                  try {
                    await navigator.clipboard.writeText(text);
                    toast.success(
                      "Kind output copied",
                      `${logLines.length} line${logLines.length === 1 ? "" : "s"} on the clipboard.`
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
                onClick={() => setLogLines([])}
                className="text-[11px] text-zinc-500 hover:text-zinc-300"
              >
                Clear
              </button>
            </div>
          </div>
          <div
            ref={logRef}
            className="px-3.5 py-2.5 max-h-44 overflow-y-auto font-mono text-xs leading-relaxed space-y-0.5"
          >
            {logLines.map((l, i) => (
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

      {/* Tabs: Clusters / Monitoring */}
      <div className="flex items-center gap-2 border-b border-zinc-800/80">
        {(["clusters", "monitor"] as const).map((k) => (
          <button
            key={k}
            onClick={() => setPageTab(k)}
            className={`px-3 py-1.5 text-xs font-medium transition-colors ${
              pageTab === k
                ? "text-white border-b-2 border-indigo-400"
                : "text-zinc-400 hover:text-zinc-200"
            }`}
          >
            {k === "clusters" ? "Clusters" : "Monitoring"}
          </button>
        ))}
      </div>

      {pageTab === "clusters" && (
        <>
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
      <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
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
                        {c.has_kubeconfig && (
                          <span className="text-[10px] px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-400 border border-zinc-700/60 font-mono">
                            kubeconfig
                          </span>
                        )}
                        {prefsFor(c.name).autoStart && (
                          <span
                            title="Starts with the engine (staggered)"
                            className="text-[10px] px-1.5 py-0.5 rounded bg-amber-500/10 text-amber-300 border border-amber-500/30"
                          >
                            auto-start
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
                </div>

                <div className="mt-4 grid grid-cols-2 gap-2 text-xs">
                  <div className="rounded-lg bg-zinc-900/70 px-2.5 py-2">
                    <p className="text-zinc-500">Nodes</p>
                    <p className="text-white font-medium mt-0.5">
                      {c.running_nodes}/{c.node_count} running
                    </p>
                  </div>
                  <div className="rounded-lg bg-zinc-900/70 px-2.5 py-2">
                    <p className="text-zinc-500">Age</p>
                    <p className="text-white font-medium mt-0.5">
                      {c.created ? formatRelativeTime(c.created) : "—"}
                    </p>
                  </div>
                </div>

                <div className="mt-4 flex items-center gap-2">
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
                    className="gap-1.5 flex-1"
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
                    disabled={!!busyOp || !status?.installed}
                    title="Copy KUBECONFIG path for this cluster"
                    className="gap-1.5 flex-1"
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

      {/* Live monitoring for the selected running cluster */}
      {pageTab === "monitor" && !monitorFor && (
        <div className="py-16 text-center text-zinc-500">
          <Activity className="w-10 h-10 mx-auto text-zinc-600 mb-3" />
          <p className="text-sm text-zinc-300 font-medium">
            No running cluster to monitor
          </p>
          <p className="text-xs mt-1">
            Start a cluster from the Clusters tab to see live health, pods and resources.
          </p>
        </div>
      )}
      {pageTab === "monitor" && monitorFor && (
        <div className="mt-2 space-y-3">
          {runningClusters.length > 1 && (
            <div className="flex items-center gap-2 text-xs text-zinc-400">
              <span>Watching cluster</span>
              <select
                value={monitorFor}
                onChange={(e) => setMonitorFor(e.target.value)}
                className="h-8 rounded-lg bg-zinc-950 border border-zinc-800 px-2 text-xs text-zinc-200 focus:outline-none focus:border-indigo-500"
              >
                {runningClusters.map((c) => (
                  <option key={c.name} value={c.name}>
                    {c.name}
                  </option>
                ))}
              </select>
            </div>
          )}
          <ClusterMonitor key={monitorFor} cluster={monitorFor} />
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
                  <span>Auto-start with engine</span>
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
                    <div className="rounded-lg bg-zinc-950 border border-zinc-800 px-3 py-2 max-h-32 overflow-y-auto font-mono text-[11px] space-y-0.5">
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
