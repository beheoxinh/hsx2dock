import React, { useEffect, useState } from "react";
import { Terminal, Power, Save, FileCode2, Check, HardDrive, Palette, FolderEdit, AlertTriangle, ArrowRight, X, RefreshCw, DownloadCloud, PackageCheck } from "lucide-react";
import { Card } from "../components/ui/Card";
import { useToast } from "../components/ui/Toast";
import { Button } from "../components/ui/Button";
import { EndpointsCard } from "../components/settings/EndpointsCard";
import { RegistriesCard } from "../components/settings/RegistriesCard";
import { NotificationsCard } from "../components/settings/NotificationsCard";
import { KubernetesCard } from "../components/settings/KubernetesCard";
import { api, events } from "../lib/api";
import { invalidateTerminalSettings } from "../lib/terminalSettings";
import type {
  AppConfig,
  CliBridgeStatus,
  EngineUpdateInfo,
  EngineInstallProgress,
  CliResolution,
  CliShimStatus,
  ComposeUpdateInfo,
} from "../types";

const humanError = (e: unknown): string => {
  const s = String(e);
  if (/403|forbidden|rate/i.test(s) && /github/i.test(s)) {
    return "GitHub refused the request (rate limited). Try again later.";
  }
  if (/429/i.test(s)) return "GitHub rate limited. Try again later.";
  if (/timeout|timed out/i.test(s)) return "Request timed out. Check your connection.";
  if (/network|connect|dns/i.test(s)) return "Network error. Check your connection.";
  return s;
};

export const SettingsPage: React.FC = () => {
  const toast = useToast();
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [cliStatus, setCliStatus] = useState<CliBridgeStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  // Modal Migration State
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [newPathInput, setNewPathInput] = useState("");
  const [step, setStep] = useState<"input" | "ask_move" | "ask_clear" | "migrating">("input");
  const [pathInspection, setPathInspection] = useState<{
    exists: boolean;
    is_dir: boolean;
    is_empty: boolean;
    current_data_root: string;
    fs_error?: string | null;
  } | null>(null);
  const [shouldMoveData, setShouldMoveData] = useState(false);
  const [, setShouldClearOld] = useState(false);
  const [migrationError, setMigrationError] = useState("");

  const loadSettings = async () => {
    try {
      const [cfg, cli] = await Promise.all([
        api.getAppConfig(),
        api.getCliBridgeStatus(),
      ]);
      setConfig(cfg);
      setCliStatus(cli);
    } catch (e) {
      console.error(e);
    }
  };

  // Engine update: what the official channel reports, plus in-flight state.
  const [release, setRelease] = useState<EngineUpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<EngineInstallProgress | null>(null);

  const checkForEngineUpdate = async (announce?: boolean) => {
    setChecking(true);
    try {
      const info = await api.checkEngineUpdate();
      setRelease(info);
      if (announce && !info.update_available) {
        toast.success(
          "Engine is up to date",
          `Installed ${info.installed_version ?? "nothing"} · channel reports ${info.latest_version}`
        );
      }
    } catch (e) {
      toast.error("Update check failed", humanError(e));
    } finally {
      setChecking(false);
    }
  };

  const applyEngineUpdate = async () => {
    if (!release) return;
    setUpdating(true);
    setUpdateProgress(null);
    try {
      const version = await api.installEngineUpdate(release.latest_version);
      toast.success("Engine updated", `Docker Engine ${version} is now active.`);
      await checkForEngineUpdate();
    } catch (e) {
      toast.error("Engine update failed", String(e));
    } finally {
      setUpdating(false);
      setUpdateProgress(null);
    }
  };

  const [cliResolution, setCliResolution] = useState<CliResolution | null>(null);
  const [shimStatus, setShimStatus] = useState<CliShimStatus | null>(null);
  const [shimBusy, setShimBusy] = useState(false);
  const [compose, setCompose] = useState<ComposeUpdateInfo | null>(null);
  const [checkingCompose, setCheckingCompose] = useState(false);
  const [updatingCompose, setUpdatingCompose] = useState(false);

  const checkDockerCli = async () => {
    try {
      const res = await api.checkDockerCli();
      setCliResolution(res.resolution);
      setShimStatus(res.shim);
    } catch (e) {
      toast.error("CLI check failed", String(e));
    }
  };

  const handleToggleShim = async () => {
    setShimBusy(true);
    try {
      const target = !(shimStatus?.installed ?? false);
      const st = await api.toggleCliShim(target);
      setShimStatus(st);
      toast.success(
        target ? "Docker shim installed" : "Docker shim removed",
        st.path ?? "~/.local/bin/docker"
      );
    } catch (e) {
      toast.error("Shim operation failed", String(e));
    } finally {
      setShimBusy(false);
    }
  };

  const checkCompose = async (announce?: boolean) => {
    setCheckingCompose(true);
    try {
      const info = await api.checkComposeUpdate();
      setCompose(info);
      if (announce) {
        if (info.update_available) {
          toast.info("Compose update available", `${info.latest_version} ready to install`);
        } else {
          toast.success("Compose up to date", `Latest available: ${info.latest_version}`);
        }
      }
    } catch (e) {
      toast.error("Compose check failed", humanError(e));
    } finally {
      setCheckingCompose(false);
    }
  };

  const applyComposeUpdate = async () => {
    if (!compose?.update_available) return;
    setUpdatingCompose(true);
    try {
      const version = compose.latest_version;
      await api.installComposeUpdate(version);
      toast.success("Docker Compose updated", `Now using ${version}`);
      await checkCompose();
      await checkDockerCli();
    } catch (e) {
      toast.error("Compose update failed", String(e));
    } finally {
      setUpdatingCompose(false);
    }
  };

  useEffect(() => {
    loadSettings();
    checkForEngineUpdate().catch(() => {});
    checkDockerCli().catch(() => {});
    checkCompose().catch(() => {});
    let dispose: (() => void) | undefined;
    events
      .onEngineInstallProgress((progress) => setUpdateProgress(progress))
      .then((fn) => {
        dispose = fn;
      })
      .catch(() => {});
    return () => {
      dispose?.();
    };
  }, []);

  const handleToggleCliBridge = async (enabled: boolean) => {
    try {
      const res = await api.toggleCliBridge(enabled);
      setCliStatus(res);
      if (config) setConfig({ ...config, cli_bridge_enabled: enabled });
    } catch (e) {
      toast.error("CLI Bridge Error", String(e));
    }
  };

  const performMigration = async (clearOldSelection: boolean) => {
    const clean = newPathInput.trim();
    if (!clean) return;
    try {
      setStep("migrating");
      setMigrationError("");
      await api.migrateDataRoot(clean, shouldMoveData, clearOldSelection);
      // Reload updated config
      await loadSettings();
      setIsModalOpen(false);
    } catch (err) {
      setMigrationError(String(err));
      setStep("ask_clear");
    }
  };

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!config) return;

    try {
      setSaving(true);
      // Registries and endpoints are managed by their own cards and saved
      // independently — re-read the latest config so this save only touches
      // the fields this form actually owns and never clobbers them.
      const latest = await api.getAppConfig();
      const merged: AppConfig = {
        ...latest,
        cli_bridge_enabled: config.cli_bridge_enabled,
        cli_mode: config.cli_mode,
        auto_start_daemon: config.auto_start_daemon,
        theme: config.theme,
        compose_file: config.compose_file,
        terminal_shell: config.terminal_shell,
        terminal_font_size: config.terminal_font_size,
        engine_memory_limit_mb: Math.max(
          0,
          Math.floor(Number(config.engine_memory_limit_mb) || 0)
        ),
        engine_cpu_cores: (config.engine_cpu_cores ?? "").trim(),
      };
      await api.saveAppConfig(merged);
      setConfig(merged);
      invalidateTerminalSettings();
      setSaved(true);
      await checkDockerCli().catch(() => {});
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      toast.error("Save Error", String(e));
    } finally {
      setSaving(false);
    }
  };

  if (!config) return null;

  return (
    <div className="p-8 space-y-6 max-w-4xl mx-auto h-full overflow-y-auto">
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-white">Application Settings</h2>
        <p className="text-sm text-zinc-400 mt-1">
          Tune bundled engine parameters, CLI socket bridges, and default paths
        </p>
      </div>

      <RegistriesCard />

      <NotificationsCard />

      <KubernetesCard />

      <form onSubmit={handleSave} className="space-y-6">


        {/* Docker CLI */}
        <Card className="space-y-4">
          <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
            <div className="p-2 rounded-lg bg-zinc-800/80 border border-zinc-700/50">
              <Terminal className="w-5 h-5 text-emerald-400" />
            </div>
            <div className="flex-1">
              <h3 className="font-semibold text-white">Docker CLI</h3>
              <p className="text-xs text-zinc-400 mt-0.5">
                Prefer a docker CLI you already have, or use the one bundled with the app.
              </p>
            </div>
          </div>

          {cliResolution ? (
            <div className="space-y-3">
              <div className="flex items-center gap-2 flex-wrap">
                <span
                  className={`text-xs font-medium px-2 py-0.5 rounded border ${
                    cliResolution.source === "system"
                      ? "bg-sky-500/10 text-sky-300 border-sky-500/30"
                      : "bg-emerald-500/10 text-emerald-300 border-emerald-500/30"
                  }`}
                >
                  Using {cliResolution.source === "system" ? "system docker" : "bundled docker"}{" "}
                  {cliResolution.version}
                </span>
                <span className="text-xs text-zinc-400">{cliResolution.reason}</span>
              </div>

              <div className="grid gap-2 text-xs">
                <div className="flex items-center justify-between gap-3 p-2.5 rounded-lg bg-zinc-950 border border-zinc-800">
                  <span className="text-zinc-400">System</span>
                  <span className="font-mono text-zinc-300 truncate">
                    {cliResolution.system_path
                      ? `${cliResolution.system_path} · ${cliResolution.system_version}`
                      : "not installed"}
                  </span>
                </div>
                <div className="flex items-center justify-between gap-3 p-2.5 rounded-lg bg-zinc-950 border border-zinc-800">
                  <span className="text-zinc-400">Bundled</span>
                  <span className="font-mono text-zinc-300 truncate">
                    {cliResolution.bundled_version
                      ? `${cliResolution.bundled_path} · ${cliResolution.bundled_version}`
                      : "no engine installed"}
                  </span>
                </div>
              </div>
            </div>
          ) : (
            <p className="text-xs text-zinc-500">Checking which docker CLI is available…</p>
          )}

          <div className="space-y-2">
            <label className="text-xs font-medium text-zinc-300">Preference</label>
            <select
              value={config?.cli_mode ?? "auto"}
              onChange={(e) => setConfig((c) => (c ? { ...c, cli_mode: e.target.value } : c))}
              className="w-full px-3 py-2 rounded-lg bg-zinc-950 border border-zinc-800 text-sm text-white focus:border-blue-500 focus:outline-none"
            >
              <option value="auto">Automatic — use whichever is newer</option>
              <option value="system">Always use the system docker</option>
              <option value="bundled">Always use the bundled docker</option>
            </select>
          </div>

          {/* Terminal access: how a shell reaches this engine. The socket
              bridge is the right answer when a docker already exists; the
              shim only when there is none to bridge. */}
          <div className="space-y-3 pt-1">
            <p className="text-xs font-medium text-zinc-400 uppercase tracking-wide">
              Terminal access
            </p>

            <div className="flex items-start justify-between gap-4 p-3 rounded-lg bg-zinc-950 border border-zinc-800">
              <div className="flex-1">
                <p className="text-sm text-zinc-200">Socket bridge</p>
                <p className="text-xs text-zinc-500 mt-0.5">
                  Symlinks <span className="font-mono">$XDG_RUNTIME_DIR/docker.sock</span> and
                  registers a docker context, so your existing{" "}
                  <span className="font-mono">docker</span> reaches this app's engine.
                </p>
              </div>
              <label className="relative inline-flex items-center cursor-pointer shrink-0">
                <input
                  type="checkbox"
                  className="sr-only peer"
                  checked={config.cli_bridge_enabled}
                  onChange={(e) => handleToggleCliBridge(e.target.checked)}
                />
                <div className="w-11 h-6 bg-zinc-800 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-emerald-600"></div>
              </label>
            </div>

            <div className="flex items-center justify-between px-3 py-2 text-xs">
              <span className="text-zinc-400">Bridge status</span>
              <span
                className={`font-mono font-medium ${
                  !config.cli_bridge_enabled
                    ? "text-zinc-500"
                    : cliStatus?.link_state
                      ? "text-emerald-400"
                      : "text-amber-400"
                }`}
              >
                {/* Three distinct states: the toggle is off (nothing to
                    report), the link is live, or the toggle is on but the
                    symlink/context is missing — which IS a problem. */}
                {!config.cli_bridge_enabled
                  ? "Off"
                  : cliStatus?.link_state
                    ? "Connected & Active"
                    : "Enabled, but not connected"}
              </span>
            </div>
          </div>

          <div className="flex items-start justify-between gap-4 p-3 rounded-lg bg-zinc-950 border border-zinc-800">
            <div className="flex-1">
              <p className="text-sm text-zinc-200">Expose docker in your terminal</p>
              <p className="text-xs text-zinc-500 mt-0.5">
                Installs <span className="font-mono">{shimStatus?.path ?? "~/.local/bin/docker"}</span>{" "}
                so the <span className="font-mono">docker</span> command reaches this app's engine —
                useful when no docker is installed system-wide.
              </p>
              {shimStatus?.installed && shimStatus.effect === "shadowed" && (
                <p className="text-xs text-amber-400 mt-1">
                  Inactive — <span className="font-mono">{shimStatus.shadows}</span> comes
                  first on your PATH, so the shell still uses that docker.
                </p>
              )}
              {shimStatus?.installed && shimStatus.stale && (
                <p className="text-xs text-amber-400/90">
                  This shim still points at{" "}
                  <span className="font-mono">{shimStatus.target}</span>, not the CLI
                  selected above. Re-install it to pick up the new choice.
                </p>
              )}
              {shimStatus?.installed && shimStatus.effect === "not_on_path" && (
                <p className="text-xs text-amber-400 mt-1">
                  Inactive — <span className="font-mono">~/.local/bin</span> is not on your
                  PATH, so a shell will not find this shim.
                </p>
              )}
              {!shimStatus?.installed && shimStatus?.shadows && (
                <p className="text-xs text-amber-400 mt-1">
                  You already have <span className="font-mono">{shimStatus.shadows}</span>.
                  Installing the shim would hide it from your terminal — prefer the socket
                  bridge above instead.
                </p>
              )}
            </div>
            <button
              type="button"
              onClick={handleToggleShim}
              disabled={shimBusy}
              className={`shrink-0 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors border ${
                shimStatus?.installed
                  ? "bg-emerald-500/10 text-emerald-300 border-emerald-500/30 hover:bg-emerald-500/20"
                  : "bg-zinc-800 text-zinc-300 border-zinc-700 hover:bg-zinc-700"
              } disabled:opacity-50`}
            >
              {shimBusy ? "Working…" : shimStatus?.installed ? "Remove" : "Install"}
            </button>
          </div>

          <div className="flex items-center justify-between gap-3 pt-1">
            <div className="text-xs text-zinc-400">
              {compose ? (
                <>
                  Docker Compose: <span className="font-mono text-zinc-300">
                    {compose.installed_version ?? "not installed"}
                  </span>
                  {compose.update_available && (
                    <>
                      {" "}· <span className="text-amber-300">{compose.latest_version} available</span>
                    </>
                  )}
                </>
              ) : (
                "Docker Compose: not checked"
              )}
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => checkCompose(true)}
                disabled={checkingCompose || updatingCompose}
                className="px-3 py-1.5 rounded-lg bg-zinc-800 hover:bg-zinc-700 disabled:opacity-50 text-xs font-medium text-zinc-200 transition-colors"
              >
                {checkingCompose ? "Checking…" : "Check Compose"}
              </button>
              <button
                type="button"
                onClick={applyComposeUpdate}
                disabled={
                  updatingCompose || checkingCompose || !compose?.update_available
                }
                className="px-3 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 disabled:opacity-50 text-xs font-medium text-white transition-colors"
              >
                {updatingCompose ? "Updating…" : "Update Compose"}
              </button>
            </div>
          </div>
        </Card>

        {/* Engine Update */}
        <Card className="space-y-4">
          <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
            <Power className="w-5 h-5 text-amber-400" />
            <div>
              <h3 className="font-semibold text-white">Docker Engine</h3>
              <p className="text-xs text-zinc-400">
                Which engine this app drives — the bundled one or a remote daemon —
                plus auto-start and updates for the bundled engine.
              </p>
            </div>
          </div>

          {/* Endpoints (bundled + remote) — the "which engine" half of this card. */}
          <EndpointsCard />

          {/* Lifecycle: the one engine setting that is not an update. */}
          <div className="flex items-center justify-between py-2">
            <div>
              <span className="font-medium text-sm text-zinc-200 block">
                Auto-start on App Launch
              </span>
              <span className="text-xs text-zinc-400 block mt-0.5">
                Boot the rootless dockerd when this app opens
              </span>
            </div>
            <label className="relative inline-flex items-center cursor-pointer">
              <input
                type="checkbox"
                checked={config.auto_start_daemon}
                onChange={(e) =>
                  setConfig({ ...config, auto_start_daemon: e.target.checked })
                }
                className="sr-only peer"
              />
              <div className="w-11 h-6 bg-zinc-800 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-blue-600"></div>
            </label>
          </div>

          {/* Resource limits */}
          <div className="pt-3 mt-1 border-t border-zinc-800/60">
            <span className="font-medium text-sm text-zinc-200 block">
              Engine Resource Limits
            </span>
            <span className="text-xs text-zinc-400 block mt-0.5">
              Caps host RAM/CPU dockerd and all its containers may use, so a
              heavy build or runaway container cannot starve the OS. 0 / empty =
              unlimited.
            </span>
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 mt-3">
              <label className="space-y-1">
                <span className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider">
                  Memory cap (MiB)
                </span>
                <input
                  type="number"
                  min={0}
                  step={128}
                  value={config.engine_memory_limit_mb ?? 0}
                  onChange={(e) =>
                    setConfig({
                      ...config,
                      engine_memory_limit_mb: Math.max(
                        0,
                        Math.floor(Number(e.target.value) || 0)
                      ),
                    })
                  }
                  placeholder="0 = unlimited"
                  aria-label="Engine memory cap in MiB"
                  className="h-9 w-full rounded-lg bg-zinc-900 border border-zinc-800 px-3 text-sm text-white focus:outline-none focus:border-blue-500 transition-colors"
                />
              </label>
              <label className="space-y-1">
                <span className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider">
                  CPU cap (cores)
                </span>
                <input
                  type="text"
                  inputMode="decimal"
                  value={config.engine_cpu_cores ?? ""}
                  onChange={(e) =>
                    setConfig({ ...config, engine_cpu_cores: e.target.value })
                  }
                  placeholder="e.g. 2.5 — empty = unlimited"
                  aria-label="Engine CPU cap in cores"
                  className="h-9 w-full rounded-lg bg-zinc-900 border border-zinc-800 px-3 text-sm text-white focus:outline-none focus:border-blue-500 transition-colors"
                />
              </label>
            </div>
          </div>

          <div className="flex items-center justify-between gap-4">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-sm">
                {updating ? (
                  <>
                    <RefreshCw className="w-4 h-4 text-sky-400 animate-spin" />
                    <span className="text-zinc-200">
                      {updateProgress?.label ?? "Starting…"}
                    </span>
                  </>
                ) : release?.update_available ? (
                  <>
                    <DownloadCloud className="w-4 h-4 text-sky-400" />
                    <span className="text-zinc-200">
                      Docker Engine{" "}
                      <span className="font-mono text-sky-300">{release.latest_version}</span> is available
                    </span>
                  </>
                ) : release ? (
                  <>
                    <PackageCheck className="w-4 h-4 text-emerald-400" />
                    <span className="text-zinc-200">
                      {release.installed_version
                        ? `Up to date · ${release.installed_version}`
                        : `No engine installed · latest is ${release.latest_version}`}
                    </span>
                  </>
                ) : (
                  <span className="text-zinc-400">
                    {checking ? "Checking for updates…" : "Update status unknown"}
                  </span>
                )}
              </div>
              {release && !updating && (
                <p className="text-[11px] text-zinc-500 mt-1">
                  Installed: {release.installed_version ?? "none"} · Latest: {release.latest_version}
                  {release.update_available && release.latest_version !== release.pinned_version && (
                    <> · checksums for this version come from Docker over TLS (not pinned)</>
                  )}
                </p>
              )}
            </div>

            <div className="flex items-center gap-2 shrink-0">
              <Button
                type="button"
                variant="ghost"
                disabled={checking || updating}
                onClick={() => checkForEngineUpdate(true)}
              >
                <RefreshCw className={`w-4 h-4 ${checking ? "animate-spin" : ""}`} />
                <span>Check</span>
              </Button>
              <Button
                type="button"
                variant="primary"
                disabled={updating || checking || !release?.update_available}
                onClick={applyEngineUpdate}
              >
                <DownloadCloud className="w-4 h-4" />
                <span>{release?.installed_version ? "Update Engine" : "Install Engine"}</span>
              </Button>
            </div>
          </div>

          {updating && (
            <div className="space-y-2">
              <div className="h-1.5 w-full rounded-full bg-zinc-800 overflow-hidden">
                <div
                  className="h-full rounded-full bg-sky-500 transition-all duration-300"
                  style={{ width: `${Math.round((updateProgress?.fraction ?? 0) * 100)}%` }}
                />
              </div>
              <p className="text-[11px] text-zinc-400">
                The engine restarts when this finishes. Container and image data is untouched.
              </p>
            </div>
          )}
        </Card>

        {/* Engine Storage Location (Data Root) */}
        <Card className="space-y-4">
          <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
            <HardDrive className="w-5 h-5 text-emerald-400" />
            <div>
              <h3 className="font-semibold text-white">Engine Storage Location (Data Root)</h3>
              <p className="text-xs text-zinc-400">
                Custom directory path for images, containers, and volumes (`dockerd --data-root`)
              </p>
            </div>
          </div>

          <div>
            <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-2">
              Current Data Root Path
            </label>
            <div className="flex items-center gap-3">
              <input
                type="text"
                readOnly
                disabled
                value={config.data_root || "Default: ~/.local/share/hsx2dock/data"}
                className="flex-1 h-10 px-3.5 rounded-xl bg-zinc-950/80 border border-zinc-800 text-sm text-zinc-300 font-mono cursor-not-allowed select-text opacity-90"
              />
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  setNewPathInput(config.data_root || "");
                  setStep("input");
                  setPathInspection(null);
                  setShouldMoveData(false);
                  setShouldClearOld(false);
                  setMigrationError("");
                  setIsModalOpen(true);
                }}
                className="gap-2 h-10 px-4 shrink-0 font-medium"
              >
                <FolderEdit className="w-4 h-4 text-emerald-400" />
                <span>Change Path</span>
              </Button>
            </div>
            <p className="text-xs text-zinc-500 mt-2">
              * Note: Modifying the path uses an interactive wizard to safely check folder contents, offer data migration, and switch paths seamlessly.
            </p>
          </div>
        </Card>

        {/* Theme Settings */}
        <Card className="space-y-4">
          <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
            <Palette className="w-5 h-5 text-indigo-400" />
            <div>
              <h3 className="font-semibold text-white">Appearance & Theme</h3>
              <p className="text-xs text-zinc-400">Switch color theme for application interface</p>
            </div>
          </div>

          <div className="flex items-center justify-between py-2">
            <div>
              <span className="font-medium text-sm text-zinc-200 block">Theme Mode</span>
              <span className="text-xs text-zinc-400 block mt-0.5">
                Choose between Dark, Light or System default
              </span>
            </div>
            <select
              value={config.theme || "dark"}
              onChange={(e) => {
                const val = e.target.value;
                setConfig({ ...config, theme: val });
                if (val === "dark") {
                  document.documentElement.classList.add("dark");
                  document.documentElement.classList.remove("light");
                  localStorage.setItem("theme", "dark");
                } else if (val === "light") {
                  document.documentElement.classList.remove("dark");
                  document.documentElement.classList.add("light");
                  localStorage.setItem("theme", "light");
                } else {
                  localStorage.removeItem("theme");
                  if (window.matchMedia("(prefers-color-scheme: dark)").matches) {
                    document.documentElement.classList.add("dark");
                  } else {
                    document.documentElement.classList.remove("dark");
                  }
                }
              }}
              className="h-10 px-3 rounded-xl bg-zinc-900 border border-zinc-800 text-xs text-white focus:outline-none focus:border-blue-500 cursor-pointer [&>option]:bg-zinc-900 [&>option]:text-white"
            >
              <option value="dark">Dark Theme</option>
              <option value="light">Light Theme</option>
              <option value="system">System Default</option>
            </select>
          </div>
        </Card>

        {/* Compose File Path */}
        <Card className="space-y-4">
          <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
            <FileCode2 className="w-5 h-5 text-purple-400" />
            <div>
              <h3 className="font-semibold text-white">Compose Workspace</h3>
              <p className="text-xs text-zinc-400">Default project file location</p>
            </div>
          </div>

          <div>
            <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-2">
              Default Compose YAML Path
            </label>
            <input
              type="text"
              placeholder="e.g. ~/projects/my-app/docker-compose.yml"
              value={config.compose_file}
              onChange={(e) => setConfig({ ...config, compose_file: e.target.value })}
              className="w-full h-10 px-3.5 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500 font-mono"
            />
          </div>
        </Card>

        {/* Interactive Terminal */}
        <Card className="space-y-4">
          <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
            <Terminal className="w-5 h-5 text-emerald-400" />
            <div>
              <h3 className="font-semibold text-white">Interactive Terminal</h3>
              <p className="text-xs text-zinc-400">
                Shell and font used by the container terminal and the Debug action
              </p>
            </div>
          </div>

          <div className="space-y-2">
            <label className="text-sm font-medium text-zinc-200 block">
              Default Shell
            </label>
            <span className="text-xs text-zinc-400 block -mt-1">
              Opened when you attach a terminal or debug a stopped container. The
              shell must exist inside the container — <code className="text-zinc-300">sh</code>{" "}
              is the safe default for minimal images.
            </span>
            <input
              list="terminal-shell-presets"
              value={config.terminal_shell || "sh"}
              onChange={(e) => setConfig({ ...config, terminal_shell: e.target.value })}
              placeholder="sh"
              className="w-full h-10 px-3.5 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500 font-mono"
            />
            <datalist id="terminal-shell-presets">
              <option value="sh" />
              <option value="bash" />
              <option value="ash" />
              <option value="zsh" />
              <option value="fish" />
            </datalist>
          </div>

          <div className="flex items-center justify-between py-1">
            <div>
              <span className="font-medium text-sm text-zinc-200 block">
                Terminal Font Size
              </span>
              <span className="text-xs text-zinc-400 block mt-0.5">
                Font size in pixels for terminal panes (8–32)
              </span>
            </div>
            <input
              type="number"
              min={8}
              max={32}
              value={config.terminal_font_size ?? 12}
              onChange={(e) => {
                const n = parseInt(e.target.value, 10);
                setConfig({
                  ...config,
                  terminal_font_size: Number.isFinite(n)
                    ? Math.min(Math.max(n, 8), 32)
                    : 12,
                });
              }}
              className="w-24 h-10 px-3.5 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500 font-mono text-center"
            />
          </div>
        </Card>

        {/* Save button */}
        <div className="flex justify-end gap-3 pt-4">
          <Button type="submit" variant="primary" isLoading={saving} className="gap-2">
            {saved ? <Check className="w-4 h-4 text-emerald-400" /> : <Save className="w-4 h-4" />}
            <span>{saved ? "Saved Changes" : "Save Settings"}</span>
          </Button>
        </div>
      </form>

      {/* Change Data Root Interactive Wizard Modal */}
      {isModalOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/70 backdrop-blur-sm animate-in fade-in duration-200">
          <div className="bg-zinc-900 border border-zinc-800 rounded-2xl w-full max-w-xl overflow-hidden shadow-2xl flex flex-col">
            {/* Modal Header */}
            <div className="h-14 px-6 border-b border-zinc-800 flex items-center justify-between">
              <div className="flex items-center gap-2.5">
                <HardDrive className="w-5 h-5 text-emerald-400" />
                <h3 className="font-semibold text-white">Change Engine Data Root Path</h3>
              </div>
              {step !== "migrating" && (
                <button
                  type="button"
                  onClick={() => setIsModalOpen(false)}
                  className="text-zinc-400 hover:text-white"
                >
                  <X className="w-5 h-5" />
                </button>
              )}
            </div>

            {/* Modal Body */}
            <div className="p-6 space-y-5">
              {migrationError && (
                <div className="p-3 bg-rose-500/10 border border-rose-500/20 rounded-xl flex items-center gap-2.5 text-xs text-rose-400">
                  <AlertTriangle className="w-4 h-4 shrink-0" />
                  <span>{migrationError}</span>
                </div>
              )}

              {/* Step 1: Input Path */}
              {step === "input" && (
                <div className="space-y-4">
                  <p className="text-sm text-zinc-300">
                    Enter the new absolute destination folder for storing all container layers, images, and volumes:
                  </p>
                  <div>
                    <label className="block text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-2">
                      New Directory Path
                    </label>
                    <input
                      type="text"
                      placeholder="e.g. /mnt/data/docker-storage"
                      value={newPathInput}
                      onChange={(e) => setNewPathInput(e.target.value)}
                      className="w-full h-11 px-3.5 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500 font-mono"
                    />
                  </div>
                  <div className="text-xs text-zinc-400 bg-zinc-950/60 p-3.5 rounded-xl border border-zinc-800/60 flex flex-col gap-1.5">
                    <span className="font-semibold text-zinc-300 block mb-0.5">Note:</span>
                    <p className="m-0 block leading-relaxed">• The folder will be checked to determine if it is brand new or already has files.</p>
                    <p className="m-0 block leading-relaxed">• If the folder is empty or not yet created, you can choose to transfer existing data.</p>
                  </div>
                </div>
              )}

              {/* Step 2: Destination is empty/new -> Ask to move existing data */}
              {step === "ask_move" && (
                <div className="space-y-4">
                  <div className="p-4 bg-blue-500/10 border border-blue-500/20 rounded-xl space-y-2">
                    <div className="flex items-center gap-2 font-medium text-sm text-blue-300">
                      <Check className="w-4 h-4 text-blue-400" />
                      <span>Destination Directory is Empty / New</span>
                    </div>
                    <p className="text-xs text-zinc-300">
                      Target location: <span className="font-mono text-white">{newPathInput}</span>
                    </p>
                  </div>

                  <p className="text-sm text-zinc-200">
                    Do you want to <span className="font-semibold text-emerald-400">transfer (clone/move) your existing containers, images and volumes</span> to this new location?
                  </p>

                  <div className="grid grid-cols-2 gap-3 pt-2">
                    <button
                      type="button"
                      onClick={() => {
                        setShouldMoveData(true);
                        setStep("ask_clear");
                      }}
                      className="p-4 rounded-xl border border-emerald-500/40 bg-emerald-500/10 hover:bg-emerald-500/20 text-left transition-colors group cursor-pointer"
                    >
                      <span className="font-semibold text-sm text-white block group-hover:text-emerald-300">
                        Yes, Move Data
                      </span>
                      <span className="text-xs text-zinc-400 mt-1 block">
                        Preserves all current images, volumes, and containers.
                      </span>
                    </button>

                    <button
                      type="button"
                      onClick={() => {
                        setShouldMoveData(false);
                        setStep("ask_clear");
                      }}
                      className="p-4 rounded-xl border border-zinc-800 bg-zinc-950 hover:bg-zinc-800/60 text-left transition-colors group cursor-pointer"
                    >
                      <span className="font-semibold text-sm text-white block group-hover:text-zinc-200">
                        No, Start Fresh
                      </span>
                      <span className="text-xs text-zinc-400 mt-1 block">
                        Initialize a fresh empty Docker data root environment.
                      </span>
                    </button>
                  </div>
                </div>
              )}

              {/* Step 3: Ask to clear old data root directory */}
              {step === "ask_clear" && (
                <div className="space-y-4">
                  <div className="p-4 bg-amber-500/10 border border-amber-500/20 rounded-xl space-y-2">
                    <div className="flex items-center gap-2 font-medium text-sm text-amber-300">
                      <AlertTriangle className="w-4 h-4 text-amber-400" />
                      <span>Clean Up Old Storage</span>
                    </div>
                    <p className="text-xs text-zinc-300">
                      Old location: <span className="font-mono text-zinc-200">{pathInspection?.current_data_root || "default data root"}</span>
                    </p>
                  </div>

                  <p className="text-sm text-zinc-200">
                    Do you want to <span className="font-semibold text-amber-400">delete and clear all files at the old data root path</span> to reclaim disk space?
                  </p>

                  <div className="grid grid-cols-2 gap-3 pt-2">
                    <button
                      type="button"
                      onClick={async () => {
                        setShouldClearOld(true);
                        await performMigration(true);
                      }}
                      className="p-4 rounded-xl border border-rose-500/40 bg-rose-500/10 hover:bg-rose-500/20 text-left transition-colors group cursor-pointer"
                    >
                      <span className="font-semibold text-sm text-white block group-hover:text-rose-300">
                        Yes, Delete Old Files
                      </span>
                      <span className="text-xs text-zinc-400 mt-1 block">
                        Reclaim storage on the previous filesystem.
                      </span>
                    </button>

                    <button
                      type="button"
                      onClick={async () => {
                        setShouldClearOld(false);
                        await performMigration(false);
                      }}
                      className="p-4 rounded-xl border border-zinc-800 bg-zinc-950 hover:bg-zinc-800/60 text-left transition-colors group cursor-pointer"
                    >
                      <span className="font-semibold text-sm text-white block group-hover:text-zinc-200">
                        Keep Old Files
                      </span>
                      <span className="text-xs text-zinc-400 mt-1 block">
                        Leave files intact as a backup.
                      </span>
                    </button>
                  </div>
                </div>
              )}

              {/* Step 4: Migrating progress spinner */}
              {step === "migrating" && (
                <div className="py-8 flex flex-col items-center justify-center space-y-4">
                  <div className="w-10 h-10 border-4 border-emerald-500 border-t-transparent rounded-full animate-spin" />
                  <div className="text-center">
                    <p className="font-semibold text-white">Applying Data Root Changes...</p>
                    <p className="text-xs text-zinc-400 mt-1">
                      Stopping daemon, migrating storage directories, updating configurations, and restarting engine.
                    </p>
                  </div>
                </div>
              )}
            </div>

            {/* Modal Footer */}
            {step === "input" && (
              <div className="h-16 px-6 border-t border-zinc-800 flex items-center justify-between bg-zinc-950/40">
                <Button
                  type="button"
                  variant="ghost"
                  onClick={() => setIsModalOpen(false)}
                >
                  Cancel
                </Button>
                <Button
                  type="button"
                  variant="primary"
                  onClick={async () => {
                    const clean = newPathInput.trim();
                    if (!clean) {
                      setMigrationError("Please specify a valid folder path");
                      return;
                    }
                    try {
                      setMigrationError("");
                      const inspection = await api.checkDataRootPath(clean);
                      setPathInspection(inspection);
                      // Refuse up front: a filesystem without d_type cannot
                      // run the engine's snapshotters, so every image pull
                      // would fail with a lease error.
                      if (inspection.fs_error) {
                        setMigrationError(inspection.fs_error);
                        return;
                      }
                      const normalize = (s: string) => s.replace(/\/+$/, "");
                      if (normalize(clean) === normalize(inspection.current_data_root)) {
                        setMigrationError("This path is already the active data root — nothing to migrate.");
                        return;
                      }
                      if (!inspection.exists || inspection.is_empty) {
                        setStep("ask_move");
                      } else {
                        // Folder already exists and contains files -> ask to clear old or switch directly
                        setShouldMoveData(false);
                        setStep("ask_clear");
                      }
                    } catch (e) {
                      setMigrationError(String(e));
                    }
                  }}
                  className="gap-2"
                >
                  <span>Continue</span>
                  <ArrowRight className="w-4 h-4" />
                </Button>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
