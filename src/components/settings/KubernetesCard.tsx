import React, { useEffect, useState } from "react";
import { Hexagon, Loader2, Save } from "lucide-react";
import { Card } from "../ui/Card";
import { Button } from "../ui/Button";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";
import { BinaryUpdatePanel } from "./BinaryUpdatePanel";
import { onBinaryInstalled } from "../../lib/binaryUpdates";
import { KindOrphanCleanup } from "./KindOrphanCleanup";
import type { KindInstallStatus } from "../../types";

/**
 * kind (Kubernetes) preferences: the node count and node image pre-filled in
 * the Create-cluster form. Saves on explicit Save (text fields), always via a
 * fresh config read so other Settings fields are never clobbered.
 */
export const KubernetesCard: React.FC<{ visible?: boolean }> = ({ visible = true }) => {
  const toast = useToast();
  const [status, setStatus] = useState<KindInstallStatus | null>(null);
  const [nodes, setNodes] = useState(1);
  const [image, setImage] = useState("");
  const [saving, setSaving] = useState(false);
  const [autoStart, setAutoStart] = useState(false);
  const [startDelay, setStartDelay] = useState(5);

  useEffect(() => {
    api.kindStatus().then(setStatus).catch(() => {});
    api
      .getAppConfig()
      .then((cfg) => {
        setNodes(Math.min(5, Math.max(1, cfg.kind_default_nodes ?? 1)));
        setImage(cfg.kind_default_image ?? "");
        setAutoStart(cfg.kind_auto_start ?? false);
        setStartDelay(Math.min(120, Math.max(0, cfg.kind_auto_start_delay ?? 5)));
      })
      .catch(() => {});
    return onBinaryInstalled((target) => {
      if (target === "kind") api.kindStatus().then(setStatus).catch(() => {});
    });
  }, []);

  const handleSave = async () => {
    try {
      setSaving(true);
      await api.patchAppConfig({
        kind_default_nodes: nodes,
        kind_default_image: image.trim(),
        kind_auto_start: autoStart,
        kind_auto_start_delay: startDelay,
      });
      toast.success("Kubernetes defaults saved", "Used by the Create-cluster form.");
    } catch (e) {
      toast.error("Could not save Kubernetes settings", String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Card className="space-y-4">
      <div className="flex items-start gap-3">
        <div className="w-8 h-8 rounded-lg bg-blue-500/10 border border-blue-500/30 flex items-center justify-center shrink-0">
          <Hexagon className="w-4 h-4 text-blue-400" />
        </div>
        <div className="min-w-0">
          <h3 className="font-semibold text-white">Kubernetes (kind)</h3>
          <p className="text-xs text-zinc-400 mt-1">
            {status?.installed
              ? `kind v${status.version ?? status.latest} installed — ${status.path}`
              : status && !status.supported
                ? "kind has no published binary for this platform."
                : "kind binary not installed — install it below."}
          </p>
        </div>
      </div>

      <BinaryUpdatePanel
        target="kind"
        name="kind"
        icon={Hexagon}
        iconClass="text-blue-400"
        autoCheck={visible}
        description={status?.path ?? "Runs local Kubernetes clusters in Docker containers."}
        blockedReason={status && !status.supported ? "kind publishes no binary for this platform." : null}
      />

      <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
        <div>
          <label className="block text-xs font-medium text-zinc-400 mb-1.5">
            Default nodes
          </label>
          <input
            type="number"
            min={1}
            max={5}
            value={nodes}
            onChange={(e) =>
              setNodes(Math.min(5, Math.max(1, Number(e.target.value) || 1)))
            }
            className="w-full h-9 px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500"
          />
        </div>
        <div className="sm:col-span-2">
          <label className="block text-xs font-medium text-zinc-400 mb-1.5">
            Default node image{" "}
            <span className="text-zinc-600 font-normal">(optional — blank = kind's own)</span>
          </label>
          <input
            type="text"
            value={image}
            onChange={(e) => setImage(e.target.value)}
            placeholder="kindest/node:…"
            className="w-full h-9 px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white placeholder-zinc-600 focus:outline-none focus:border-blue-500"
          />
        </div>
      </div>

      <div className="border-t border-white/5 pt-4 space-y-3">
        <label className="flex items-start gap-2 text-sm text-zinc-300 cursor-pointer">
          <input
            type="checkbox"
            checked={autoStart}
            onChange={(e) => setAutoStart(e.target.checked)}
            className="mt-0.5 rounded border-zinc-700 bg-zinc-950"
          />
          <span>Auto-start clusters when the engine comes up</span>
        </label>
        <p className="text-xs text-zinc-500 leading-relaxed">
          Only clusters with their card's Auto-start switch on are started, one after
          another with the delay below — so the host CPU/RAM is never slammed by every
          cluster booting at once. The master switch lives here and on the Kubernetes
          page.
        </p>
        {autoStart && (
          <div className="max-w-[220px]">
            <label className="block text-xs font-medium text-zinc-400 mb-1.5">
              Delay between starts (seconds)
            </label>
            <input
              type="number"
              min={0}
              max={120}
              value={startDelay}
              onChange={(e) =>
                setStartDelay(Math.min(120, Math.max(0, Number(e.target.value) || 0)))
              }
              className="w-full h-9 px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500"
            />
          </div>
        )}
      </div>

      <div className="flex justify-end">
        <Button
          variant="primary"
          size="sm"
          onClick={handleSave}
          disabled={saving}
          className="gap-2"
        >
          {saving ? <Loader2 className="w-4 h-4 animate-spin" /> : <Save className="w-4 h-4" />}
          <span>{saving ? "Saving…" : "Save defaults"}</span>
        </Button>
      </div>

      <KindOrphanCleanup visible={visible} />
    </Card>
  );
};
