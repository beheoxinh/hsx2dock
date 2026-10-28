import React, { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Globe,
  X,
  Check,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { Button } from "../ui/Button";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";
import type { ContainerItem, GatewayRoute } from "../../types";

interface ContainerGatewayModalProps {
  isOpen: boolean;
  container: ContainerItem | null;
  initialPort?: number | null;
  onClose: () => void;
  onSuccess: () => void;
}

export const ContainerGatewayModal: React.FC<ContainerGatewayModalProps> = ({
  isOpen,
  container,
  initialPort,
  onClose,
  onSuccess,
}) => {
  const toast = useToast();
  const [loading, setLoading] = useState(false);
  const [domain, setDomain] = useState("");
  const [targetPort, setTargetPort] = useState<number | "">("");
  const [path, setPath] = useState("");
  const [stripPrefix, setStripPrefix] = useState(false);
  const [existingRouteId, setExistingRouteId] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen || !container) return;

    const cName = container.name.replace(/^\//, "");
    const safeName = cName.toLowerCase().replace(/[^a-z0-9-]/g, "-");

    // Check ports
    const ports = (container.published_ports || [])
      .map((p) => p.host_port)
      .filter((p): p is number => !!p);

    const activePort = initialPort || (ports.length > 0 ? ports[0] : 80);
    setTargetPort(activePort);

    api.listGatewayRoutes().then((routes) => {
      const match = routes.find(
        (r) =>
          r.container_name === cName ||
          r.target === `127.0.0.1:${activePort}`
      );

      if (match) {
        setExistingRouteId(match.id);
        setDomain(match.domain);
        setPath(match.path || "");
        setStripPrefix(!!match.strip_prefix);
        const portFromTarget = match.target.split(":")[1];
        if (portFromTarget) {
          setTargetPort(parseInt(portFromTarget, 10));
        }
      } else {
        setExistingRouteId(null);
        setDomain(`${safeName}.localhost`);
        setPath("");
        setStripPrefix(false);
      }
    }).catch(console.error);
  }, [isOpen, container, initialPort]);

  if (!isOpen || !container) return null;

  const cName = container.name.replace(/^\//, "");
  const publishedPorts = (container.published_ports || []).filter((p) => !!p.host_port);

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!domain.trim()) {
      toast.error("Validation Error", "Domain is required.");
      return;
    }
    if (!targetPort) {
      toast.error("Validation Error", "Target host port is required.");
      return;
    }

    try {
      setLoading(true);
      const existing = await api.listGatewayRoutes();
      const cleanDomain = domain.trim().toLowerCase();
      const target = `127.0.0.1:${targetPort}`;

      const newRoute: GatewayRoute = {
        id: existingRouteId || `route-${Date.now()}`,
        domain: cleanDomain,
        target,
        path: path.trim() ? (path.startsWith("/") ? path.trim() : `/${path.trim()}`) : null,
        strip_prefix: stripPrefix,
        container_name: cName,
        group_name: container.compose_project || null,
        enabled: true,
        auto_detected: true,
      };

      const updated = existing.filter(
        (r) => r.id !== newRoute.id && r.domain !== cleanDomain
      );
      updated.push(newRoute);

      await api.saveGatewayRoutes(updated);
      toast.success(
        "Gateway Route Configured",
        `Mapped https://${cleanDomain} → ${target}`
      );
      onSuccess();
      onClose();
    } catch (err) {
      toast.error("Failed to save route", String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleDelete = async () => {
    if (!existingRouteId) return;
    try {
      setLoading(true);
      const existing = await api.listGatewayRoutes();
      const updated = existing.filter((r) => r.id !== existingRouteId);
      await api.saveGatewayRoutes(updated);
      toast.info("Route Removed", `Domain mapping for ${cName} deleted.`);
      onSuccess();
      onClose();
    } catch (err) {
      toast.error("Failed to delete route", String(err));
    } finally {
      setLoading(false);
    }
  };

  return (
    <AnimatePresence>
      <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          className="fixed inset-0 bg-black/70 backdrop-blur-md"
          onClick={onClose}
        />

        <motion.div
          initial={{ opacity: 0, scale: 0.95, y: 15 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.95, y: 15 }}
          className="relative w-full max-w-lg bg-zinc-950 border border-zinc-800 rounded-2xl shadow-2xl overflow-hidden flex flex-col z-10"
        >
          {/* Header */}
          <div className="px-6 py-4 border-b border-zinc-800 flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-9 h-9 rounded-xl bg-violet-600/20 border border-violet-500/30 flex items-center justify-center text-violet-400">
                <Globe className="w-5 h-5" />
              </div>
              <div>
                <h2 className="text-base font-semibold text-white">
                  Virtual Domain Routing
                </h2>
                <p className="text-xs text-zinc-400 mt-0.5">
                  HTTPS reverse proxy for container: <span className="text-zinc-200 font-mono">{cName}</span>
                </p>
              </div>
            </div>
            <button
              onClick={onClose}
              className="text-zinc-500 hover:text-white p-1 rounded-lg hover:bg-zinc-800 transition"
            >
              <X className="w-5 h-5" />
            </button>
          </div>

          <form onSubmit={handleSave} className="p-6 space-y-4">
            {/* Domain */}
            <div>
              <label className="text-xs font-semibold text-zinc-400 uppercase tracking-wider block mb-1.5">
                Domain Name
              </label>
              <div className="flex gap-2">
                <input
                  type="text"
                  required
                  value={domain}
                  onChange={(e) => setDomain(e.target.value)}
                  placeholder="app.localhost or mydev.local"
                  className="flex-1 h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-sm text-white font-mono focus:border-violet-500 focus:outline-none"
                />
              </div>
              <p className="text-[11px] text-zinc-500 mt-1">
                Local domains ending in .localhost resolve automatically without editing /etc/hosts. Other domains will be synced with /etc/hosts.
              </p>
            </div>

            {/* Target Port */}
            <div>
              <label className="text-xs font-semibold text-zinc-400 uppercase tracking-wider block mb-1.5">
                Forward To Container Port
              </label>
              {publishedPorts.length > 0 ? (
                <div className="flex gap-2 flex-wrap mb-2">
                  {publishedPorts.map((p) => (
                    <button
                      key={p.host_port}
                      type="button"
                      onClick={() => setTargetPort(p.host_port)}
                      className={`px-3 py-1.5 rounded-lg border text-xs font-mono transition-colors ${
                        targetPort === p.host_port
                          ? "bg-violet-600/20 border-violet-500/50 text-violet-300"
                          : "bg-zinc-900 border-zinc-800 text-zinc-400 hover:text-white"
                      }`}
                    >
                      127.0.0.1:{p.host_port} &rarr; :{p.container_port}
                    </button>
                  ))}
                </div>
              ) : null}
              <input
                type="number"
                required
                value={targetPort}
                onChange={(e) => setTargetPort(e.target.value ? parseInt(e.target.value, 10) : "")}
                placeholder="Host port, e.g. 8080"
                className="w-full h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-sm text-white font-mono focus:border-violet-500 focus:outline-none"
              />
            </div>

            {/* Path prefix & strip */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-3 pt-2">
              <div>
                <label className="text-xs font-semibold text-zinc-400 uppercase tracking-wider block mb-1.5">
                  Path Prefix (Optional)
                </label>
                <input
                  type="text"
                  value={path}
                  onChange={(e) => setPath(e.target.value)}
                  placeholder="/api or /admin"
                  className="w-full h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-sm text-white font-mono focus:border-violet-500 focus:outline-none"
                />
              </div>

              <div className="flex flex-col justify-end">
                <label className="flex items-center gap-2 p-2 rounded-lg bg-zinc-900/60 border border-zinc-800 cursor-pointer text-xs text-zinc-300 h-9">
                  <input
                    type="checkbox"
                    checked={stripPrefix}
                    onChange={(e) => setStripPrefix(e.target.checked)}
                    className="rounded border-zinc-700 text-violet-600 focus:ring-violet-500 w-4 h-4"
                  />
                  <span>Strip Path Prefix before forwarding</span>
                </label>
              </div>
            </div>

            {/* Preview link */}
            {domain && (
              <div className="p-3 rounded-lg bg-violet-500/10 border border-violet-500/20 flex items-center justify-between text-xs font-mono text-violet-300">
                <span className="flex items-center gap-1.5 truncate">
                  <Globe className="w-3.5 h-3.5 text-violet-400 shrink-0" />
                  https://{domain}{path || ""} &rarr; 127.0.0.1:{targetPort || "..."}
                </span>
                <ShieldCheck className="w-4 h-4 text-emerald-400 shrink-0" />
              </div>
            )}

            {/* Actions */}
            <div className="flex items-center justify-between pt-4 border-t border-zinc-800">
              {existingRouteId ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={handleDelete}
                  disabled={loading}
                  className="text-rose-400 hover:text-rose-300 hover:bg-rose-500/10 gap-1"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                  Remove Route
                </Button>
              ) : (
                <div />
              )}

              <div className="flex items-center gap-2">
                <Button variant="ghost" size="sm" type="button" onClick={onClose} disabled={loading}>
                  Cancel
                </Button>
                <Button
                  type="submit"
                  size="sm"
                  isLoading={loading}
                  className="bg-violet-600 hover:bg-violet-500 text-white gap-1.5"
                >
                  <Check className="w-4 h-4" />
                  Save Route
                </Button>
              </div>
            </div>
          </form>
        </motion.div>
      </div>
    </AnimatePresence>
  );
};
