import React, { useEffect, useState } from "react";
import { Terminal, Power, Save, FileCode2, Check, HardDrive, Palette, FolderEdit, AlertTriangle, ArrowRight, X, DownloadCloud, Monitor, Cpu, Wrench, Boxes, Minimize2, Stethoscope, Globe } from "lucide-react";
import { Card } from "../components/ui/Card";
import { useToast } from "../components/ui/Toast";
import { Button } from "../components/ui/Button";
import { EndpointsCard } from "../components/settings/EndpointsCard";
import { EngineHealthCard } from "../components/settings/EngineHealthCard";
import { RegistriesCard } from "../components/settings/RegistriesCard";
import { NotificationsCard } from "../components/settings/NotificationsCard";
import { KubernetesCard } from "../components/settings/KubernetesCard";
import { BinaryUpdatePanel } from "../components/settings/BinaryUpdatePanel";
import { onBinaryInstalled, useBinaryUpdates } from "../lib/binaryUpdates";
import { binaryStatus } from "../lib/binaryUpdateModel";
import { useModal } from "../components/ui/ModalContext";
import { api } from "../lib/api";
import { invalidateTerminalSettings } from "../lib/terminalSettings";
import type {
  AppConfig,
  CliBridgeStatus,
  CliResolution,
  CliShimStatus,
  CaddyInstallStatus,
  UpdateTarget,
} from "../types";

/** The four configuration domains, in the order the rail presents them. */
type SettingsTab = "general" | "engine" | "tools" | "registry";

/**
 * The tab survives a remount (navigating away and back, an engine restart):
 * an update running in the Engine tab must still be on screen afterwards.
 */
let lastSettingsTab: SettingsTab = "general";

/** Binaries whose update panel lives on each tab, for the rail's dot. */
const TAB_BINARIES: Partial<Record<SettingsTab, UpdateTarget[]>> = {
  engine: ["engine", "kind"],
  tools: ["cli", "compose", "caddy"],
};

const SETTINGS_TABS: {
  id: SettingsTab;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}[] = [
  { id: "general", label: "General", icon: Monitor },
  { id: "engine", label: "Engine", icon: Cpu },
  { id: "tools", label: "Tools", icon: Wrench },
  { id: "registry", label: "Registry", icon: Boxes },
];

/** Pill switch shared by the Window Behaviour rows. */
const Switch: React.FC<{ on: boolean }> = ({ on }) => (
  <span
    aria-hidden="true"
    className={`relative w-10 rounded-none border transition-colors shrink-0 ${
      on ? "bg-emerald-500/30 border-emerald-500/50" : "bg-zinc-800 border-zinc-700"
    }`}
    style={{ height: "1.375rem" }}
  >
    <span
      className={`absolute top-1/2 -translate-y-1/2 w-4 h-4 rounded-none transition-all ${
        on ? "left-[1.3rem] bg-emerald-400" : "left-0.5 bg-zinc-500"
      }`}
    />
  </span>
);

/** One labelled switch row; `disabled` keeps the dependent option visible but
 *  explains why it cannot be changed instead of hiding it outright. */
const ToggleRow: React.FC<{
  title: string;
  desc: string;
  on: boolean;
  disabled?: boolean;
  onToggle: () => void;
}> = ({ title, desc, on, disabled = false, onToggle }) => (
  <button
    type="button"
    role="switch"
    aria-checked={on}
    disabled={disabled}
    onClick={onToggle}
    className={`w-full flex items-center justify-between gap-4 py-3 text-left ${
      disabled ? "opacity-50 cursor-not-allowed" : ""
    }`}
  >
    <span>
      <span className="block font-medium text-sm text-zinc-200">{title}</span>
      <span className="block text-xs text-zinc-400 mt-0.5">{desc}</span>
    </span>
    <Switch on={on} />
  </button>
);

export const SettingsPage: React.FC<{ refreshKey?: number }> = ({ refreshKey = 0 }) => {
  const toast = useToast();
  const modal = useModal();
  const [tab, setTabState] = useState<SettingsTab>(() => lastSettingsTab);
  const setTab = (next: SettingsTab) => {
    lastSettingsTab = next;
    setTabState(next);
  };
  const binaries = useBinaryUpdates();
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

  // Caddy Gateway state
  const [caddyStatus, setCaddyStatus] = useState<CaddyInstallStatus | null>(null);
  const [togglingCaddy, setTogglingCaddy] = useState(false);
  const [caddyHttpPort, setCaddyHttpPort] = useState<number>(80);
  const [caddyHttpsPort, setCaddyHttpsPort] = useState<number>(443);
  const [savingPorts, setSavingPorts] = useState(false);

  const [cliResolution, setCliResolution] = useState<CliResolution | null>(null);
  const [shimStatus, setShimStatus] = useState<CliShimStatus | null>(null);
  const [shimBusy, setShimBusy] = useState(false);

  const checkDockerCli = async (announce?: boolean) => {
    try {
      const res = await api.checkDockerCli();
      setCliResolution(res.resolution);
      setShimStatus(res.shim);
    } catch (e) {
      if (announce) toast.error("CLI check failed", String(e));
      else console.warn("CLI check failed:", e);
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

  const loadCaddyStatus = async () => {
    try {
      const st = await api.getGatewayStatus();
      setCaddyStatus(st);
      setCaddyHttpPort(st.http_port);
      setCaddyHttpsPort(st.https_port);
    } catch (e) {
      console.error("Failed to load Caddy status:", e);
    }
  };

  const handleToggleCaddy = async () => {
    if (!caddyStatus) return;
    setTogglingCaddy(true);
    try {
      const next = !caddyStatus.running;
      const isRunning = await api.toggleGateway(next);
      setCaddyStatus((prev) => (prev ? { ...prev, running: isRunning } : prev));
      toast.success(
        isRunning ? "Caddy Gateway Started" : "Caddy Gateway Stopped",
        isRunning ? `Listening on :${caddyHttpPort} and :${caddyHttpsPort}` : "Process terminated"
      );
    } catch (e) {
      toast.error("Gateway toggle failed", String(e));
    } finally {
      setTogglingCaddy(false);
    }
  };

  const handleSavePorts = async () => {
    if (caddyHttpPort === caddyHttpsPort) {
      toast.error("Invalid ports", "HTTP and HTTPS ports must be distinct");
      return;
    }
    setSavingPorts(true);
    try {
      await api.setGatewayPorts(caddyHttpPort, caddyHttpsPort);
      toast.success("Gateway ports updated", `HTTP: ${caddyHttpPort} · HTTPS: ${caddyHttpsPort}`);
      await loadCaddyStatus();
    } catch (e) {
      toast.error("Failed to update ports", String(e));
    } finally {
      setSavingPorts(false);
    }
  };

  useEffect(() => {
    loadSettings();
    checkDockerCli().catch(() => {});
    loadCaddyStatus().catch(() => {});
    // Release checks are not fired here: each update panel checks (cached for
    // 30 min) when its tab is first shown.
    return onBinaryInstalled((target) => {
      if (target === "caddy") void loadCaddyStatus();
      else if (target !== "kind") void checkDockerCli();
    });
  }, []);

  // The engine started/stopped (an engine update does both): refresh what
  // depends on it in place, without remounting or leaving the current tab.
  const firstRefresh = React.useRef(true);
  useEffect(() => {
    if (firstRefresh.current) {
      firstRefresh.current = false;
      return;
    }
    checkDockerCli().catch(() => {});
    loadCaddyStatus().catch(() => {});
  }, [refreshKey]);

  /** Engine updates stop the daemon: say what that means before starting. */
  const confirmEngineUpdate = (version: string | null, installed: boolean) =>
    installed
      ? modal.showConfirm(
          `Docker Engine stops while its binaries are replaced with ${version ?? "the latest release"}, then starts again. Running containers are stopped; those with a restart policy come back on their own. The update keeps its progress here even while the engine restarts.`,
          "Update Docker Engine",
          "warning"
        )
      : Promise.resolve(true);

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
      // Only the fields this form owns are sent; the backend merges them into
      // the live config, so cards that saved in the meantime are not undone.
      const fields: Partial<AppConfig> = {
        cli_bridge_enabled: config.cli_bridge_enabled,
        cli_mode: config.cli_mode,
        auto_start_daemon: config.auto_start_daemon,
        theme: config.theme,
        close_to_tray: !!config.close_to_tray,
        auto_start_on_boot: !!config.auto_start_on_boot,
        start_minimized: !!config.start_minimized,
        compose_file: config.compose_file,
        terminal_shell: config.terminal_shell,
        terminal_font_size: config.terminal_font_size,
        engine_memory_limit_mb: Math.max(
          0,
          Math.floor(Number(config.engine_memory_limit_mb) || 0)
        ),
        engine_cpu_cores: (config.engine_cpu_cores ?? "").trim(),
      };
      await api.patchAppConfig(fields);
      setConfig({ ...config, ...fields });
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
    <div className="flex h-full flex-col overflow-hidden">
      {/* Horizontal tab rail. Stays put while the active panel scrolls
          underneath it. */}
      <div className="shrink-0 px-8 pt-6">
        <div className="max-w-4xl mx-auto">
          <div
            role="tablist"
            aria-label="Settings sections"
            className="grid grid-cols-2 gap-1 border-b border-zinc-800 sm:grid-cols-4"
          >
            {SETTINGS_TABS.map(({ id, label, icon: Icon }) => {
              const active = tab === id;
              const kinds = (TAB_BINARIES[id] ?? []).map((t) => binaryStatus(binaries[t]).kind);
              const dot = kinds.includes("updating")
                ? { cls: "bg-sky-400 animate-pulse", hint: "An update is running" }
                : kinds.includes("update")
                  ? { cls: "bg-amber-400", hint: "Updates available" }
                  : null;
              return (
                <button
                  key={id}
                  type="button"
                  role="tab"
                  aria-selected={active}
                  aria-controls={`settings-panel-${id}`}
                  onClick={() => setTab(id)}
                  className={`group relative -mb-px flex w-full items-center justify-center gap-2 whitespace-nowrap rounded-t-lg border-b-2 px-4 py-3 text-sm font-medium transition-colors ${
                    active
                      ? "border-emerald-500 text-white"
                      : "border-transparent text-zinc-400 hover:bg-zinc-900/40 hover:text-zinc-200"
                  }`}
                >
                  <Icon
                    className={`h-4 w-4 shrink-0 ${
                      active ? "text-emerald-400" : "text-zinc-500 group-hover:text-zinc-300"
                    }`}
                  />
                  {label}
                  {dot && (
                    <span
                      className={`h-1.5 w-1.5 rounded-full ${dot.cls}`}
                      title={dot.hint}
                      aria-label={dot.hint}
                    />
                  )}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      <form onSubmit={handleSave} className="flex min-h-0 flex-1 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto px-8 py-6">
          <div className="max-w-4xl mx-auto space-y-6">
        {/* General — appearance first, then alerts, then the terminal. */}
        <div
          id="settings-panel-general"
          role="tabpanel"
          aria-label="General"
          hidden={tab !== "general"}
          className="space-y-6"
        >
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

          <NotificationsCard />

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

          {/* Window behaviour — what the close button does. */}
          <Card className="space-y-4">
            <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
              <Minimize2 className="w-5 h-5 text-indigo-400" />
              <div>
                <h3 className="font-semibold text-white">Window Behaviour</h3>
                <p className="text-xs text-zinc-400">
                  How the app behaves when you close it or sign in
                </p>
              </div>
            </div>

            <div className="divide-y divide-zinc-800/70">
              <ToggleRow
                title="Close to tray icon"
                desc="Closing the window keeps Hsx2Dock and its Docker engine running in the background. Reopen from the tray icon; use the tray's Quit to exit completely."
                on={!!config.close_to_tray}
                onToggle={() =>
                  setConfig({ ...config, close_to_tray: !config.close_to_tray })
                }
              />
              <ToggleRow
                title="Auto startup on boot"
                desc="Launch Hsx2Dock (and its Docker engine) when you sign in. Needs a desktop session with XDG autostart support."
                on={!!config.auto_start_on_boot}
                onToggle={() => {
                  const next = !config.auto_start_on_boot;
                  setConfig({
                    ...config,
                    auto_start_on_boot: next,
                    // The dependent switch cannot survive its parent.
                    start_minimized: next ? config.start_minimized : false,
                  });
                }}
              />
              <ToggleRow
                title="Start minimized"
                desc={
                  config.auto_start_on_boot
                    ? "Boot launches go straight to the tray instead of opening the window."
                    : "Enable Auto startup on boot first — there is no boot launch to minimize."
                }
                on={!!config.start_minimized}
                disabled={!config.auto_start_on_boot}
                onToggle={() =>
                  setConfig({ ...config, start_minimized: !config.start_minimized })
                }
              />
            </div>
          </Card>
        </div>

        {/* Engine — the daemon itself, its storage, and the Kind clusters it hosts. */}
        <div
          id="settings-panel-engine"
          role="tabpanel"
          aria-label="Engine"
          hidden={tab !== "engine"}
          className="space-y-6"
        >
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
                <div className="w-11 h-6 bg-zinc-800 peer-focus:outline-none rounded-none peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-none after:h-5 after:w-5 after:transition-all peer-checked:bg-blue-600"></div>
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

            <BinaryUpdatePanel
              target="engine"
              name="Docker Engine"
              icon={Power}
              iconClass="text-amber-400"
              autoCheck={tab === "engine"}
              description={
                binaries.engine.info?.updateAvailable &&
                binaries.engine.info.latest !== binaries.engine.info.pinned
                  ? "Checksums for this version come from Docker over TLS (not pinned in the app)."
                  : "Rootless dockerd, containerd and runc bundled with the app."
              }
              impact="Stops the engine while it is replaced, then starts it again."
              confirm={confirmEngineUpdate}
            />
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

          {/* Engine health & self-repair. Sits after storage because a moved or
              freshly-cleared data root is the condition it exists to recover
              from. */}
          <Card className="space-y-4">
            <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
              <Stethoscope className="w-5 h-5 text-cyan-400" />
              <div>
                <h3 className="font-semibold text-white">Engine Diagnostics</h3>
                <p className="text-xs text-zinc-400">
                  Diagnose and repair the engine's install, storage and containerd
                  plugins — the failures that otherwise surface as opaque
                  &ldquo;unknown service&rdquo; errors.
                </p>
              </div>
            </div>

            <EngineHealthCard />
          </Card>

          <KubernetesCard visible={tab === "engine"} />
        </div>

        {/* Tools — the two command-line surfaces this app owns. */}
        <div
          id="settings-panel-tools"
          role="tabpanel"
          aria-label="Tools"
          hidden={tab !== "tools"}
          className="space-y-6"
        >
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

                {/* The system docker cannot be replaced without root, so this
                    updates the bundled copy only. */}
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
                  <div className="w-11 h-6 bg-zinc-800 peer-focus:outline-none rounded-none peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-none after:h-5 after:w-5 after:transition-all peer-checked:bg-emerald-600"></div>
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

          </Card>

          {/* Bundled command-line binaries: one update panel each. */}
          <Card className="space-y-3">
            <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
              <div className="p-2 bg-zinc-800/80 border border-zinc-700/50">
                <DownloadCloud className="w-5 h-5 text-sky-400" />
              </div>
              <div className="flex-1">
                <h3 className="font-semibold text-white">Bundled CLI tools</h3>
                <p className="text-xs text-zinc-400 mt-0.5">
                  The docker CLI and Compose plugin shipped with the app. A system-wide docker is
                  never touched — it needs root.
                </p>
              </div>
            </div>
            <BinaryUpdatePanel
              target="cli"
              name="Docker CLI"
              icon={Terminal}
              iconClass="text-emerald-400"
              autoCheck={tab === "tools"}
              description={cliResolution?.bundled_path ?? undefined}
              blockedReason={
                binaries.cli.info && !binaries.cli.info.installed
                  ? "Install Docker Engine first — the bundled CLI comes with it."
                  : null
              }
            />
            <BinaryUpdatePanel
              target="compose"
              name="Docker Compose"
              icon={FileCode2}
              iconClass="text-purple-400"
              autoCheck={tab === "tools"}
              description="The compose plugin used by the Compose page and `docker compose`."
            />
          </Card>

          {/* Caddy Gateway Binary & Reverse Proxy */}
          <Card className="space-y-4">
            <div className="flex items-center justify-between pb-3 border-b border-zinc-800">
              <div className="flex items-center gap-3">
                <div className="p-2 rounded-lg bg-zinc-800/80 border border-zinc-700/50">
                  <Globe className="w-5 h-5 text-sky-400" />
                </div>
                <div>
                  <h3 className="font-semibold text-white">Caddy Gateway</h3>
                  <p className="text-xs text-zinc-400 mt-0.5">
                    Embedded reverse proxy and HTTPS domain router for container web apps.
                  </p>
                </div>
              </div>

              {/* Status & Power toggle */}
              <div className="flex items-center gap-3">
                <span
                  className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-none text-xs font-medium border ${
                    caddyStatus?.running
                      ? "bg-emerald-500/10 text-emerald-400 border-emerald-500/30"
                      : "bg-zinc-800 text-zinc-400 border-zinc-700"
                  }`}
                >
                  <span
                    className={`w-1.5 h-1.5 rounded-full ${
                      caddyStatus?.running ? "bg-emerald-400" : "bg-zinc-500"
                    }`}
                  />
                  {caddyStatus?.running ? "Running" : "Stopped"}
                </span>

                <Button
                  type="button"
                  variant={caddyStatus?.running ? "danger" : "primary"}
                  size="sm"
                  disabled={togglingCaddy || !caddyStatus?.installed}
                  onClick={handleToggleCaddy}
                  className="gap-1.5"
                >
                  <Power className="w-3.5 h-3.5" />
                  <span>{togglingCaddy ? "Switching…" : caddyStatus?.running ? "Stop Gateway" : "Start Gateway"}</span>
                </Button>
              </div>
            </div>

            <BinaryUpdatePanel
              target="caddy"
              name="Caddy"
              icon={Globe}
              iconClass="text-sky-400"
              autoCheck={tab === "tools"}
              description={caddyStatus?.path ?? "~/.local/share/hsx2dock/versions/caddy"}
              impact={caddyStatus?.running ? "The running gateway restarts on the new binary with its routes." : undefined}
            />

            {/* Port Management */}
            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 pt-1">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-zinc-300 block">
                  HTTP Port
                </label>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    min={1}
                    max={65535}
                    value={caddyHttpPort}
                    onChange={(e) => setCaddyHttpPort(parseInt(e.target.value, 10) || 80)}
                    className="w-full h-9 px-3 rounded-lg bg-zinc-950 border border-zinc-800 text-sm text-white font-mono focus:outline-none focus:border-blue-500"
                  />
                </div>
                <span className="text-[11px] text-zinc-500 block">
                  Default: 80. Used for plain HTTP and redirecting to HTTPS.
                </span>
              </div>

              <div className="space-y-1.5">
                <label className="text-xs font-medium text-zinc-300 block">
                  HTTPS Port
                </label>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    min={1}
                    max={65535}
                    value={caddyHttpsPort}
                    onChange={(e) => setCaddyHttpsPort(parseInt(e.target.value, 10) || 443)}
                    className="w-full h-9 px-3 rounded-lg bg-zinc-950 border border-zinc-800 text-sm text-white font-mono focus:outline-none focus:border-blue-500"
                  />
                </div>
                <span className="text-[11px] text-zinc-500 block">
                  Default: 443. Used for encrypted TLS virtual hosts.
                </span>
              </div>
            </div>

            <div className="flex items-center justify-between pt-2">
              <p className="text-xs text-zinc-500">
                Modifying ports will update Gateway config and reload Caddy if running.
              </p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={
                  savingPorts ||
                  (caddyHttpPort === (caddyStatus?.http_port ?? 80) &&
                    caddyHttpsPort === (caddyStatus?.https_port ?? 443))
                }
                onClick={handleSavePorts}
                className="gap-1.5"
              >
                <Save className="w-3.5 h-3.5" />
                <span>{savingPorts ? "Saving…" : "Apply Ports"}</span>
              </Button>
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
        </div>

        {/* Registry — logins, live status, and the default push target. */}
        <div
          id="settings-panel-registry"
          role="tabpanel"
          aria-label="Registry"
          hidden={tab !== "registry"}
          className="space-y-6"
        >
          <RegistriesCard />
        </div>
          </div>
        </div>

        {/* Sticky action bar — pinned below the scroll region, so Save stays
            reachable no matter how tall the active tab grows. */}
        <div className="shrink-0 border-t border-zinc-800 bg-zinc-950/90 px-8 py-3 backdrop-blur-xl">
          <div className="max-w-4xl mx-auto flex items-center justify-between gap-4">
            <p className="text-xs text-zinc-500">
              {saved
                ? "All changes saved."
                : "Engine, terminal and workspace preferences apply on save."}
            </p>
            <div className="flex items-center gap-3 shrink-0">
              <Button type="submit" variant="primary" isLoading={saving} className="gap-2">
                {saved ? <Check className="w-4 h-4 text-emerald-400" /> : <Save className="w-4 h-4" />}
                <span>{saved ? "Saved Changes" : "Save Settings"}</span>
              </Button>
            </div>
          </div>
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
