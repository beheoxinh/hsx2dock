import React, { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { X, FileText, Loader2, HardDrive } from "lucide-react";
import { Button } from "../ui/Button";
import { api } from "../../lib/api";
import { formatBytes, formatRelativeTime } from "../../lib/utils";
import type { ImageHistoryEntry } from "../../types";

interface ImageHistoryModalProps {
  /** Image ref (tag or id) to inspect; null closes the modal. */
  image: string | null;
  onClose: () => void;
}

/** "Step 3/5 : RUN apt-get update && rm -rf /var/lib/apt/lists/*" → trimmed. */
const layerLabel = (entry: ImageHistoryEntry): string => {
  const raw = entry.created_by.trim();
  if (!raw) return "<no command>";
  // strip the BuildKit "/bin/sh -c" prefix docker prepends
  const cleaned = raw.replace(/^\/bin\/(ba)?sh -c\s*#\(nop\)\s*/i, "").replace(/^\/bin\/(ba)?sh -c\s*/i, "");
  return cleaned || raw;
};

/**
 * Shows a local image's layer history (`docker history`) — the same view
 * Docker Desktop exposes under an image's "History" tab.
 */
export const ImageHistoryModal: React.FC<ImageHistoryModalProps> = ({
  image,
  onClose,
}) => {
  const [entries, setEntries] = useState<ImageHistoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!image) return;
    let disposed = false;
    setLoading(true);
    setError(null);
    setEntries([]);
    api
      .getImageHistory(image)
      .then((rows) => {
        if (!disposed) setEntries(rows);
      })
      .catch((e) => {
        if (!disposed) setError(String(e));
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, [image]);

  return (
    <AnimatePresence>
      {image && (
        <>
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 bg-black/70 backdrop-blur-md z-50"
            onClick={onClose}
          />
          <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.95 }}
            className="fixed inset-0 z-50 flex items-center justify-center p-6 pointer-events-none"
          >
            <div className="pointer-events-auto w-full max-w-2xl max-h-[80vh] bg-zinc-950 border border-zinc-800 rounded-2xl shadow-2xl p-6 flex flex-col">
              <div className="flex items-center justify-between mb-4">
                <h3 className="font-semibold text-lg text-white flex items-center gap-2">
                  <FileText className="w-4 h-4 text-purple-400" />
                  Layer history
                </h3>
                <Button variant="ghost" size="icon" onClick={onClose} title="Close">
                  <X className="w-5 h-5" />
                </Button>
              </div>

              <p className="text-xs text-zinc-500 mb-3 font-mono truncate">{image}</p>

              <div className="flex-1 overflow-y-auto -mx-2 select-text">
                {loading ? (
                  <div className="flex items-center justify-center gap-2 py-10 text-xs text-zinc-500">
                    <Loader2 className="w-4 h-4 animate-spin" />
                    Loading history…
                  </div>
                ) : error ? (
                  <div className="py-10 text-center">
                    <p className="text-sm text-rose-400">Could not load history</p>
                    <p className="text-xs text-zinc-500 mt-1 font-mono break-all px-6">{error}</p>
                  </div>
                ) : entries.length === 0 ? (
                  <div className="py-10 text-center text-xs text-zinc-500">No layers reported.</div>
                ) : (
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="text-left text-[11px] uppercase tracking-wide text-zinc-500 border-b border-zinc-800">
                        <th className="py-2 px-2 font-medium">Layer</th>
                        <th className="py-2 px-2 font-medium w-24">Size</th>
                        <th className="py-2 px-2 font-medium w-32">Created</th>
                      </tr>
                    </thead>
                    <tbody>
                      {entries.map((e, i) => (
                        <tr
                          key={e.id || i}
                          className="border-b border-zinc-900/60 hover:bg-zinc-900/40"
                        >
                          <td className="py-2.5 px-2">
                            <span
                              className="font-mono text-xs text-zinc-200 break-all line-clamp-2"
                              title={layerLabel(e)}
                            >
                              {layerLabel(e)}
                            </span>
                          </td>
                          <td className="py-2.5 px-2">
                            <span className="flex items-center gap-1 text-xs text-zinc-400 font-mono whitespace-nowrap">
                              {e.size > 0 ? (
                                <>
                                  <HardDrive className="w-3 h-3 text-zinc-600" />
                                  {formatBytes(e.size)}
                                </>
                              ) : (
                                <span className="text-zinc-600">—</span>
                              )}
                            </span>
                          </td>
                          <td className="py-2.5 px-2 text-xs text-zinc-500 whitespace-nowrap">
                            {formatRelativeTime(e.created)}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </div>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
};
