import React, { useCallback, useEffect, useState } from "react";
import { Loader2, RefreshCw, Trash2 } from "lucide-react";
import { api } from "../../lib/api";
import { formatBytes } from "../../lib/utils";
import { useToast } from "../ui/Toast";
import { useModal } from "../ui/ModalContext";
import type { KindOrphanState } from "../../types";

/**
 * Lists kind node-state folders left behind by clusters that no longer exist
 * (a crash, a manual `docker rm`, an old app version) and removes them on
 * request. The backend only reports a folder that no container mounts, no
 * live cluster claims and that is older than 15 minutes, and it re-checks
 * each path again right before deleting.
 */
export const KindOrphanCleanup: React.FC<{ visible?: boolean }> = ({ visible = true }) => {
  const toast = useToast();
  const modal = useModal();
  const [items, setItems] = useState<KindOrphanState[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [cleaning, setCleaning] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setItems(await api.kindOrphanStates());
    } catch (e) {
      setItems([]);
      toast.error("Could not scan kind node state", String(e));
    } finally {
      setLoading(false);
    }
    // toast is read at call time; depending on it could re-run the scan.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // The scan walks folders on disk, so only run it once the card is shown.
  useEffect(() => {
    if (visible && items === null) void load();
  }, [visible, items, load]);

  const total = (items ?? []).reduce((sum, i) => sum + i.size, 0);

  const clean = async () => {
    if (!items?.length) return;
    const ok = await modal.showConfirm(
      `Delete ${items.length} unused node-state folder${items.length === 1 ? "" : "s"} (${formatBytes(total)})? ` +
        "They belong to clusters that no longer exist. This cannot be undone.",
      "Clean up node state",
      "danger"
    );
    if (!ok) return;
    setCleaning(true);
    try {
      const n = await api.kindCleanOrphanStates(items.map((i) => i.path));
      toast.success("Node state cleaned", `${n} folder${n === 1 ? "" : "s"} removed.`);
    } catch (e) {
      toast.error("Cleanup failed", String(e));
    } finally {
      setCleaning(false);
      void load();
    }
  };

  return (
    <div className="space-y-2 border-t border-zinc-800 pt-4">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <h4 className="text-sm font-semibold text-white">Unused node state</h4>
          <p className="mt-0.5 text-xs text-zinc-500">
            Folders kept for deleted clusters. Safe to remove: nothing uses them.
          </p>
        </div>
        <div className="flex shrink-0 gap-2">
          <button
            type="button"
            title="Scan again"
            aria-label="Scan again"
            disabled={loading || cleaning}
            onClick={() => void load()}
            className="h-8 w-8 flex items-center justify-center rounded-none border border-zinc-700 bg-zinc-900 text-zinc-300 hover:bg-zinc-800 disabled:opacity-40"
          >
            <RefreshCw className={loading ? "w-3.5 h-3.5 animate-spin" : "w-3.5 h-3.5"} />
          </button>
          <button
            type="button"
            title="Delete every folder listed"
            disabled={!items?.length || loading || cleaning}
            onClick={() => void clean()}
            className="h-8 px-3 flex items-center gap-1.5 rounded-none border border-rose-500/40 bg-rose-500/10 text-xs text-rose-200 hover:bg-rose-500/20 disabled:opacity-40"
          >
            {cleaning ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Trash2 className="w-3.5 h-3.5" />}
            Clean up{items?.length ? ` (${formatBytes(total)})` : ""}
          </button>
        </div>
      </div>
      {items === null ? (
        <p className="text-xs text-zinc-500">Scanning…</p>
      ) : items.length === 0 ? (
        <p className="text-xs text-zinc-500">Nothing to clean.</p>
      ) : (
        <ul className="divide-y divide-zinc-800 rounded-none border border-zinc-800">
          {items.map((i) => (
            <li key={i.path} className="flex items-center justify-between gap-3 px-3 py-2" title={i.path}>
              <span className="min-w-0">
                <span className="block truncate font-mono text-sm text-zinc-200">{i.cluster}</span>
                <span className="block truncate text-xs text-zinc-500">{i.path}</span>
              </span>
              <span
                className="shrink-0 text-xs text-zinc-400"
                title={i.partial ? "Some files could not be read, so the real size is larger" : undefined}
              >
                {i.partial ? "≥ " : ""}
                {formatBytes(i.size)}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};
