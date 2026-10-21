import React, { useEffect, useRef, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { X, Hexagon, Loader2, CheckCircle2, Rocket, FolderOpen } from "lucide-react";
import { Button } from "../ui/Button";
import { StatusBadge } from "../ui/StatusBadge";
import { api, events } from "../../lib/api";
import type { KindCluster, KindProgressPayload } from "../../types";

interface LoadToKindModalProps {
  /** `repo:tag` to load; null closes the modal. */
  image: string | null;
  onClose: () => void;
}

/**
 * Loads a locally-present image into a kind cluster (`kind load docker-image`).
 * The log pane mirrors the `kind-progress` stream while the load runs.
 */
export const LoadToKindModal: React.FC<LoadToKindModalProps> = ({ image, onClose }) => {
  const [clusters, setClusters] = useState<KindCluster[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [loadingClusters, setLoadingClusters] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [done, setDone] = useState(false);
  const [lines, setLines] = useState<KindProgressPayload[]>([]);
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!image) return;
    // Fresh state on every open.
    setDone(false);
    setLines([]);
    setListError(null);
    setSelected(null);
    setLoadingClusters(true);
    api
      .listKindClusters()
      .then((list) => {
        setClusters(list);
        const firstRunning = list.find((c) => c.running) ?? list[0];
        if (firstRunning) setSelected(firstRunning.name);
      })
      .catch((e) => setListError(String(e)))
      .finally(() => setLoadingClusters(false));
  }, [image]);

  useEffect(() => {
    if (!image || !loading) return;
    let unlisten: (() => void) | undefined;
    events
      .onKindProgress((payload) => setLines((prev) => [...prev.slice(-149), payload]))
      .then((u) => (unlisten = u));
    return () => unlisten?.();
  }, [image, loading]);

  useEffect(() => {
    const el = logRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines]);

  const handleLoad = async () => {
    if (!image || !selected || loading) return;
    try {
      setLoading(true);
      setDone(false);
      setLines([{ action: "load", name: selected, line: `Loading ${image} into ${selected}…` }]);
      await api.loadImageToCluster(selected, image);
      setDone(true);
    } catch (e) {
      setLines((prev) => [...prev, { line: String(e) }]);
    } finally {
      setLoading(false);
    }
  };

  const handleClose = () => {
    if (loading) return;
    onClose();
  };

  return (
    <AnimatePresence>
      {image && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-6"
          onClick={handleClose}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 8 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: 6 }}
            transition={{ duration: 0.18, ease: "easeOut" }}
            className="w-full max-w-md bg-zinc-900 border border-zinc-800 rounded-2xl shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between px-5 py-4 border-b border-zinc-800">
              <div className="flex items-center gap-2.5 min-w-0">
                <div className="w-8 h-8 rounded-lg bg-blue-500/10 border border-blue-500/30 flex items-center justify-center shrink-0">
                  <Hexagon className="w-4 h-4 text-blue-400" />
                </div>
                <div className="min-w-0">
                  <h3 className="text-sm font-semibold text-white">Load to cluster</h3>
                  <p className="text-[11px] text-zinc-500 font-mono truncate" title={image}>
                    {image}
                  </p>
                </div>
              </div>
              <button
                onClick={handleClose}
                className="text-zinc-400 hover:text-white"
                title={loading ? "Wait for the load to finish" : "Close"}
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="px-5 py-4 space-y-3">
              {loadingClusters && (
                <div className="flex items-center gap-2 text-xs text-zinc-400 py-2">
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  <span>Listing clusters…</span>
                </div>
              )}

              {listError && (
                <p className="text-xs text-rose-400">{listError}</p>
              )}

              {!loadingClusters && !listError && clusters.length === 0 && (
                <div className="py-4 text-center">
                  <FolderOpen className="w-8 h-8 mx-auto text-zinc-500 mb-2" />
                  <p className="text-sm text-zinc-300">No kind clusters yet</p>
                  <p className="text-xs text-zinc-500 mt-1">
                    Create one on the Kubernetes page, then load this image.
                  </p>
                </div>
              )}

              {clusters.length > 0 && (
                <div className="space-y-1.5 max-h-44 overflow-y-auto pr-1">
                  {clusters.map((c) => {
                    const isSel = selected === c.name;
                    return (
                      <button
                        key={c.name}
                        type="button"
                        onClick={() => !loading && setSelected(c.name)}
                        disabled={loading}
                        className={`w-full flex items-center justify-between px-3 py-2.5 rounded-xl border text-left transition-colors ${
                          isSel
                            ? "border-blue-500/60 bg-blue-500/10"
                            : "border-zinc-800 bg-zinc-950 hover:border-zinc-700"
                        }`}
                      >
                        <span className="flex items-center gap-2 min-w-0">
                          <Hexagon
                            className={`w-4 h-4 shrink-0 ${isSel ? "text-blue-400" : "text-zinc-500"}`}
                          />
                          <span className="text-sm text-white truncate">{c.name}</span>
                        </span>
                        <StatusBadge
                          status={c.running ? "Running" : "Stopped"}
                          pulse={c.running}
                        />
                      </button>
                    );
                  })}
                </div>
              )}

              {(lines.length > 0 || loading || done) && (
                <div className="rounded-lg bg-zinc-950 border border-zinc-800 px-3 py-2 max-h-28 overflow-y-auto font-mono text-[11px] space-y-0.5 select-text">
                  {lines.map((l, i) => (
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
              <Button variant="ghost" size="sm" onClick={handleClose} disabled={loading}>
                {done ? "Close" : "Cancel"}
              </Button>
              {!done && (
                <Button
                  variant="primary"
                  size="sm"
                  onClick={handleLoad}
                  disabled={loading || !selected || clusters.length === 0}
                  className="gap-2"
                >
                  {loading ? (
                    <Loader2 className="w-4 h-4 animate-spin" />
                  ) : done ? (
                    <CheckCircle2 className="w-4 h-4" />
                  ) : (
                    <Rocket className="w-4 h-4" />
                  )}
                  <span>{loading ? "Loading…" : "Load Image"}</span>
                </Button>
              )}
              {done && (
                <Button variant="primary" size="sm" onClick={handleClose} className="gap-2">
                  <CheckCircle2 className="w-4 h-4" />
                  <span>Done</span>
                </Button>
              )}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
};
