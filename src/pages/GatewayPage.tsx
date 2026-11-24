import React, { useState, useEffect } from "react";
import { setGatewayRoutes } from "../lib/containerStore";
import {
  Globe,
  ShieldCheck,
  ShieldAlert,
  Power,
  Plus,
  Trash2,
  ExternalLink,
  Edit2,
  FileText,
  Server,
  CheckCircle2,
  AlertTriangle,
  Info,
  RefreshCw,
  Settings,
  Box,
  Compass,
} from "lucide-react";
import { HostsManagerModal } from "../components/gateway/HostsManagerModal";
import { motion, AnimatePresence } from "framer-motion";
import { api } from "../lib/api";
import { useToast } from "../components/ui/Toast";
import { Button } from "../components/ui/Button";
import { SkeletonList } from "../components/ui/Skeleton";
import type {
  CaddyInstallStatus,
  GatewayRoute,
  HostsSyncStatus,
  ContainerItem,
  DaemonState,
} from "../types";

interface GatewayPageProps {
  daemonState: DaemonState;
}

export const GatewayPage: React.FC<GatewayPageProps> = ({ daemonState }) => {
  const toast = useToast();
  const [status, setStatus] = useState<CaddyInstallStatus | null>(null);
  const [routes, setRoutes] = useState<GatewayRoute[]>([]);
  const [hostsSync, setHostsSync] = useState<HostsSyncStatus | null>(null);
  const [allContainers, setAllContainers] = useState<ContainerItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [actionInProgress, setActionInProgress] = useState<string | null>(null);

  // Modal State
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [modalMode, setModalMode] = useState<"freeland" | "mapped">("mapped");
  const [isHostsManagerOpen, setIsHostsManagerOpen] = useState(false);
  const [editingRoute, setEditingRoute] = useState<GatewayRoute | null>(null);
  const [formSubdomain, setFormSubdomain] = useState("");
  const [formSuffix, setFormSuffix] = useState(".localhost");
  const [formCustomSuffix, setFormCustomSuffix] = useState("");
  const [formTarget, setFormTarget] = useState("");
  const [formPath, setFormPath] = useState("");
  const [formStripPrefix, setFormStripPrefix] = useState(false);
  const [formContainerName, setFormContainerName] = useState("");
  const [formGroupName, setFormGroupName] = useState("");

  // Helper to parse domain into subdomain and suffix
  const parseDomain = (domain: string) => {
    let clean = domain.trim();
    if (clean.startsWith("http://")) clean = clean.substring(7);
    if (clean.startsWith("https://")) clean = clean.substring(8);

    if (clean.endsWith(".localhost")) {
      return { sub: clean.replace(/\.localhost$/, ""), suff: ".localhost", custom: "" };
    } else if (clean.endsWith(".local")) {
      return { sub: clean.replace(/\.local$/, ""), suff: ".local", custom: "" };
    } else if (clean.endsWith(".internal")) {
      return { sub: clean.replace(/\.internal$/, ""), suff: ".internal", custom: "" };
    } else if (clean.endsWith(".dev")) {
      return { sub: clean.replace(/\.dev$/, ""), suff: ".dev", custom: "" };
    } else {
      return { sub: clean, suff: "custom", custom: "" };
    }
  };

  const getFullDomain = () => {
    const sub = formSubdomain.trim().replace(/^\.+|\.+$/g, "");
    if (!sub) return "";
    if (formSuffix === "custom") {
      const cust = formCustomSuffix.trim();
      if (!cust) return sub;
      return `${sub}${cust.startsWith(".") ? cust : `.${cust}`}`;
    }
    return `${sub}${formSuffix}`;
  };

  const fetchData = async () => {
    try {
      setLoading(true);
      const [st, r, hs] = await Promise.all([
        api.getGatewayStatus(),
        api.listGatewayRoutes(),
        api.checkHostsSync(),
      ]);
      setStatus(st);
      setRoutes(r);
      setHostsSync(hs);

      if (daemonState === "Running") {
        try {
          const list = await api.listContainers(true);
          setAllContainers(list);
        } catch (e) {
          console.error("Failed to fetch running containers:", e);
        }
      } else {
        setAllContainers([]);
      }
    } catch (err) {
      console.error("Failed to fetch gateway status:", err);
      toast.error("Failed to load Gateway state", String(err));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    fetchData();
  }, [daemonState]);

  const handleToggleGateway = async () => {
    if (!status) return;
    try {
      setActionInProgress("toggle");
      const next = !status.running;
      await api.toggleGateway(next);
      toast.success(
        next ? "Gateway Started" : "Gateway Stopped",
        next ? "Caddy reverse proxy is now listening" : "Caddy process stopped"
      );
      await fetchData();
    } catch (err) {
      toast.error("Gateway toggle failed", String(err));
    } finally {
      setActionInProgress(null);
    }
  };

  const handleTrustCa = async () => {
    try {
      setActionInProgress("trust");
      await api.trustGatewayCa();
      toast.success("Root CA Trusted", "Caddy local root certificate was installed to system trust store.");
      await fetchData();
    } catch (err) {
      toast.error("Trust CA Failed", String(err));
    } finally {
      setActionInProgress(null);
    }
  };

  const handleSyncHosts = async () => {
    try {
      setActionInProgress("hosts");
      await api.syncHostsFile();
      toast.success("Hosts File Updated", "/etc/hosts was synced with active virtual domains.");
      await fetchData();
    } catch (err) {
      toast.error("Sync Hosts Failed", String(err));
    } finally {
      setActionInProgress(null);
    }
  };

  const handleToggleAutoStart = async () => {
    if (!status) return;
    try {
      const next = !status.auto_start;
      await api.setGatewayAutoStart(next);
      setStatus({ ...status, auto_start: next });
      toast.info(
        "Auto-start updated",
        next ? "Gateway will automatically start with Hsx2Dock" : "Gateway auto-start disabled"
      );
    } catch (err) {
      toast.error("Failed to update auto-start", String(err));
    }
  };

  const handleToggleRoute = async (routeId: string) => {
    const updated = routes.map((r) =>
      r.id === routeId ? { ...r, enabled: !r.enabled } : r
    );
    try {
      await api.saveGatewayRoutes(updated);
      setGatewayRoutes(updated);
      setRoutes(updated);
      toast.info("Route Updated", "Gateway route toggle saved.");
    } catch (err) {
      toast.error("Failed to update route", String(err));
    }
  };

  const handleDeleteRoute = async (routeId: string) => {
    const updated = routes.filter((r) => r.id !== routeId);
    try {
      await api.saveGatewayRoutes(updated);
      setGatewayRoutes(updated);
      setRoutes(updated);
      toast.success("Route Deleted", "Virtual domain mapping removed.");
      // Auto check hosts sync after deletion
      const hs = await api.checkHostsSync();
      setHostsSync(hs);
    } catch (err) {
      toast.error("Failed to delete route", String(err));
    }
  };

  const handleSaveModal = async (e: React.FormEvent) => {
    e.preventDefault();
    const finalDomain = getFullDomain();
    if (!finalDomain || !formTarget.trim()) {
      toast.error("Validation error", "Domain and Target are required.");
      return;
    }

    let cleanDomain = finalDomain.toLowerCase();
    if (cleanDomain.startsWith("http://")) cleanDomain = cleanDomain.substring(7);
    if (cleanDomain.startsWith("https://")) cleanDomain = cleanDomain.substring(8);

    let cleanTarget = formTarget.trim();
    if (cleanTarget.startsWith("http://")) cleanTarget = cleanTarget.substring(7);
    if (cleanTarget.startsWith("https://")) cleanTarget = cleanTarget.substring(8);

    const newRoute: GatewayRoute = {
      id: editingRoute ? editingRoute.id : `route-${Date.now()}`,
      domain: cleanDomain,
      target: cleanTarget,
      path: formPath.trim() ? formPath.trim() : null,
      strip_prefix: formStripPrefix,
      container_name: modalMode === "mapped" ? (formContainerName.trim() || null) : null,
      group_name: modalMode === "mapped" ? (formGroupName.trim() || null) : null,
      enabled: editingRoute ? editingRoute.enabled : true,
      auto_detected: editingRoute ? editingRoute.auto_detected : false,
    };

    let updated: GatewayRoute[];
    if (editingRoute) {
      updated = routes.map((r) => (r.id === editingRoute.id ? newRoute : r));
    } else {
      updated = [...routes, newRoute];
    }

    try {
      await api.saveGatewayRoutes(updated);
      setGatewayRoutes(updated);
      setRoutes(updated);
      setIsModalOpen(false);
      setEditingRoute(null);
      toast.success("Route Saved", `Mapping for ${cleanDomain} saved.`);
      const hs = await api.checkHostsSync();
      setHostsSync(hs);
    } catch (err) {
      toast.error("Save Route Failed", String(err));
    }
  };

  // Collect open published container ports from allContainers (running only)
  const openContainerPorts: Array<{
    container: ContainerItem;
    hostPort: number;
    containerPort: number;
  }> = [];

  allContainers.forEach((c) => {
    if (c.state === "running" && c.published_ports) {
      c.published_ports.forEach((p) => {
        if (p.host_port) {
          openContainerPorts.push({
            container: c,
            hostPort: p.host_port,
            containerPort: p.container_port,
          });
        }
      });
    }
  });

  return (
    <div className="h-full overflow-y-auto px-6 py-6 space-y-6 text-zinc-100">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 border-b border-zinc-800/80 pb-5">
        <div>
          <div className="flex items-center gap-3">
            <div className="p-2.5 rounded-xl bg-violet-500/10 border border-violet-500/20 text-violet-400">
              <Globe className="w-6 h-6" />
            </div>
            <div>
              <h1 className="text-xl font-semibold tracking-tight text-zinc-100 flex items-center gap-2">
                HTTPS Gateway & Local Proxy
                {status?.running && (
                  <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                    <span className="w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
                    Active
                  </span>
                )}
              </h1>
              <p className="text-xs text-zinc-400 mt-0.5">
                Caddy-powered automated SSL reverse proxy for local container services and custom virtual domains.
              </p>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2.5">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchData}
            disabled={loading}
            className="text-xs gap-1.5 border-zinc-800 bg-zinc-900/60 hover:bg-zinc-800 text-zinc-300"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
            Refresh
          </Button>

          <Button
            size="sm"
            onClick={handleToggleGateway}
            disabled={actionInProgress === "toggle"}
            className={`text-xs gap-1.5 font-medium ${
              status?.running
                ? "bg-rose-500/20 hover:bg-rose-500/30 text-rose-300 border border-rose-500/30"
                : "bg-emerald-600 hover:bg-emerald-500 text-white"
            }`}
          >
            <Power className="w-3.5 h-3.5" />
            {status?.running ? "Stop Gateway" : "Start Gateway"}
          </Button>
        </div>
      </div>

      {/* Top Status Cards */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
        {/* Gateway State Card */}
        <div className="p-4 rounded-xl border border-zinc-800/80 bg-zinc-900/40 space-y-3">
          <div className="flex items-center justify-between text-xs text-zinc-400">
            <span className="flex items-center gap-1.5 font-medium text-zinc-300">
              <Server className="w-4 h-4 text-violet-400" />
              Caddy Supervisor
            </span>
            <span className="font-mono text-[11px] text-zinc-500">
              v{status?.version || status?.latest || "2.11.7"}
            </span>
          </div>

          <div className="flex items-baseline justify-between pt-1">
            <div className="text-sm font-medium">
              {status?.running ? (
                <span className="text-emerald-400">Listening on :{status?.http_port} & :{status?.https_port}</span>
              ) : (
                <span className="text-zinc-500">Inactive</span>
              )}
            </div>
            <span className="text-xs font-mono text-zinc-500">Admin: 127.0.0.1:2019</span>
          </div>

          <div className="text-[11px] text-zinc-500 flex items-center justify-between border-t border-zinc-800/60 pt-2">
            <span className="flex items-center gap-1.5">
              <span>Auto-start with app</span>
            </span>
            <button
              type="button"
              onClick={handleToggleAutoStart}
              className={`px-2 py-0.5 rounded text-[10px] font-medium transition ${
                status?.auto_start
                  ? "bg-emerald-500/20 text-emerald-300 border border-emerald-500/30"
                  : "bg-zinc-800 text-zinc-400 border border-zinc-700"
              }`}
            >
              {status?.auto_start ? "Enabled" : "Disabled"}
            </button>
          </div>
        </div>

        {/* Local CA Trust Card */}
        <div className="p-4 rounded-xl border border-zinc-800/80 bg-zinc-900/40 space-y-3">
          <div className="flex items-center justify-between text-xs text-zinc-400">
            <span className="flex items-center gap-1.5 font-medium text-zinc-300">
              {status?.trusted ? (
                <ShieldCheck className="w-4 h-4 text-emerald-400" />
              ) : (
                <ShieldAlert className="w-4 h-4 text-amber-400" />
              )}
              SSL Root Authority
            </span>
            <span className="text-[11px] font-mono text-zinc-500">Internal CA</span>
          </div>

          <div className="flex items-center justify-between pt-1">
            <div className="text-xs">
              {status?.trusted ? (
                <span className="text-emerald-400 flex items-center gap-1">
                  <CheckCircle2 className="w-3.5 h-3.5" /> CA Trusted System-wide
                </span>
              ) : (
                <span className="text-amber-400 flex items-center gap-1">
                  <AlertTriangle className="w-3.5 h-3.5" /> Untrusted Root CA
                </span>
              )}
            </div>
            <Button
              variant="outline"
              size="sm"
              onClick={handleTrustCa}
              disabled={actionInProgress === "trust"}
              className="text-xs h-7 px-2.5 border-zinc-700 bg-zinc-800/60 hover:bg-zinc-700 text-zinc-200"
            >
              Trust CA
            </Button>
          </div>

          <div className="text-[11px] text-zinc-500 border-t border-zinc-800/60 pt-2">
            Auto generates valid HTTPS certificates for virtual domains.
          </div>
        </div>

        {/* /etc/hosts Sync Card */}
        <div className="p-4 rounded-xl border border-zinc-800/80 bg-zinc-900/40 space-y-3">
          <div className="flex items-center justify-between text-xs text-zinc-400">
            <span className="flex items-center gap-1.5 font-medium text-zinc-300">
              <FileText className="w-4 h-4 text-sky-400" />
              /etc/hosts Sync
            </span>
            <span className="text-[11px] font-mono text-zinc-500">Local DNS</span>
          </div>

          <div className="flex items-center justify-between pt-1">
            <div className="text-xs">
              {hostsSync?.synced ? (
                <span className="text-emerald-400 flex items-center gap-1">
                  <CheckCircle2 className="w-3.5 h-3.5" /> Managed Domains Synced
                </span>
              ) : (
                <span className="text-amber-400 flex items-center gap-1">
                  <AlertTriangle className="w-3.5 h-3.5" />
                  {hostsSync?.managed_domains.length ? "Sync Required" : "Up to date"}
                </span>
              )}
            </div>
            <div className="flex items-center gap-1.5">
              <Button
                variant="outline"
                size="sm"
                onClick={handleSyncHosts}
                disabled={actionInProgress === "hosts" || (hostsSync?.synced && hostsSync.managed_domains.length === 0)}
                className="text-xs h-7 px-2.5 border-zinc-700 bg-zinc-800/60 hover:bg-zinc-700 text-zinc-200"
              >
                Sync Hosts
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setIsHostsManagerOpen(true)}
                className="text-xs h-7 px-2 border-zinc-700 bg-zinc-800/60 hover:bg-zinc-700 text-zinc-300"
                title="Open Advanced /etc/hosts File Manager"
              >
                <Settings className="w-3 h-3" />
              </Button>
            </div>
          </div>

          <div className="text-[11px] text-zinc-500 border-t border-zinc-800/60 pt-2 flex items-center justify-between">
            <span>{hostsSync?.tampered ? "Tampered externally" : "RFC 6761 .localhost: native"}</span>
            <button
              type="button"
              onClick={() => setIsHostsManagerOpen(true)}
              className="text-sky-400 hover:underline"
            >
              Manage hosts file &rarr;
            </button>
          </div>
        </div>
      </div>

      {/* Virtual Domain Routes Table */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-sm font-semibold tracking-tight text-zinc-200">
              Active Virtual Domain Routes
            </h2>
            <p className="text-xs text-zinc-500">
              Incoming requests to these domains will be automatically routed via HTTPS reverse proxy.
            </p>
          </div>

          <Button
            size="sm"
            onClick={() => {
              setEditingRoute(null);
              setModalMode("mapped");
              setFormSubdomain("");
              setFormSuffix(".localhost");
              setFormCustomSuffix("");
              setFormTarget("");
              setFormPath("");
              setFormStripPrefix(false);
              setFormContainerName("");
              setFormGroupName("");
              setIsModalOpen(true);
            }}
            className="text-xs gap-1.5 bg-zinc-100 hover:bg-white text-zinc-900 font-medium"
          >
            <Plus className="w-3.5 h-3.5" />
            Add Route
          </Button>
        </div>

        {loading && routes.length === 0 && <SkeletonList rows={3} actions={3} />}
        {!loading && routes.length === 0 ? (
          <div className="p-8 text-center rounded-xl border border-dashed border-zinc-800 bg-zinc-900/20 space-y-2">
            <Globe className="w-8 h-8 text-zinc-600 mx-auto" />
            <div className="text-sm font-medium text-zinc-300">No Virtual Domain Routes</div>
            <p className="text-xs text-zinc-500 max-w-sm mx-auto">
              Map your container ports to custom domains like <code className="text-violet-400">myapp.localhost</code> or <code className="text-violet-400">api.local.dev</code> with zero hassle.
            </p>
          </div>
        ) : (
          <div className="rounded-xl border border-zinc-800/80 bg-zinc-900/40 overflow-hidden">
            <table className="w-full text-left text-xs">
              <thead className="bg-zinc-950/60 border-b border-zinc-800/80 text-zinc-400 font-medium">
                <tr>
                  <th className="py-2.5 px-4 w-12">Status</th>
                  <th className="py-2.5 px-4">Type</th>
                  <th className="py-2.5 px-4">Virtual Domain & Path</th>
                  <th className="py-2.5 px-4">Upstream Target</th>
                  <th className="py-2.5 px-4">Container / Group</th>
                  <th className="py-2.5 px-4 text-right">Actions</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-zinc-800/60 text-zinc-300">
                {routes.map((r) => {
                  const url = `https://${r.domain}`;
                  const isMapped = Boolean(r.container_name && r.container_name.trim().length > 0);

                  // Check container status if it's mapped
                  let containerRunning = false;
                  let targetContainer: ContainerItem | undefined;
                  if (isMapped) {
                    targetContainer = allContainers.find((c) => {
                      const cleanName = c.name.replace(/^\//, "");
                      return cleanName === r.container_name || c.id === r.container_name || c.short_id === r.container_name;
                    });
                    containerRunning = targetContainer?.state === "running";
                  }

                  const isInactive = isMapped && !containerRunning;

                  return (
                    <tr
                      key={r.id}
                      className={`transition ${
                        isInactive
                          ? "bg-zinc-950/40 text-zinc-500 opacity-60"
                          : "hover:bg-zinc-800/30 text-zinc-300"
                      }`}
                    >
                      <td className="py-3 px-4">
                        <button
                          type="button"
                          onClick={() => handleToggleRoute(r.id)}
                          className={`w-4 h-4 rounded-full border transition flex items-center justify-center ${
                            r.enabled
                              ? "bg-emerald-500/20 border-emerald-500 text-emerald-400"
                              : "border-zinc-700 bg-zinc-800 text-transparent"
                          }`}
                        >
                          <span className={`w-1.5 h-1.5 rounded-full ${r.enabled ? "bg-emerald-400" : "bg-transparent"}`} />
                        </button>
                      </td>
                      <td className="py-3 px-4">
                        {isMapped ? (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-[10px] font-medium bg-violet-500/10 text-violet-300 border border-violet-500/25">
                            <Box className="w-3 h-3 text-violet-400" />
                            Mapped
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-md text-[10px] font-medium bg-amber-500/10 text-amber-300 border border-amber-500/25">
                            <Compass className="w-3 h-3 text-amber-400" />
                            Freeland
                          </span>
                        )}
                      </td>
                      <td className="py-3 px-4">
                        <div className="flex items-center gap-2">
                          <span className={`font-medium ${isInactive ? "text-zinc-400" : "text-zinc-100"}`}>
                            {r.domain}
                          </span>
                          {r.path && (
                            <span className="px-1.5 py-0.5 rounded text-[10px] font-mono bg-zinc-800 border border-zinc-700 text-sky-300">
                              {r.path}
                              {r.strip_prefix ? " (strip)" : ""}
                            </span>
                          )}
                          {r.enabled && status?.running && !isInactive && (
                            <a
                              href={url + (r.path || "")}
                              target="_blank"
                              rel="noreferrer"
                              className="text-zinc-500 hover:text-violet-400 transition"
                              title={`Open ${url + (r.path || "")}`}
                            >
                              <ExternalLink className="w-3.5 h-3.5" />
                            </a>
                          )}
                        </div>
                      </td>
                      <td className="py-3 px-4 font-mono text-zinc-400">
                        {r.target}
                      </td>
                      <td className="py-3 px-4">
                        <div className="flex flex-col">
                          {isMapped ? (
                            <div className="flex items-center gap-1.5">
                              <span
                                className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                                  containerRunning ? "bg-emerald-400" : "bg-rose-500"
                                }`}
                              />
                              <span className={containerRunning ? "text-zinc-200" : "text-zinc-400"}>
                                {r.container_name}
                              </span>
                              {!containerRunning && (
                                <span className="text-[10px] text-rose-400 font-medium">
                                  ({targetContainer ? targetContainer.state : "offline"})
                                </span>
                              )}
                            </div>
                          ) : (
                            <span className="text-zinc-500 italic">Custom Target</span>
                          )}
                          {r.group_name && (
                            <span className="text-[10px] text-zinc-500 font-mono">
                              group: {r.group_name}
                            </span>
                          )}
                        </div>
                      </td>
                      <td className="py-3 px-4 text-right">
                        <div className="flex items-center justify-end gap-1.5">
                          <button
                            type="button"
                            onClick={() => {
                              setEditingRoute(r);
                              const parsed = parseDomain(r.domain);
                              setModalMode(r.container_name ? "mapped" : "freeland");
                              setFormSubdomain(parsed.sub);
                              setFormSuffix(parsed.suff);
                              setFormCustomSuffix(parsed.custom);
                              setFormTarget(r.target);
                              setFormPath(r.path || "");
                              setFormStripPrefix(r.strip_prefix || false);
                              setFormContainerName(r.container_name || "");
                              setFormGroupName(r.group_name || "");
                              setIsModalOpen(true);
                            }}
                            className="p-1.5 text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 rounded transition"
                            title="Edit Route"
                          >
                            <Edit2 className="w-3.5 h-3.5" />
                          </button>
                          <button
                            type="button"
                            onClick={() => handleDeleteRoute(r.id)}
                            className="p-1.5 text-zinc-400 hover:text-rose-400 hover:bg-rose-500/10 rounded transition"
                            title="Delete Route"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Information Callout */}
      <div className="p-4 rounded-xl border border-zinc-800/80 bg-zinc-900/30 flex items-start gap-3 text-xs text-zinc-400">
        <Info className="w-4 h-4 text-violet-400 shrink-0 mt-0.5" />
        <div className="space-y-1">
          <div className="font-medium text-zinc-300">How Local HTTPS Routing Works</div>
          <p>
            Any domain ending with <code className="text-zinc-300 font-mono">.localhost</code> (e.g. <code className="text-violet-400 font-mono">api.localhost</code>) automatically resolves to 127.0.0.1 in all modern browsers without needing <code className="text-zinc-300 font-mono">/etc/hosts</code> modifications.
          </p>
          <p>
            For custom top-level domains like <code className="text-violet-400 font-mono">myproject.local.dev</code>, click the <strong>Sync Hosts</strong> button above to automatically map them in <code className="text-zinc-300 font-mono">/etc/hosts</code>.
          </p>
        </div>
      </div>

      {/* Add / Edit Route Modal */}
      <AnimatePresence>
        {isModalOpen && (
          <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm">
            <motion.div
              initial={{ opacity: 0, scale: 0.95 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.95 }}
              className="w-full max-w-md bg-zinc-900 border border-zinc-800 rounded-xl shadow-2xl overflow-hidden"
            >
              <div className="p-4 border-b border-zinc-800 flex items-center justify-between">
                <h3 className="text-sm font-semibold text-zinc-100 flex items-center gap-2">
                  <Globe className="w-4 h-4 text-violet-400" />
                  {editingRoute ? "Edit Virtual Domain Route" : "New Virtual Domain Route"}
                </h3>
                <button
                  type="button"
                  onClick={() => setIsModalOpen(false)}
                  className="text-zinc-500 hover:text-zinc-300 text-sm"
                >
                  &times;
                </button>
              </div>

              {/* Mode Switcher Tabs */}
              <div className="flex border-b border-zinc-800 bg-zinc-950/70 p-1 gap-1">
                <button
                  type="button"
                  disabled={!!editingRoute}
                  onClick={() => setModalMode("mapped")}
                  className={`flex-1 py-1.5 px-3 text-xs font-medium rounded-lg transition flex items-center justify-center gap-2 ${
                    modalMode === "mapped"
                      ? "bg-zinc-800 text-violet-300 shadow-sm"
                      : "text-zinc-400 hover:text-zinc-200"
                  } ${editingRoute && modalMode !== "mapped" ? "opacity-30 cursor-not-allowed" : ""}`}
                >
                  <Box className="w-3.5 h-3.5" />
                  Map Active Container
                  {editingRoute && modalMode === "mapped" && (
                    <span className="text-[10px] text-zinc-500 font-normal">(Locked)</span>
                  )}
                </button>
                <button
                  type="button"
                  disabled={!!editingRoute}
                  onClick={() => {
                    setModalMode("freeland");
                    if (!editingRoute) {
                      setFormContainerName("");
                      setFormGroupName("");
                    }
                  }}
                  className={`flex-1 py-1.5 px-3 text-xs font-medium rounded-lg transition flex items-center justify-center gap-2 ${
                    modalMode === "freeland"
                      ? "bg-zinc-800 text-amber-300 shadow-sm"
                      : "text-zinc-400 hover:text-zinc-200"
                  } ${editingRoute && modalMode !== "freeland" ? "opacity-30 cursor-not-allowed" : ""}`}
                >
                  <Compass className="w-3.5 h-3.5" />
                  Freeland / Custom Target
                  {editingRoute && modalMode === "freeland" && (
                    <span className="text-[10px] text-zinc-500 font-normal">(Locked)</span>
                  )}
                </button>
              </div>

              <form onSubmit={handleSaveModal} className="p-4 space-y-4">
                {modalMode === "mapped" ? (
                  <>
                    {/* Active Container Port Picker */}
                    <div>
                      <label className="block text-xs font-medium text-zinc-300 mb-1">
                        Select Running Container Port
                      </label>
                      {openContainerPorts.length > 0 ? (
                        <select
                          className="w-full px-3 py-2 text-xs bg-zinc-950 border border-zinc-800 rounded-lg text-zinc-200 focus:outline-none focus:border-violet-500"
                          onChange={(e) => {
                            const val = e.target.value;
                            if (!val) return;
                            const [cId, hostPortStr] = val.split("|");
                            const c = allContainers.find((x) => x.id === cId);
                            if (!c) return;
                            const hp = parseInt(hostPortStr, 10);
                            setFormTarget(`127.0.0.1:${hp}`);
                            const safeName = c.name.replace(/^\//, "").toLowerCase().replace(/[^a-z0-9-]/g, "-");
                            setFormSubdomain(safeName);
                            setFormSuffix(".localhost");
                            setFormCustomSuffix("");
                            setFormContainerName(c.name.replace(/^\//, ""));
                            setFormGroupName(c.compose_project || "");
                          }}
                          defaultValue={
                            formContainerName && formTarget
                              ? openContainerPorts.find(
                                  (p) =>
                                    p.container.name.replace(/^\//, "") === formContainerName &&
                                    `127.0.0.1:${p.hostPort}` === formTarget
                                )
                                ? `${openContainerPorts.find(
                                    (p) =>
                                      p.container.name.replace(/^\//, "") === formContainerName &&
                                      `127.0.0.1:${p.hostPort}` === formTarget
                                  )?.container.id}|${openContainerPorts.find(
                                    (p) =>
                                      p.container.name.replace(/^\//, "") === formContainerName &&
                                      `127.0.0.1:${p.hostPort}` === formTarget
                                  )?.hostPort}`
                                : ""
                              : ""
                          }
                        >
                          <option value="" disabled>-- Select a container port to map --</option>
                          {openContainerPorts.map((item, idx) => {
                            const cName = item.container.name.replace(/^\//, "");
                            return (
                              <option
                                key={`${item.container.id}-${item.hostPort}-${idx}`}
                                value={`${item.container.id}|${item.hostPort}`}
                              >
                                {cName} ({item.container.compose_project ? `compose: ${item.container.compose_project}` : "isolated"}) → :{item.hostPort}
                              </option>
                            );
                          })}
                        </select>
                      ) : (
                        <div className="p-3 rounded-lg border border-zinc-800 bg-zinc-950/60 text-xs text-zinc-400 space-y-1">
                          <p className="text-amber-400 font-medium flex items-center gap-1.5">
                            <AlertTriangle className="w-3.5 h-3.5" />
                            No running containers with published ports
                          </p>
                          <p className="text-[11px] text-zinc-500">
                            Start a container with forwarded host ports, or switch to the <strong>Freeland</strong> tab to proxy any custom target manually.
                          </p>
                        </div>
                      )}
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-zinc-300 mb-1">
                        Virtual Domain & Suffix
                      </label>
                      <div className="flex items-center gap-2">
                        <div className="flex-1 relative">
                          <input
                            type="text"
                            required
                            placeholder="e.g. app or api"
                            value={formSubdomain}
                            onChange={(e) => setFormSubdomain(e.target.value)}
                            className="w-full px-3 py-1.5 text-xs bg-zinc-950 border border-zinc-800 rounded-lg text-zinc-100 font-mono focus:outline-none focus:border-violet-500"
                          />
                        </div>
                        <div className="flex items-center gap-1">
                          {[".localhost", ".local", ".internal", ".dev"].map((suff) => (
                            <button
                              key={suff}
                              type="button"
                              onClick={() => setFormSuffix(suff)}
                              className={`px-2 py-1 rounded text-xs font-mono transition-colors border ${
                                formSuffix === suff
                                  ? "bg-violet-600/25 border-violet-500/60 text-violet-300 font-semibold"
                                  : "bg-zinc-950 border-zinc-800 text-zinc-400 hover:text-zinc-200"
                              }`}
                            >
                              {suff}
                            </button>
                          ))}
                          <button
                            type="button"
                            onClick={() => setFormSuffix("custom")}
                            className={`px-2 py-1 rounded text-xs font-mono transition-colors border ${
                              formSuffix === "custom"
                                ? "bg-violet-600/25 border-violet-500/60 text-violet-300 font-semibold"
                                : "bg-zinc-950 border-zinc-800 text-zinc-400 hover:text-zinc-200"
                            }`}
                          >
                            custom
                          </button>
                        </div>
                      </div>

                      {formSuffix === "custom" && (
                        <div className="flex items-center gap-2 pt-1.5">
                          <span className="text-[11px] text-zinc-400">Custom Suffix:</span>
                          <input
                            type="text"
                            placeholder=".example.com or .orb.local"
                            value={formCustomSuffix}
                            onChange={(e) => setFormCustomSuffix(e.target.value)}
                            className="flex-1 h-7 px-2 rounded bg-zinc-950 border border-zinc-800 text-xs text-white font-mono focus:border-violet-500 focus:outline-none"
                          />
                        </div>
                      )}

                      <div className="flex items-center justify-between text-[11px] text-zinc-500 mt-1">
                        <span>Full domain: <code className="text-violet-300 font-mono font-medium">{getFullDomain() || "—"}</code></span>
                        <span>{formSuffix === ".localhost" ? "No /etc/hosts sync needed" : "Requires hosts sync"}</span>
                      </div>
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-zinc-300 mb-1">
                        Upstream Target (Host & Port)
                      </label>
                      <input
                        type="text"
                        required
                        placeholder="e.g. 127.0.0.1:8080"
                        value={formTarget}
                        onChange={(e) => setFormTarget(e.target.value)}
                        className="w-full px-3 py-2 text-xs bg-zinc-950 border border-zinc-800 rounded-lg text-zinc-100 font-mono focus:outline-none focus:border-violet-500"
                      />
                    </div>
                  </>
                ) : (
                  <>
                    {/* Freeland / Custom Route Mode */}
                    <div>
                      <label className="block text-xs font-medium text-zinc-300 mb-1">
                        Virtual Domain & Suffix
                      </label>
                      <div className="flex items-center gap-2">
                        <div className="flex-1 relative">
                          <input
                            type="text"
                            required
                            placeholder="e.g. web or dev-service"
                            value={formSubdomain}
                            onChange={(e) => setFormSubdomain(e.target.value)}
                            className="w-full px-3 py-1.5 text-xs bg-zinc-950 border border-zinc-800 rounded-lg text-zinc-100 font-mono focus:outline-none focus:border-violet-500"
                          />
                        </div>
                        <div className="flex items-center gap-1">
                          {[".localhost", ".local", ".internal", ".dev"].map((suff) => (
                            <button
                              key={suff}
                              type="button"
                              onClick={() => setFormSuffix(suff)}
                              className={`px-2 py-1 rounded text-xs font-mono transition-colors border ${
                                formSuffix === suff
                                  ? "bg-violet-600/25 border-violet-500/60 text-violet-300 font-semibold"
                                  : "bg-zinc-950 border-zinc-800 text-zinc-400 hover:text-zinc-200"
                              }`}
                            >
                              {suff}
                            </button>
                          ))}
                          <button
                            type="button"
                            onClick={() => setFormSuffix("custom")}
                            className={`px-2 py-1 rounded text-xs font-mono transition-colors border ${
                              formSuffix === "custom"
                                ? "bg-violet-600/25 border-violet-500/60 text-violet-300 font-semibold"
                                : "bg-zinc-950 border-zinc-800 text-zinc-400 hover:text-zinc-200"
                            }`}
                          >
                            custom
                          </button>
                        </div>
                      </div>

                      {formSuffix === "custom" && (
                        <div className="flex items-center gap-2 pt-1.5">
                          <span className="text-[11px] text-zinc-400">Custom Suffix:</span>
                          <input
                            type="text"
                            placeholder=".example.com or .orb.local"
                            value={formCustomSuffix}
                            onChange={(e) => setFormCustomSuffix(e.target.value)}
                            className="flex-1 h-7 px-2 rounded bg-zinc-950 border border-zinc-800 text-xs text-white font-mono focus:border-violet-500 focus:outline-none"
                          />
                        </div>
                      )}

                      <div className="flex items-center justify-between text-[11px] text-zinc-500 mt-1">
                        <span>Full domain: <code className="text-violet-300 font-mono font-medium">{getFullDomain() || "—"}</code></span>
                        <span>{formSuffix === ".localhost" ? "No /etc/hosts sync needed" : "Requires hosts sync"}</span>
                      </div>
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-zinc-300 mb-1">
                        Upstream Target (Host & Port)
                      </label>
                      <input
                        type="text"
                        required
                        placeholder="e.g. 127.0.0.1:3000 or 192.168.1.50:8080"
                        value={formTarget}
                        onChange={(e) => setFormTarget(e.target.value)}
                        className="w-full px-3 py-2 text-xs bg-zinc-950 border border-zinc-800 rounded-lg text-zinc-100 font-mono focus:outline-none focus:border-violet-500"
                      />
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-zinc-300 mb-1">
                        Associated Label / Note (Optional)
                      </label>
                      <input
                        type="text"
                        placeholder="e.g. External service or dev API"
                        value={formContainerName}
                        onChange={(e) => setFormContainerName(e.target.value)}
                        className="w-full px-3 py-2 text-xs bg-zinc-950 border border-zinc-800 rounded-lg text-zinc-100 focus:outline-none focus:border-violet-500"
                      />
                    </div>
                  </>
                )}

                {/* Path Mapping & Strip Prefix Option */}
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 pt-1 border-t border-zinc-800/60">
                  <div>
                    <label className="block text-xs font-medium text-zinc-300 mb-1">
                      Path Prefix (Optional)
                    </label>
                    <input
                      type="text"
                      placeholder="e.g. /api or /dashboard"
                      value={formPath}
                      onChange={(e) => setFormPath(e.target.value)}
                      className="w-full px-3 py-2 text-xs bg-zinc-950 border border-zinc-800 rounded-lg text-zinc-100 font-mono focus:outline-none focus:border-violet-500"
                    />
                  </div>
                  <div className="flex items-center pt-5">
                    <label className="flex items-center gap-2 cursor-pointer text-xs text-zinc-300">
                      <input
                        type="checkbox"
                        checked={formStripPrefix}
                        onChange={(e) => setFormStripPrefix(e.target.checked)}
                        className="rounded border-zinc-700 bg-zinc-950 text-violet-600 focus:ring-0"
                      />
                      <span>Strip path before forwarding</span>
                    </label>
                  </div>
                </div>

                <div className="pt-2 flex items-center justify-end gap-2 border-t border-zinc-800/80">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => setIsModalOpen(false)}
                    className="text-xs border-zinc-800 text-zinc-400 hover:text-zinc-200"
                  >
                    Cancel
                  </Button>
                  <Button
                    type="submit"
                    size="sm"
                    className="text-xs bg-violet-600 hover:bg-violet-500 text-white font-medium"
                  >
                    {editingRoute ? "Save Changes" : "Create Route"}
                  </Button>
                </div>
              </form>
            </motion.div>
          </div>
        )}
      </AnimatePresence>

      {/* Advanced /etc/hosts Manager Modal */}
      <HostsManagerModal
        isOpen={isHostsManagerOpen}
        onClose={() => setIsHostsManagerOpen(false)}
        onUpdated={fetchData}
      />
    </div>
  );
};
