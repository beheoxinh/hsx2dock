import React, { useState, useEffect } from "react";
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

  // UI state
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [loading, setLoading] = useState(false);

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
            host_ip: p.host_ip || "127.0.0.1",
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
      setCommand(d.command || (d.command_vec ? d.command_vec.join(" ") : ""));
      setNetwork(d.network || "");

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

      const parsedCommand = command.trim()
        ? command.trim().split(/\s+/)
        : undefined;

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

      if (isEdit && editTarget) {
        // Recreate the container
        const newId = await api.recreateContainer(editTarget.id, config);
        toast.success(
          "Container Recreated",
          `Container ${name || editTarget.details.name} updated and started.`
        );
        onSuccess(newId);
      } else {
        // Run fresh container
        const newId = await api.runContainer(config);
        toast.success(
          "Container Launched",
          `New container instance created: ${name || newId.slice(0, 12)}`
        );
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
                variant={isEdit ? "primary" : "primary"}
                isLoading={loading}
                className={isEdit ? "bg-amber-600 hover:bg-amber-500 text-white" : ""}
              >
                {isEdit ? "Save & Recreate Container" : "Run Container"}
              </Button>
            </div>
          </form>
        </motion.div>
      </div>
    </AnimatePresence>
  );
};
