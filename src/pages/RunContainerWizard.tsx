import React, { useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { X, Plus, Trash2, Box, ArrowRight } from "lucide-react";
import { Button } from "../components/ui/Button";
import { useToast } from "../components/ui/Toast";
import { api } from "../lib/api";
import type { RunConfig, PortMapping, EnvVar } from "../types";

interface RunContainerWizardProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess: () => void;
}

export const RunContainerWizard: React.FC<RunContainerWizardProps> = ({
  isOpen,
  onClose,
  onSuccess,
}) => {
  const toast = useToast();
  const [image, setImage] = useState("");
  const [name, setName] = useState("");
  const [restartPolicy, setRestartPolicy] = useState("unless-stopped");
  const [ports, setPorts] = useState<PortMapping[]>([
    { host_ip: "127.0.0.1", host_port: "8080", container_port: "80" },
  ]);
  const [envVars, setEnvVars] = useState<EnvVar[]>([]);
  const [memoryMb, setMemoryMb] = useState("");
  const [cpus, setCpus] = useState("");
  const [loading, setLoading] = useState(false);

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

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!image.trim()) {
      toast.error("Validation Error", "Image name is required");
      return;
    }

    try {
      setLoading(true);
      const config: RunConfig = {
        image: image.trim(),
        name: name.trim() || undefined,
        restart_policy: restartPolicy,
        ports: ports.filter((p) => p.container_port.trim() !== ""),
        env: envVars.filter((e) => e.key.trim() !== ""),
        resources: {
          memory_mb: memoryMb.trim() || undefined,
          cpus: cpus.trim() || undefined,
        },
      };

      await api.runContainer(config);
      onSuccess();
      onClose();
    } catch (err) {
      toast.error("Failed to run container", String(err));
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
              <div className="w-8 h-8 rounded-lg bg-blue-600/20 text-blue-400 flex items-center justify-center border border-blue-500/30">
                <Box className="w-4 h-4" />
              </div>
              <h3 className="font-semibold text-white">Run New Container</h3>
            </div>
            <Button variant="ghost" size="icon" onClick={onClose} title="Close">
              <X className="w-5 h-5" />
            </Button>
          </div>

          {/* Form */}
          <form onSubmit={handleSubmit} className="flex-1 overflow-y-auto p-6 space-y-6">
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
              </div>
              <p className="text-[11px] leading-relaxed text-zinc-500 mt-2">
                <span className="font-mono">127.0.0.1</span> keeps the port on
                this machine. Use <span className="font-mono">0.0.0.0</span>{" "}
                only when another device must reach it — Docker publishes those
                ports around the host firewall.
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
                  <span className="text-xs text-zinc-600 block italic">No environment variables set</span>
                )}
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
                  <option value="unless-stopped" className="bg-zinc-900 text-white">Unless Stopped</option>
                  <option value="always" className="bg-zinc-900 text-white">Always</option>
                  <option value="on-failure" className="bg-zinc-900 text-white">On Failure</option>
                  <option value="no" className="bg-zinc-900 text-white">Never (No)</option>
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

            {/* Footer Buttons */}
            <div className="pt-4 border-t border-zinc-800 flex items-center justify-end gap-3">
              <Button type="button" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" isLoading={loading}>
                Run Container
              </Button>
            </div>
          </form>
        </motion.div>
      </div>
    </AnimatePresence>
  );
};
