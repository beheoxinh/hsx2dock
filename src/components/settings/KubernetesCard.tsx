import React, { useEffect, useState } from "react";
import {
  DownloadCloud,
  Hexagon,
  Loader2,
  RefreshCw,
  Save,
} from "lucide-react";
import { Card } from "../ui/Card";
import { Button } from "../ui/Button";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";
import {
  UpdateProgressPanel,
  useUpdateRun,
} from "./UpdateProgressPanel";
import type { KindInstallStatus, KindUpdateInfo } from "../../types";

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
  const [autoStart, setAutoStart] = useState(false);
  const [startDelay, setStartDelay] = useState(5);
  const [kindUpdate, setKindUpdate] = useState<KindUpdateInfo | null>(null);
  const [checkingKind, setCheckingKind] = useState(false);
  const [updatingKind, setUpdatingKind] = useState(false);
  const { run: kindRun, reset: resetKindRun } = useUpdateRun("kind");

  useEffect(() => {
    api.kindStatus().then(setStatus).catch(() => {});
    // Background check: a failure simply means "no update known".
    api.checkKindUpdate().then(setKindUpdate).catch(() => {});
    api
      .getAppConfig()
      .then((cfg) => {
        setNodes(Math.min(5, Math.max(1, cfg.kind_default_nodes ?? 1)));
        setImage(cfg.kind_default_image ?? "");
        setAutoStart(cfg.kind_auto_start ?? false);
        setStartDelay(Math.min(120, Math.max(0, cfg.kind_auto_start_delay ?? 5)));
      })
      .catch(() => {});
  }, []);

  const checkForKindUpdate = async (announce?: boolean) => {
    setCheckingKind(true);
    try {
      const info = await api.checkKindUpdate();
      setKindUpdate(info);
      if (announce) {
        if (info.update_available) {
          toast.info(
            "kind update available",
            `${info.latest_version} ready to install`
          );
        } else {
          toast.success(
            "kind is up to date",
            `Installed: ${info.installed_version ?? "none"}`
          );
        }
      }
    } catch (e) {
      if (announce) toast.error("kind update check failed", String(e));
    } finally {
      setCheckingKind(false);
    }
  };

  const applyKindUpdate = async () => {
    setUpdatingKind(true);
    resetKindRun();
    try {
      if (kindUpdate?.latest_version) {
        await api.installKindUpdate(kindUpdate.latest_version);
      } else {
        await api.installKind();
      }
      toast.success(
        "kind updated",
        `kind ${kindUpdate?.latest_version ?? status?.latest ?? ""} is ready.`
      );
      setStatus(await api.kindStatus());
      await checkForKindUpdate();
    } catch (e) {
      toast.error("kind update failed", String(e));
    } finally {
      setUpdatingKind(false);
    }
  };

  const handleSave = async () => {
    try {
      setSaving(true);
      const latest = await api.getAppConfig();
      await api.saveAppConfig({
        ...latest,
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

      {/* Binary updates: install when missing, update when a newer release
          exists. Progress and log come from the unified update channel. */}
      <div className="flex items-center justify-between gap-3 p-3 rounded-lg bg-zinc-950 border border-zinc-800">
        <div className="min-w-0">
          <p className="text-xs font-medium text-zinc-200">kind binary</p>
          <p className="text-[11px] text-zinc-500 mt-0.5">
            {!status?.installed
              ? "Not installed"
              : kindUpdate
                ? kindUpdate.update_available
                  ? `Update ${kindUpdate.installed_version ?? "unknown"} → ${kindUpdate.latest_version}`
                  : `Up to date · ${kindUpdate.installed_version ?? kindUpdate.latest_version}`
                : "Not checked yet"}
          </p>
        </div>
        <div className="flex items-center gap-2 shrink-0">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={checkingKind || updatingKind}
            onClick={() => checkForKindUpdate(true)}
          >
            <RefreshCw
              className={`w-4 h-4 ${checkingKind ? "animate-spin" : ""}`}
            />
            <span>Check</span>
          </Button>
          {(!status?.installed || kindUpdate?.update_available) && (
            <Button
              type="button"
              variant="primary"
              size="sm"
              disabled={updatingKind || status?.supported === false}
              onClick={applyKindUpdate}
            >
              <DownloadCloud className="w-4 h-4" />
              <span>
                {updatingKind
                  ? "Updating…"
                  : status?.installed
                    ? "Update kind"
                    : "Install kind"}
              </span>
            </Button>
          )}
        </div>
      </div>

      {kindRun && (
        <UpdateProgressPanel run={kindRun} onDismiss={resetKindRun} />
      )}

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
    </Card>
  );
};
