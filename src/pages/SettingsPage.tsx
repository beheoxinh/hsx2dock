import React, { useEffect, useState } from "react";
import { Terminal, Power, Save, FileCode2, Check, HardDrive, Palette, FolderEdit, AlertTriangle, ArrowRight, X } from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { api } from "../lib/api";
import type { AppConfig, CliBridgeStatus } from "../types";

export const SettingsPage: React.FC = () => {
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

  useEffect(() => {
    loadSettings();
  }, []);

  const handleToggleCliBridge = async (enabled: boolean) => {
    try {
      const res = await api.toggleCliBridge(enabled);
      setCliStatus(res);
      if (config) setConfig({ ...config, cli_bridge_enabled: enabled });
    } catch (e) {
      alert(`Toggle CLI bridge failed: ${e}`);
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
      await api.saveAppConfig(config);
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      alert(`Failed to save config: ${e}`);
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

      <form onSubmit={handleSave} className="space-y-6">
        {/* CLI Integration Bridge */}
        <Card className="space-y-4">
          <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
            <Terminal className="w-5 h-5 text-blue-400" />
            <div>
              <h3 className="font-semibold text-white">Docker CLI Bridge</h3>
              <p className="text-xs text-zinc-400">
                Allow external terminal CLI commands like `docker ps` to connect to this rootless engine
              </p>
            </div>
          </div>

          <div className="flex items-center justify-between py-2">
            <div>
              <span className="font-medium text-sm text-zinc-200 block">
                Enable Socket Bridge
              </span>
              <span className="text-xs text-zinc-400 block mt-0.5">
                Creates a symlink in `$XDG_RUNTIME_DIR/docker.sock` and configures context
              </span>
            </div>
            <label className="relative inline-flex items-center cursor-pointer">
              <input
                type="checkbox"
                checked={config.cli_bridge_enabled}
                onChange={(e) => handleToggleCliBridge(e.target.checked)}
                className="sr-only peer"
              />
              <div className="w-11 h-6 bg-zinc-800 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-blue-600"></div>
            </label>
          </div>

          <div className="p-3 bg-zinc-950/60 rounded-xl border border-zinc-800/60 text-xs flex items-center justify-between">
            <span className="text-zinc-400">Symlink Status:</span>
            <span
              className={`font-mono font-medium ${
                cliStatus?.link_state ? "text-emerald-400" : "text-zinc-400"
              }`}
            >
              {cliStatus?.link_state ? "Connected & Active" : "Inactive / Disconnected"}
            </span>
          </div>
        </Card>

        {/* Engine Lifecycle */}
        <Card className="space-y-4">
          <div className="flex items-center gap-3 pb-3 border-b border-zinc-800">
            <Power className="w-5 h-5 text-amber-400" />
            <div>
              <h3 className="font-semibold text-white">Engine Autostart</h3>
              <p className="text-xs text-zinc-400">Control engine daemon startup behavior</p>
            </div>
          </div>

          <div className="flex items-center justify-between py-2">
            <div>
              <span className="font-medium text-sm text-zinc-200 block">
                Auto-start on App Launch
              </span>
              <span className="text-xs text-zinc-400 block mt-0.5">
                Automatically boot the rootless dockerd background process when opening the desktop suite
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
                value={config.data_root || "Default: ~/.local/share/docker-manager/data"}
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
