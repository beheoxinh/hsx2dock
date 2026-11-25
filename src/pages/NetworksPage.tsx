import { useModal } from "../components/ui/ModalContext";
import { useToast } from "../components/ui/Toast";
import React, { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { Network, Plus, Trash2, Search, X } from "lucide-react";
import { Card } from "../components/ui/Card";
import { SkeletonList } from "../components/ui/Skeleton";
import { Button } from "../components/ui/Button";
import { api } from "../lib/api";
import { truncateId } from "../lib/utils";
import type { NetworkItem, DaemonState } from "../types";
import { EngineGate } from "../components/ui/EngineGate";
import { useProgressiveList } from "../hooks/useProgressiveList";

interface NetworksPageProps {
  refreshKey?: number;
  /** Engine state — the page renders an EngineGate panel unless Running. */
  daemonState?: DaemonState;
}

export const NetworksPage: React.FC<NetworksPageProps> = ({
  refreshKey,
  daemonState = "Running",
}) => {
  const modal = useModal();
  const toast = useToast();
  const [networks, setNetworks] = useState<NetworkItem[]>([]);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [modalOpen, setModalOpen] = useState(false);
  const [name, setName] = useState("");
  const [driver, setDriver] = useState("bridge");
  const [subnet, setSubnet] = useState("");
  const [gateway, setGateway] = useState("");
  const [creating, setCreating] = useState(false);

  const loadNetworks = async () => {
    try {
      setLoading(true);
      const list = await api.listNetworks();
      setNetworks(list);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadNetworks();
  }, [refreshKey]);

  const handleCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;

    try {
      setCreating(true);
      await api.createNetwork(
        name.trim(),
        driver,
        subnet.trim() || undefined,
        gateway.trim() || undefined
      );
      setModalOpen(false);
      setName("");
      setSubnet("");
      setGateway("");
      await loadNetworks();
    } catch (err) {
      toast.error("Create Network Failed", String(err));
    } finally {
      setCreating(false);
    }
  };

  const handleRemove = async (id: string, netName: string) => {
    const confirmed = await modal.showConfirm(`Are you sure you want to remove network '${netName}'?`, "Remove Network", "danger");
    if (!confirmed) return;
    try {
      await api.removeNetwork(id);
      await loadNetworks();
    } catch (err) {
      toast.error("Remove Network Failed", String(err));
    }
  };

  const handlePrune = async () => {
    const confirmed = await modal.showConfirm("Are you sure you want to prune all unused Docker networks?", "Prune Networks", "warning");
    if (!confirmed) return;
    try {
      const pruned = await api.pruneNetworks();
      toast.success("Networks Pruned", `Pruned ${pruned} networks.`);
      await loadNetworks();
    } catch (err) {
      toast.error("Prune Error", String(err));
    }
  };

  // Kind networks and protected cluster topologies are managed via the Kubernetes tab.
  // Hide them from the user's networks list.
  const filteredNetworks = networks.filter(
    (n) =>
      !n.protected &&
      n.name !== "kind" &&
      n.name.toLowerCase().includes(search.toLowerCase())
  );

  // Engine off → designed feature panel instead of fetch errors.
  const progressive = useProgressiveList(filteredNetworks, search);

  if (daemonState !== "Running") {
    return (
      <EngineGate state={daemonState} resource="networks">
        {null}
      </EngineGate>
    );
  }

  return (
    <div className="p-8 space-y-6 max-w-7xl mx-auto h-full flex flex-col">
      <div className="flex flex-col sm:flex-row sm:flex-wrap sm:items-center justify-between gap-4">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">Networks</h2>
          <p className="text-sm text-zinc-400 mt-1">
            Configure software bridges, subnets, and container network topologies
          </p>
        </div>

        <div className="flex items-center gap-3">
          <div className="relative w-64">
            <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
            <input
              type="text"
              placeholder="Search networks..."
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
            <span>Create Network</span>
          </Button>
        </div>
      </div>

      {/* Networks List */}
      <div className="flex-1 overflow-y-auto space-y-3 pb-8">
        {loading && networks.length === 0 && <SkeletonList rows={4} actions={3} />}
        <AnimatePresence>
          {progressive.visible.map((net) => {
            const isDefault = ["bridge", "host", "none"].includes(net.name);

            return (
              <motion.div
                key={net.id}
                layout={progressive.animate}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, scale: 0.98 }}
              >
                <Card className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 p-4 hover:border-zinc-700/80 transition-all">
                  <div className="flex items-start gap-4">
                    <div className="w-10 h-10 rounded-xl bg-blue-500/10 border border-blue-500/20 flex items-center justify-center text-blue-400 shrink-0">
                      <Network className="w-5 h-5" />
                    </div>

                    <div>
                      <div className="flex items-center gap-2">
                        <span className="font-semibold text-white tracking-tight text-sm">
                          {net.name}
                        </span>
                        <span className="px-2 py-0.5 rounded-none text-[11px] bg-zinc-800 text-zinc-400 font-mono">
                          {net.driver}
                        </span>
                        {net.protected && (
                          <span className="px-1.5 py-0.5 rounded text-[10px] bg-sky-500/10 text-sky-300 border border-sky-500/30">
                            kind
                          </span>
                        )}
                        {isDefault && (
                          <span className="px-1.5 py-0.5 rounded text-[10px] bg-zinc-800/80 text-zinc-500">
                            system
                          </span>
                        )}
                      </div>
                      <div className="flex items-center gap-3 text-xs text-zinc-400 mt-1 font-mono">
                        <span>{truncateId(net.id)}</span>
                        {net.subnet && (
                          <>
                            <span>•</span>
                            <span>Subnet: {net.subnet}</span>
                          </>
                        )}
                        {net.gateway && (
                          <>
                            <span>•</span>
                            <span>Gateway: {net.gateway}</span>
                          </>
                        )}
                      </div>
                    </div>
                  </div>

                  {!isDefault && !net.protected && (
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => handleRemove(net.id, net.name)}
                      title="Remove Network"
                      className="text-zinc-400 hover:text-rose-400 hover:bg-rose-500/10"
                    >
                      <Trash2 className="w-4 h-4" />
                    </Button>
                  )}
                </Card>
              </motion.div>
            );
          })}
        </AnimatePresence>
        {progressive.hasMore && <div ref={progressive.sentinelRef} className="h-8" aria-hidden="true" />}

        {!loading && filteredNetworks.length === 0 && (
          <div className="py-16 text-center text-zinc-400">
            <Network className="w-10 h-10 mx-auto text-zinc-400 mb-3" />
            <p className="font-medium text-white">No networks found</p>
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
              <h3 className="font-semibold text-lg text-white">Create Network</h3>
              <Button variant="ghost" size="icon" onClick={() => setModalOpen(false)} title="Close">
                <X className="w-5 h-5" />
              </Button>
            </div>

            <form onSubmit={handleCreate} className="space-y-4">
              <div>
                <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                  Network Name *
                </label>
                <input
                  type="text"
                  placeholder="e.g. app-tier, internal-db"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  required
                  className="w-full h-10 px-3.5 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500"
                />
              </div>

              <div>
                <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                  Driver
                </label>
                <select
                  value={driver}
                  onChange={(e) => setDriver(e.target.value)}
                  className="w-full h-10 px-3 rounded-xl bg-zinc-900 border border-zinc-800 text-xs text-white focus:outline-none focus:border-blue-500 cursor-pointer"
                >
                  <option value="bridge" className="bg-zinc-900 text-white">Bridge (Default)</option>
                  <option value="macvlan" className="bg-zinc-900 text-white">Macvlan</option>
                  <option value="ipvlan" className="bg-zinc-900 text-white">IPvlan</option>
                  <option value="overlay" className="bg-zinc-900 text-white">Overlay</option>
                </select>
              </div>

              <div>
                <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                  Subnet (Optional)
                </label>
                <input
                  type="text"
                  placeholder="e.g. 172.28.0.0/16"
                  value={subnet}
                  onChange={(e) => setSubnet(e.target.value)}
                  className="w-full h-10 px-3.5 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500"
                />
              </div>

              <div>
                <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                  Gateway (Optional)
                </label>
                <input
                  type="text"
                  placeholder="e.g. 172.28.0.1"
                  value={gateway}
                  onChange={(e) => setGateway(e.target.value)}
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
