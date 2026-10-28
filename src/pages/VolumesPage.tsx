import { useModal } from "../components/ui/ModalContext";
import { useToast } from "../components/ui/Toast";
import React, { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { HardDrive, Plus, Trash2, Search, X, AlertTriangle, Copy, Check } from "lucide-react";
import { Card } from "../components/ui/Card";
import { SkeletonList } from "../components/ui/Skeleton";
import { Button } from "../components/ui/Button";
import { api } from "../lib/api";
import { formatBytes } from "../lib/utils";
import type { VolumeItem, DaemonState } from "../types";
import { EngineGate } from "../components/ui/EngineGate";

interface VolumesPageProps {
  refreshKey?: number;
  /** Engine state — the page renders an EngineGate panel unless Running. */
  daemonState?: DaemonState;
}

export const VolumesPage: React.FC<VolumesPageProps> = ({
  refreshKey,
  daemonState = "Running",
}) => {
  const modal = useModal();
  const toast = useToast();
  const [volumes, setVolumes] = useState<VolumeItem[]>([]);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [modalOpen, setModalOpen] = useState(false);
  const [newVolumeName, setNewVolumeName] = useState("");
  const [newDriver, setNewDriver] = useState("local");
  const [creating, setCreating] = useState(false);
  const [copiedPath, setCopiedPath] = useState<string | null>(null);
  const [autoPrune, setAutoPrune] = useState(false);
  const [pruningAuto, setPruningAuto] = useState(false);

  const loadVolumes = async (checkAutoPrune: boolean = true) => {
    try {
      setLoading(true);
      const list = await api.listVolumes();
      setVolumes(list);

      if (checkAutoPrune) {
        try {
          const cfg = await api.getAppConfig();
          const enabled = !!cfg.auto_prune_isolated_volumes;
          setAutoPrune(enabled);

          // An isolated/alone volume is one not protected and not currently mounted by any container
          const hasIsolated = list.some((v) => !v.protected && !v.in_use);

          if (enabled && hasIsolated) {
            setPruningAuto(true);
            const count = await api.pruneVolumes();
            if (count > 0) {
              toast.success("Auto-cleared Isolated Volumes", `Cleaned ${count} unused volume${count === 1 ? "" : "s"}`);
              const updated = await api.listVolumes();
              setVolumes(updated);
            }
          }
        } catch {
          // ignore background check errors
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
    loadVolumes();
  }, [refreshKey]);

  const handleCopyPath = (path: string) => {
    navigator.clipboard.writeText(path);
    setCopiedPath(path);
    toast.success("Path Copied", path);
    setTimeout(() => {
      setCopiedPath((current) => (current === path ? null : current));
    }, 2000);
  };

  const handleToggleAutoPrune = async () => {
    const next = !autoPrune;
    setAutoPrune(next);
    try {
      const current = await api.getAppConfig();
      await api.saveAppConfig({
        ...current,
        auto_prune_isolated_volumes: next,
      });
      toast.success(
        next ? "Auto-clean Isolated Enabled" : "Auto-clean Isolated Disabled",
        next ? "Unused/alone volumes will be cleaned automatically" : "Automatic cleaning turned off"
      );
      if (next) {
        await loadVolumes(true);
      }
    } catch (e) {
      setAutoPrune(!next);
      toast.error("Failed to update setting", String(e));
    }
  };

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newVolumeName.trim()) return;

    try {
      setCreating(true);
      await api.createVolume(newVolumeName.trim(), newDriver);
      setModalOpen(false);
      setNewVolumeName("");
      await loadVolumes();
    } catch (err) {
      toast.error("Create Volume Failed", String(err));
    } finally {
      setCreating(false);
    }
  };

  const handleRemove = async (name: string) => {
    const confirmed = await modal.showConfirm(`Are you sure you want to remove volume '${name}'?`, "Remove Volume", "danger");
    if (!confirmed) return;
    try {
      await api.removeVolume(name, true);
      await loadVolumes();
    } catch (err) {
      toast.error("Remove Volume Failed", String(err));
    }
  };

  const totalSize = volumes.reduce((sum, v) => sum + (v.size ?? 0), 0);

  const handlePrune = async () => {
    const confirmed = await modal.showConfirm(
      `Prune all unused Docker volumes?${totalSize > 0 ? `\n\nTotal size of all volumes (including in-use): ${formatBytes(totalSize)}. Only unused volumes are removed.` : ""}`,
      "Prune Volumes",
      "warning"
    );
    if (!confirmed) return;
    try {
      const deletedCount = await api.pruneVolumes();
      toast.success("Volumes Pruned", `Pruned ${deletedCount} unused volumes.`);
      await loadVolumes();
    } catch (err) {
      toast.error("Prune Error", String(err));
    }
  };

  // Kind-managed volumes belong to cluster nodes and are managed through Kubernetes.
  // Hide them from the user's volume list so core cluster state is kept out of sight.
  const filteredVolumes = volumes.filter(
    (v) => !v.protected && v.name.toLowerCase().includes(search.toLowerCase())
  );

  // Engine off → designed feature panel instead of fetch errors.
  if (daemonState !== "Running") {
    return (
      <EngineGate state={daemonState} resource="volumes">
        {null}
      </EngineGate>
    );
  }

  return (
    <div className="p-8 space-y-6 max-w-7xl mx-auto h-full flex flex-col">
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">Volumes</h2>
          <p className="text-sm text-zinc-400 mt-1">
            Manage persistent storage volumes and host directory mountpoints
          </p>
        </div>

        <div className="flex items-center gap-3">
          <div className="relative w-64">
            <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
            <input
              type="text"
              placeholder="Search volumes..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-full h-9 pl-9 pr-4 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white placeholder-zinc-400 focus:outline-none focus:border-blue-500"
            />
          </div>

          {/* Auto-prune Isolated Volumes Toggle */}
          <button
            type="button"
            onClick={handleToggleAutoPrune}
            title={
              autoPrune
                ? "Auto-clear isolated/unused volumes is ON (automatically removes unattached volumes)"
                : "Auto-clear isolated/unused volumes is OFF (click to enable)"
            }
            className={`flex items-center gap-2 h-9 px-3 rounded-xl border text-xs font-medium transition-all ${
              autoPrune
                ? "bg-emerald-500/10 border-emerald-500/30 text-emerald-300 hover:bg-emerald-500/20 shadow-sm shadow-emerald-500/10"
                : "bg-zinc-900/90 border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-700"
            }`}
          >
            <span
              className={`w-2 h-2 rounded-full transition-colors ${
                autoPrune
                  ? pruningAuto
                    ? "bg-emerald-400 animate-ping"
                    : "bg-emerald-400"
                  : "bg-zinc-600"
              }`}
            />
            <span className="whitespace-nowrap font-medium">Auto-clean Isolated</span>
            <div
              className={`relative inline-flex h-4 w-7 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out ${
                autoPrune ? "bg-emerald-500" : "bg-zinc-700"
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
            variant="primary"
            size="sm"
            onClick={() => setModalOpen(true)}
            className="gap-2"
          >
            <Plus className="w-4 h-4" />
            <span>Create Volume</span>
          </Button>
        </div>
      </div>

      {/* Volumes List */}
      <div className="flex-1 overflow-y-auto space-y-3 pb-8">
        {loading && volumes.length === 0 && <SkeletonList rows={4} actions={4} />}
        <AnimatePresence>
          {filteredVolumes.map((vol) => (
            <motion.div
              key={vol.name}
              layout
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.98 }}
            >
              <Card className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 p-4 hover:border-zinc-700/80 transition-all">
                <div className="flex items-start gap-3.5 min-w-0 flex-1">
                  <div className="w-10 h-10 rounded-xl bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center text-emerald-400 shrink-0 mt-0.5">
                    <HardDrive className="w-5 h-5" />
                  </div>

                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      {vol.in_use ? (
                        <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-emerald-500/10 text-emerald-300 border border-emerald-500/20 shrink-0">
                          in use
                        </span>
                      ) : (
                        <span className="text-[10px] font-medium px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-400 border border-zinc-700/60 shrink-0">
                          isolated
                        </span>
                      )}
                      {vol.protected && (
                        <span className="text-[10px] font-medium uppercase tracking-wider px-1.5 py-0.5 rounded bg-sky-500/10 text-sky-300 border border-sky-500/30 shrink-0">
                          kind
                        </span>
                      )}
                      <span className="font-semibold text-white tracking-tight text-sm truncate">
                        {vol.name}
                      </span>
                    </div>

                    <div className="flex items-center gap-2 text-xs text-zinc-400 mt-1.5">
                      <span className="px-1.5 py-0.5 rounded bg-zinc-800/80 text-zinc-300 text-[11px] font-mono shrink-0">
                        {vol.driver}
                      </span>
                      {typeof vol.size === "number" && (
                        <span className="flex items-center gap-1 font-sans text-zinc-300 text-xs shrink-0">
                          {formatBytes(vol.size)}
                          {vol.restricted_dirs && vol.restricted_dirs > 0 ? (
                            <span
                              className="flex items-center gap-1 text-amber-400"
                              title={`Size is incomplete: ${vol.restricted_dirs} subtrees skipped due to restricted permissions`}
                            >
                              <AlertTriangle className="w-3 h-3" />
                              partial
                            </span>
                          ) : null}
                        </span>
                      )}

                      {/* Mountpoint Path - smaller font, full width towards the right, with copy button in front */}
                      <div className="flex items-center gap-1.5 min-w-0 flex-1 ml-1 group/path">
                        <button
                          type="button"
                          onClick={() => handleCopyPath(vol.mountpoint)}
                          title="Copy mountpoint path"
                          className="p-1 rounded-md text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 transition-all shrink-0 opacity-70 group-hover/path:opacity-100"
                        >
                          {copiedPath === vol.mountpoint ? (
                            <Check className="w-3.5 h-3.5 text-emerald-400" />
                          ) : (
                            <Copy className="w-3.5 h-3.5" />
                          )}
                        </button>
                        <span
                          className="font-mono text-[11px] text-zinc-400 truncate flex-1 hover:text-zinc-200 transition-colors"
                          title={vol.mountpoint}
                        >
                          {vol.mountpoint}
                        </span>
                      </div>
                    </div>
                  </div>
                </div>

                <div className="flex items-center gap-1 shrink-0 self-end sm:self-center ml-2">
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => handleRemove(vol.name)}
                    disabled={vol.protected}
                    title={
                      vol.protected
                        ? "Managed by kind — remove the cluster in Kubernetes"
                        : "Remove Volume"
                    }
                    className={
                      vol.protected
                        ? "text-zinc-600 cursor-not-allowed"
                        : "text-zinc-400 hover:text-rose-400 hover:bg-rose-500/10"
                    }
                  >
                    <Trash2 className="w-4 h-4" />
                  </Button>
                </div>
              </Card>
            </motion.div>
          ))}
        </AnimatePresence>

        {!loading && filteredVolumes.length === 0 && (
          <div className="py-16 text-center text-zinc-400">
            <HardDrive className="w-10 h-10 mx-auto text-zinc-400 mb-3" />
            <p className="font-medium text-white">No volumes found</p>
            <p className="text-xs text-zinc-400 mt-1">
              Create a named volume for persistent container data
            </p>
          </div>
        )}
      </div>

      {/* Create Modal */}
      {modalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
          <div
            className="fixed inset-0 bg-black/70 backdrop-blur-md"
            onClick={() => setModalOpen(false)}
          />
          <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.95 }}
            className="relative w-full max-w-md bg-zinc-950 border border-zinc-800 rounded-2xl shadow-2xl p-6 z-10 space-y-5"
          >
            <div className="flex items-center justify-between">
              <h3 className="font-semibold text-lg text-white">Create Volume</h3>
              <Button variant="ghost" size="icon" onClick={() => setModalOpen(false)} title="Close">
                <X className="w-5 h-5" />
              </Button>
            </div>

            <form onSubmit={handleCreate} className="space-y-4">
              <div>
                <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                  Volume Name *
                </label>
                <input
                  type="text"
                  placeholder="e.g. pg_data, app_uploads"
                  value={newVolumeName}
                  onChange={(e) => setNewVolumeName(e.target.value)}
                  required
                  className="w-full h-10 px-3.5 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500"
                />
              </div>

              <div>
                <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                  Driver
                </label>
                <input
                  type="text"
                  value={newDriver}
                  onChange={(e) => setNewDriver(e.target.value)}
                  className="w-full h-10 px-3.5 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500"
                />
              </div>

              <div className="pt-2 flex justify-end gap-3">
                <Button type="button" variant="ghost" onClick={() => setModalOpen(false)}>
                  Cancel
                </Button>
                <Button type="submit" variant="primary" isLoading={creating}>
                  Create
                </Button>
              </div>
            </form>
          </motion.div>
        </div>
      )}
    </div>
  );
};
