import { useModal } from "../components/ui/ModalContext";
import { useToast } from "../components/ui/Toast";
import React, { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { HardDrive, Plus, Trash2, Search, X } from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { api } from "../lib/api";
import type { VolumeItem } from "../types";

interface VolumesPageProps {
  refreshKey?: number;
}

export const VolumesPage: React.FC<VolumesPageProps> = ({ refreshKey }) => {
  const modal = useModal();
  const toast = useToast();
  const [volumes, setVolumes] = useState<VolumeItem[]>([]);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [modalOpen, setModalOpen] = useState(false);
  const [newVolumeName, setNewVolumeName] = useState("");
  const [newDriver, setNewDriver] = useState("local");
  const [creating, setCreating] = useState(false);

  const loadVolumes = async () => {
    try {
      setLoading(true);
      const list = await api.listVolumes();
      setVolumes(list);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadVolumes();
  }, [refreshKey]);

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

  const handlePrune = async () => {
    const confirmed = await modal.showConfirm("Are you sure you want to prune all unused Docker volumes?", "Prune Volumes", "warning");
    if (!confirmed) return;
    try {
      const deletedCount = await api.pruneVolumes();
      toast.success("Volumes Pruned", `Pruned ${deletedCount} unused volumes.`);
      await loadVolumes();
    } catch (err) {
      toast.error("Prune Error", String(err));
    }
  };

  const filteredVolumes = volumes.filter((v) =>
    v.name.toLowerCase().includes(search.toLowerCase())
  );

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
                <div className="flex items-start gap-4">
                  <div className="w-10 h-10 rounded-xl bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center text-emerald-400 shrink-0">
                    <HardDrive className="w-5 h-5" />
                  </div>

                  <div>
                    <span className="font-semibold text-white tracking-tight text-sm">
                      {vol.name}
                    </span>
                    <div className="flex items-center gap-2 text-xs text-zinc-400 mt-1 font-mono">
                      <span className="px-2 py-0.5 rounded bg-zinc-800 text-zinc-300">
                        {vol.driver}
                      </span>
                      <span className="truncate max-w-[450px]" title={vol.mountpoint}>
                        {vol.mountpoint}
                      </span>
                    </div>
                  </div>
                </div>

                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => handleRemove(vol.name)}
                  title="Remove Volume"
                  className="text-zinc-400 hover:text-rose-400 hover:bg-rose-500/10"
                >
                  <Trash2 className="w-4 h-4" />
                </Button>
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
              <Button variant="ghost" size="icon" onClick={() => setModalOpen(false)}>
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
