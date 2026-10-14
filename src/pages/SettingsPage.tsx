import React, { useEffect, useState } from "react";
import { Terminal, Power, Save, FileCode2, Check } from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { api } from "../lib/api";
import type { AppConfig, CliBridgeStatus } from "../types";

export const SettingsPage: React.FC = () => {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [cliStatus, setCliStatus] = useState<CliBridgeStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

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
    </div>
  );
};
