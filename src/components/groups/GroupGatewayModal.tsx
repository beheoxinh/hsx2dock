import React, { useState, useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Globe,
  X,
  Check,
  ExternalLink,
  ShieldCheck,
} from "lucide-react";
import { Button } from "../ui/Button";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";
import type { ContainerItem, GatewayRoute } from "../../types";

interface GroupGatewayModalProps {
  isOpen: boolean;
  groupName: string;
  members: ContainerItem[];
  onClose: () => void;
  onSuccess: () => void;
}

interface ServiceMappingDraft {
  containerId: string;
  containerName: string;
  hostPort: number;
  containerPort: string;
  enabled: boolean;
  subdomain: string;
  domainSuffix: string;
  path: string;
  stripPrefix: boolean;
  existingRouteId?: string;
}

export const GroupGatewayModal: React.FC<GroupGatewayModalProps> = ({
  isOpen,
  groupName,
  members,
  onClose,
  onSuccess,
}) => {
  const toast = useToast();
  const [loading, setLoading] = useState(false);
  const [domainSuffix, setDomainSuffix] = useState(".localhost");
  const [customSuffix, setCustomSuffix] = useState("");
  const [drafts, setDrafts] = useState<ServiceMappingDraft[]>([]);
  const [existingRoutes, setExistingRoutes] = useState<GatewayRoute[]>([]);

  useEffect(() => {
    if (!isOpen) return;

    api
      .listGatewayRoutes()
      .then((routes) => {
        setExistingRoutes(routes);
        const safeGroup = groupName.toLowerCase().replace(/[^a-z0-9-]/g, "-");
        const list: ServiceMappingDraft[] = [];

        for (const c of members) {
          const cName = c.name.replace(/^\//, "");
          const safeCName = cName.toLowerCase().replace(/[^a-z0-9-]/g, "-");

          if (c.published_ports && c.published_ports.length > 0) {
            for (const p of c.published_ports) {
              if (!p.host_port) continue;
              const target = `127.0.0.1:${p.host_port}`;
              const match = routes.find(
                (r) =>
                  r.target === target ||
                  (r.container_name === cName && r.target.endsWith(`:${p.host_port}`))
              );

              let sub = `${safeCName}.${safeGroup}`;
              let suff = ".localhost";
              let pth = "";
              let strp = false;

              if (match) {
                // If existing route exists, parse it
                pth = match.path || "";
                strp = !!match.strip_prefix;
                if (match.domain.endsWith(".localhost")) {
                  suff = ".localhost";
                  sub = match.domain.replace(/\.localhost$/, "");
                } else if (match.domain.endsWith(".local")) {
                  suff = ".local";
                  sub = match.domain.replace(/\.local$/, "");
                } else if (match.domain.endsWith(".internal")) {
                  suff = ".internal";
                  sub = match.domain.replace(/\.internal$/, "");
                } else {
                  suff = "custom";
                  sub = match.domain;
                }
              }

              list.push({
                containerId: c.id,
                containerName: cName,
                hostPort: p.host_port,
                containerPort: String(p.container_port || ""),
                enabled: true,
                subdomain: sub,
                domainSuffix: suff,
                path: pth,
                stripPrefix: strp,
                existingRouteId: match?.id,
              });
            }
          }
        }
        setDrafts(list);
      })
      .catch((e) => console.error(e));
  }, [isOpen, groupName, members]);

  const handleApplySuffixToAll = (suffix: string) => {
    setDomainSuffix(suffix);
    setDrafts((prev) =>
      prev.map((d) => ({
        ...d,
        domainSuffix: suffix,
      }))
    );
  };

  const handleSave = async () => {
    try {
      setLoading(true);
      const routesToSave = [...existingRoutes];
      let added = 0;
      let updated = 0;

      for (const d of drafts) {
        if (!d.enabled) {
          // If disabled and previously existed, remove it
          if (d.existingRouteId) {
            const idx = routesToSave.findIndex((r) => r.id === d.existingRouteId);
            if (idx !== -1) {
              routesToSave.splice(idx, 1);
            }
          }
          continue;
        }

        const effectiveSuffix =
          d.domainSuffix === "custom"
            ? (customSuffix.startsWith(".") ? customSuffix : `.${customSuffix}`)
            : d.domainSuffix;

        const cleanSub = d.subdomain.trim().replace(/^\.+|\.+$/g, "");
        if (!cleanSub) continue;

        const finalDomain = d.domainSuffix === "custom" && !customSuffix.trim()
          ? cleanSub
          : `${cleanSub}${effectiveSuffix}`;

        const target = `127.0.0.1:${d.hostPort}`;
        const newRoute: GatewayRoute = {
          id: d.existingRouteId || `route-${Date.now()}-${d.hostPort}`,
          domain: finalDomain.toLowerCase(),
          target,
          path: d.path.trim() ? (d.path.startsWith("/") ? d.path.trim() : `/${d.path.trim()}`) : null,
          strip_prefix: d.stripPrefix,
          container_name: d.containerName,
          group_name: groupName,
          enabled: true,
          auto_detected: true,
        };

        const existingIdx = routesToSave.findIndex(
          (r) => r.id === newRoute.id || r.domain === newRoute.domain
        );

        if (existingIdx !== -1) {
          routesToSave[existingIdx] = newRoute;
          updated++;
        } else {
          routesToSave.push(newRoute);
          added++;
        }
      }

      await api.saveGatewayRoutes(routesToSave);
      toast.success(
        "Group Gateway Configured",
        `Configured ${added + updated} routes for group '${groupName}'`
      );
      onSuccess();
      onClose();
    } catch (e) {
      toast.error("Failed to configure gateway", String(e));
    } finally {
      setLoading(false);
    }
  };

  if (!isOpen) return null;

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
          className="relative w-full max-w-3xl bg-zinc-950 border border-zinc-800 rounded-2xl shadow-2xl overflow-hidden flex flex-col max-h-[90vh] z-10"
        >
          {/* Header */}
          <div className="px-6 py-4 border-b border-zinc-800 flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div className="w-9 h-9 rounded-xl bg-violet-600/20 border border-violet-500/30 flex items-center justify-center text-violet-400">
                <Globe className="w-5 h-5" />
              </div>
              <div>
                <h2 className="text-base font-semibold text-white flex items-center gap-2">
                  Configure HTTPS Gateway for Group
                  <span className="text-xs px-2 py-0.5 rounded-md bg-zinc-800 text-indigo-300 font-mono">
                    {groupName}
                  </span>
                </h2>
                <p className="text-xs text-zinc-400 mt-0.5">
                  Customize domain names, extensions, and path prefixes before routing.
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

          {/* Quick Preset Selector */}
          <div className="px-6 py-3 bg-zinc-900/60 border-b border-zinc-800/80 flex items-center justify-between flex-wrap gap-2 text-xs">
            <span className="text-zinc-400 font-medium">Domain Suffix for all services:</span>
            <div className="flex items-center gap-2">
              {[".localhost", ".local", ".internal", "custom"].map((suff) => (
                <button
                  key={suff}
                  type="button"
                  onClick={() => handleApplySuffixToAll(suff)}
                  className={`px-2.5 py-1 rounded-lg border font-mono transition-colors ${
                    domainSuffix === suff
                      ? "bg-violet-600/20 border-violet-500/50 text-violet-300"
                      : "bg-zinc-800/60 border-zinc-700/60 text-zinc-400 hover:text-zinc-200"
                  }`}
                >
                  {suff}
                </button>
              ))}
              {domainSuffix === "custom" && (
                <input
                  type="text"
                  placeholder=".mycorp.dev"
                  value={customSuffix}
                  onChange={(e) => setCustomSuffix(e.target.value)}
                  className="px-2 py-0.5 rounded bg-zinc-950 border border-zinc-700 text-white font-mono text-xs w-28 focus:outline-none focus:border-violet-500"
                />
              )}
            </div>
          </div>

          {/* Service Routes List */}
          <div className="flex-1 overflow-y-auto p-6 space-y-4">
            {drafts.length === 0 ? (
              <div className="text-center py-12 text-zinc-500 text-sm">
                No published container ports found in this group. Ensure containers have exposed host ports.
              </div>
            ) : (
              drafts.map((d, idx) => {
                const effectiveSuffix =
                  d.domainSuffix === "custom"
                    ? (customSuffix.startsWith(".") ? customSuffix : `.${customSuffix}`)
                    : d.domainSuffix;
                const previewUrl = `https://${d.subdomain}${effectiveSuffix}${d.path || ""}`;

                return (
                  <div
                    key={`${d.containerId}-${d.hostPort}`}
                    className={`p-4 rounded-xl border transition-all ${
                      d.enabled
                        ? "bg-zinc-900/40 border-zinc-800"
                        : "bg-zinc-950/40 border-zinc-800/40 opacity-60"
                    }`}
                  >
                    <div className="flex items-center justify-between mb-3">
                      <div className="flex items-center gap-2">
                        <input
                          type="checkbox"
                          checked={d.enabled}
                          onChange={(e) => {
                            const copy = [...drafts];
                            copy[idx].enabled = e.target.checked;
                            setDrafts(copy);
                          }}
                          className="rounded border-zinc-700 text-violet-600 focus:ring-violet-500 w-4 h-4"
                        />
                        <span className="font-semibold text-sm text-white">
                          {d.containerName}
                        </span>
                        <span className="font-mono text-xs px-2 py-0.5 rounded bg-zinc-800 text-zinc-400">
                          :{d.hostPort} &rarr; :{d.containerPort}
                        </span>
                      </div>

                      <div className="flex items-center gap-1.5 text-xs font-mono text-violet-400">
                        <ExternalLink className="w-3 h-3" />
                        <span className="truncate max-w-[280px]">{previewUrl}</span>
                      </div>
                    </div>

                    {d.enabled && (
                      <div className="grid grid-cols-1 md:grid-cols-12 gap-3 pt-2 border-t border-zinc-800/60">
                        {/* Domain prefix */}
                        <div className="md:col-span-5 space-y-1">
                          <label className="text-[11px] font-medium text-zinc-400">
                            Subdomain / Hostname
                          </label>
                          <input
                            type="text"
                            value={d.subdomain}
                            onChange={(e) => {
                              const copy = [...drafts];
                              copy[idx].subdomain = e.target.value;
                              setDrafts(copy);
                            }}
                            className="w-full h-8 px-2.5 rounded-lg bg-zinc-950 border border-zinc-800 text-xs text-white font-mono focus:border-violet-500 focus:outline-none"
                            placeholder="api.myproject"
                          />
                        </div>

                        {/* Suffix selection */}
                        <div className="md:col-span-3 space-y-1">
                          <label className="text-[11px] font-medium text-zinc-400">
                            Suffix
                          </label>
                          <select
                            value={d.domainSuffix}
                            onChange={(e) => {
                              const copy = [...drafts];
                              copy[idx].domainSuffix = e.target.value;
                              setDrafts(copy);
                            }}
                            className="w-full h-8 px-2 rounded-lg bg-zinc-950 border border-zinc-800 text-xs text-white font-mono focus:border-violet-500 focus:outline-none"
                          >
                            <option value=".localhost">.localhost</option>
                            <option value=".local">.local</option>
                            <option value=".internal">.internal</option>
                            <option value="custom">Custom</option>
                          </select>
                        </div>

                        {/* Path prefix */}
                        <div className="md:col-span-4 space-y-1">
                          <label className="text-[11px] font-medium text-zinc-400 flex items-center justify-between">
                            <span>Path (Optional)</span>
                            <label className="flex items-center gap-1 cursor-pointer font-sans text-[10px] text-zinc-400">
                              <input
                                type="checkbox"
                                checked={d.stripPrefix}
                                onChange={(e) => {
                                  const copy = [...drafts];
                                  copy[idx].stripPrefix = e.target.checked;
                                  setDrafts(copy);
                                }}
                                className="rounded border-zinc-700 text-violet-600 focus:ring-violet-500 w-3 h-3"
                              />
                              <span>Strip</span>
                            </label>
                          </label>
                          <input
                            type="text"
                            value={d.path}
                            placeholder="/api"
                            onChange={(e) => {
                              const copy = [...drafts];
                              copy[idx].path = e.target.value;
                              setDrafts(copy);
                            }}
                            className="w-full h-8 px-2.5 rounded-lg bg-zinc-950 border border-zinc-800 text-xs text-white font-mono focus:border-violet-500 focus:outline-none"
                          />
                        </div>
                      </div>
                    )}
                  </div>
                );
              })
            )}
          </div>

          {/* Footer */}
          <div className="px-6 py-4 border-t border-zinc-800 bg-zinc-950/50 flex items-center justify-between">
            <div className="flex items-center gap-2 text-xs text-zinc-400">
              <ShieldCheck className="w-4 h-4 text-emerald-400" />
              <span>Caddy will automatically generate SSL certificates for these domains.</span>
            </div>
            <div className="flex items-center gap-3">
              <Button variant="ghost" size="sm" onClick={onClose} disabled={loading}>
                Cancel
              </Button>
              <Button
                variant="primary"
                size="sm"
                onClick={handleSave}
                isLoading={loading}
                className="bg-violet-600 hover:bg-violet-500 text-white gap-1.5"
              >
                <Check className="w-4 h-4" />
                Apply Gateway Routes
              </Button>
            </div>
          </div>
        </motion.div>
      </div>
    </AnimatePresence>
  );
};
