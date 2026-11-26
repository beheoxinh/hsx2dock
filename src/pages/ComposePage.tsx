import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FilePlus2, FileCode2, Layers, PackagePlus, RefreshCw, SearchX } from "lucide-react";
import { Button } from "../components/ui/Button";
import { useToast } from "../components/ui/Toast";
import { EngineGate } from "../components/ui/EngineGate";
import { ComposeProjectCard } from "../components/compose/ComposeProjectCard";
import { ComposeDrawer } from "../components/compose/ComposeDrawer";
import { ComposeProgressPanel, type ComposeOp } from "../components/compose/ComposeProgressPanel";
import {
  AddProjectDialog,
  AlertsDialog,
  BuildDialog,
  DownDialog,
  ExportDialog,
  PullDialog,
  SearchBox,
  UpDialog,
} from "../components/compose/ComposeDialogs";
import { NewProjectDialog } from "../components/compose/NewProjectDialog";
import { inputClass } from "../components/compose/composeUi";
import { useOpLineBatcher } from "../hooks/useOpLineBatcher";
import { useStableCallback } from "../hooks/useStableCallback";
import { api, disposeListener, events } from "../lib/api";
import { useContainerStore } from "../lib/containerStore";
import { alertsFor, clearAlerts, mergeAlerts, ALERT_KIND_LABEL } from "../lib/composeAlerts";
import { historyOf, pruneHistory, recordSample } from "../lib/composeHistory";
import { pickStats, projectUsage, type ProjectUsage } from "../lib/composeStats";
import {
  filterProjects,
  newOpId,
  projectStatus,
  sortProjects,
  type ComposeServiceRow,
  type ComposeSortKey,
  type ComposeStatusFilter,
} from "../lib/composeProject";
import { cn, retainKeys } from "../lib/utils";
import type {
  ComposeAlert,
  ComposeBuildOptions,
  ComposeDownOptions,
  ComposeImageUpdate,
  ComposeProjectInfo,
  ComposePullOptions,
  ComposeServiceAction,
  ComposeUpOptions,
  ContainerStats,
  DaemonState,
} from "../types";

interface ComposePageProps {
  /** Engine state — the page renders an EngineGate panel unless Running. */
  daemonState?: DaemonState;
}

/** Fallback refresh. Docker events trigger the real refreshes, so this only catches what events miss. */
const POLL_MS = 30000;
/** Docker events arrive in bursts (one per container); one reload covers a burst. */
const EVENT_DEBOUNCE_MS = 600;
/** Image checks are spaced out so a page visit does not fire one registry request per project at once. */
const UPDATE_STAGGER_MS = 800;
/** Operations kept in the progress panel; running ones are never dropped. */
const MAX_OPS = 20;

const STATUS_FILTERS: { key: ComposeStatusFilter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "running", label: "Running" },
  { key: "partial", label: "Partial" },
  { key: "stopped", label: "Stopped" },
  { key: "error", label: "Error" },
];

/** Shared empty list so cards without updates keep the same prop identity. */
const NO_UPDATES: ComposeImageUpdate[] = [];

/** Stable identity of a project row; also the key of its sparkline history. */
const projectKey = (p: ComposeProjectInfo) => `${p.name}|${p.path}`;

/** Appends an op, dropping the oldest finished ones first when over the cap. */
function pushOp(prev: ComposeOp[], op: ComposeOp): ComposeOp[] {
  const extra = prev.length + 1 - MAX_OPS;
  const kept = extra > 0 ? prev.filter((o, i) => i >= extra || o.status === "running") : prev;
  return [...kept, op];
}

/** Project name of a compose file: its folder, which is also the compose project name. */
function folderName(filePath: string): string {
  const dir = filePath.replace(/\/[^/]*$/, "");
  return dir.split("/").pop() || filePath;
}

export const ComposePage: React.FC<ComposePageProps> = ({ daemonState = "Running" }) => {
  const toast = useToast();
  const { gatewayRoutes } = useContainerStore();
  const [projects, setProjects] = useState<ComposeProjectInfo[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [stats, setStats] = useState<Record<string, ContainerStats>>({});
  /** Per-project CPU/memory of the last tick, as primitives the cards compare. */
  const [usage, setUsage] = useState<Record<string, ProjectUsage>>({});
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<ComposeStatusFilter>("all");
  const [sortKey, setSortKey] = useState<ComposeSortKey>("status");
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [ops, setOps] = useState<ComposeOp[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [creating, setCreating] = useState(false);
  const [upFor, setUpFor] = useState<ComposeProjectInfo | null>(null);
  const [downFor, setDownFor] = useState<ComposeProjectInfo | null>(null);
  const [pullFor, setPullFor] = useState<ComposeProjectInfo | null>(null);
  const [buildFor, setBuildFor] = useState<ComposeProjectInfo | null>(null);
  const [exportFor, setExportFor] = useState<ComposeProjectInfo | null>(null);
  const [alertsProject, setAlertsProject] = useState<string | null>(null);
  const [alerts, setAlerts] = useState<ComposeAlert[]>([]);
  /** Registry check results per compose file path. */
  const [updates, setUpdates] = useState<Record<string, ComposeImageUpdate[]>>({});
  // Progress lines are applied once per frame (see the hook).
  const pushOpLine = useOpLineBatcher(setOps);
  const engineUp = daemonState === "Running";
  const loadSeq = useRef(0);
  const projectsRef = useRef<ComposeProjectInfo[]>([]);
  const refreshTimer = useRef<number | null>(null);
  // Paths already image-checked during this page visit (one check per project per visit).
  const checkedUpdates = useRef(new Set<string>());
  const updateTimers = useRef<number[]>([]);
  // False after unmount; timers check it so StrictMode's double effects cannot cancel their work.
  const alive = useRef(false);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const load = useCallback(async (manual = false) => {
    const seq = ++loadSeq.current;
    if (manual) setRefreshing(true);
    try {
      const list = await api.listComposeProjects();
      // A slow older poll must not overwrite a newer answer.
      if (seq !== loadSeq.current) return;
      setProjects(list);
      setLoadError(null);
      // Sparkline history of projects that are gone is dropped with them.
      pruneHistory(new Set(list.map(projectKey)));
      // Registry badges and their "already checked" marks follow the project
      // list too, so a removed project's entries do not pile up. A project that
      // comes back is checked again (the backend caches the registry answer).
      const livePaths = new Set(list.map((p) => p.path).filter(Boolean));
      setUpdates((prev) => retainKeys(prev, livePaths));
      for (const path of checkedUpdates.current) if (!livePaths.has(path)) checkedUpdates.current.delete(path);
    } catch (e) {
      if (seq === loadSeq.current) setLoadError(String(e));
    } finally {
      if (seq === loadSeq.current) setLoaded(true);
      if (manual) setRefreshing(false);
    }
  }, []);

  projectsRef.current = projects;

  useEffect(() => {
    if (!engineUp) return;
    void load();
    const t = window.setInterval(() => {
      if (!document.hidden) void load();
    }, POLL_MS);
    return () => window.clearInterval(t);
  }, [engineUp, load]);

  // Container events drive the refresh; the debounce folds a burst into one reload.
  const scheduleRefresh = useCallback(() => {
    if (refreshTimer.current !== null) window.clearTimeout(refreshTimer.current);
    refreshTimer.current = window.setTimeout(() => {
      refreshTimer.current = null;
      if (alive.current) void load();
    }, EVENT_DEBOUNCE_MS);
  }, [load]);

  useEffect(() => {
    const offStats = disposeListener(
      events.onContainerStatsTick((tick) => {
        const all = tick ?? {};
        const nextUsage: Record<string, ProjectUsage> = {};
        // History is fed from the same ticks the cards aggregate; idle projects get no sample.
        for (const p of projectsRef.current) {
          const key = projectKey(p);
          const u = projectUsage(p, all);
          nextUsage[key] = u;
          if (u.sampled > 0) recordSample(key, u.cpu, u.mem);
        }
        setUsage(nextUsage);
        setStats(all);
      })
    );
    const offProgress = disposeListener(events.onComposeProgress(({ op_id, line }) => pushOpLine(op_id, line)));
    const offEvent = disposeListener(events.onComposeEvent(() => scheduleRefresh()));
    const offAlert = disposeListener(
      events.onComposeAlert((a) => {
        setAlerts((prev) => mergeAlerts([a], prev));
        toast.error(`${a.project}/${a.service}: ${ALERT_KIND_LABEL[a.kind] ?? a.kind}`, a.message);
      })
    );
    const offWatch = disposeListener(
      events.onComposeWatchEnded((e) => {
        // A watch ends with its own op id; a non-zero exit is a failure, a stop from the menu is not.
        const failed = e.code !== null && e.code !== 0;
        setOps((prev) =>
          prev.map((o) =>
            o.id === e.op_id
              ? {
                  ...o,
                  status: failed ? "error" : "ok",
                  error: failed ? `Watch exited with code ${e.code}` : undefined,
                  finished: Date.now(),
                }
              : o
          )
        );
        if (failed) toast.error("Watch stopped with an error", `exit code ${e.code}`);
        else toast.info("Watch stopped");
        void load();
      })
    );
    return () => {
      offStats();
      offProgress();
      offEvent();
      offAlert();
      offWatch();
      if (refreshTimer.current !== null) window.clearTimeout(refreshTimer.current);
    };
  }, [load, scheduleRefresh, toast, pushOpLine]);

  // Alerts raised before the page opened: seeded once, merged with live ones.
  useEffect(() => {
    api
      .composeRecentAlerts()
      .then((list) => setAlerts((prev) => mergeAlerts(list, prev)))
      .catch(() => {
        /* no engine yet: the list fills in from live events */
      });
  }, []);

  /** Registry check of one project's images; a failed check just shows no badge. */
  const refreshUpdates = useCallback(async (path: string, force: boolean) => {
    try {
      const list = await api.composeImageUpdates(path, force);
      if (alive.current) setUpdates((prev) => ({ ...prev, [path]: list }));
    } catch {
      /* offline or no registry access: no badge */
    }
  }, []);

  // Lazy image check: once per project per visit, staggered, only for files that load.
  useEffect(() => {
    if (!engineUp) return;
    // Stagger within this batch only; a later batch starts its own spacing.
    let slot = 0;
    for (const p of projects) {
      if (!p.path || p.config_error || checkedUpdates.current.has(p.path)) continue;
      checkedUpdates.current.add(p.path);
      const path = p.path;
      updateTimers.current.push(
        window.setTimeout(() => {
          if (alive.current) void refreshUpdates(path, false);
        }, slot * UPDATE_STAGGER_MS)
      );
      slot += 1;
    }
  }, [projects, engineUp, refreshUpdates]);

  useEffect(() => {
    const timers = updateTimers;
    return () => {
      for (const t of timers.current) window.clearTimeout(t);
      timers.current = [];
    };
  }, []);

  /**
   * Runs one compose operation: marks the project busy, shows it in the
   * progress panel (live lines arrive through `compose-progress`), then
   * refreshes the list.
   */
  const runOp = useCallback(
    async (project: string, title: string, fn: (opId: string) => Promise<unknown>) => {
      const id = newOpId();
      setOps((prev) => pushOp(prev, { id, project, title, status: "running", lines: [], started: Date.now() }));
      setBusy((prev) => new Set(prev).add(project));
      try {
        await fn(id);
        setOps((prev) => prev.map((o) => (o.id === id ? { ...o, status: "ok", finished: Date.now() } : o)));
      } catch (e) {
        const error = String(e);
        setOps((prev) =>
          prev.map((o) => (o.id === id ? { ...o, status: "error", error, finished: Date.now() } : o))
        );
        toast.error(`${title} failed`, error.length > 300 ? `${error.slice(0, 300)}…` : error);
      } finally {
        setBusy((prev) => {
          const next = new Set(prev);
          next.delete(project);
          return next;
        });
        void load();
      }
    },
    [load, toast]
  );

  const up = (p: ComposeProjectInfo, options?: ComposeUpOptions) =>
    runOp(p.name, options ? `Up ${p.name} (options)` : `Up ${p.name}`, (opId) => api.composeUp(p.path, options, opId));

  /** Pulls images; after a successful pull the update badge is re-checked without the registry cache. */
  const pull = (p: ComposeProjectInfo, options: ComposePullOptions, title: string) =>
    runOp(p.name, title, async (opId) => {
      await api.composePull(p.path, options, opId);
      void refreshUpdates(p.path, true);
    });

  const projectAction = (p: ComposeProjectInfo, action: "start" | "stop" | "restart") =>
    runOp(p.name, `${action[0].toUpperCase()}${action.slice(1)} ${p.name}`, (opId) => api.composeProjectAction(p.name, action, opId));

  const down = (p: ComposeProjectInfo, options: ComposeDownOptions) =>
    runOp(p.name, `Down ${p.name}`, (opId) =>
      p.path && !p.config_error ? api.composeDown(p.path, options, opId) : api.composeProjectAction(p.name, "down", opId)
    );

  const serviceAction = (p: ComposeProjectInfo, row: ComposeServiceRow, action: ComposeServiceAction, replicas?: number) =>
    runOp(
      p.name,
      action === "scale" ? `Scale ${p.name}/${row.name} to ${replicas}` : `${action} ${p.name}/${row.name}`,
      (opId) => api.composeServiceAction(p.name, p.path || null, row.name, action, replicas, opId)
    );

  /**
   * `compose watch` runs until stopped, so its output stays in the progress
   * panel as a running op; the `compose-watch-ended` event closes it.
   */
  const startWatch = async (p: ComposeProjectInfo) => {
    const id = newOpId();
    setOps((prev) => pushOp(prev, { id, project: p.name, title: `Watch ${p.name}`, status: "running", lines: [], started: Date.now() }));
    try {
      await api.composeWatchStart(p.path, id);
      toast.success("Watch started", p.name);
      void load();
    } catch (e) {
      setOps((prev) =>
        prev.map((o) => (o.id === id ? { ...o, status: "error", error: String(e), finished: Date.now() } : o))
      );
      toast.error("Watch did not start", String(e));
    }
  };

  const stopWatch = async (p: ComposeProjectInfo) => {
    try {
      await api.composeWatchStop(p.path);
    } catch (e) {
      toast.error("Could not stop the watch", String(e));
    }
  };

  const clearProjectAlerts = async (project: string) => {
    try {
      await api.composeClearAlerts(project);
      setAlerts((prev) => clearAlerts(prev, project));
      setAlertsProject(null);
    } catch (e) {
      toast.error("Could not clear alerts", String(e));
    }
  };

  const togglePin = async (p: ComposeProjectInfo) => {
    try {
      if (p.pinned) {
        await api.composeUnpinProject(p.path);
        toast.success("Removed from list", p.path);
      } else {
        await api.composePinProject(p.path);
        toast.success("Saved to list", p.path);
      }
      void load();
    } catch (e) {
      toast.error("Could not update the list", String(e));
    }
  };

  const copyPath = async (p: ComposeProjectInfo) => {
    try {
      await navigator.clipboard.writeText(p.path);
      toast.success("Path copied", p.path);
    } catch (e) {
      toast.error("Copy failed", String(e));
    }
  };

  const openFolder = async (p: ComposeProjectInfo) => {
    try {
      await api.composeOpenFolder(p.path);
    } catch (e) {
      toast.error("Could not open the folder", String(e));
    }
  };

  const counts = useMemo(() => {
    const c: Record<ComposeStatusFilter, number> = { all: projects.length, running: 0, partial: 0, stopped: 0, error: 0 };
    for (const p of projects) c[projectStatus(p)] += 1;
    return c;
  }, [projects]);

  const visible = useMemo(
    () => sortProjects(filterProjects(projects, query, statusFilter), sortKey),
    [projects, query, statusFilter, sortKey]
  );

  const selectedProject = selected ? projects.find((p) => projectKey(p) === selected) ?? null : null;

  // Registry badges reduced to the actionable ones; recomputed only when checks change.
  const runnableUpdates = useMemo(() => {
    const out: Record<string, ComposeImageUpdate[]> = {};
    for (const [path, list] of Object.entries(updates)) out[path] = list.filter((u) => u.status === "update");
    return out;
  }, [updates]);

  // The drawer gets a new stats record only when its own containers' values changed.
  const drawerStatsRef = useRef<Record<string, ContainerStats>>({});
  if (selectedProject) {
    drawerStatsRef.current = pickStats(
      drawerStatsRef.current,
      stats,
      selectedProject.services.map((c) => c.id)
    );
  }
  const drawerStats = drawerStatsRef.current;

  // Card handlers: stable identities that always run the latest closure, so the
  // memoised cards skip re-rendering on every stats tick.
  const openProject = useStableCallback((p: ComposeProjectInfo) => setSelected(projectKey(p)));
  const upProject = useStableCallback((p: ComposeProjectInfo) => void up(p));
  const upOptionsProject = useStableCallback((p: ComposeProjectInfo) => setUpFor(p));
  const restartProject = useStableCallback((p: ComposeProjectInfo) => void projectAction(p, "restart"));
  const stopProject = useStableCallback((p: ComposeProjectInfo) => void projectAction(p, "stop"));
  const downProject = useStableCallback((p: ComposeProjectInfo) => setDownFor(p));
  const pullProject = useStableCallback((p: ComposeProjectInfo) => setPullFor(p));
  const buildProject = useStableCallback((p: ComposeProjectInfo) => setBuildFor(p));
  const pullUpdatesProject = useStableCallback((p: ComposeProjectInfo) => {
    const names = (updates[p.path] ?? []).filter((u) => u.status === "update").map((u) => u.service);
    void pull(p, { services: names }, `Pull updates for ${p.name}`);
  });
  const showAlertsProject = useStableCallback((p: ComposeProjectInfo) => setAlertsProject(p.name));
  const startWatchProject = useStableCallback((p: ComposeProjectInfo) => void startWatch(p));
  const stopWatchProject = useStableCallback((p: ComposeProjectInfo) => void stopWatch(p));
  const exportProject = useStableCallback((p: ComposeProjectInfo) => setExportFor(p));
  const togglePinProject = useStableCallback((p: ComposeProjectInfo) => void togglePin(p));
  const copyPathProject = useStableCallback((p: ComposeProjectInfo) => void copyPath(p));
  const openFolderProject = useStableCallback((p: ComposeProjectInfo) => void openFolder(p));

  if (!engineUp) {
    return (
      <EngineGate state={daemonState} resource="compose projects">
        {null}
      </EngineGate>
    );
  }

  return (
    <div className="mx-auto max-w-7xl space-y-5 p-8">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">Docker Compose</h2>
          <p className="mt-1 text-sm text-zinc-400">
            Multi-container projects: lifecycle per project and per service, logs, config and env files
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="ghost"
            className="rounded-none"
            onClick={() => void load(true)}
            title="Reload the project list"
          >
            <RefreshCw className={cn("h-4 w-4", refreshing && "animate-spin")} />
          </Button>
          <Button size="sm" variant="outline" className="rounded-none" onClick={() => setAdding(true)}>
            <FilePlus2 className="h-4 w-4" /> Add project
          </Button>
          <Button size="sm" variant="primary" className="rounded-none" onClick={() => setCreating(true)}>
            <PackagePlus className="h-4 w-4" /> New project
          </Button>
        </div>
      </div>

      {projects.length > 0 && (
        <div className="flex flex-wrap items-center gap-2">
          <SearchBox value={query} onChange={setQuery} placeholder="Search projects, services, images, paths" />
          <div className="flex border border-zinc-700" role="group" aria-label="Filter by status">
            {STATUS_FILTERS.map((f, i) => (
              <button
                key={f.key}
                type="button"
                onClick={() => setStatusFilter(f.key)}
                disabled={f.key !== "all" && counts[f.key] === 0 && statusFilter !== f.key}
                aria-pressed={statusFilter === f.key}
                className={cn(
                  "flex items-center gap-1.5 px-2.5 py-1.5 text-xs disabled:opacity-40",
                  i > 0 && "border-l border-zinc-700",
                  statusFilter === f.key ? "bg-zinc-800 text-white" : "text-zinc-400 hover:text-zinc-200"
                )}
              >
                {f.label}
                <span className="font-mono text-[10px] text-zinc-500">{counts[f.key]}</span>
              </button>
            ))}
          </div>
          <select
            value={sortKey}
            onChange={(e) => setSortKey(e.target.value as ComposeSortKey)}
            className={cn(inputClass, "w-auto rounded-none")}
            aria-label="Sort projects"
          >
            <option value="status">Sort: status</option>
            <option value="name">Sort: name</option>
            <option value="services">Sort: services</option>
          </select>
        </div>
      )}

      {loadError && (
        <p className="border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">{loadError}</p>
      )}

      {!loaded ? (
        <div className="space-y-2">
          {[0, 1, 2].map((i) => (
            <div key={i} className="h-[92px] animate-pulse border border-zinc-800 bg-zinc-900/40" />
          ))}
        </div>
      ) : projects.length === 0 ? (
        <div className="border border-dashed border-zinc-800 py-20 text-center">
          <FileCode2 className="mx-auto mb-3 h-10 w-10 text-zinc-600" />
          <h3 className="text-lg font-semibold text-white">No compose projects yet</h3>
          <p className="mx-auto mt-1 max-w-md text-sm text-zinc-400">
            Start from a template, describe running containers, import an export, or add a compose file or scan a folder
            for them. Projects started elsewhere show up here on their own while their containers exist.
          </p>
          <div className="mt-4 flex items-center justify-center gap-2">
            <Button size="sm" variant="outline" className="rounded-none" onClick={() => setAdding(true)}>
              <FilePlus2 className="h-4 w-4" /> Add project
            </Button>
            <Button size="sm" variant="primary" className="rounded-none" onClick={() => setCreating(true)}>
              <PackagePlus className="h-4 w-4" /> New project
            </Button>
          </div>
        </div>
      ) : visible.length === 0 ? (
        <div className="border border-dashed border-zinc-800 py-14 text-center text-sm text-zinc-400">
          <SearchX className="mx-auto mb-2 h-8 w-8 text-zinc-600" />
          No project matches.
          <button
            type="button"
            className="ml-1 text-indigo-300 hover:underline"
            onClick={() => {
              setQuery("");
              setStatusFilter("all");
            }}
          >
            Clear filters
          </button>
        </div>
      ) : (
        <div className="space-y-2">
          {visible.map((p) => {
            const key = projectKey(p);
            const projectAlerts = alertsFor(alerts, p.name);
            const u = usage[key];
            return (
              <ComposeProjectCard
                key={key}
                project={p}
                busy={busy.has(p.name)}
                cpu={u?.cpu ?? 0}
                mem={u?.mem ?? 0}
                sampled={u?.sampled ?? 0}
                history={historyOf(key)}
                alertCount={projectAlerts.length}
                latestAlert={projectAlerts[0]?.message}
                updates={runnableUpdates[p.path] ?? NO_UPDATES}
                onOpen={openProject}
                onUp={upProject}
                onUpOptions={upOptionsProject}
                onRestart={restartProject}
                onStop={stopProject}
                onDown={downProject}
                onPull={pullProject}
                onBuild={buildProject}
                onPullUpdates={pullUpdatesProject}
                onShowAlerts={showAlertsProject}
                onStartWatch={startWatchProject}
                onStopWatch={stopWatchProject}
                onExport={exportProject}
                onTogglePin={togglePinProject}
                onCopyPath={copyPathProject}
                onOpenFolder={openFolderProject}
              />
            );
          })}
          <p className="flex items-center gap-1.5 pt-1 text-[11px] text-zinc-600">
            <Layers className="h-3 w-3" /> {visible.length} of {projects.length} projects · updates on Docker events, full
            refresh every {POLL_MS / 1000}s
          </p>
        </div>
      )}

      {selectedProject && (
        <ComposeDrawer
          key={projectKey(selectedProject)}
          project={selectedProject}
          stats={drawerStats}
          routes={gatewayRoutes}
          busy={busy.has(selectedProject.name)}
          onClose={() => setSelected(null)}
          onUp={() => void up(selectedProject)}
          onUpOptions={() => setUpFor(selectedProject)}
          onRestart={() => void projectAction(selectedProject, "restart")}
          onStop={() => void projectAction(selectedProject, "stop")}
          onDown={() => setDownFor(selectedProject)}
          onServiceAction={(row, action, replicas) => void serviceAction(selectedProject, row, action, replicas)}
          onRefresh={() => void load()}
        />
      )}

      {adding && (
        <AddProjectDialog
          known={projects.map((p) => p.path).filter(Boolean)}
          onClose={() => setAdding(false)}
          onAdded={() => {
            setAdding(false);
            void load();
          }}
        />
      )}
      {creating && (
        <NewProjectDialog
          onClose={() => setCreating(false)}
          onCreated={(path, upNow) => {
            setCreating(false);
            void load();
            if (upNow) {
              const name = folderName(path);
              void runOp(name, `Up ${name}`, (opId) => api.composeUp(path, undefined, opId));
            }
          }}
        />
      )}
      {upFor && (
        <UpDialog
          project={upFor}
          onClose={() => setUpFor(null)}
          onSubmit={(options) => {
            const p = upFor;
            setUpFor(null);
            void up(p, options);
          }}
        />
      )}
      {downFor && (
        <DownDialog
          project={downFor}
          onClose={() => setDownFor(null)}
          onSubmit={(options) => {
            const p = downFor;
            setDownFor(null);
            void down(p, options);
          }}
        />
      )}
      {pullFor && (
        <PullDialog
          project={pullFor}
          onClose={() => setPullFor(null)}
          onSubmit={(options) => {
            const p = pullFor;
            setPullFor(null);
            void pull(p, options, `Pull ${p.name}`);
          }}
        />
      )}
      {buildFor && (
        <BuildDialog
          project={buildFor}
          onClose={() => setBuildFor(null)}
          onSubmit={(options: ComposeBuildOptions) => {
            const p = buildFor;
            setBuildFor(null);
            void runOp(p.name, `Build ${p.name}`, (opId) => api.composeBuild(p.path, options, opId));
          }}
        />
      )}
      {exportFor && (
        <ExportDialog project={exportFor} onClose={() => setExportFor(null)} onDone={() => setExportFor(null)} />
      )}
      {alertsProject !== null && (
        <AlertsDialog
          project={alertsProject}
          alerts={alertsFor(alerts, alertsProject)}
          onClear={() => void clearProjectAlerts(alertsProject)}
          onClose={() => setAlertsProject(null)}
        />
      )}

      <ComposeProgressPanel
        ops={ops}
        onDismiss={(id) => setOps((prev) => prev.filter((o) => o.id !== id))}
        onClear={() => setOps((prev) => prev.filter((o) => o.status === "running"))}
      />
    </div>
  );
};
