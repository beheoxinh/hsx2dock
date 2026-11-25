import React, { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { X } from "lucide-react";
import { Button } from "../ui/Button";
import { useModal } from "../ui/ModalContext";
import { useToast } from "../ui/Toast";
import { api, disposeListener } from "../../lib/api";
import { formatBytes, formatRelativeTime } from "../../lib/utils";
import { isSingleControlPlane, validateCloneName } from "../../lib/snapshots";
import type { ClusterSnapshot, SnapshotProgressPayload } from "../../types";

type Busy = "create" | "restore" | "clone" | "delete";

const INPUT =
  "h-9 w-full min-w-0 px-3 rounded-none bg-zinc-950 border border-zinc-800 text-sm text-white placeholder-zinc-600 focus:outline-none focus:border-blue-500 disabled:opacity-60";

/** `created` is RFC 3339; the relative-time helper takes Unix seconds. NaN shows as "-". */
function createdSeconds(created: string): number {
  return Math.floor(Date.parse(created) / 1000);
}

interface SnapshotsModalProps {
  cluster: string;
  /** Whether the cluster has running nodes; a running cluster is stopped while a snapshot is taken. */
  running: boolean;
  onClose: () => void;
  /** Called after an operation that changes the cluster or creates one. */
  onChanged: () => void;
}

/**
 * Lists, takes, restores, clones and deletes the snapshots of one cluster.
 * Only one operation runs at a time; every action is disabled meanwhile.
 */
export const SnapshotsModal: React.FC<SnapshotsModalProps> = ({
  cluster,
  running,
  onClose,
  onChanged,
}) => {
  const toast = useToast();
  const modal = useModal();
  const [snapshots, setSnapshots] = useState<ClusterSnapshot[] | null>(null);
  const [busy, setBusy] = useState<Busy | null>(null);
  // Mirrors `busy` for handlers that run before React re-renders.
  const busyRef = useRef(false);
  // Confirm dialogs open on top; Escape must not close this modal under them.
  const dialogOpen = useRef(false);
  const [note, setNote] = useState("");
  const [progress, setProgress] = useState<{ line: string; error: string | null } | null>(null);
  // The cluster whose progress events are shown: this one, or a clone target.
  const watchRef = useRef(cluster);
  const [cloneOf, setCloneOf] = useState<string | null>(null);
  const [cloneName, setCloneName] = useState("");
  const [clusterNames, setClusterNames] = useState<string[] | null>(null);
  const [namesLoading, setNamesLoading] = useState(false);

  const locked = busy !== null;

  const load = useCallback(async () => {
    try {
      setSnapshots(await api.listClusterSnapshots(cluster));
    } catch (e) {
      setSnapshots([]);
      toast.error("Could not load snapshots", String(e));
    }
  }, [cluster, toast]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(
    () =>
      disposeListener(
        listen<SnapshotProgressPayload>("snapshot-progress", (e) => {
          const p = e.payload;
          if (p.cluster !== cluster && p.cluster !== watchRef.current) return;
          setProgress({ line: p.line, error: p.error });
        })
      ),
    [cluster]
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busyRef.current && !dialogOpen.current) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const confirmAction = async (message: string, title: string, type: "warning" | "danger") => {
    dialogOpen.current = true;
    try {
      return await modal.showConfirm(message, title, type);
    } finally {
      dialogOpen.current = false;
    }
  };

  /** Runs one operation under the single busy flag, reporting failures as a toast. */
  const run = async (kind: Busy, watch: string, failTitle: string, op: () => Promise<void>) => {
    if (busyRef.current) return;
    busyRef.current = true;
    watchRef.current = watch;
    setBusy(kind);
    setProgress(null);
    try {
      await op();
    } catch (e) {
      toast.error(failTitle, String(e));
    } finally {
      watchRef.current = cluster;
      busyRef.current = false;
      setBusy(null);
    }
  };

  const takeSnapshot = async () => {
    if (busyRef.current) return;
    if (running) {
      const ok = await confirmAction(
        `${cluster} is running. It will be stopped while the snapshot is taken and started again afterwards. This takes several minutes, and the API server is unavailable until the cluster is back.`,
        "Stop the cluster and snapshot it",
        "warning"
      );
      if (!ok) return;
    }
    await run("create", cluster, "Snapshot failed", async () => {
      const id = await api.createClusterSnapshot(cluster, note.trim() || undefined);
      toast.success("Snapshot saved", id);
      setNote("");
      await load();
      onChanged();
    });
  };

  const restore = async (snap: ClusterSnapshot) => {
    if (busyRef.current) return;
    const ok = await confirmAction(
      `Restoring ${snap.id} replaces the current state of ${cluster}. Everything created or changed since the snapshot is lost, including workloads and cluster data. The cluster is stopped and rebuilt, which takes several minutes.`,
      "Restore snapshot",
      "danger"
    );
    if (!ok) return;
    await run("restore", cluster, "Restore failed", async () => {
      const msg = await api.restoreClusterSnapshot(cluster, snap.id);
      toast.info("Restore finished", msg);
      await load();
      onChanged();
    });
  };

  const openClone = async (snap: ClusterSnapshot) => {
    setCloneOf(snap.id);
    setCloneName("");
    setNamesLoading(true);
    setClusterNames(null);
    try {
      const list = await api.listKindClusters();
      setClusterNames(list.map((c) => c.name));
    } catch (e) {
      toast.error("Could not load cluster names", String(e));
    } finally {
      setNamesLoading(false);
    }
  };

  const cloneIssue =
    clusterNames && cloneName !== "" ? validateCloneName(cloneName, clusterNames) : null;
  const cloneReady = clusterNames !== null && validateCloneName(cloneName, clusterNames) === null;

  const submitClone = async (snap: ClusterSnapshot) => {
    if (busyRef.current || !cloneReady) return;
    const target = cloneName.trim();
    await run("clone", target, "Clone failed", async () => {
      const msg = await api.restoreClusterSnapshot(cluster, snap.id, target);
      toast.success("Clone created", msg);
      setCloneOf(null);
      setCloneName("");
      await load();
      onChanged();
    });
  };

  const remove = async (snap: ClusterSnapshot) => {
    if (busyRef.current) return;
    const ok = await confirmAction(
      `Delete snapshot ${snap.id} of ${cluster}? Its saved node images and archived state are removed. This cannot be undone.`,
      "Delete snapshot",
      "danger"
    );
    if (!ok) return;
    await run("delete", cluster, "Delete failed", async () => {
      await api.deleteClusterSnapshot(cluster, snap.id);
      toast.success("Snapshot deleted", snap.id);
      if (cloneOf === snap.id) setCloneOf(null);
      await load();
    });
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={() => !locked && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="snapshots-title"
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[90vh] w-full max-w-3xl flex-col rounded-none border border-zinc-800 bg-zinc-900 shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-zinc-800 px-5 py-4">
          <div>
            <h2 id="snapshots-title" className="text-sm font-semibold text-white">
              Snapshots: {cluster}
            </h2>
            <p className="mt-0.5 text-[11px] text-zinc-500">
              Full copies of every node, kept until you delete them.
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={locked}
            aria-label="Close snapshots"
            title="Close"
            className="rounded-none p-1 text-zinc-400 hover:text-white outline-none focus-visible:ring-2 focus-visible:ring-blue-500 disabled:opacity-40"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="flex-1 space-y-5 overflow-y-auto px-5 py-4">
          <section className="space-y-3 rounded-none border border-zinc-800 p-4">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-400">
              Take snapshot
            </h3>
            <p className="text-xs leading-relaxed text-amber-300/90">
              {running
                ? `${cluster} is running. It is stopped while the snapshot is taken and started again afterwards.`
                : `${cluster} is stopped, so its state is saved as it is.`}{" "}
              A snapshot takes several minutes.
            </p>
            <div className="flex flex-wrap gap-2">
              <input
                type="text"
                value={note}
                onChange={(e) => setNote(e.target.value)}
                placeholder="Note (optional)"
                aria-label="Snapshot note"
                maxLength={200}
                disabled={locked}
                className={`${INPUT} flex-1`}
              />
              <Button
                variant="primary"
                size="md"
                className="rounded-none shrink-0"
                onClick={takeSnapshot}
                disabled={locked}
                isLoading={busy === "create"}
              >
                Take snapshot
              </Button>
            </div>
          </section>

          {busy && (
            <div
              role="status"
              className={`min-h-9 break-all rounded-none border px-3 py-2 font-mono text-xs ${
                progress?.error
                  ? "border-rose-500/40 bg-rose-500/10 text-rose-300"
                  : "border-zinc-800 bg-zinc-950 text-zinc-300"
              }`}
            >
              {progress?.error ?? progress?.line ?? "Working…"}
            </div>
          )}

          <section>
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-zinc-400">
              Saved snapshots
            </h3>
            {snapshots === null ? (
              <p className="text-sm text-zinc-500">Loading…</p>
            ) : snapshots.length === 0 ? (
              <p className="rounded-none border border-dashed border-zinc-800 px-4 py-6 text-center text-sm text-zinc-500">
                No snapshots yet.
              </p>
            ) : (
              <ul className="divide-y divide-zinc-800 rounded-none border border-zinc-800">
                {snapshots.map((snap) => {
                  const cloneable = isSingleControlPlane(snap);
                  const cloneOpen = cloneOf === snap.id;
                  return (
                    <li key={`${snap.cluster}/${snap.id}`} className="px-4 py-3">
                      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
                        <div className="min-w-0 flex-1 basis-48">
                          <p className="truncate font-mono text-sm text-white" title={snap.id}>
                            {snap.id}
                          </p>
                          <p className="truncate text-xs text-zinc-500" title={snap.note ?? undefined}>
                            {snap.note || "No note"}
                          </p>
                        </div>
                        <div className="flex shrink-0 divide-x divide-zinc-800 text-xs text-zinc-400">
                          <div className="px-3" title={snap.created}>
                            {formatRelativeTime(createdSeconds(snap.created))}
                          </div>
                          <div className="px-3">
                            {snap.nodes.length} {snap.nodes.length === 1 ? "node" : "nodes"}
                          </div>
                          <div className="px-3">{formatBytes(snap.sizeBytes)}</div>
                        </div>
                        <div className="flex shrink-0 flex-wrap gap-2">
                          <Button
                            size="sm"
                            variant="secondary"
                            className="rounded-none"
                            disabled={locked}
                            onClick={() => void restore(snap)}
                          >
                            Restore
                          </Button>
                          <span
                            className="inline-flex"
                            title={cloneable ? undefined : "Only single control-plane snapshots can be cloned"}
                          >
                            <Button
                              size="sm"
                              variant="outline"
                              className="rounded-none"
                              disabled={locked || !cloneable}
                              title={cloneable ? "Clone this snapshot into a new cluster" : "Only single control-plane snapshots can be cloned"}
                              onClick={() => (cloneOpen ? setCloneOf(null) : void openClone(snap))}
                            >
                              {cloneOpen ? "Cancel clone" : "Clone…"}
                            </Button>
                          </span>
                          <Button
                            size="sm"
                            variant="ghost"
                            className="rounded-none text-rose-300 hover:text-rose-200"
                            disabled={locked}
                            onClick={() => void remove(snap)}
                          >
                            Delete
                          </Button>
                        </div>
                      </div>

                      {cloneOpen && (
                        <div className="mt-3 space-y-2 rounded-none border border-zinc-800 bg-zinc-950/60 p-3">
                          <label htmlFor={`clone-name-${snap.id}`} className="block text-xs text-zinc-400">
                            New cluster name
                          </label>
                          <div className="flex flex-wrap gap-2">
                            <input
                              id={`clone-name-${snap.id}`}
                              type="text"
                              autoFocus
                              value={cloneName}
                              onChange={(e) => setCloneName(e.target.value)}
                              placeholder="e.g. dev-copy"
                              disabled={locked || namesLoading}
                              className={`${INPUT} flex-1`}
                            />
                            <Button
                              size="sm"
                              variant="primary"
                              className="rounded-none shrink-0"
                              disabled={locked || !cloneReady}
                              isLoading={busy === "clone"}
                              onClick={() => void submitClone(snap)}
                            >
                              Create clone
                            </Button>
                          </div>
                          <p className="text-xs">
                            {namesLoading ? (
                              <span className="text-zinc-500">Checking existing clusters…</span>
                            ) : clusterNames === null ? (
                              <span className="text-rose-400">Cluster names are unavailable, so a clone cannot be checked.</span>
                            ) : cloneIssue ? (
                              <span className="text-rose-400">{cloneIssue}</span>
                            ) : (
                              <span className="text-zinc-500">
                                The clone is created with this snapshot&apos;s state under the new name.
                              </span>
                            )}
                          </p>
                        </div>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}
          </section>
        </div>
      </div>
    </div>
  );
};
