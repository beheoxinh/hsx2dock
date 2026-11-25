import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FilePlus2, FileCode2, Layers, RefreshCw, SearchX } from "lucide-react";
import { Button } from "../components/ui/Button";
import { useToast } from "../components/ui/Toast";
import { EngineGate } from "../components/ui/EngineGate";
import { ComposeProjectCard } from "../components/compose/ComposeProjectCard";
import { ComposeDrawer } from "../components/compose/ComposeDrawer";
import { ComposeProgressPanel, appendOpLine, type ComposeOp } from "../components/compose/ComposeProgressPanel";
import { AddProjectDialog, DownDialog, SearchBox, UpDialog } from "../components/compose/ComposeDialogs";
import { inputClass } from "../components/compose/composeUi";
import { api, disposeListener, events } from "../lib/api";
import { useContainerStore } from "../lib/containerStore";
import {
  filterProjects,
  newOpId,
  projectStatus,
  sortProjects,
  type ComposeServiceRow,
  type ComposeSortKey,
  type ComposeStatusFilter,
} from "../lib/composeProject";
import { cn } from "../lib/utils";
import type {
  ComposeDownOptions,
  ComposeProjectInfo,
  ComposeServiceAction,
  ComposeUpOptions,
  ContainerStats,
  DaemonState,
} from "../types";

interface ComposePageProps {
  /** Engine state — the page renders an EngineGate panel unless Running. */
  daemonState?: DaemonState;
}

const POLL_MS = 5000;

const STATUS_FILTERS: { key: ComposeStatusFilter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "running", label: "Running" },
  { key: "partial", label: "Partial" },
  { key: "stopped", label: "Stopped" },
  { key: "error", label: "Error" },
];

export const ComposePage: React.FC<ComposePageProps> = ({ daemonState = "Running" }) => {
  const toast = useToast();
  const { gatewayRoutes } = useContainerStore();
  const [projects, setProjects] = useState<ComposeProjectInfo[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [stats, setStats] = useState<Record<string, ContainerStats>>({});
  const [query, setQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<ComposeStatusFilter>("all");
  const [sortKey, setSortKey] = useState<ComposeSortKey>("status");
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [ops, setOps] = useState<ComposeOp[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [upFor, setUpFor] = useState<ComposeProjectInfo | null>(null);
  const [downFor, setDownFor] = useState<ComposeProjectInfo | null>(null);
  const engineUp = daemonState === "Running";
  const loadSeq = useRef(0);

  const load = useCallback(async (manual = false) => {
    const seq = ++loadSeq.current;
    if (manual) setRefreshing(true);
    try {
      const list = await api.listComposeProjects();
      // A slow older poll must not overwrite a newer answer.
      if (seq !== loadSeq.current) return;
      setProjects(list);
      setLoadError(null);
    } catch (e) {
      if (seq === loadSeq.current) setLoadError(String(e));
    } finally {
      if (seq === loadSeq.current) setLoaded(true);
      if (manual) setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    if (!engineUp) return;
    void load();
    const t = window.setInterval(() => {
      if (!document.hidden) void load();
    }, POLL_MS);
    return () => window.clearInterval(t);
  }, [engineUp, load]);

  useEffect(() => {
    const offStats = disposeListener(events.onContainerStatsTick((tick) => setStats(tick ?? {})));
    const offProgress = disposeListener(
      events.onComposeProgress(({ op_id, line }) => setOps((prev) => appendOpLine(prev, op_id, line)))
    );
    return () => {
      offStats();
      offProgress();
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
      setOps((prev) => [...prev.slice(-19), { id, project, title, status: "running", lines: [], started: Date.now() }]);
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

  const projectKey = (p: ComposeProjectInfo) => `${p.name}|${p.path}`;
  const selectedProject = selected ? projects.find((p) => projectKey(p) === selected) ?? null : null;

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
          <Button size="sm" variant="ghost" onClick={() => void load(true)} title="Reload the project list">
            <RefreshCw className={cn("h-4 w-4", refreshing && "animate-spin")} />
          </Button>
          <Button size="sm" variant="primary" onClick={() => setAdding(true)}>
            <FilePlus2 className="h-4 w-4" /> Add project
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
            className={cn(inputClass, "w-auto")}
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
            Add a compose file or scan a folder for them. Projects started elsewhere show up here on their own while their
            containers exist.
          </p>
          <Button className="mt-4" size="sm" variant="primary" onClick={() => setAdding(true)}>
            <FilePlus2 className="h-4 w-4" /> Add project
          </Button>
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
          {visible.map((p) => (
            <ComposeProjectCard
              key={`${p.name}|${p.path}`}
              project={p}
              stats={stats}
              busy={busy.has(p.name)}
              onOpen={() => setSelected(projectKey(p))}
              onUp={() => void up(p)}
              onUpOptions={() => setUpFor(p)}
              onRestart={() => void projectAction(p, "restart")}
              onStop={() => void projectAction(p, "stop")}
              onDown={() => setDownFor(p)}
              onTogglePin={() => void togglePin(p)}
              onCopyPath={() => void copyPath(p)}
              onOpenFolder={() => void openFolder(p)}
            />
          ))}
          <p className="flex items-center gap-1.5 pt-1 text-[11px] text-zinc-600">
            <Layers className="h-3 w-3" /> {visible.length} of {projects.length} projects · refreshes every{" "}
            {POLL_MS / 1000}s
          </p>
        </div>
      )}

      {selectedProject && (
        <ComposeDrawer
          key={projectKey(selectedProject)}
          project={selectedProject}
          stats={stats}
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

      <ComposeProgressPanel
        ops={ops}
        onDismiss={(id) => setOps((prev) => prev.filter((o) => o.id !== id))}
        onClear={() => setOps((prev) => prev.filter((o) => o.status === "running"))}
      />
    </div>
  );
};
