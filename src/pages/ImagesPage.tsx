import { useModal } from "../components/ui/ModalContext";
import { useToast } from "../components/ui/Toast";
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
  Hammer,
  Upload,
  Hexagon,
  Tag,
  FileText,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { SkeletonList } from "../components/ui/Skeleton";
import { Button } from "../components/ui/Button";
import { BuildImageModal } from "../components/images/BuildImageModal";
import { LoadToKindModal } from "../components/images/LoadToKindModal";
import { TagImageModal } from "../components/images/TagImageModal";
import { ImageHistoryModal } from "../components/images/ImageHistoryModal";
import { api, events } from "../lib/api";
import { formatBytes, formatRelativeTime, truncateId } from "../lib/utils";
import type {
  ImageItem,
  SearchResult,
  PullProgressPayload,
  PushProgressPayload,
  RegistryStatus,
} from "../types";
import type { DaemonState } from "../types";
import { EngineGate } from "../components/ui/EngineGate";

interface ImagesPageProps {
  refreshKey?: number;
  /** Engine state — the page renders an EngineGate panel unless Running. */
  daemonState?: DaemonState;
}

export const ImagesPage: React.FC<ImagesPageProps> = ({
  refreshKey,
  daemonState = "Running",
}) => {
  const modal = useModal();
  const toast = useToast();
  const [images, setImages] = useState<ImageItem[]>([]);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [pullModalOpen, setPullModalOpen] = useState(false);
  const [buildModalOpen, setBuildModalOpen] = useState(false);
  const [imageToPull, setImageToPull] = useState("");
  const [pullStatus, setPullStatus] = useState<PullProgressPayload | null>(null);
  const [isPulling, setIsPulling] = useState(false);
  const [hubResults, setHubResults] = useState<SearchResult[]>([]);
  const [searchingHub, setSearchingHub] = useState(false);
  const [registries, setRegistries] = useState<RegistryStatus[]>([]);
  const [pushModalOpen, setPushModalOpen] = useState(false);
  const [pushTarget, setPushTarget] = useState("");
  const [loadTarget, setLoadTarget] = useState<string | null>(null);
  const [tagTarget, setTagTarget] = useState<ImageItem | null>(null);
  const [historyTarget, setHistoryTarget] = useState<string | null>(null);
  const [isPushing, setIsPushing] = useState(false);
  const [pushStatus, setPushStatus] = useState<PushProgressPayload | null>(null);
  const [autoPrune, setAutoPrune] = useState(false);
  const [pruningAuto, setPruningAuto] = useState(false);

  const loadImages = async (checkAutoPrune: boolean = true) => {
    try {
      setLoading(true);
      const list = await api.listImages();
      setImages(list);

      if (checkAutoPrune) {
        try {
          const cfg = await api.getAppConfig();
          const enabled = !!cfg.auto_prune_dangling_images;
          setAutoPrune(enabled);

          const hasDangling = list.some(
            (img) => !img.tags || img.tags.length === 0 || img.tags.every((t) => t === "<none>:<none>" || t.startsWith("sha256:"))
          );

          if (enabled && hasDangling) {
            setPruningAuto(true);
            const [count, space] = await api.pruneImages();
            if (count > 0) {
              toast.success("Auto-pruned Dangling Images", `Cleaned ${count} images (${formatBytes(space)})`);
              const updated = await api.listImages();
              setImages(updated);
            }
          }
        } catch {
          // ignore background auto-prune check error
        } finally {
          setPruningAuto(false);
        }
      }
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadImages();
    api.listRegistries().then(setRegistries).catch(() => {});

    let unlistenPull: (() => void) | undefined;
    events.onPullProgress((payload) => {
      setPullStatus(payload);
      if (payload.done) {
        setIsPulling(false);
        loadImages();
      }
    }).then((u) => (unlistenPull = u));

    let unlistenPush: (() => void) | undefined;
    events.onPushProgress((payload) => {
      setPushStatus(payload);
      if (payload.done) {
        setIsPushing(false);
      }
    }).then((u) => (unlistenPush = u));

    return () => {
      if (unlistenPull) unlistenPull();
      if (unlistenPush) unlistenPush();
    };
  }, [refreshKey]);

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

  /** Picks the first usable tag: fully-qualified, else first non-sha tag. */
  const primaryTag = (img: ImageItem): string => {
    const named = img.tags.find((t) => t && !t.startsWith("sha256:"));
    return named ?? `${img.id.slice(0, 12)}:latest`;
  };

  /**
   * Builds a sensible push target: an existing fully-qualified tag wins;
   * otherwise the first saved registry + username + local repo name.
   */
  const suggestPushRefWith = (img: ImageItem, regs: RegistryStatus[]): string => {
    const qualified = img.tags.find(
      (t) =>
        t.includes("/") &&
        (t.includes(".") || t.includes(":") || t.startsWith("localhost"))
    );
    if (qualified) return qualified;

    const base = primaryTag(img);
    const colon = base.lastIndexOf(":");
    const repo = colon > 0 ? base.slice(0, colon) : base;
    const tag = colon > 0 ? base.slice(colon + 1) : "latest";
    // Strip any docker.io/library prefix from the local name.
    const shortRepo = repo.replace(/^docker\.io\/(library\/)?/, "");

    const reg = regs.find((r) => r.is_default) ?? regs[0];
    if (!reg) return `docker.io/library/${shortRepo}:${tag}`;
    if (reg.host === "docker.io") return `docker.io/${shortRepo}:${tag}`;
    return `${reg.host}/${reg.username}/${shortRepo}:${tag}`;
  };

  const suggestPushRef = (img: ImageItem): string =>
    suggestPushRefWith(img, registries);

  const openPush = (img: ImageItem) => {
    setPushStatus(null);
    // Refresh logins first so the suggested ref uses the latest registry.
    api
      .listRegistries()
      .then((regs) => {
        setRegistries(regs);
        setPushTarget(suggestPushRefWith(img, regs));
      })
      .catch(() => setPushTarget(suggestPushRef(img)));
    setPushModalOpen(true);
  };

  const handlePush = async () => {
    const target = pushTarget.trim();
    if (!target || isPushing) return;
    try {
      setIsPushing(true);
      setPushStatus({
        image: target,
        status: "Starting push…",
        progress: null,
        current: null,
        total: null,
        done: false,
        error: null,
      });
      await api.pushImage(target);
    } catch (err) {
      setPushStatus({
        image: target,
        status: "Failed",
        progress: null,
        current: null,
        total: null,
        done: true,
        error: String(err),
      });
      setIsPushing(false);
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
    const confirmed = await modal.showConfirm("Are you sure you want to remove this image?", "Remove Image", "danger");
    if (!confirmed) return;
    try {
      await api.removeImage(id, true);
      await loadImages();
    } catch (e) {
      toast.error("Remove Error", String(e));
    }
  };

  const handleToggleAutoPrune = async () => {
    const next = !autoPrune;
    setAutoPrune(next);
    try {
      const current = await api.getAppConfig();
      await api.saveAppConfig({
        ...current,
        auto_prune_dangling_images: next,
      });
      toast.success(
        next ? "Auto-prune Dangling Enabled" : "Auto-prune Dangling Disabled",
        next ? "Dangling/untagged images will be cleaned automatically" : "Automatic cleaning turned off"
      );
      if (next) {
        await loadImages(true);
      }
    } catch (e) {
      setAutoPrune(!next);
      toast.error("Failed to update Auto-prune setting", String(e));
    }
  };

  const handlePrune = async () => {
    const confirmed = await modal.showConfirm("Are you sure you want to prune all dangling/unused images?", "Prune Images", "warning");
    if (!confirmed) return;
    try {
      const [count, space] = await api.pruneImages();
      toast.success("Images Pruned", `Removed ${count} images, reclaimed ${formatBytes(space)}`);
      await loadImages(false);
    } catch (e) {
      toast.error("Prune Error", String(e));
    }
  };

  const filteredImages = images.filter((img) => {
    const tagsStr = img.tags.join(" ").toLowerCase();
    const q = search.toLowerCase();
    return tagsStr.includes(q) || img.id.toLowerCase().includes(q);
  });

  // Engine off → designed feature panel instead of fetch errors.
  if (daemonState !== "Running") {
    return (
      <EngineGate state={daemonState} resource="images">
        {null}
      </EngineGate>
    );
  }

  return (
    <div className="p-8 space-y-6 max-w-7xl mx-auto h-full flex flex-col">
      {/* Header & Actions */}
      <div className="flex flex-col sm:flex-row sm:flex-wrap sm:items-center justify-between gap-4">
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

          {/* Auto-prune Dangling Toggle */}
          <button
            type="button"
            onClick={handleToggleAutoPrune}
            title={
              autoPrune
                ? "Auto-clear dangling images is ON (automatically removes untagged images)"
                : "Auto-clear dangling images is OFF (click to enable)"
            }
            className={`flex items-center gap-2 h-9 px-3 rounded-xl border text-xs font-medium transition-all ${
              autoPrune
                ? "bg-amber-500/10 border-amber-500/30 text-amber-300 hover:bg-amber-500/20 shadow-sm shadow-amber-500/10"
                : "bg-zinc-900/90 border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-700"
            }`}
          >
            <span
              className={`w-2 h-2 rounded-full transition-colors ${
                autoPrune
                  ? pruningAuto
                    ? "bg-amber-400 animate-ping"
                    : "bg-amber-400"
                  : "bg-zinc-600"
              }`}
            />
            <span className="whitespace-nowrap font-medium">Auto-clean Dangling</span>
            <div
              className={`relative inline-flex h-4 w-7 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out ${
                autoPrune ? "bg-amber-500" : "bg-zinc-700"
              }`}
            >
              <span
                className={`pointer-events-none inline-block h-3 w-3 transform rounded-full bg-white shadow-lg ring-0 transition duration-200 ease-in-out ${
                  autoPrune ? "translate-x-3" : "translate-x-0"
                }`}
              />
            </div>
          </button>

          <Button variant="outline" size="sm" onClick={handlePrune} className="gap-2">
            <Trash2 className="w-4 h-4 text-rose-400" />
            <span>Prune</span>
          </Button>

          <Button
            variant="secondary"
            size="sm"
            onClick={() => setBuildModalOpen(true)}
          >
            <Hammer className="w-4 h-4" />
            <span>Build</span>
          </Button>

          <Button
            variant="primary"
            size="sm"
            onClick={() => {
              // Re-read logins so a registry added in Settings shows up here.
              api.listRegistries().then(setRegistries).catch(() => {});
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
        {loading && images.length === 0 && <SkeletonList rows={5} actions={4} />}
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
                    onClick={() =>
                      setLoadTarget(
                        img.tags.find((t) => t && !t.startsWith("sha256:")) ?? img.id
                      )
                    }
                    title="Load to kind cluster"
                    className="text-zinc-400 hover:text-blue-400 hover:bg-blue-500/10"
                  >
                    <Hexagon className="w-4 h-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => setTagTarget(img)}
                    title="Add tag"
                    className="text-zinc-400 hover:text-amber-400 hover:bg-amber-500/10"
                  >
                    <Tag className="w-4 h-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => setHistoryTarget(primaryTag(img))}
                    title="Layer history"
                    className="text-zinc-400 hover:text-purple-400 hover:bg-purple-500/10"
                  >
                    <FileText className="w-4 h-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => openPush(img)}
                    title="Push to a registry"
                    className="text-zinc-400 hover:text-amber-400 hover:bg-amber-500/10"
                  >
                    <Upload className="w-4 h-4" />
                  </Button>
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
                title="Close"
              >
                <X className="w-5 h-5" />
              </Button>
            </div>

            {/* Input + Search */}
            <div className="space-y-3">
              {registries.length > 0 && (
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-[11px] text-zinc-600">From registry:</span>
                  {registries.map((reg) => (
                    <button
                      key={reg.id}
                      type="button"
                      disabled={isPulling}
                      title={`Pull from ${reg.name} (login: ${reg.username})`}
                      onClick={() => {
                        const current = imageToPull.trim();
                        if (!current) {
                          setImageToPull(`${reg.host}/`);
                          return;
                        }
                        // Swap the leading registry host if one is present.
                        const stripped = current.includes("/")
                          ? current.replace(/^[^/]+\//, "")
                          : current;
                        setImageToPull(`${reg.host}/${stripped}`);
                      }}
                      className="rounded-md border border-zinc-800 bg-zinc-900/50 px-2 py-0.5 text-[11px] text-zinc-400 hover:border-zinc-700 hover:text-zinc-200 transition-colors disabled:opacity-50"
                    >
                      {reg.host}
                    </button>
                  ))}
                </div>
              )}
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

      {/* Push Image Modal */}
      {pushModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
          <div
            className="fixed inset-0 bg-black/70 backdrop-blur-md"
            onClick={() => !isPushing && setPushModalOpen(false)}
          />
          <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.95 }}
            className="relative w-full max-w-xl bg-zinc-950 border border-zinc-800 rounded-2xl shadow-2xl overflow-hidden p-6 z-10 space-y-5"
          >
            <div className="flex items-center justify-between">
              <h3 className="text-lg font-bold text-white flex items-center gap-2">
                <Upload className="w-5 h-5 text-amber-400" />
                Push Image
              </h3>
              <Button
                variant="ghost"
                size="icon"
                disabled={isPushing}
                onClick={() => setPushModalOpen(false)}
                title="Close"
              >
                <X className="w-5 h-5" />
              </Button>
            </div>

            <div className="space-y-3">
              {registries.length > 0 && (
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-[11px] text-zinc-600">Push to:</span>
                  {registries.map((reg) => (
                    <button
                      key={reg.id}
                      type="button"
                      disabled={isPushing}
                      title={`${reg.name} (login: ${reg.username})`}
                      onClick={() => {
                        const current = pushTarget.trim();
                        const stripped = current.includes("/")
                          ? current.replace(/^[^/]+\//, "")
                          : current;
                        if (reg.host === "docker.io") {
                          setPushTarget(`docker.io/${stripped}`);
                        } else {
                          setPushTarget(`${reg.host}/${reg.username}/${stripped}`);
                        }
                      }}
                      className="rounded-md border border-zinc-800 bg-zinc-900/50 px-2 py-0.5 text-[11px] text-zinc-400 hover:border-zinc-700 hover:text-zinc-200 transition-colors disabled:opacity-50"
                    >
                      {reg.host}
                    </button>
                  ))}
                </div>
              )}
              {registries.length === 0 && (
                <p className="text-[11px] text-amber-400/90">
                  No registry logins saved — add one in Settings → Registries,
                  or the push will be refused by the registry.
                </p>
              )}
              <div className="flex gap-2">
                <input
                  type="text"
                  placeholder="ghcr.io/your-user/image:tag"
                  value={pushTarget}
                  onChange={(e) => setPushTarget(e.target.value)}
                  disabled={isPushing}
                  className="flex-1 h-10 px-3.5 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white font-mono focus:outline-none focus:border-amber-500"
                />
                <Button
                  variant="primary"
                  onClick={handlePush}
                  isLoading={isPushing}
                  disabled={!pushTarget.trim() || isPushing}
                >
                  Push
                </Button>
              </div>
            </div>

            {/* Progress */}
            {pushStatus && (
              <div className="space-y-2 rounded-xl border border-zinc-800 bg-zinc-900/40 p-3">
                <div className="flex items-center justify-between text-xs">
                  <span className="font-mono text-zinc-400 truncate">
                    {pushStatus.image}
                  </span>
                  {pushStatus.done ? (
                    pushStatus.error ? (
                      <span className="text-rose-400 flex items-center gap-1 font-semibold shrink-0">
                        <AlertCircle className="w-3.5 h-3.5" /> Failed
                      </span>
                    ) : (
                      <span className="text-emerald-400 flex items-center gap-1 font-semibold shrink-0">
                        <CheckCircle className="w-3.5 h-3.5" /> Pushed
                      </span>
                    )
                  ) : (
                    <span className="text-amber-400 animate-pulse shrink-0">
                      Pushing…
                    </span>
                  )}
                </div>
                {pushStatus.current && pushStatus.total ? (
                  <div className="w-full bg-zinc-800 rounded-full h-1.5 overflow-hidden">
                    <div
                      className="bg-amber-500 h-1.5 rounded-full transition-all"
                      style={{
                        width: `${Math.min(
                          100,
                          (pushStatus.current / pushStatus.total) * 100
                        )}%`,
                      }}
                    />
                  </div>
                ) : (
                  <div className="text-[11px] text-zinc-500 font-mono">
                    {pushStatus.status}
                  </div>
                )}
                {pushStatus.error && (
                  <p className="text-xs text-rose-400">{pushStatus.error}</p>
                )}
              </div>
            )}
          </motion.div>
        </div>
      )}

      <BuildImageModal
        isOpen={buildModalOpen}
        onClose={() => setBuildModalOpen(false)}
        onSuccess={() => loadImages()}
      />

      <LoadToKindModal image={loadTarget} onClose={() => setLoadTarget(null)} />

      <TagImageModal
        image={tagTarget}
        onClose={() => setTagTarget(null)}
        onSuccess={() => loadImages()}
      />

      <ImageHistoryModal image={historyTarget} onClose={() => setHistoryTarget(null)} />
    </div>
  );
};
