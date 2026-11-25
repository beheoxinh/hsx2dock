import { useModal } from "../components/ui/ModalContext";
import { useToast } from "../components/ui/Toast";
import React, { useEffect, useRef, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Hexagon,
  Plus,
  X,
  Download,
  Loader2,
  RefreshCw,
  AlertTriangle,
  Rocket,
  ArrowLeft,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { StatusBadge } from "../components/ui/StatusBadge";
import { Switch } from "../components/ui/Switch";
import { ClusterMonitor } from "../components/k8s/ClusterMonitor";
import { ClusterCard } from "../components/k8s/ClusterCard";
import { ClusterToolbar, type BulkProgress } from "../components/k8s/ClusterToolbar";
import { ClusterPrefsDialog } from "../components/k8s/ClusterPrefsDialog";
import { YamlApplyModal } from "../components/k8s/YamlApplyModal";
import { KubeShellModal } from "../components/k8s/KubeShellModal";
import { api, disposeListener, events } from "../lib/api";
import { peekDashboardClusters } from "../lib/dashboardStore";
import {
  browserStorage,
  collectTags,
  filterClusters,
  isAccessStale,
  normalizeColor,
  normalizeTags,
  readListView,
  settleWithConcurrency,
  sortClusters,
  statusCounts,
  writeListView,
  type ListView,
  type SortKey,
  type StatusFilter,
} from "../lib/k8sClusterList";
import type {
  ClusterAccess,
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
    const trimmed: KindClusterPrefs = {
      autoStart: prefsEdit.autoStart,
      nodeCpuM: Math.max(0, Math.round(prefsEdit.nodeCpuM || 0)),
      nodeMemoryMb: Math.max(0, Math.round(prefsEdit.nodeMemoryMb || 0)),
      color: normalizeColor(prefsEdit.color),
      tags: normalizeTags(prefsEdit.tags ?? []),
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
      setPowerFor(c.name);
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
      setPowerFor(null);
    }
  };

  // List view: sort and filters are remembered per browser; the search text is not.
  const [view, setView] = useState<ListView>(() => readListView(browserStorage()));
  const patchView = (patch: Partial<ListView>) => setView((v) => ({ ...v, ...patch }));
  useEffect(() => {
    writeListView(browserStorage(), view);
  }, [view]);
  const [query, setQuery] = useState("");

  // Bulk selection by cluster name; selections of vanished clusters are dropped.
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  const [bulk, setBulk] = useState<BulkProgress | null>(null);
  useEffect(() => {
    setSelected((prev) => {
      const names = new Set(clusters.map((c) => c.name));
      const next = new Set([...prev].filter((n) => names.has(n)));
      return next.size === prev.size ? prev : next;
    });
  }, [clusters]);

  // Host access per cluster. Fetched only for running clusters, reused for
  // 30 s, so the 10 s poll does not turn into a TCP probe per card per tick.
  const [access, setAccess] = useState<Record<string, { data: ClusterAccess | null; at: number }>>({});
  const accessInflight = useRef(new Set<string>());
  const refreshAccess = (name: string) => {
    if (accessInflight.current.has(name)) return;
    accessInflight.current.add(name);
    api
      .getClusterAccess(name)
      .then((data) => setAccess((p) => ({ ...p, [name]: { data, at: Date.now() } })))
      .catch(() => setAccess((p) => ({ ...p, [name]: { data: null, at: Date.now() } })))
      .finally(() => accessInflight.current.delete(name));
  };
  useEffect(() => {
    const now = Date.now();
    const live = new Set<string>();
    for (const c of clusters) {
      if (!c.running) continue;
      live.add(c.name);
      if (isAccessStale(access[c.name]?.at, now)) refreshAccess(c.name);
    }
    // A stopped cluster's entry is dropped, so its next start fetches afresh
    // instead of showing the answer from before it stopped.
    setAccess((prev) => {
      const gone = Object.keys(prev).filter((n) => !live.has(n));
      if (gone.length === 0) return prev;
      const next = { ...prev };
      for (const n of gone) delete next[n];
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [clusters, access]);

  const [powerFor, setPowerFor] = useState<string | null>(null);
  const [shellFor, setShellFor] = useState<string | null>(null);
  const [applyFor, setApplyFor] = useState<{ cluster: string; tab: "yaml" | "quick" } | null>(null);
  const [menuOpenFor, setMenuOpenFor] = useState<string | null>(null);

  const allTags = collectTags(clusters.map((c) => prefsByName[c.name]));
  const counts = statusCounts(clusters);
  const visible = sortClusters(
    filterClusters(clusters, (name) => prefsByName[name], {
      query,
      status: view.status,
      tag: view.tag,
    }),
    view.sort
  );
  const filtersActive = query.trim() !== "" || view.status !== "all" || view.tag !== null;
  const clearFilters = () => {
    setQuery("");
    patchView({ status: "all", tag: null });
  };

  const selectedClusters = clusters.filter((c) => selected.has(c.name));
  const startable = selectedClusters.filter((c) => !c.running).map((c) => c.name);
  const stoppable = selectedClusters.filter((c) => c.running).map((c) => c.name);
  const visibleNames = visible.map((c) => c.name);
  const visibleSelectedCount = visibleNames.filter((n) => selected.has(n)).length;
  const allVisibleSelected = visibleNames.length > 0 && visibleSelectedCount === visibleNames.length;
  const someVisibleSelected = visibleSelectedCount > 0;
  const toggleAllVisible = () => {
    setSelected((prev) => {
      const next = new Set(prev);
      for (const n of visibleNames) {
        if (allVisibleSelected) next.delete(n);
        else next.add(n);
      }
      return next;
    });
  };
  const toggleSelected = (name: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  /** Start or stop the given clusters, two at a time; failures are reported, not fatal. */
  const bulkRun = async (action: "start" | "stop", names: string[]) => {
    if (busyOp || names.length === 0) return;
    setBusyOp("bulk");
    let failed = 0;
    setBulk({ action, done: 0, total: names.length, failed: 0 });
    try {
      const results = await settleWithConcurrency(
        names,
        2,
        async (name) => {
          try {
            if (action === "start") await api.startKindCluster(name);
            else await api.stopKindCluster(name);
          } catch (e) {
            failed++;
            setBulk((b) => (b ? { ...b, failed } : b));
            throw e;
          }
        },
        (done, total) => setBulk((b) => (b ? { ...b, done, total } : b))
      );
      const errors = results.flatMap((r) => (r.ok ? [] : [r]));
      const okCount = names.length - errors.length;
      if (errors.length === 0) {
        toast.success(
          `Bulk ${action} finished`,
          `${okCount} cluster${okCount === 1 ? "" : "s"} ${action === "start" ? "started" : "stopped"}.`
        );
      } else {
        toast.error(
          `${errors.length} of ${names.length} could not be ${action === "start" ? "started" : "stopped"}`,
          errors
            .slice(0, 3)
            .map((r) => `${r.item}: ${String(r.error)}`)
            .join("; ")
        );
      }
    } finally {
      setBulk(null);
      setBusyOp(null);
      await loadAll();
    }
  };

  /** Merge into / remove from the user's ~/.kube/config, after a confirm. */
  const hostKubeconfigAction = async (c: KindCluster, mode: "add" | "remove") => {
    if (busyOp) return;
    const path = access[c.name]?.data?.hostKubeconfig ?? "~/.kube/config";
    const backup = `${path}.hsx2dock.bak`;
    const confirmed = await modal.showConfirm(
      mode === "add"
        ? `This edits your kubeconfig (${path}): the kind-${c.name} context is merged in and becomes the current context. A backup is kept at ${backup}.`
        : `This edits your kubeconfig (${path}): the kind-${c.name} entries are removed. A backup is kept at ${backup}.`,
      mode === "add" ? "Add to ~/.kube/config" : "Remove from ~/.kube/config",
      mode === "add" ? "warning" : "danger"
    );
    if (!confirmed) return;
    try {
      setBusyOp("host-kubeconfig");
      const msg =
        mode === "add"
          ? await api.exportKubeconfigToHost(c.name)
          : await api.removeKubeconfigFromHost(c.name);
      toast.success(mode === "add" ? "Added to kubeconfig" : "Removed from kubeconfig", msg);
      refreshAccess(c.name);
    } catch (err) {
      toast.error(
        mode === "add" ? "Could not add to kubeconfig" : "Could not remove from kubeconfig",
        String(err)
      );
    } finally {
      setBusyOp(null);
    }
  };

  const openHostTerminal = async (name: string) => {
    try {
      const msg = await api.openClusterTerminal(name);
      toast.info("Host terminal opened", msg);
    } catch (err) {
      toast.error("Could not open terminal", String(err));
    }
  };

  const copyContext = async (name: string) => {
    const ctx = `kind-${name}`;
    try {
      await navigator.clipboard.writeText(ctx);
      toast.success("Context copied", ctx);
    } catch (e) {
      toast.error("Copy failed", String(e));
    }
  };

  const openPrefs = (name: string) => {
    const p = prefsFor(name);
    setPrefsEdit({
      name,
      autoStart: p.autoStart,
      nodeCpuM: p.nodeCpuM,
      nodeMemoryMb: p.nodeMemoryMb,
      color: p.color ?? "",
      tags: p.tags ?? [],
    });
  };

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

          {/* Toolbar: search, filters, sort, bulk actions */}
          {clusters.length > 0 && (
            <ClusterToolbar
              query={query}
              onQuery={setQuery}
              status={view.status}
              onStatus={(st: StatusFilter) => patchView({ status: st })}
              counts={counts}
              tag={view.tag}
              onTag={(t) => patchView({ tag: t })}
              tags={allTags}
              sort={view.sort}
              onSort={(k: SortKey) => patchView({ sort: k })}
              visibleCount={visible.length}
              totalCount={clusters.length}
              selectedCount={selectedClusters.length}
              allVisibleSelected={allVisibleSelected}
              someVisibleSelected={someVisibleSelected}
              onToggleAllVisible={toggleAllVisible}
              onClearSelection={() => setSelected(new Set())}
              startableCount={startable.length}
              stoppableCount={stoppable.length}
              onBulkStart={() => bulkRun("start", startable)}
              onBulkStop={() => bulkRun("stop", stoppable)}
              bulkProgress={bulk}
              disabled={!!busyOp}
            />
          )}

          {/* Cluster grid */}
          {visible.length > 0 && (
            <div className="grid grid-cols-1 gap-4">
              <AnimatePresence mode="popLayout">
                {visible.map((c) => (
                  <motion.div
                    key={c.name}
                    layout
                    initial={{ opacity: 0, scale: 0.97 }}
                    animate={{ opacity: 1, scale: 1 }}
                    exit={{ opacity: 0, scale: 0.95 }}
                    transition={{ duration: 0.16 }}
                    className="relative"
                    style={{ zIndex: menuOpenFor === c.name ? 30 : undefined }}
                  >
                    <ClusterCard
                      cluster={c}
                      prefs={prefsFor(c.name)}
                      access={access[c.name] ? access[c.name].data : undefined}
                      history={history[c.name]}
                      selected={selected.has(c.name)}
                      busy={!!busyOp}
                      powerBusy={powerFor === c.name}
                      kindInstalled={!!status?.installed}
                      prefsLoaded={prefsLoaded}
                      logLines={logLines.filter((l) => l.name === c.name)}
                      onToggleSelected={() => toggleSelected(c.name)}
                      onToggleAutoStart={() => toggleClusterAutoStart(c)}
                      onOpenMonitor={() => setDetailFor(c.name)}
                      onTogglePower={() => handleTogglePower(c)}
                      onOpenShell={() => setShellFor(c.name)}
                      onApply={(tab) => setApplyFor({ cluster: c.name, tab })}
                      onPrefs={() => openPrefs(c.name)}
                      onCopyKubeconfig={() => handleKubeconfig(c.name)}
                      onAddHostKubeconfig={() => hostKubeconfigAction(c, "add")}
                      onRemoveHostKubeconfig={() => hostKubeconfigAction(c, "remove")}
                      onOpenHostTerminal={() => openHostTerminal(c.name)}
                      onCopyContext={() => copyContext(c.name)}
                      onDelete={() => handleDelete(c.name)}
                      onClearLog={() => setLogLines((prev) => prev.filter((l) => l.name !== c.name))}
                      onMenuOpenChange={(open) => setMenuOpenFor(open ? c.name : null)}
                    />
                  </motion.div>
                ))}
              </AnimatePresence>
            </div>
          )}

          {/* Empty state: no clusters at all */}
          {!loading && clusters.length === 0 && (
            <div className="py-16 text-center text-zinc-400">
              <Hexagon className="w-10 h-10 mx-auto text-zinc-400 mb-3" />
              <p className="font-medium text-white">No kind clusters yet</p>
              <p className="text-xs text-zinc-500 mt-1">
                {status?.installed
                  ? "Create your first local Kubernetes cluster — nodes run as containers in this app's engine."
                  : "Install the kind binary above to get started."}
              </p>
              {status?.installed && (
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

          {/* Empty state: filters hide everything */}
          {clusters.length > 0 && visible.length === 0 && (
            <div className="py-12 text-center text-zinc-400">
              <p className="font-medium text-white">No clusters match</p>
              <p className="text-xs text-zinc-500 mt-1">
                {filtersActive
                  ? "Try a different search, or clear the filters."
                  : "Nothing to show."}
              </p>
              {filtersActive && (
                <Button variant="outline" size="sm" onClick={clearFilters} className="mt-4">
                  Clear filters
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

      {/* Cluster settings: colour, tags, auto-start, per-node caps */}
      <AnimatePresence>
        {prefsEdit && (
          <ClusterPrefsDialog
            draft={{
              ...prefsEdit,
              color: prefsEdit.color ?? "",
              tags: prefsEdit.tags ?? [],
            }}
            knownTags={allTags}
            saving={prefsSaving}
            onChange={(next) => setPrefsEdit(next)}
            onSave={savePrefs}
            onClose={() => setPrefsEdit(null)}
          />
        )}
      </AnimatePresence>

      {/* kubectl shell and YAML / quick deploy, scoped to the cluster they were opened for */}
      {shellFor && <KubeShellModal cluster={shellFor} onClose={() => setShellFor(null)} />}
      {applyFor && (
        <YamlApplyModal
          cluster={applyFor.cluster}
          initialTab={applyFor.tab}
          onClose={() => setApplyFor(null)}
          onApplied={() => refreshClusters()}
        />
      )}

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
