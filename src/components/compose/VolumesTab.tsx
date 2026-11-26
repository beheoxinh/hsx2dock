import React, { useCallback, useEffect, useState } from "react";
import { AlertCircle, Archive, ArchiveRestore, Loader2, RefreshCw, X } from "lucide-react";
import { useModal } from "../ui/ModalContext";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import { cn, formatBytes } from "../../lib/utils";
import type { ComposeProjectVolume } from "../../types";
import { IconAction, inputClass } from "./composeUi";
import type { DrawerRunResult } from "./useDrawerOps";

export interface VolumesTabProps {
  project: string;
  busy: boolean;
  run: <T>(title: string, fn: (opId: string) => Promise<T>) => Promise<DrawerRunResult<T>>;
}

/**
 * The project's named volumes with backup and restore. Backups are archives
 * in ~/hsx2dock-backups/volumes; restore replaces a volume's content, so it
 * is confirmed and refused while a running container uses the volume.
 */
export const VolumesTab: React.FC<VolumesTabProps> = ({ project, busy, run }) => {
  const modal = useModal();
  const toast = useToast();
  const [volumes, setVolumes] = useState<ComposeProjectVolume[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  // The restore form opens inline under one volume at a time.
  const [restoreFor, setRestoreFor] = useState<string | null>(null);
  const [archive, setArchive] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setVolumes(await api.composeProjectVolumes(project));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [project]);

  useEffect(() => {
    void load();
  }, [load]);

  const backup = async (v: ComposeProjectVolume) => {
    const r = await run(`Backup ${v.name}`, (opId) => api.volumeBackup(v.name, undefined, opId));
    if (r.ok) toast.success("Volume backed up", r.value);
  };

  const restore = async (v: ComposeProjectVolume) => {
    const path = archive.trim();
    if (!path) return;
    const ok = await modal.showConfirm(
      `Replace all content of volume "${v.name}" with ${path}? The current content is overwritten and this cannot be undone.`,
      "Restore volume",
      "warning"
    );
    if (!ok) return;
    const r = await run(`Restore ${v.name}`, (opId) => api.volumeRestore(v.name, path, opId));
    if (r.ok) {
      toast.success("Volume restored", v.name);
      setRestoreFor(null);
      setArchive("");
      void load();
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 text-[11px] text-zinc-500">
          Backups go to <span className="font-mono text-zinc-400">~/hsx2dock-backups/volumes</span>.
        </p>
        <IconAction title="Refresh volumes" onClick={() => void load()} disabled={loading}>
          {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
        </IconAction>
      </div>

      {error && (
        <p className="flex items-center gap-1.5 border border-rose-500/40 bg-rose-500/10 px-3 py-1.5 font-mono text-[11px] text-rose-200">
          <AlertCircle className="h-3 w-3 shrink-0" aria-hidden="true" />
          {error}
        </p>
      )}

      {volumes === null && !error ? (
        <p className="p-6 text-center text-sm text-zinc-500">Loading volumes…</p>
      ) : volumes && volumes.length === 0 ? (
        <p className="p-6 text-center text-sm text-zinc-500">This project has no named volumes.</p>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="divide-y divide-zinc-800 border border-zinc-800">
            <div className="hidden grid-cols-[minmax(0,1.6fr)_6rem_5rem_minmax(0,1fr)_auto] gap-3 bg-zinc-900/60 px-3 py-1.5 text-[10px] uppercase tracking-wider text-zinc-500 lg:grid">
              <span>Volume</span>
              <span>Driver</span>
              <span>Size</span>
              <span>Used by</span>
              <span className="text-right">Actions</span>
            </div>
            {(volumes ?? []).map((v) => {
              const inUse = v.running_users.length > 0;
              const open = restoreFor === v.name;
              return (
                <div key={v.name}>
                  <div className="grid grid-cols-1 items-center gap-2 px-3 py-2 text-xs lg:grid-cols-[minmax(0,1.6fr)_6rem_5rem_minmax(0,1fr)_auto] lg:gap-3">
                    <span className="truncate font-mono text-zinc-100" title={v.name}>
                      {v.name}
                    </span>
                    <span className="font-mono text-zinc-400">{v.driver || "—"}</span>
                    <span className="font-mono text-zinc-300">{v.size == null ? "—" : formatBytes(v.size)}</span>
                    <span
                      className={cn("truncate", inUse ? "text-amber-300" : "text-zinc-500")}
                      title={v.used_by.length ? v.used_by.join(", ") : "Not used by any container"}
                    >
                      {v.used_by.length ? v.used_by.join(", ") : "—"}
                    </span>
                    <div className="flex items-center justify-end gap-1">
                      <IconAction
                        title={`Backup ${v.name} to ~/hsx2dock-backups/volumes`}
                        onClick={() => void backup(v)}
                        disabled={busy}
                      >
                        <Archive className="h-3.5 w-3.5" />
                      </IconAction>
                      <IconAction
                        title={
                          inUse
                            ? `Stop ${v.running_users.join(", ")} first`
                            : `Restore ${v.name} from an archive`
                        }
                        onClick={() => {
                          setRestoreFor(open ? null : v.name);
                          setArchive("");
                        }}
                        disabled={busy || inUse}
                      >
                        <ArchiveRestore className="h-3.5 w-3.5" />
                      </IconAction>
                    </div>
                  </div>
                  {open && (
                    <div className="flex flex-col gap-2 border-t border-zinc-800 bg-zinc-900/40 px-3 py-2 sm:flex-row sm:items-center">
                      <label htmlFor={`restore-${v.name}`} className="shrink-0 text-[11px] text-zinc-400">
                        Archive path
                      </label>
                      <input
                        id={`restore-${v.name}`}
                        autoFocus
                        value={archive}
                        onChange={(e) => setArchive(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") void restore(v);
                          if (e.key === "Escape") setRestoreFor(null);
                        }}
                        placeholder="/home/you/hsx2dock-backups/volumes/….tar.gz"
                        className={cn(inputClass, "min-w-0 flex-1 font-mono")}
                      />
                      <div className="flex shrink-0 items-center gap-1">
                        <button
                          type="button"
                          onClick={() => void restore(v)}
                          disabled={busy || !archive.trim()}
                          className="h-8 border border-amber-500/50 px-3 text-xs text-amber-200 hover:bg-amber-500/10 disabled:cursor-not-allowed disabled:opacity-40"
                        >
                          Restore
                        </button>
                        <IconAction title="Cancel restore" onClick={() => setRestoreFor(null)}>
                          <X className="h-4 w-4" />
                        </IconAction>
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
};
