import React, { useState, useEffect, useMemo } from "react";
import { primeContainerStore, setGatewayRoutes } from "../lib/containerStore";
import { joinArgs, splitArgs } from "../lib/shellArgs";
import { motion, AnimatePresence } from "framer-motion";
import {
  X,
  Plus,
  Trash2,
  Box,
  ArrowRight,
  RotateCw,
  ChevronDown,
  ChevronUp,
  Globe,
  ShieldCheck,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { useToast } from "../components/ui/Toast";
import { api } from "../lib/api";
import type {
  RunConfig,
  PortMapping,
  EnvVar,
  VolumeMount,
  ContainerDetails,
} from "../types";

export interface EditContainerTarget {
  id: string;
  details: ContainerDetails;
}

interface RunContainerWizardProps {
  isOpen: boolean;
  editTarget?: EditContainerTarget | null;
  onClose: () => void;
  onSuccess: (newId?: string) => void;
}

export const RunContainerWizard: React.FC<RunContainerWizardProps> = ({
  isOpen,
  editTarget = null,
  onClose,
  onSuccess,
}) => {
  const toast = useToast();
  const isEdit = !!editTarget;

  // Hot fields (always visible)
  const [image, setImage] = useState("");
  const [name, setName] = useState("");
  const [restartPolicy, setRestartPolicy] = useState("unless-stopped");
  const [memoryMb, setMemoryMb] = useState("");
  const [cpus, setCpus] = useState("");

  // Recreate fields (advanced in edit mode)
  const [ports, setPorts] = useState<PortMapping[]>([
    { host_ip: "127.0.0.1", host_port: "8080", container_port: "80" },
  ]);
  const [envVars, setEnvVars] = useState<EnvVar[]>([]);
  const [volumes, setVolumes] = useState<VolumeMount[]>([]);
  const [command, setCommand] = useState("");
  const [network, setNetwork] = useState("");

  // Gateway routing options
  const [enableGateway, setEnableGateway] = useState(false);
  const [gatewaySubdomain, setGatewaySubdomain] = useState("");
  const [gatewaySuffix, setGatewaySuffix] = useState(".localhost");
  const [gatewayCustomSuffix, setGatewayCustomSuffix] = useState("");
  const [gatewayPath, setGatewayPath] = useState("");
  const [gatewayStripPrefix, setGatewayStripPrefix] = useState(false);

  // Construct full domain based on subdomain and selected suffix
  const effectiveGatewayDomain = useMemo(() => {
    const sub = gatewaySubdomain.trim().replace(/^\.+|\.+$/g, "");
    if (!sub) return "";
    if (gatewaySuffix === "custom") {
      const cust = gatewayCustomSuffix.trim();
      if (!cust) return sub;
      return `${sub}${cust.startsWith(".") ? cust : `.${cust}`}`;
    }
    return `${sub}${gatewaySuffix}`;
  }, [gatewaySubdomain, gatewaySuffix, gatewayCustomSuffix]);
  // UI state
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [loading, setLoading] = useState(false);

  // Determine if advanced/recreate-required parameters have changed compared to original inspect details
  const hasAdvancedChanged = useMemo(() => {
    if (!isEdit || !editTarget) return false;
    const d = editTarget.details;

    // Image check
    if (image.trim() !== (d.image || "").trim()) return true;

    // Command check
    const origCmd = d.command_vec ? d.command_vec : [];
    const curCmd = command.trim() ? splitArgs(command.trim()) : [];
    if (curCmd.length !== origCmd.length || curCmd.some((a, i) => a !== origCmd[i])) return true;

    // Network check
    const origNet = (d.network || "").trim();
    if (network.trim() !== origNet) return true;

    // Ports check
    const curPorts = ports
      .filter((p) => p.container_port.trim() !== "")
      .map((p) => `${p.host_ip ?? ""}:${p.host_port || ""}:${p.container_port}/${p.protocol || "tcp"}`)
      .sort();
    const origPorts = (d.port_mappings || [])
      .filter((p) => (p.container_port || "").trim() !== "")
      .map((p) => `${p.host_ip ?? ""}:${p.host_port || ""}:${p.container_port}/${p.protocol || "tcp"}`)
      .sort();
    if (curPorts.length !== origPorts.length || curPorts.some((val, i) => val !== origPorts[i])) {
      return true;
    }

    // Env vars check
    const curEnv = envVars
      .filter((e) => e.key.trim() !== "")
      .map((e) => `${e.key}=${e.value}`)
      .sort();
    const origEnvRaw = d.env_pairs && d.env_pairs.length > 0
      ? d.env_pairs.map((e) => `${e.key}=${e.value}`)
      : (d.env || []).map((line) => line);
    const origEnv = origEnvRaw.sort();
    if (curEnv.length !== origEnv.length || curEnv.some((val, i) => val !== origEnv[i])) {
      return true;
    }

    // Volumes check
    const curVols = volumes
      .filter((v) => v.source.trim() !== "" && v.target.trim() !== "")
      .map((v) => `${v.kind || "bind"}:${v.source}:${v.target}:${!!v.read_only}`)
      .sort();
    const origVols = (d.volume_mounts || [])
      .filter((v) => (v.source || "").trim() !== "" && (v.target || "").trim() !== "")
      .map((v) => `${v.kind || "bind"}:${v.source}:${v.target}:${!!v.read_only}`)
      .sort();
    if (curVols.length !== origVols.length || curVols.some((val, i) => val !== origVols[i])) {
      return true;
    }

    return false;
  }, [isEdit, editTarget, image, command, network, ports, envVars, volumes]);

  // Sync state when dialog opens or editTarget changes
  useEffect(() => {
    if (!isOpen) return;

    if (editTarget) {
      const d = editTarget.details;
      setImage(d.image || "");
      setName(d.name.replace(/^\//, "") || "");
      setRestartPolicy(
        d.restart_policy || d.limits?.restart_policy || "unless-stopped"
      );
      setMemoryMb(d.limits?.memory_mb || d.resources?.memory_mb || "");
      setCpus(d.limits?.cpus || d.resources?.cpus || "");

      // Ports from inspect
      if (d.port_mappings && d.port_mappings.length > 0) {
        setPorts(
          d.port_mappings.map((p) => ({
            // Empty means "all interfaces"; turning it into 127.0.0.1 made
            // a LAN-reachable service localhost-only after any edit.
            host_ip: p.host_ip ?? "",
            host_port: p.host_port || "",
            container_port: p.container_port || "",
            protocol: p.protocol || "tcp",
          }))
        );
      } else {
        setPorts([]);
      }

      // ENV pairs from inspect
      if (d.env_pairs && d.env_pairs.length > 0) {
        setEnvVars(d.env_pairs.map((e) => ({ key: e.key, value: e.value })));
      } else if (d.env && d.env.length > 0) {
        setEnvVars(
          d.env.map((line) => {
            const idx = line.indexOf("=");
            return idx !== -1
              ? { key: line.slice(0, idx), value: line.slice(idx + 1) }
              : { key: line, value: "" };
          })
        );
      } else {
        setEnvVars([]);
      }

      // Volume mounts
      if (d.volume_mounts && d.volume_mounts.length > 0) {
        setVolumes(
          d.volume_mounts.map((v) => ({
            kind: v.kind || "bind",
            source: v.source || "",
            target: v.target || "",
            read_only: !!v.read_only,
          }))
        );
      } else {
        setVolumes([]);
      }

      // Command & Network
      // `command_vec` is the container's CMD. `command` is Path+Args, i.e.
      // the entrypoint too — sent back as CMD it ran the entrypoint twice.
      setCommand(d.command_vec ? joinArgs(d.command_vec) : "");
      setNetwork(d.network || "");

      // Load existing gateway route if container has one
      const cName = d.name.replace(/^\//, "");
      api.listGatewayRoutes().then((routes) => {
        const found = routes.find((r) => r.container_name === cName);
        if (found) {
          setEnableGateway(true);
          const dom = found.domain;
          if (dom.endsWith(".localhost")) {
            setGatewaySuffix(".localhost");
            setGatewaySubdomain(dom.replace(/\.localhost$/, ""));
          } else if (dom.endsWith(".local")) {
            setGatewaySuffix(".local");
            setGatewaySubdomain(dom.replace(/\.local$/, ""));
          } else if (dom.endsWith(".internal")) {
            setGatewaySuffix(".internal");
            setGatewaySubdomain(dom.replace(/\.internal$/, ""));
          } else if (dom.endsWith(".dev")) {
            setGatewaySuffix(".dev");
            setGatewaySubdomain(dom.replace(/\.dev$/, ""));
          } else {
            setGatewaySuffix("custom");
            setGatewaySubdomain(dom);
            setGatewayCustomSuffix("");
          }
          setGatewayPath(found.path || "");
          setGatewayStripPrefix(!!found.strip_prefix);
        } else {
          setEnableGateway(false);
          setGatewaySubdomain(cName.toLowerCase().replace(/[^a-z0-9-]/g, "-"));
          setGatewaySuffix(".localhost");
          setGatewayCustomSuffix("");
          setGatewayPath("");
          setGatewayStripPrefix(false);
        }
      }).catch(console.error);

      // Default collapsed when opened
      setShowAdvanced(false);
    } else {
      // New container defaults
      setImage("");
      setName("");
      setRestartPolicy("unless-stopped");
      setPorts([
        { host_ip: "127.0.0.1", host_port: "8080", container_port: "80" },
      ]);
      setEnvVars([]);
      setVolumes([]);
      setCommand("");
      setNetwork("");
      setMemoryMb("");
      setCpus("");
      setEnableGateway(false);
      setGatewaySubdomain("");
      setGatewaySuffix(".localhost");
      setGatewayCustomSuffix("");
      setGatewayPath("");
      setGatewayStripPrefix(false);
      setShowAdvanced(false);
    }
  }, [isOpen, editTarget]);

  if (!isOpen) return null;

  const handleAddPort = () => {
    setPorts([
      ...ports,
      { host_ip: "127.0.0.1", container_port: "", host_port: "" },
    ]);
  };

  const handleRemovePort = (index: number) => {
    setPorts(ports.filter((_, i) => i !== index));
  };

  const handleAddEnv = () => {
    setEnvVars([...envVars, { key: "", value: "" }]);
  };

  const handleRemoveEnv = (index: number) => {
    setEnvVars(envVars.filter((_, i) => i !== index));
  };

  const handleAddVolume = () => {
    setVolumes([
      ...volumes,
      { kind: "bind", source: "", target: "", read_only: false },
    ]);
  };

  const handleRemoveVolume = (index: number) => {
    setVolumes(volumes.filter((_, i) => i !== index));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!image.trim()) {
      toast.error("Validation Error", "Image name is required");
      return;
    }

    try {
      setLoading(true);

      // Shell-style split: `nginx -g "daemon off;"` is three arguments.
      const parsedCommand = command.trim() ? splitArgs(command.trim()) : undefined;

      const config: RunConfig = {
        image: image.trim(),
        name: name.trim() || undefined,
        restart_policy: restartPolicy,
        command: parsedCommand,
        network: network.trim() || undefined,
        ports: ports.filter((p) => p.container_port.trim() !== ""),
        env: envVars.filter((e) => e.key.trim() !== ""),
        volumes: volumes.filter(
          (v) => v.source.trim() !== "" && v.target.trim() !== ""
        ),
        resources: {
          memory_mb: memoryMb.trim() || undefined,
          cpus: cpus.trim() || undefined,
        },
      };

      const finalContainerName = (name.trim() || (isEdit && editTarget ? editTarget.details.name : "")).replace(/^\//, "");

      if (isEdit && editTarget) {
        let activeContainerId = editTarget.id;

        if (hasAdvancedChanged) {
          // Advanced parameters changed -> Recreate the container
          activeContainerId = await api.recreateContainer(editTarget.id, config);
          // The backend re-attaches the new id to its group and broadcasts it;
          // re-reading the store also covers a missed event.
          void primeContainerStore(true);
          toast.success(
            "Container Recreated",
            `Container ${name || editTarget.details.name} updated and recreated.`
          );
        } else {
          // Only hot configurations changed -> In-place update without recreation
          const currentName = editTarget.details.name.replace(/^\//, "");
          if (name.trim() && name.trim() !== currentName) {
            await api.renameContainer(editTarget.id, name.trim());
          }

          // Update resources & restart policy
          await api.updateContainerResources(editTarget.id, {
            memory_mb: memoryMb.trim() || undefined,
            cpus: cpus.trim() || undefined,
            restart_policy: restartPolicy,
          });

          toast.success(
            "Changes Saved",
            `Container ${name.trim() || currentName} updated in-place.`
          );
        }

        // Sync gateway route if configured
        const currentPorts = config.ports && config.ports.length > 0 ? config.ports : (editTarget.details.port_mappings || []);
        const firstPort = currentPorts[0]?.host_port || "80";
        try {
          const routes = await api.listGatewayRoutes();
          const cleanDomain = effectiveGatewayDomain.toLowerCase();
          const updated = routes.filter(
            (r) => r.container_name !== finalContainerName && (cleanDomain ? r.domain !== cleanDomain : true)
          );

          if (enableGateway && cleanDomain) {
            const routeId = `route-${Date.now()}`;
            const target = `127.0.0.1:${firstPort}`;
            updated.push({
              id: routeId,
              domain: cleanDomain,
              target,
              path: gatewayPath.trim() ? (gatewayPath.startsWith("/") ? gatewayPath.trim() : `/${gatewayPath.trim()}`) : null,
              strip_prefix: gatewayStripPrefix,
              container_name: finalContainerName,
              group_name: null,
              enabled: true,
              auto_detected: true,
            });
          }

          await api.saveGatewayRoutes(updated);
          setGatewayRoutes(updated);
        } catch (gErr) {
          console.warn("Failed to update gateway route:", gErr);
        }

        onSuccess(activeContainerId);
      } else {
        // Run fresh container
        const newId = await api.runContainer(config);
        const effectiveName = finalContainerName || newId.slice(0, 12);
        toast.success(
          "Container Launched",
          `New container instance created: ${effectiveName}`
        );

        // Sync gateway route if configured
        if (enableGateway && effectiveGatewayDomain) {
          const firstPort = config.ports?.[0]?.host_port || "80";
          try {
            const routes = await api.listGatewayRoutes();
            const cleanDomain = effectiveGatewayDomain.toLowerCase();
            const routeId = `route-${Date.now()}`;
            const target = `127.0.0.1:${firstPort}`;
            const updated = routes.filter((r) => r.domain !== cleanDomain);
            updated.push({
              id: routeId,
              domain: cleanDomain,
              target,
              path: gatewayPath.trim() ? (gatewayPath.startsWith("/") ? gatewayPath.trim() : `/${gatewayPath.trim()}`) : null,
              strip_prefix: gatewayStripPrefix,
              container_name: effectiveName,
              group_name: null,
              enabled: true,
              auto_detected: true,
            });
            await api.saveGatewayRoutes(updated);
            setGatewayRoutes(updated);
          } catch (gErr) {
            console.warn("Failed to create gateway route:", gErr);
          }
        }

        onSuccess(newId);
      }
      onClose();
    } catch (err) {
      toast.error(
        isEdit ? "Failed to recreate container" : "Failed to run container",
        String(err)
      );
    } finally {
      setLoading(false);
    }
  };

  return (
    <AnimatePresence>
      <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
        {/* Backdrop */}
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          className="fixed inset-0 bg-black/70 backdrop-blur-md"
          onClick={onClose}
        />

        {/* Modal Window */}
        <motion.div
          initial={{ opacity: 0, scale: 0.95, y: 15 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.95, y: 15 }}
          className="relative w-full max-w-2xl bg-zinc-950 border border-zinc-800 rounded-2xl shadow-2xl overflow-hidden flex flex-col max-h-[90vh] z-10"
        >
          {/* Header */}
          <div className="px-6 py-4 border-b border-zinc-800 flex items-center justify-between">
            <div className="flex items-center gap-3">
              <div
                className={`w-8 h-8 rounded-lg flex items-center justify-center border ${
                  isEdit
                    ? "bg-amber-600/20 text-amber-400 border-amber-500/30"
                    : "bg-blue-600/20 text-blue-400 border-blue-500/30"
                }`}
              >
                {isEdit ? (
                  <RotateCw className="w-4 h-4" />
                ) : (
                  <Box className="w-4 h-4" />
                )}
              </div>
              <div>
                <h3 className="font-semibold text-white">
                  {isEdit
                    ? `Edit Container: ${editTarget.details.name.replace(
                        /^\//,
                        ""
                      )}`
                    : "Run New Container"}
                </h3>
                {isEdit && (
                  <p className="text-[11px] text-zinc-400">
                    Sửa đổi thông số và recreate container tự động mà không mất volume
                  </p>
                )}
              </div>
            </div>
            <Button variant="ghost" size="icon" onClick={onClose} title="Close">
              <X className="w-5 h-5" />
            </Button>
          </div>

          {/* Form */}
          <form
            onSubmit={handleSubmit}
            className="flex-1 overflow-y-auto p-6 space-y-6"
          >
            {/* Section 1: Standard & Resource Settings */}
            <div className="space-y-4">
              <div className="flex items-center justify-between">
                <h4 className="text-xs font-semibold text-zinc-300 uppercase tracking-wider flex items-center gap-2">
                  <span>General & Resources</span>
                </h4>
              </div>

              {/* Image & Name */}
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                <div>
                  <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                    Docker Image *
                  </label>
                  <input
                    type="text"
                    placeholder="e.g. nginx:latest, postgres:17"
                    value={image}
                    onChange={(e) => setImage(e.target.value)}
                    required
                    className="w-full h-10 px-3 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500 transition-colors"
                  />
                </div>

                <div>
                  <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                    Container Name
                  </label>
                  <input
                    type="text"
                    placeholder="Optional custom container name"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    className="w-full h-10 px-3 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500 transition-colors"
                  />
                </div>
              </div>

              {/* Resource Limits & Restart Policy */}
              <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
                <div>
                  <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                    Restart Policy
                  </label>
                  <select
                    value={restartPolicy}
                    onChange={(e) => setRestartPolicy(e.target.value)}
                    className="w-full h-10 px-3 rounded-xl bg-zinc-900 border border-zinc-800 text-xs text-white dark:text-white dark:bg-zinc-900 focus:outline-none focus:border-blue-500 cursor-pointer [&>option]:bg-zinc-900 [&>option]:text-white"
                  >
                    <option value="unless-stopped" className="bg-zinc-900 text-white">
                      Unless Stopped
                    </option>
                    <option value="always" className="bg-zinc-900 text-white">
                      Always
                    </option>
                    <option value="on-failure" className="bg-zinc-900 text-white">
                      On Failure
                    </option>
                    <option value="no" className="bg-zinc-900 text-white">
                      Never (No)
                    </option>
                  </select>
                </div>

                <div>
                  <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                    Memory Limit (MB)
                  </label>
                  <input
                    type="number"
                    placeholder="e.g. 512"
                    value={memoryMb}
                    onChange={(e) => setMemoryMb(e.target.value)}
                    className="w-full h-10 px-3 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500"
                  />
                </div>

                <div>
                  <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                    CPU Limit (Cores)
                  </label>
                  <input
                    type="text"
                    placeholder="e.g. 1.5"
                    value={cpus}
                    onChange={(e) => setCpus(e.target.value)}
                    className="w-full h-10 px-3 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500"
                  />
                </div>
              </div>

              {/* HTTPS Gateway Virtual Domain (Always accessible, no recreate required) */}
              <div className="p-4 rounded-xl border border-violet-500/30 bg-violet-500/5 space-y-3">
                <div className="flex items-center justify-between">
                  <label className="flex items-center gap-2 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={enableGateway}
                      onChange={(e) => {
                        setEnableGateway(e.target.checked);
                        if (e.target.checked && !gatewaySubdomain) {
                          const safe = (name || "app").toLowerCase().replace(/[^a-z0-9-]/g, "-");
                          setGatewaySubdomain(safe);
                        }
                      }}
                      className="rounded border-zinc-700 text-violet-600 focus:ring-violet-500 w-4 h-4"
                    />
                    <span className="text-xs font-semibold text-violet-300 uppercase tracking-wider flex items-center gap-1.5">
                      <Globe className="w-3.5 h-3.5 text-violet-400" />
                      HTTPS Gateway Route
                    </span>
                  </label>
                  <span className="text-[10px] font-mono text-violet-400 bg-violet-500/10 border border-violet-500/20 px-2 py-0.5 rounded">
                    Caddy Native
                  </span>
                </div>

                {enableGateway && (
                  <div className="space-y-3 pt-2 border-t border-violet-500/20">
                    <div className="space-y-1.5">
                      <label className="text-[11px] font-medium text-zinc-300 block">
                        Virtual Domain Name & Suffix
                      </label>
                      <div className="flex items-center gap-2">
                        <div className="flex-1 relative">
                          <input
                            type="text"
                            value={gatewaySubdomain}
                            onChange={(e) => setGatewaySubdomain(e.target.value)}
                            placeholder="my-service"
                            className="w-full h-8 px-2.5 rounded-lg bg-zinc-900 border border-zinc-700 text-xs text-white font-mono focus:border-violet-500 focus:outline-none"
                          />
                        </div>
                        <div className="flex items-center gap-1">
                          {[".localhost", ".local", ".internal", ".dev"].map((suff) => (
                            <button
                              key={suff}
                              type="button"
                              onClick={() => setGatewaySuffix(suff)}
                              className={`px-2 py-1 rounded text-xs font-mono transition-colors border ${
                                gatewaySuffix === suff
                                  ? "bg-violet-600/25 border-violet-500/60 text-violet-300 font-semibold"
                                  : "bg-zinc-900 border-zinc-800 text-zinc-400 hover:text-zinc-200"
                              }`}
                            >
                              {suff}
                            </button>
                          ))}
                          <button
                            type="button"
                            onClick={() => setGatewaySuffix("custom")}
                            className={`px-2 py-1 rounded text-xs font-mono transition-colors border ${
                              gatewaySuffix === "custom"
                                ? "bg-violet-600/25 border-violet-500/60 text-violet-300 font-semibold"
                                : "bg-zinc-900 border-zinc-800 text-zinc-400 hover:text-zinc-200"
                            }`}
                          >
                            custom
                          </button>
                        </div>
                      </div>

                      {gatewaySuffix === "custom" && (
                        <div className="flex items-center gap-2 pt-1">
                          <span className="text-[11px] text-zinc-400">Custom Suffix / TLD:</span>
                          <input
                            type="text"
                            placeholder=".example.com or .orb.local"
                            value={gatewayCustomSuffix}
                            onChange={(e) => setGatewayCustomSuffix(e.target.value)}
                            className="flex-1 h-7 px-2 rounded bg-zinc-900 border border-zinc-700 text-xs text-white font-mono focus:border-violet-500 focus:outline-none"
                          />
                        </div>
                      )}

                      <p className="text-[10px] text-zinc-500">
                        {gatewaySuffix === ".localhost"
                          ? "RFC 6761 .localhost resolves automatically in all browsers without editing hosts."
                          : "Non-.localhost domains will be automatically synchronized with /etc/hosts via Local DNS sync."}
                      </p>
                    </div>

                    <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                      <div>
                        <label className="text-[11px] font-medium text-zinc-300 block mb-1">
                          Path Prefix (Optional)
                        </label>
                        <input
                          type="text"
                          value={gatewayPath}
                          onChange={(e) => setGatewayPath(e.target.value)}
                          placeholder="/api"
                          className="w-full h-8 px-2.5 rounded-lg bg-zinc-900 border border-zinc-700 text-xs text-white font-mono focus:border-violet-500 focus:outline-none"
                        />
                      </div>

                      <div className="flex flex-col justify-end">
                        <label className="flex items-center gap-2 p-1.5 rounded-lg bg-zinc-900/60 border border-zinc-800 cursor-pointer text-xs text-zinc-300 h-8">
                          <input
                            type="checkbox"
                            checked={gatewayStripPrefix}
                            onChange={(e) => setGatewayStripPrefix(e.target.checked)}
                            className="rounded border-zinc-700 text-violet-600 focus:ring-violet-500 w-3.5 h-3.5"
                          />
                          <span className="text-[11px]">Strip prefix before upstream</span>
                        </label>
                      </div>
                    </div>

                    {effectiveGatewayDomain && (
                      <div className="p-2 rounded bg-violet-500/10 border border-violet-500/20 flex items-center justify-between text-[11px] font-mono text-violet-300">
                        <span>https://{effectiveGatewayDomain}{gatewayPath || ""} &rarr; 127.0.0.1:{ports[0]?.host_port || "8080"}</span>
                        <ShieldCheck className="w-3.5 h-3.5 text-emerald-400 shrink-0" />
                      </div>
                    )}
                  </div>
                )}
              </div>
            </div>

            {/* Section 2: Recreate Parameters (Ports, ENV, Volumes, Network, Command) */}
            <div className="pt-4 border-t border-zinc-800/80 space-y-4">
              <div
                className="flex items-center justify-between cursor-pointer select-none"
                onClick={() => setShowAdvanced(!showAdvanced)}
              >
                <div className="flex items-center gap-2">
                  <h4 className="text-xs font-semibold text-amber-300 uppercase tracking-wider flex items-center gap-2">
                    <span>{isEdit ? "Advanced Configuration (Requires Recreate)" : "Advanced Configuration"}</span>
                  </h4>
                  {isEdit && (
                    <span className="text-[10px] font-mono px-2 py-0.5 rounded-full bg-amber-500/10 text-amber-400 border border-amber-500/20">
                      recreate
                    </span>
                  )}
                </div>
                <button
                  type="button"
                  className="text-zinc-400 hover:text-white p-1 rounded-md"
                >
                  {showAdvanced ? (
                    <ChevronUp className="w-4 h-4" />
                  ) : (
                    <ChevronDown className="w-4 h-4" />
                  )}
                </button>
              </div>

              {showAdvanced && (
                <div className="space-y-6 pt-2">
                  {/* Informational note for edit & recreate */}
                  {isEdit && (
                    <p className="text-xs text-zinc-300 leading-relaxed">
                      Network mappings, environment variables, mounts, and command are fixed at container creation time. Saving changes here will automatically stop, remove, and recreate the container while preserving its name and volume data.
                    </p>
                  )}

                  {/* Port Forwarding */}
                  <div>
                    <div className="flex items-center justify-between mb-2">
                      <label className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">
                        Port Forwarding
                      </label>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={handleAddPort}
                        className="text-xs text-blue-400 gap-1 h-7"
                      >
                        <Plus className="w-3.5 h-3.5" />
                        Add Port
                      </Button>
                    </div>

                    <div className="space-y-2">
                      {ports.map((p, idx) => (
                        <div key={idx} className="flex items-center gap-3">
                          <input
                            type="text"
                            placeholder="Host IP"
                            title="Bind address. 127.0.0.1 keeps the port on this machine; 0.0.0.0 exposes it on every interface."
                            aria-label="Host IP to bind the port to"
                            value={p.host_ip ?? ""}
                            onChange={(e) => {
                              const copy = [...ports];
                              copy[idx].host_ip = e.target.value;
                              setPorts(copy);
                            }}
                            className="w-28 shrink-0 h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white font-mono focus:outline-none focus:border-blue-500"
                          />
                          <input
                            type="text"
                            placeholder="Host Port (e.g. 8080)"
                            value={p.host_port}
                            onChange={(e) => {
                              const copy = [...ports];
                              copy[idx].host_port = e.target.value;
                              setPorts(copy);
                            }}
                            className="flex-1 min-w-0 h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white focus:outline-none focus:border-blue-500"
                          />
                          <ArrowRight className="w-4 h-4 text-zinc-600 shrink-0" />
                          <input
                            type="text"
                            placeholder="Container Port (e.g. 80)"
                            value={p.container_port}
                            onChange={(e) => {
                              const copy = [...ports];
                              copy[idx].container_port = e.target.value;
                              setPorts(copy);
                            }}
                            className="flex-1 min-w-0 h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white focus:outline-none focus:border-blue-500"
                          />
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            onClick={() => handleRemovePort(idx)}
                            title="Remove port"
                            className="text-zinc-500 hover:text-rose-400 h-9 w-9 shrink-0"
                          >
                            <Trash2 className="w-4 h-4" />
                          </Button>
                        </div>
                      ))}
                      {ports.length === 0 && (
                        <span className="text-xs text-zinc-600 block italic">
                          No ports exposed
                        </span>
                      )}
                    </div>
                    <p className="text-[11px] leading-relaxed text-zinc-500 mt-2">
                      <span className="font-mono">127.0.0.1</span> keeps the port
                      on this machine. Use <span className="font-mono">0.0.0.0</span>{" "}
                      only when another device must reach it.
                    </p>
                  </div>



                  {/* Environment Variables */}
                  <div>
                    <div className="flex items-center justify-between mb-2">
                      <label className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">
                        Environment Variables
                      </label>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={handleAddEnv}
                        className="text-xs text-blue-400 gap-1 h-7"
                      >
                        <Plus className="w-3.5 h-3.5" />
                        Add Variable
                      </Button>
                    </div>

                    <div className="space-y-2">
                      {envVars.map((env, idx) => (
                        <div key={idx} className="flex items-center gap-3">
                          <input
                            type="text"
                            placeholder="KEY (e.g. POSTGRES_PASSWORD)"
                            value={env.key}
                            onChange={(e) => {
                              const copy = [...envVars];
                              copy[idx].key = e.target.value;
                              setEnvVars(copy);
                            }}
                            className="flex-1 h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white focus:outline-none focus:border-blue-500"
                          />
                          <span className="text-zinc-600">=</span>
                          <input
                            type="text"
                            placeholder="VALUE"
                            value={env.value}
                            onChange={(e) => {
                              const copy = [...envVars];
                              copy[idx].value = e.target.value;
                              setEnvVars(copy);
                            }}
                            className="flex-1 h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white focus:outline-none focus:border-blue-500"
                          />
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            onClick={() => handleRemoveEnv(idx)}
                            title="Remove env var"
                            className="text-zinc-500 hover:text-rose-400 h-9 w-9 shrink-0"
                          >
                            <Trash2 className="w-4 h-4" />
                          </Button>
                        </div>
                      ))}
                      {envVars.length === 0 && (
                        <span className="text-xs text-zinc-600 block italic">
                          No environment variables set
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Volume Mounts */}
                  <div>
                    <div className="flex items-center justify-between mb-2">
                      <label className="text-xs font-semibold text-zinc-400 uppercase tracking-wider">
                        Volume Mounts & Storage
                      </label>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={handleAddVolume}
                        className="text-xs text-blue-400 gap-1 h-7"
                      >
                        <Plus className="w-3.5 h-3.5" />
                        Add Mount
                      </Button>
                    </div>

                    <div className="space-y-2">
                      {volumes.map((vol, idx) => (
                        <div key={idx} className="flex items-center gap-3">
                          <select
                            value={vol.kind}
                            onChange={(e) => {
                              const copy = [...volumes];
                              copy[idx].kind = e.target.value as "bind" | "volume";
                              setVolumes(copy);
                            }}
                            className="w-24 shrink-0 h-9 px-2 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white focus:outline-none focus:border-blue-500"
                          >
                            <option value="bind">Host Path</option>
                            <option value="volume">Named Vol</option>
                          </select>
                          <input
                            type="text"
                            placeholder="Host path / Volume name"
                            value={vol.source}
                            onChange={(e) => {
                              const copy = [...volumes];
                              copy[idx].source = e.target.value;
                              setVolumes(copy);
                            }}
                            className="flex-1 min-w-0 h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white font-mono focus:outline-none focus:border-blue-500"
                          />
                          <ArrowRight className="w-4 h-4 text-zinc-600 shrink-0" />
                          <input
                            type="text"
                            placeholder="Container path (e.g. /app/data)"
                            value={vol.target}
                            onChange={(e) => {
                              const copy = [...volumes];
                              copy[idx].target = e.target.value;
                              setVolumes(copy);
                            }}
                            className="flex-1 min-w-0 h-9 px-3 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white font-mono focus:outline-none focus:border-blue-500"
                          />
                          <label className="flex items-center gap-1.5 text-xs text-zinc-400 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={vol.read_only}
                              onChange={(e) => {
                                const copy = [...volumes];
                                copy[idx].read_only = e.target.checked;
                                setVolumes(copy);
                              }}
                              className="rounded border-zinc-700 bg-zinc-900 text-blue-500 focus:ring-0"
                            />
                            <span className="text-[11px]">RO</span>
                          </label>
                          <Button
                            type="button"
                            variant="ghost"
                            size="icon"
                            onClick={() => handleRemoveVolume(idx)}
                            title="Remove mount"
                            className="text-zinc-500 hover:text-rose-400 h-9 w-9 shrink-0"
                          >
                            <Trash2 className="w-4 h-4" />
                          </Button>
                        </div>
                      ))}
                      {volumes.length === 0 && (
                        <span className="text-xs text-zinc-600 block italic">
                          No custom volumes mounted
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Command & Network */}
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                    <div>
                      <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                        Command Override
                      </label>
                      <input
                        type="text"
                        placeholder="e.g. sh -c 'sleep 1000'"
                        value={command}
                        onChange={(e) => setCommand(e.target.value)}
                        className="w-full h-10 px-3 rounded-xl bg-zinc-900 border border-zinc-800 text-xs text-white font-mono focus:outline-none focus:border-blue-500"
                      />
                    </div>
                    <div>
                      <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-1.5">
                        Network
                      </label>
                      <input
                        type="text"
                        placeholder="e.g. bridge, host, custom-net"
                        value={network}
                        onChange={(e) => setNetwork(e.target.value)}
                        className="w-full h-10 px-3 rounded-xl bg-zinc-900 border border-zinc-800 text-xs text-white focus:outline-none focus:border-blue-500"
                      />
                    </div>
                  </div>
                </div>
              )}
            </div>

            {/* Footer Buttons */}
            <div className="pt-4 border-t border-zinc-800 flex items-center justify-end gap-3">
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                isLoading={loading}
                className={
                  isEdit
                    ? hasAdvancedChanged
                      ? "bg-amber-600 hover:bg-amber-500 text-white"
                      : "bg-blue-600 hover:bg-blue-500 text-white"
                    : ""
                }
              >
                {isEdit
                  ? hasAdvancedChanged
                    ? "Recreate"
                    : "Save Changes"
                  : "Run Container"}
              </Button>
            </div>
          </form>
        </motion.div>
      </div>
    </AnimatePresence>
  );
};
