import React, { useEffect, useState } from "react";
import { Hexagon, Loader2, Save } from "lucide-react";
import { Card } from "../ui/Card";
import { Button } from "../ui/Button";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";
import type { KindInstallStatus } from "../../types";

/**
 * kind (Kubernetes) preferences: the node count and node image pre-filled in
 * the Create-cluster form. Saves on explicit Save (text fields), always via a
 * fresh config read so other Settings fields are never clobbered.
 */
export const KubernetesCard: React.FC = () => {
  const toast = useToast();
  const [status, setStatus] = useState<KindInstallStatus | null>(null);
  const [nodes, setNodes] = useState(1);
  const [image, setImage] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    api.kindStatus().then(setStatus).catch(() => {});
    api
      .getAppConfig()
      .then((cfg) => {
        setNodes(Math.min(5, Math.max(1, cfg.kind_default_nodes ?? 1)));
        setImage(cfg.kind_default_image ?? "");
      })
      .catch(() => {});
  }, []);

  const handleSave = async () => {
    try {
      setSaving(true);
      const latest = await api.getAppConfig();
      await api.saveAppConfig({
        ...latest,
        kind_default_nodes: nodes,
        kind_default_image: image.trim(),
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
                : "kind binary not installed — install it from the Kubernetes page."}
          </p>
        </div>
      </div>

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
    </Card>
  );
};
