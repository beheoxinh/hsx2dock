import React, { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Layers,
  Download,
  Trash2,
  Search,
  Star,
  Clock,
  HardDrive,
  X,
  CheckCircle,
  AlertCircle,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { api, events } from "../lib/api";
import { formatBytes, formatRelativeTime, truncateId } from "../lib/utils";
import type { ImageItem, SearchResult, PullProgressPayload } from "../types";

export const ImagesPage: React.FC = () => {
  const [images, setImages] = useState<ImageItem[]>([]);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [pullModalOpen, setPullModalOpen] = useState(false);
  const [imageToPull, setImageToPull] = useState("");
  const [pullStatus, setPullStatus] = useState<PullProgressPayload | null>(null);
  const [isPulling, setIsPulling] = useState(false);
  const [hubResults, setHubResults] = useState<SearchResult[]>([]);
  const [searchingHub, setSearchingHub] = useState(false);

  const loadImages = async () => {
    try {
      setLoading(true);
      const list = await api.listImages();
      setImages(list);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadImages();

    let unlistenPull: (() => void) | undefined;
    events.onPullProgress((payload) => {
      setPullStatus(payload);
      if (payload.done) {
        setIsPulling(false);
        loadImages();
      }
    }).then((u) => (unlistenPull = u));

    return () => {
      if (unlistenPull) unlistenPull();
    };
  }, []);

  const handlePull = async (imgName?: string) => {
    const target = imgName || imageToPull;
    if (!target.trim()) return;

    try {
      setIsPulling(true);
      setPullStatus({
        image: target,
        status: "Initiating pull request...",
        progress: null,
        current: null,
        total: null,
        done: false,
        error: null,
      });
      await api.pullImage(target.trim());
    } catch (err) {
      setPullStatus({
        image: target,
        status: "Failed",
        progress: null,
        current: null,
        total: null,
        done: true,
        error: String(err),
      });
      setIsPulling(false);
    }
  };

  const handleSearchHub = async () => {
    if (!imageToPull.trim()) return;
    try {
      setSearchingHub(true);
      const results = await api.searchHub(imageToPull.trim());
      setHubResults(results);
    } catch (e) {
      console.error(e);
    } finally {
      setSearchingHub(false);
    }
  };

  const handleRemove = async (id: string) => {
    if (!confirm("Are you sure you want to remove this image?")) return;
    try {
      await api.removeImage(id, true);
      await loadImages();
    } catch (e) {
      alert(`Remove error: ${e}`);
    }
  };

  const handlePrune = async () => {
    if (!confirm("Prune all dangling/unused images?")) return;
    try {
      const [count, space] = await api.pruneImages();
      alert(`Removed ${count} images, reclaimed ${formatBytes(space)}`);
      await loadImages();
    } catch (e) {
      alert(`Prune error: ${e}`);
    }
  };

  const filteredImages = images.filter((img) => {
    const tagsStr = img.tags.join(" ").toLowerCase();
    const q = search.toLowerCase();
    return tagsStr.includes(q) || img.id.toLowerCase().includes(q);
  });

  return (
    <div className="p-8 space-y-6 max-w-7xl mx-auto h-full flex flex-col">
      {/* Header & Actions */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">Images</h2>
          <p className="text-sm text-zinc-400 mt-1">
            Browse local docker layers, pull from registry, and inspect tags
          </p>
        </div>

        <div className="flex items-center gap-3">
          <div className="relative w-64">
            <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
            <input
              type="text"
              placeholder="Search images or tags..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-full h-9 pl-9 pr-4 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white placeholder-zinc-400 focus:outline-none focus:border-blue-500"
            />
          </div>

          <Button variant="outline" size="sm" onClick={handlePrune} className="gap-2">
            <Trash2 className="w-4 h-4 text-rose-400" />
            <span>Prune</span>
          </Button>

          <Button
            variant="primary"
            size="sm"
            onClick={() => {
              setPullModalOpen(true);
              setPullStatus(null);
              setHubResults([]);
            }}
            className="gap-2"
          >
            <Download className="w-4 h-4" />
            <span>Pull Image</span>
          </Button>
        </div>
      </div>

      {/* Images List */}
      <div className="flex-1 overflow-y-auto space-y-3 pb-8">
        <AnimatePresence>
          {filteredImages.map((img) => (
            <motion.div
              key={img.id}
              layout
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.98 }}
            >
              <Card className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 p-4 hover:border-zinc-700/80 transition-all">
                <div className="flex items-start gap-4">
                  <div className="w-10 h-10 rounded-xl bg-purple-500/10 border border-purple-500/20 flex items-center justify-center text-purple-400 shrink-0">
                    <Layers className="w-5 h-5" />
                  </div>

                  <div>
                    <div className="flex flex-wrap items-center gap-2">
                      {img.tags.length > 0 ? (
                        img.tags.map((tag, idx) => (
                          <span
                            key={idx}
                            className="font-semibold text-white tracking-tight hover:text-blue-400 cursor-pointer"
                          >
                            {tag}
                          </span>
                        ))
                      ) : (
                        <span className="text-zinc-500 italic text-sm">&lt;none&gt;:&lt;none&gt;</span>
                      )}
                    </div>
                    <div className="flex items-center gap-3 text-xs text-zinc-400 mt-1.5 font-mono">
                      <span>{truncateId(img.id)}</span>
                      <span>•</span>
                      <span className="flex items-center gap-1 font-sans">
                        <HardDrive className="w-3 h-3 text-zinc-500" />
                        {formatBytes(img.size)}
                      </span>
                      <span>•</span>
                      <span className="flex items-center gap-1 font-sans">
                        <Clock className="w-3 h-3 text-zinc-500" />
                        {formatRelativeTime(img.created)}
                      </span>
                    </div>
                  </div>
                </div>

                <div className="flex items-center gap-2 self-end sm:self-center">
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => handleRemove(img.id)}
                    title="Remove Image"
                    className="text-zinc-400 hover:text-rose-400 hover:bg-rose-500/10"
                  >
                    <Trash2 className="w-4 h-4" />
                  </Button>
                </div>
              </Card>
            </motion.div>
          ))}
        </AnimatePresence>

        {!loading && filteredImages.length === 0 && (
          <div className="py-16 text-center text-zinc-400">
            <Layers className="w-10 h-10 mx-auto text-zinc-400 mb-3" />
            <p className="font-medium text-white">No images found</p>
            <p className="text-xs text-zinc-400 mt-1">
              Pull an image from Docker Hub or a registry to get started
            </p>
          </div>
        )}
      </div>

      {/* Pull Image Modal */}
      {pullModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
          <div
            className="fixed inset-0 bg-black/70 backdrop-blur-md"
            onClick={() => !isPulling && setPullModalOpen(false)}
          />
          <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.95 }}
            className="relative w-full max-w-xl bg-zinc-950 border border-zinc-800 rounded-2xl shadow-2xl overflow-hidden p-6 z-10 space-y-5"
          >
            <div className="flex items-center justify-between">
              <h3 className="font-semibold text-lg text-white">Pull Docker Image</h3>
              <Button
                variant="ghost"
                size="icon"
                disabled={isPulling}
                onClick={() => setPullModalOpen(false)}
              >
                <X className="w-5 h-5" />
              </Button>
            </div>

            {/* Input + Search */}
            <div className="space-y-3">
              <div className="flex gap-2">
                <input
                  type="text"
                  placeholder="Image tag (e.g. alpine:latest, postgres:17)"
                  value={imageToPull}
                  onChange={(e) => setImageToPull(e.target.value)}
                  disabled={isPulling}
                  className="flex-1 h-10 px-3.5 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500"
                />
                <Button
                  variant="secondary"
                  onClick={handleSearchHub}
                  isLoading={searchingHub}
                  disabled={isPulling || !imageToPull.trim()}
                >
                  Search Hub
                </Button>
                <Button
                  variant="primary"
                  onClick={() => handlePull()}
                  isLoading={isPulling}
                  disabled={!imageToPull.trim()}
                >
                  Pull
                </Button>
              </div>

              {/* Presets */}
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-xs text-zinc-400">Quick presets:</span>
                {["alpine:latest", "ubuntu:24.04", "redis:alpine", "postgres:17"].map(
                  (preset) => (
                    <button
                      key={preset}
                      onClick={() => setImageToPull(preset)}
                      className="px-2.5 py-1 text-xs rounded-lg bg-zinc-900 border border-zinc-800 hover:border-zinc-700 text-zinc-300 hover:text-white transition-colors cursor-pointer"
                    >
                      {preset}
                    </button>
                  )
                )}
              </div>
            </div>

            {/* Pull Progress Display */}
            {pullStatus && (
              <div className="bg-zinc-900/90 border border-zinc-800/80 rounded-xl p-4 space-y-2">
                <div className="flex items-center justify-between text-xs">
                  <span className="font-medium text-white">{pullStatus.status}</span>
                  {pullStatus.done ? (
                    pullStatus.error ? (
                      <span className="text-rose-400 flex items-center gap-1 font-semibold">
                        <AlertCircle className="w-3.5 h-3.5" /> Failed
                      </span>
                    ) : (
                      <span className="text-emerald-400 flex items-center gap-1 font-semibold">
                        <CheckCircle className="w-3.5 h-3.5" /> Finished
                      </span>
                    )
                  ) : (
                    <span className="text-blue-400 animate-pulse">Pulling...</span>
                  )}
                </div>

                {pullStatus.current && pullStatus.total ? (
                  <div className="w-full bg-zinc-800 rounded-full h-2 overflow-hidden">
                    <div
                      className="bg-blue-500 h-2 rounded-full transition-all duration-200"
                      style={{
                        width: `${Math.min(
                          100,
                          (pullStatus.current / pullStatus.total) * 100
                        )}%`,
                      }}
                    />
                  </div>
                ) : null}

                {pullStatus.error && (
                  <p className="text-xs text-rose-400 mt-1">{pullStatus.error}</p>
                )}
              </div>
            )}

            {/* Docker Hub Search Results */}
            {hubResults.length > 0 && (
              <div className="space-y-2 max-h-60 overflow-y-auto pr-1">
                <span className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">
                  Docker Hub Matches
                </span>
                <div className="divide-y divide-zinc-800/60 border border-zinc-800 rounded-xl overflow-hidden">
                  {hubResults.map((res) => (
                    <div
                      key={res.name}
                      className="p-3 flex items-center justify-between hover:bg-zinc-900 transition-colors cursor-pointer"
                      onClick={() => {
                        setImageToPull(res.name);
                        handlePull(res.name);
                      }}
                    >
                      <div>
                        <div className="font-semibold text-sm text-white">{res.name}</div>
                        <div className="text-xs text-zinc-400 line-clamp-1">
                          {res.description}
                        </div>
                      </div>
                      <div className="flex items-center gap-1 text-xs text-amber-400 font-medium">
                        <Star className="w-3.5 h-3.5 fill-current" />
                        <span>{res.star_count}</span>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </motion.div>
        </div>
      )}
    </div>
  );
};
