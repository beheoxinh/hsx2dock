import React, { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CircleDashed,
  Loader2,
  RefreshCw,
  Stethoscope,
  Wrench,
  XCircle,
} from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import type {
  ContainerdPlugin,
  EngineCheck,
  EngineDoctorReport,
  EngineHealth,
} from "../../types";

/** One colour language for checks and plugins, so the two lists read alike. */
const TONE: Record<
  EngineHealth,
  { border: string; bg: string; text: string; label: string }
> = {
  ok: {
    border: "border-emerald-500/30",
    bg: "bg-emerald-500/5",
    text: "text-emerald-400",
    label: "Healthy",
  },
  warning: {
    border: "border-amber-500/30",
    bg: "bg-amber-500/5",
    text: "text-amber-400",
    label: "Warning",
  },
  error: {
    border: "border-rose-500/30",
    bg: "bg-rose-500/5",
    text: "text-rose-400",
    label: "Broken",
  },
  missing: {
    border: "border-zinc-700/60",
    bg: "bg-zinc-900/40",
    text: "text-zinc-400",
    label: "Not set up",
  },
};

const HealthIcon: React.FC<{ health: EngineHealth; className?: string }> = ({
  health,
  className = "w-4 h-4",
}) => {
  switch (health) {
    case "ok":
      return <CheckCircle2 className={`${className} text-emerald-400`} />;
    case "warning":
      return <AlertTriangle className={`${className} text-amber-400`} />;
    case "error":
      return <XCircle className={`${className} text-rose-400`} />;
    default:
      return <CircleDashed className={`${className} text-zinc-500`} />;
  }
};

/** A plugin row — containerd logs these one per line at startup. */
const PluginRow: React.FC<{ plugin: ContainerdPlugin }> = ({ plugin }) => {
  const tone = TONE[plugin.health];
  // `io.containerd.grpc.v1.leases` → `grpc.v1.leases`; the prefix is noise.
  const shortId = plugin.id.replace(/^io\.containerd\./, "");
  return (
    <div className="flex items-start gap-2 py-1.5">
      <HealthIcon health={plugin.health} className="w-3.5 h-3.5 mt-0.5 shrink-0" />
      <div className="min-w-0 flex-1">
        <div className="font-mono text-[11px] text-zinc-300 truncate" title={plugin.id}>
          {shortId}
        </div>
        {plugin.health !== "ok" && (
          <div className={`text-[11px] ${tone.text} break-words`}>{plugin.detail}</div>
        )}
      </div>
      <span className={`text-[10px] uppercase font-semibold ${tone.text} shrink-0`}>
        {tone.label}
      </span>
    </div>
  );
};

/** States where a repair is worth offering: broken or misconfigured. */
const needsFixing = (h: EngineHealth) => h === "error" || h === "warning";

const CheckRow: React.FC<{
  check: EngineCheck;
  /** Any repair is in flight — every Repair button is disabled meanwhile. */
  busy: boolean;
  /** This row's own repair is the one running. */
  running: boolean;
  onRepair: (id: string) => void;
}> = ({ check, busy, running, onRepair }) => {
  const tone = TONE[check.health];
  return (
    <div className={`rounded-lg border px-3 py-2.5 ${tone.border} ${tone.bg}`}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-start gap-2 min-w-0">
          <HealthIcon health={check.health} className="w-4 h-4 mt-0.5 shrink-0" />
          <div className="min-w-0">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-sm font-medium text-white">{check.label}</span>
              <span className={`text-[10px] uppercase font-semibold ${tone.text}`}>
                {tone.label}
              </span>
              {check.auto && (
                <span
                  className="text-[10px] uppercase font-semibold text-cyan-400/80"
                  title="Applied automatically before every engine start"
                >
                  auto
                </span>
              )}
            </div>
            <p className="text-xs text-zinc-400 mt-0.5 break-words">{check.detail}</p>
            {check.path && (
              <p
                className="font-mono text-[10px] text-zinc-500 mt-1 truncate"
                title={check.path}
              >
                {check.path}
              </p>
            )}
          </div>
        </div>

        {check.repairable && needsFixing(check.health) && (
          <Button
            variant="secondary"
            size="sm"
            disabled={busy}
            onClick={() => onRepair(check.id)}
            className="shrink-0 whitespace-nowrap"
            title={`Repair: ${check.label}`}
          >
            {running ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Wrench className="w-3.5 h-3.5" />
            )}
            <span>Repair</span>
          </Button>
        )}
      </div>
    </div>
  );
};

/**
 * Engine health & self-repair.
 *
 * Docker's failures are opaque — a dangling snapshotter link surfaces as
 * "unknown service containerd.services.leases.v1.Leases" with no hint about the
 * cause. This card runs the same checks the engine runs on itself before every
 * start, shows every containerd plugin's load state, and lets the user apply
 * the repairs by hand. It is the visible half of `docker/doctor.rs`; the
 * invisible half runs automatically on every engine start.
 */
export const EngineHealthCard: React.FC = () => {
  const toast = useToast();
  const [report, setReport] = useState<EngineDoctorReport | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  /**
   * A remote endpoint is active: the checks read the *bundled* engine's data
   * root, so they would describe the wrong machine. The backend refuses rather
   * than mislead — this flag turns that refusal into a designed state.
   */
  const [remoteActive, setRemoteActive] = useState(false);
  const [repairing, setRepairing] = useState<string | null>(null);
  const [repairingAll, setRepairingAll] = useState(false);
  const [showPlugins, setShowPlugins] = useState(false);

  const load = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      setRemoteActive(false);
      setReport(await api.diagnoseEngine());
    } catch (e) {
      const message = String(e);
      // The guard's wording, not an exception: show the designed state.
      if (/Engine controls are unavailable/i.test(message)) {
        setRemoteActive(true);
        setReport(null);
      } else {
        setError(message);
      }
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const runRepair = async (ids?: string[]) => {
    const all = !ids || ids.length === 0;
    if (all) setRepairingAll(true);
    else setRepairing(ids[0]);
    try {
      const result = await api.repairEngine(ids);
      const fixed = result.repaired.length;
      const failed = result.failed.length;
      if (failed === 0 && fixed > 0) {
        toast.success(
          "Engine repaired",
          `${fixed} check${fixed === 1 ? "" : "s"} fixed. Restart the engine to pick up the change.`
        );
      } else if (failed > 0) {
        toast.error(
          "Repair incomplete",
          result.failed.map(([id, why]) => `${id}: ${why}`).join("\n")
        );
      } else {
        toast.info("Nothing to repair", "No check needed a fix.");
      }
      await load();
    } catch (e) {
      toast.error("Repair failed", String(e));
    } finally {
      setRepairing(null);
      setRepairingAll(false);
    }
  };

  const repairable = report?.checks.filter((c) => c.repairable) ?? [];
  const needsRepair = repairable.filter((c) => needsFixing(c.health));
  const failingPlugins = report?.plugins.filter((p) => needsFixing(p.health)) ?? [];
  const tone = TONE[report?.health ?? "missing"];
  const busy = repairingAll || repairing !== null;

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-3">
        <div className="min-w-0 flex-1">
          <h4 className="text-sm font-semibold text-white flex items-center gap-2">
            <Stethoscope className="w-4 h-4 text-cyan-400 shrink-0" />
            Engine Health &amp; Self-Repair
          </h4>
          <p className="text-xs text-zinc-500 mt-1">
            Checks the engine's install, data root, snapshotter links and containerd
            plugins. Repairs never delete images or containers, and the same safe
            fixes run automatically before every engine start.
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            onClick={load}
            disabled={loading || busy}
            title="Run the checks again"
            className="whitespace-nowrap"
          >
            <RefreshCw className={`w-4 h-4 ${loading ? "animate-spin" : ""}`} />
            <span>Re-check</span>
          </Button>
          {needsRepair.length > 0 && (
            <Button
              variant="primary"
              size="sm"
              onClick={() => runRepair(needsRepair.map((c) => c.id))}
              disabled={busy}
              className="whitespace-nowrap"
              title="Repair every failing check"
            >
              {repairingAll ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <Wrench className="w-4 h-4" />
              )}
              <span>Repair {needsRepair.length}</span>
            </Button>
          )}
        </div>
      </div>

      {remoteActive && (
        <div className="rounded-lg border border-zinc-700/60 bg-zinc-900/40 px-3 py-2 text-xs text-zinc-400">
          A remote endpoint is active, so the bundled engine is not the one in use.
          Switch back to the bundled engine in <span className="text-zinc-300">Endpoints</span>{" "}
          above to diagnose and repair it.
        </div>
      )}

      {error && (
        <div className="rounded-lg border border-rose-500/30 bg-rose-500/5 px-3 py-2 text-xs text-rose-300 break-words">
          Could not run the engine checks: {error}
        </div>
      )}

      {loading && !report ? (
        <div className="flex items-center gap-2 py-3 text-xs text-zinc-500">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Inspecting the engine…
        </div>
      ) : remoteActive ? null : report ? (
        <>
          {/* Summary strip: overall state, data root, engine reachability. */}
          <div
            className={`flex flex-wrap items-center gap-x-4 gap-y-1 rounded-lg border px-3 py-2 text-xs ${tone.border} ${tone.bg}`}
          >
            <span className={`flex items-center gap-1.5 font-medium ${tone.text}`}>
              <HealthIcon health={report.health} className="w-4 h-4" />
              {report.health === "ok" ? "All checks passed" : tone.label}
            </span>
            <span className="text-zinc-500">
              Engine{" "}
              <span className={report.engine_running ? "text-emerald-400" : "text-zinc-400"}>
                {report.engine_running ? "running" : "stopped"}
              </span>
            </span>
            <span className="font-mono text-[11px] text-zinc-500 truncate max-w-full" title={report.data_root}>
              data root: {report.data_root}
            </span>
          </div>

          <div className="space-y-2">
            {report.checks.map((check) => (
              <CheckRow
                key={check.id}
                check={check}
                busy={busy}
                running={repairing === check.id || repairingAll}
                onRepair={(id) => runRepair([id])}
              />
            ))}
          </div>

          {/* Containerd plugins — collapsed by default; the list is long. */}
          <div className="rounded-lg border border-zinc-800 bg-zinc-900/30">
            <button
              type="button"
              onClick={() => setShowPlugins((v) => !v)}
              className="flex w-full items-center justify-between gap-3 px-3 py-2.5 text-left"
              aria-expanded={showPlugins}
            >
              <span className="flex items-center gap-2 text-sm font-medium text-white">
                {showPlugins ? (
                  <ChevronDown className="w-4 h-4 text-zinc-400" />
                ) : (
                  <ChevronRight className="w-4 h-4 text-zinc-400" />
                )}
                Containerd plugins
                <span className="text-xs font-normal text-zinc-500">
                  {report.plugins.length === 0
                    ? "no plugin data yet"
                    : `${report.plugins.length} loaded${
                        failingPlugins.length > 0
                          ? `, ${failingPlugins.length} failing`
                          : ""
                      }`}
                </span>
              </span>
              {failingPlugins.length > 0 && (
                <span className="flex items-center gap-1 text-[11px] text-amber-400 shrink-0">
                  <AlertTriangle className="w-3.5 h-3.5" />
                  {failingPlugins.length} failing
                </span>
              )}
            </button>

            {showPlugins && (
              <div className="border-t border-zinc-800 px-3 py-2">
                {report.plugins.length === 0 ? (
                  <p className="py-2 text-xs text-zinc-500">
                    The engine has not logged its plugin startup yet. Start the engine
                    and re-check.
                  </p>
                ) : (
                  <div className="divide-y divide-zinc-800/60">
                    {report.plugins.map((p) => (
                      <PluginRow key={p.id} plugin={p} />
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        </>
      ) : null}
    </div>
  );
};
