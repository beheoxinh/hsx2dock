import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  CheckCircle2,
  Combine,
  Database,
  FileCode2,
  FileKey2,
  GitFork,
  Layers,
  ListTree,
  Loader2,
  Play,
  Power,
  RotateCw,
  ScrollText,
  SlidersHorizontal,
  Square,
  TerminalSquare,
  X,
  XCircle,
} from "lucide-react";
import { useModal } from "../ui/ModalContext";
import { useToast } from "../ui/Toast";
import { GroupLogsModal } from "../groups/GroupLogsModal";
import { ServicesTab } from "./ServicesTab";
import { GraphTab } from "./GraphTab";
import { ConfigTab } from "./ConfigTab";
import { EnvTab } from "./EnvTab";
import { TerminalTab } from "./TerminalTab";
import { VolumesTab } from "./VolumesTab";
import { MergedTab } from "./MergedTab";
import { useDrawerOps } from "./useDrawerOps";
import { ExposeDialog, ScaleDialog } from "./ComposeDialogs";
import { IconAction, InfoCell, STATUS_STYLE } from "./composeUi";
import { api } from "../../lib/api";
import { projectUsage } from "../../lib/composeStats";
import { isRunning, projectCounts, projectStatus, serviceRows, type ComposeServiceRow } from "../../lib/composeProject";
import { cn, formatBytes } from "../../lib/utils";
import type {
  ComposeProjectInfo,
  ComposeServiceAction,
  ContainerItem,
  ContainerStats,
  GatewayRoute,
} from "../../types";

export type DrawerTab = "services" | "logs" | "graph" | "volumes" | "config" | "merged" | "env" | "terminal";

export interface ComposeDrawerProps {
  project: ComposeProjectInfo;
  stats: Record<string, ContainerStats>;
  routes: GatewayRoute[];
  busy: boolean;
  initialTab?: DrawerTab;
  onClose: () => void;
  onUp: () => void;
  onUpOptions: () => void;
  onRestart: () => void;
  onStop: () => void;
  onDown: () => void;
  onServiceAction: (row: ComposeServiceRow, action: ComposeServiceAction, replicas?: number) => void;
  onRefresh: () => void;
}

const TABS: { key: DrawerTab; label: string; icon: React.ReactNode; needsFile?: boolean }[] = [
  { key: "services", label: "Services", icon: <ListTree className="h-3.5 w-3.5" /> },
  { key: "logs", label: "Logs", icon: <ScrollText className="h-3.5 w-3.5" /> },
  { key: "graph", label: "Graph", icon: <GitFork className="h-3.5 w-3.5" /> },
  { key: "volumes", label: "Volumes", icon: <Database className="h-3.5 w-3.5" /> },
  { key: "config", label: "Compose file", icon: <FileCode2 className="h-3.5 w-3.5" />, needsFile: true },
  { key: "merged", label: "Merged config", icon: <Combine className="h-3.5 w-3.5" />, needsFile: true },
  { key: "env", label: "Env", icon: <FileKey2 className="h-3.5 w-3.5" />, needsFile: true },
  { key: "terminal", label: "Terminal", icon: <TerminalSquare className="h-3.5 w-3.5" /> },
];

/** Last path segment, for the file selector. */
function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/** The minimal ContainerItem the shared log viewer needs. */
function asContainerItems(rows: ComposeServiceRow[]): ContainerItem[] {
  return rows.flatMap((r) =>
    r.containers.map((c) => ({
      id: c.id,
      short_id: c.id.slice(0, 12),
      name: c.name,
      image: c.image,
      state: c.state,
      status: c.state,
      created: 0,
      ports: c.ports,
      compose_service: r.name,
    }))
  );
}

/**
 * Everything about one compose project in a right-hand panel: per-service
 * table with actions, live logs, the depends_on graph, the compose file and
 * env file editors, and a shell into any running container.
 */
export const ComposeDrawer: React.FC<ComposeDrawerProps> = (props) => {
  const { project: p, stats, routes, busy, onClose } = props;
  const modal = useModal();
  const [tab, setTab] = useState<DrawerTab>(props.initialTab ?? "services");
  const [visited, setVisited] = useState<Set<DrawerTab>>(new Set([props.initialTab ?? "services"]));
  const [dirty, setDirty] = useState<{ config: boolean; env: boolean }>({ config: false, env: false });
  const [logService, setLogService] = useState<string | null>(null);
  const [terminalId, setTerminalId] = useState<string | null>(null);
  const [scaleRow, setScaleRow] = useState<ComposeServiceRow | null>(null);
  const [exposeRow, setExposeRow] = useState<ComposeServiceRow | null>(null);
  // Which file the Compose file tab edits: the base file or the override file.
  const [cfgFile, setCfgFile] = useState<"base" | "override">("base");
  const [hiddenOpId, setHiddenOpId] = useState<string | null>(null);
  const toast = useToast();
  // Pull, build and volume operations run from the drawer (see useDrawerOps).
  const ops = useDrawerOps();
  const anyBusy = busy || ops.busy;
  const activeCfg: "base" | "override" = p.override_file && cfgFile === "override" ? "override" : "base";
  const configPath = activeCfg === "override" && p.override_file ? p.override_file : p.path;
  const shownOp = ops.latest && ops.latest.id !== hiddenOpId ? ops.latest : null;

  const rows = useMemo(() => serviceRows(p), [p]);
  const status = projectStatus(p);
  const style = STATUS_STYLE[status];
  const { running, total } = projectCounts(p);
  const usage = projectUsage(p, stats);
  const hasFile = Boolean(p.path) && !p.config_error;
  const canEditFile = Boolean(p.path);

  const open = (next: DrawerTab) => {
    setTab(next);
    setVisited((prev) => (prev.has(next) ? prev : new Set(prev).add(next)));
  };

  const requestClose = useCallback(async () => {
    if (dirty.config || dirty.env) {
      const ok = await modal.showConfirm(
        "The compose file or env file has unsaved changes. Close and discard them?",
        "Unsaved changes",
        "warning"
      );
      if (!ok) return;
    }
    onClose();
  }, [dirty, modal, onClose]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Dialogs above the drawer handle their own Escape first.
      if (e.key === "Escape" && !scaleRow && !exposeRow) void requestClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [requestClose, scaleRow, exposeRow]);

  // Switching files remounts the editor (key below); unsaved edits of the old file are dropped only on confirm.
  const switchConfigFile = async (next: "base" | "override") => {
    if (next === activeCfg) return;
    if (dirty.config) {
      const ok = await modal.showConfirm(
        "The open compose file has unsaved changes. Switch files and discard them?",
        "Unsaved changes",
        "warning"
      );
      if (!ok) return;
      setConfigDirty(false);
    }
    setCfgFile(next);
  };

  // The list is refreshed after each run, so the project card shows the new state too.
  const pullService = async (row: ComposeServiceRow): Promise<boolean> => {
    const r = await ops.run(p.name, `Pull ${row.name}`, (opId) => api.composePull(p.path, { services: [row.name] }, opId));
    props.onRefresh();
    if (r.ok) toast.info(`Pulled ${row.name}`, "Recreate the service to run the new image.");
    return r.ok;
  };

  const buildService = async (row: ComposeServiceRow): Promise<boolean> => {
    const r = await ops.run(p.name, `Build ${row.name}`, (opId) => api.composeBuild(p.path, { services: [row.name] }, opId));
    props.onRefresh();
    if (r.ok) toast.info(`Built ${row.name}`, "Recreate the service to run the new image.");
    return r.ok;
  };

  const setConfigDirty = useCallback((d: boolean) => setDirty((s) => (s.config === d ? s : { ...s, config: d })), []);
  const setEnvDirty = useCallback((d: boolean) => setDirty((s) => (s.env === d ? s : { ...s, env: d })), []);

  // Logs: the selected service's containers, or all. Keyed by the id list so
  // the viewer does not restart on every 5s refresh of the project.
  const logRows = logService ? rows.filter((r) => r.name === logService) : rows;
  const logKey = logRows.flatMap((r) => r.containers.map((c) => c.id)).join(",");
  const logContainers = useMemo(() => asContainerItems(logRows), [logKey]);

  const anyRunning = running > 0;

  return (
    <div className="fixed inset-0 z-50 flex justify-end bg-black/50" onClick={() => void requestClose()}>
      <aside
        role="dialog"
        aria-label={`Compose project ${p.name}`}
        onClick={(e) => e.stopPropagation()}
        className="flex h-full w-[min(1120px,94vw)] flex-col border-l border-zinc-800 bg-zinc-950 shadow-2xl"
      >
        {/* Header */}
        <div className="border-b border-zinc-800 px-5 py-3">
          <div className="flex items-center gap-3">
            <span
              role="img"
              aria-label={`Status: ${style.label}`}
              title={`${style.label} — ${running}/${total} services running`}
              className={cn("flex h-9 w-9 shrink-0 items-center justify-center border", style.box)}
            >
              {busy ? (
                <Loader2 className={cn("h-4 w-4 animate-spin", style.icon)} aria-hidden="true" />
              ) : (
                <Layers className={cn("h-4 w-4", style.icon)} aria-hidden="true" />
              )}
            </span>
            <div className="min-w-0 flex-1">
              <h2 className="truncate text-lg font-semibold text-white">{p.name}</h2>
              <p className="truncate font-mono text-[11px] text-zinc-500" title={p.path}>
                {p.path || "compose file unknown — found from container labels"}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-1">
              {anyRunning ? (
                <IconAction title="Restart all services" onClick={props.onRestart} disabled={busy}>
                  <RotateCw className="h-4 w-4" />
                </IconAction>
              ) : null}
              <IconAction
                title={hasFile ? "Up (create and start everything)" : "Needs a readable compose file"}
                onClick={props.onUp}
                disabled={busy || !hasFile}
                className="text-emerald-300"
              >
                <Play className="h-4 w-4" />
              </IconAction>
              <IconAction title="Up with options…" onClick={props.onUpOptions} disabled={busy || !hasFile}>
                <SlidersHorizontal className="h-4 w-4" />
              </IconAction>
              <IconAction title="Stop all services" onClick={props.onStop} disabled={busy || !anyRunning}>
                <Square className="h-4 w-4" />
              </IconAction>
              <IconAction title="Down (remove containers and networks)…" onClick={props.onDown} disabled={busy} danger>
                <Power className="h-4 w-4" />
              </IconAction>
              <span className="mx-1 h-6 w-px bg-zinc-800" aria-hidden="true" />
              <IconAction title="Close (Esc)" onClick={() => void requestClose()}>
                <X className="h-4 w-4" />
              </IconAction>
            </div>
          </div>
          <div className="mt-3 grid grid-cols-2 divide-zinc-800 border border-zinc-800 sm:grid-cols-5 sm:divide-x">
            <InfoCell label="Services">
              {running}/{total} running
            </InfoCell>
            <InfoCell label="Containers">{p.services.length}</InfoCell>
            <InfoCell label="CPU">{usage.sampled ? `${usage.cpu.toFixed(1)}%` : "—"}</InfoCell>
            <InfoCell label="Memory">{usage.sampled ? formatBytes(usage.mem) : "—"}</InfoCell>
            <InfoCell label="Profiles" title={(p.profiles ?? []).join(", ")}>
              {(p.profiles ?? []).length ? p.profiles!.join(", ") : "—"}
            </InfoCell>
          </div>
          {p.config_error && (
            <p className="mt-2 border border-rose-500/40 bg-rose-500/10 px-3 py-1.5 font-mono text-[11px] text-rose-200">
              {p.config_error}
            </p>
          )}
        </div>

        {/* Tabs */}
        <div role="tablist" className="flex gap-0 overflow-x-auto border-b border-zinc-800 px-3">
          {TABS.map((t) => {
            const disabled = t.needsFile && !canEditFile;
            const isDirty = (t.key === "config" && dirty.config) || (t.key === "env" && dirty.env);
            return (
              <button
                key={t.key}
                type="button"
                role="tab"
                aria-selected={tab === t.key}
                disabled={disabled}
                title={disabled ? "This project has no known compose file" : undefined}
                onClick={() => open(t.key)}
                className={cn(
                  "flex items-center gap-1.5 whitespace-nowrap border-b-2 px-3 py-2 text-xs",
                  tab === t.key ? "border-indigo-500 text-white" : "border-transparent text-zinc-400 hover:text-zinc-200",
                  disabled && "cursor-not-allowed opacity-40"
                )}
              >
                {t.icon}
                {t.label}
                {isDirty && <span className="h-1.5 w-1.5 rounded-full bg-amber-400" aria-label="unsaved" />}
              </button>
            );
          })}
        </div>

        {/* Output of the last drawer operation (pull, build, volume backup/restore). */}
        {shownOp && (
          <div className="mx-4 mt-3 shrink-0 border border-zinc-800 bg-zinc-900/60">
            <div className="flex items-center gap-2 px-3 py-1.5 text-xs">
              {shownOp.status === "running" ? (
                <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-sky-300" aria-label="Running" />
              ) : shownOp.status === "ok" ? (
                <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-400" aria-label="Done" />
              ) : (
                <XCircle className="h-3.5 w-3.5 shrink-0 text-rose-400" aria-label="Failed" />
              )}
              <span className="min-w-0 flex-1 truncate text-zinc-200">{shownOp.title}</span>
              <IconAction title="Hide output" onClick={() => setHiddenOpId(shownOp.id)}>
                <X className="h-4 w-4" />
              </IconAction>
            </div>
            {(shownOp.lines.length > 0 || shownOp.error) && (
              <pre className="max-h-40 overflow-auto whitespace-pre-wrap border-t border-zinc-800 p-2 font-mono text-[11px] text-zinc-400">
                {shownOp.lines.slice(-200).join("\n")}
                {shownOp.error && <span className="text-rose-300">{`\n${shownOp.error}`}</span>}
              </pre>
            )}
          </div>
        )}

        {/* Body */}
        <div className="min-h-0 flex-1 overflow-hidden p-4">
          {tab === "services" && (
            <div className="h-full overflow-y-auto">
              <ServicesTab
                rows={rows}
                stats={stats}
                routes={routes}
                hasFile={hasFile}
                filePath={p.path}
                busy={anyBusy}
                onPull={pullService}
                onBuild={buildService}
                onAction={(row, action) => props.onServiceAction(row, action)}
                onScale={setScaleRow}
                onExpose={setExposeRow}
                onLogs={(row) => {
                  setLogService(row.name);
                  open("logs");
                }}
                onTerminal={(row) => {
                  setTerminalId(row.containers.find(isRunning)?.id ?? null);
                  open("terminal");
                }}
              />
            </div>
          )}

          {tab === "logs" && (
            <div className="flex h-full min-h-0 flex-col gap-2">
              <div className="flex flex-wrap gap-1">
                {[null, ...rows.filter((r) => r.containers.length > 0).map((r) => r.name)].map((name) => (
                  <button
                    key={name ?? "__all"}
                    type="button"
                    onClick={() => setLogService(name)}
                    className={cn(
                      "border px-2 py-0.5 text-[11px]",
                      logService === name
                        ? "border-indigo-500/60 bg-indigo-500/15 text-indigo-200"
                        : "border-zinc-700 text-zinc-400 hover:text-zinc-200"
                    )}
                  >
                    {name ?? "All services"}
                  </button>
                ))}
              </div>
              <div className="min-h-0 flex-1">
                {logContainers.length > 0 ? (
                  <GroupLogsModal
                    key={logKey}
                    embedded
                    isOpen
                    onClose={() => undefined}
                    groupName={logService ? `${p.name} / ${logService}` : p.name}
                    containers={logContainers}
                  />
                ) : (
                  <p className="p-6 text-center text-sm text-zinc-500">No containers yet — bring the project up first.</p>
                )}
              </div>
            </div>
          )}

          {tab === "volumes" && (
            <div className="h-full">
              <VolumesTab project={p.name} busy={anyBusy} run={(title, fn) => ops.run(p.name, title, fn)} />
            </div>
          )}

          {tab === "graph" && (
            <div className="h-full overflow-auto">
              <GraphTab
                rows={rows}
                onSelect={(row) => {
                  if (row.containers.length) {
                    setLogService(row.name);
                    open("logs");
                  }
                }}
              />
            </div>
          )}

          {/* Editors stay mounted once opened so drafts survive tab switches. */}
          {canEditFile && visited.has("config") && (
            <div className={cn("flex h-full flex-col gap-2", tab !== "config" && "hidden")}>
              {p.override_file && (
                <div role="group" aria-label="File to edit" className="flex shrink-0 items-center gap-3 text-xs">
                  <div className="inline-flex border border-zinc-700">
                    {(["base", "override"] as const).map((k) => {
                      const path = k === "base" ? p.path : p.override_file!;
                      const active = activeCfg === k;
                      return (
                        <button
                          key={k}
                          type="button"
                          aria-pressed={active}
                          title={path}
                          onClick={() => void switchConfigFile(k)}
                          className={cn(
                            "px-3 py-1 font-mono text-[11px]",
                            active ? "bg-indigo-500/15 text-indigo-200" : "text-zinc-400 hover:text-zinc-200"
                          )}
                        >
                          {baseName(path)}
                        </button>
                      );
                    })}
                  </div>
                  <span className="text-[11px] text-zinc-500">The override file is merged onto the base file.</span>
                </div>
              )}
              {/* Keyed by file so each editor starts with its own clean state. */}
              <div className="min-h-0 flex-1">
                <ConfigTab
                  key={configPath}
                  filePath={configPath}
                  busy={busy}
                  onSaved={props.onRefresh}
                  onApply={props.onUp}
                  onDirtyChange={setConfigDirty}
                  active={tab === "config"}
                />
              </div>
            </div>
          )}
          {canEditFile && tab === "merged" && <MergedTab key={p.path} filePath={p.path} />}
          {canEditFile && visited.has("env") && (
            <div className={cn("h-full", tab !== "env" && "hidden")}>
              <EnvTab
                filePath={p.path}
                busy={busy}
                onSaved={props.onRefresh}
                onApply={props.onUp}
                onDirtyChange={setEnvDirty}
              />
            </div>
          )}

          {tab === "terminal" && <TerminalTab rows={rows} containerId={terminalId} onSelect={setTerminalId} />}
        </div>
      </aside>

      {/* Dialog clicks must not reach the drawer backdrop below. */}
      <div onClick={(e) => e.stopPropagation()}>
      {scaleRow && (
        <ScaleDialog
          service={scaleRow.name}
          current={scaleRow.containers.length}
          declared={scaleRow.spec?.replicas ?? 1}
          onClose={() => setScaleRow(null)}
          onSubmit={(n) => {
            props.onServiceAction(scaleRow, "scale", n);
            setScaleRow(null);
          }}
        />
      )}
      {exposeRow && <ExposeDialog project={p} row={exposeRow} onClose={() => setExposeRow(null)} />}
      </div>
    </div>
  );
};
