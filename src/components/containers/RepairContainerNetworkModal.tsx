import React, { useState, useEffect } from "react";
import { AlertTriangle, Wrench, RefreshCw, Network } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import type { ContainerItem, NetworkItem } from "../../types";

interface RepairContainerNetworkModalProps {
  isOpen: boolean;
  container: ContainerItem | null;
  onClose: () => void;
  onSuccess: () => void;
}

export const RepairContainerNetworkModal: React.FC<RepairContainerNetworkModalProps> = ({
  isOpen,
  container,
  onClose,
  onSuccess,
}) => {
  const toast = useToast();
  const [loading, setLoading] = useState(false);
  const [availableNetworks, setAvailableNetworks] = useState<NetworkItem[]>([]);
  const [selectedOption, setSelectedOption] = useState<"recreate" | "switch">("recreate");
  const [targetNetwork, setTargetNetwork] = useState("");

  const brokenNet = container?.broken_networks?.[0] || container?.networks?.[0] || "docker-network";

  useEffect(() => {
    if (!isOpen || !container) return;

    api
      .listNetworks()
      .then((nets) => {
        setAvailableNetworks(nets);
        // Default target network to bridge or first available network that isn't broken
        const validNets = nets.filter((n) => n.name !== "none" && n.name !== "host");
        const defaultChoice = validNets.find((n) => n.name === "bridge") || validNets[0];
        if (defaultChoice) {
          setTargetNetwork(defaultChoice.name);
        }
      })
      .catch((err) => console.error("Failed to load networks", err));

    setSelectedOption("recreate");
  }, [isOpen, container]);

  if (!isOpen || !container) return null;

  const containerName = container.name.replace(/^\//, "");

  const handleRepair = async () => {
    try {
      setLoading(true);
      const isRecreate = selectedOption === "recreate";
      const finalTargetNet = isRecreate ? brokenNet : targetNetwork;

      if (!finalTargetNet) {
        toast.error("Vui lòng chọn hoặc nhập tên network!");
        return;
      }

      await api.repairContainerNetwork(
        container.id,
        brokenNet,
        finalTargetNet,
        isRecreate
      );

      toast.success(
        "Đã sửa mạng thành công",
        `Container "${containerName}" đã được kết nối lại mạng "${finalTargetNet}".`
      );
      onSuccess();
      onClose();
    } catch (err) {
      toast.error("Không thể sửa mạng", String(err));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm animate-in fade-in duration-150">
      <div
        className="w-full max-w-md rounded-2xl bg-zinc-900 border border-zinc-800 p-6 shadow-2xl space-y-5 text-white"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-start gap-3">
          <div className="p-2.5 rounded-xl bg-amber-500/10 border border-amber-500/20 text-amber-400 shrink-0">
            <AlertTriangle className="w-5 h-5" />
          </div>
          <div className="space-y-1">
            <h3 className="text-base font-semibold text-white">
              Sửa kết nối mạng cho container
            </h3>
            <p className="text-xs text-zinc-400">
              Container <span className="text-zinc-200 font-medium font-mono">{containerName}</span> đang bị mất network{" "}
              <span className="text-amber-300 font-mono font-medium">"{brokenNet}"</span> (ID mạng cũ không còn tồn tại trên Docker).
            </p>
          </div>
        </div>

        {/* Options */}
        <div className="space-y-3">
          {/* Option 1: Recreate / Reattach to same network name */}
          <label
            className={`flex items-start gap-3 p-3.5 rounded-xl border cursor-pointer transition-all ${
              selectedOption === "recreate"
                ? "bg-indigo-500/10 border-indigo-500/50 text-white"
                : "bg-zinc-950/40 border-zinc-800 text-zinc-400 hover:border-zinc-700"
            }`}
          >
            <input
              type="radio"
              name="repair_option"
              value="recreate"
              checked={selectedOption === "recreate"}
              onChange={() => setSelectedOption("recreate")}
              className="mt-1 accent-indigo-500"
            />
            <div className="space-y-1 text-xs">
              <div className="font-semibold text-zinc-200 flex items-center gap-1.5">
                <RefreshCw className="w-3.5 h-3.5 text-indigo-400" />
                <span>Khôi phục / Gán lại vào mạng "{brokenNet}"</span>
              </div>
              <p className="text-zinc-400 text-[11px] leading-relaxed">
                Tự động tạo lại mạng <code className="text-indigo-300">{brokenNet}</code> nếu chưa có và gán lại container vào đúng ID mạng hiện tại.
              </p>
            </div>
          </label>

          {/* Option 2: Switch to another existing network */}
          <label
            className={`flex items-start gap-3 p-3.5 rounded-xl border cursor-pointer transition-all ${
              selectedOption === "switch"
                ? "bg-indigo-500/10 border-indigo-500/50 text-white"
                : "bg-zinc-950/40 border-zinc-800 text-zinc-400 hover:border-zinc-700"
            }`}
          >
            <input
              type="radio"
              name="repair_option"
              value="switch"
              checked={selectedOption === "switch"}
              onChange={() => setSelectedOption("switch")}
              className="mt-1 accent-indigo-500"
            />
            <div className="space-y-2 text-xs flex-1">
              <div className="font-semibold text-zinc-200 flex items-center gap-1.5">
                <Network className="w-3.5 h-3.5 text-emerald-400" />
                <span>Chuyển sang một mạng Docker khác</span>
              </div>
              <p className="text-zinc-400 text-[11px] leading-relaxed">
                Ngắt kết nối mạng cũ đã hỏng và chuyển container sang một network đang hoạt động.
              </p>

              {selectedOption === "switch" && (
                <div className="pt-1">
                  <select
                    value={targetNetwork}
                    onChange={(e) => setTargetNetwork(e.target.value)}
                    className="w-full bg-zinc-900 border border-zinc-700 rounded-lg px-3 py-1.5 text-xs text-white focus:outline-none focus:border-indigo-500"
                  >
                    {availableNetworks
                      .filter((n) => n.name !== "none")
                      .map((n) => (
                        <option key={n.id} value={n.name}>
                          {n.name} ({n.driver})
                        </option>
                      ))}
                  </select>
                </div>
              )}
            </div>
          </label>
        </div>

        {/* Action Buttons */}
        <div className="flex items-center justify-end gap-2.5 pt-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={onClose}
            disabled={loading}
          >
            Hủy
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={handleRepair}
            disabled={loading}
            className="gap-1.5 bg-amber-600 hover:bg-amber-500 text-white border-none shadow-lg shadow-amber-950/40"
          >
            {loading ? (
              <RefreshCw className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Wrench className="w-3.5 h-3.5" />
            )}
            <span>Xác nhận sửa mạng</span>
          </Button>
        </div>
      </div>
    </div>
  );
};
