import React, { useEffect, useState } from "react";
import { Bell } from "lucide-react";
import { Card } from "../ui/Card";
import { Switch } from "../ui/Switch";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";
import { cn } from "../../lib/utils";
import type { K8sAlertPrefs, KindCluster, NotificationPrefs } from "../../types";

/** Matches the Rust `NotificationPrefs::default()` — all notifications on. */
const DEFAULTS: NotificationPrefs = {
  build_done: true,
  transfer_done: true,
  engine_state: true,
  kind_events: true,
  tray_hidden: true,
};

const ROWS: { key: keyof NotificationPrefs; label: string; desc: string }[] = [
  {
    key: "build_done",
    label: "Image builds",
    desc: "A build finished — succeeded, failed or was cancelled.",
  },
  {
    key: "transfer_done",
    label: "Pull & push",
    desc: "An image finished downloading or uploading.",
  },
  {
    key: "engine_state",
    label: "Engine status",
    desc: "The Docker engine started, stopped, or died unexpectedly.",
  },
  {
    key: "kind_events",
    label: "Kubernetes clusters",
    desc: "A kind cluster was created or deleted.",
  },
  {
    key: "tray_hidden",
    label: "Hidden to tray",
    desc: "The window was closed into the tray — a reminder that the app and engine are still running.",
  },
];

/** Used when the config has no `k8s_alerts` yet (matches the backend defaults). */
const K8S_DEFAULTS: K8sAlertPrefs = {
  enabled: true,
  crash_loop: true,
  image_pull: true,
  oom: true,
  restart_spike: true,
  node_not_ready: true,
  api_down: true,
  pvc_pending: true,
  muted_clusters: [],
};

const K8S_ROWS: { key: keyof Omit<K8sAlertPrefs, "enabled" | "muted_clusters">; label: string; desc: string }[] = [
  { key: "crash_loop", label: "Crash loops", desc: "A pod keeps restarting in a crash loop." },
  { key: "image_pull", label: "Image pull failures", desc: "A pod cannot pull its image." },
  { key: "oom", label: "Out-of-memory kills", desc: "A container was killed for exceeding its memory limit." },
  { key: "restart_spike", label: "Restart spikes", desc: "Containers restarted many times in a short window." },
  { key: "node_not_ready", label: "Node not ready", desc: "A cluster node stopped reporting ready." },
  { key: "api_down", label: "API server down", desc: "The cluster's API server stopped answering." },
  { key: "pvc_pending", label: "Pending volume claims", desc: "A persistent volume claim stays unbound." },
];

/**
 * Desktop-notification preferences. Each toggle saves immediately (the tray
 * menu reads the same config, so the two stay in sync), and saving always
 * re-reads the config first so it can never clobber the Settings page form.
 */
export const NotificationsCard: React.FC = () => {
  const toast = useToast();
  const [prefs, setPrefs] = useState<NotificationPrefs | null>(null);
  const [busy, setBusy] = useState(false);
  const [clusters, setClusters] = useState<KindCluster[]>([]);
  const [addName, setAddName] = useState("");

  useEffect(() => {
    api
      .getAppConfig()
      .then((cfg) => setPrefs({ ...DEFAULTS, ...(cfg.notifications ?? {}) }))
      .catch(() => setPrefs(DEFAULTS));
    api
      .listKindClusters()
      .then(setClusters)
      .catch(() => setClusters([]));
  }, []);

  const toggle = async (key: keyof NotificationPrefs) => {
    if (!prefs || busy) return;
    const next = { ...prefs, [key]: !prefs[key] };
    setPrefs(next);
    try {
      setBusy(true);
      // Only this field: the Settings form may have saved others since this
      // card mounted, and they must not be clobbered.
      await api.patchAppConfig({ notifications: next });
      toast.success(
        next[key] ? "Notifications enabled" : "Notifications muted",
        ROWS.find((r) => r.key === key)?.label ?? ""
      );
    } catch (e) {
      setPrefs((prev) => (prev ? { ...prev, [key]: !next[key] } : prev));
      toast.error("Could not save notification settings", String(e));
    } finally {
      setBusy(false);
    }
  };

  const k8s: K8sAlertPrefs = { ...K8S_DEFAULTS, ...(prefs?.k8s_alerts ?? {}) };
  const mutable = prefs !== null;

  /** Saves the whole alert block; rolls back only that block on failure. */
  const saveK8s = async (next: K8sAlertPrefs, okTitle: string) => {
    if (!prefs || busy) return;
    const prevK8s = k8s;
    setPrefs({ ...prefs, k8s_alerts: next });
    try {
      setBusy(true);
      await api.patchAppConfig({ notifications: { ...prefs, k8s_alerts: next } });
      toast.success(okTitle);
    } catch (e) {
      setPrefs((cur) => (cur ? { ...cur, k8s_alerts: prevK8s } : cur));
      toast.error("Could not save Kubernetes alert settings", String(e));
    } finally {
      setBusy(false);
    }
  };

  const addMuted = async () => {
    const name = addName;
    if (!name || k8s.muted_clusters.includes(name)) return;
    setAddName("");
    await saveK8s({ ...k8s, muted_clusters: [...k8s.muted_clusters, name] }, `Alerts muted for ${name}`);
  };

  const removeMuted = (name: string) =>
    saveK8s({ ...k8s, muted_clusters: k8s.muted_clusters.filter((n) => n !== name) }, `Alerts unmuted for ${name}`);

  const unmutedClusters = clusters.filter((c) => !k8s.muted_clusters.includes(c.name));

  return (
    <Card className="space-y-4">
      <div className="flex items-start gap-3">
        <div className="w-8 h-8 rounded-lg bg-blue-500/10 border border-blue-500/30 flex items-center justify-center shrink-0">
          <Bell className="w-4 h-4 text-blue-400" />
        </div>
        <div>
          <h3 className="font-semibold text-white">Notifications</h3>
          <p className="text-xs text-zinc-400 mt-1">
            Desktop alerts for background work and window events. The tray menu
            mirrors these toggles, so either place works.
          </p>
        </div>
      </div>

      <div className="space-y-1">
        {ROWS.map((row) => {
          const on = prefs ? prefs[row.key] : DEFAULTS[row.key];
          return (
            <button
              key={row.key}
              type="button"
              role="switch"
              aria-checked={!!on}
              onClick={() => toggle(row.key)}
              className="w-full flex items-center justify-between gap-4 px-3 py-2.5 rounded-xl hover:bg-zinc-900/60 transition-colors text-left"
            >
              <span className="min-w-0">
                <span className="block text-sm text-white">{row.label}</span>
                <span className="block text-xs text-zinc-500 mt-0.5">{row.desc}</span>
              </span>
              <span
                className={cn(
                  "relative w-10 h-5.5 rounded-none border transition-colors shrink-0",
                  on
                    ? "bg-emerald-500/30 border-emerald-500/50"
                    : "bg-zinc-800 border-zinc-700"
                )}
                style={{ height: "1.375rem" }}
              >
                <span
                  className={cn(
                    "absolute top-1/2 -translate-y-1/2 w-4 h-4 rounded-none transition-all",
                    on ? "left-[1.3rem] bg-emerald-400" : "left-0.5 bg-zinc-500"
                  )}
                />
              </span>
            </button>
          );
        })}
      </div>

      <div className="space-y-3 border-t border-zinc-800 pt-4">
        <div>
          <h4 className="font-semibold text-white">Kubernetes alerts</h4>
          <p className="mt-1 text-xs text-zinc-400">
            Alerts come from the cluster watcher. A desktop notification is sent for each fresh alert.
          </p>
        </div>

        <div className="divide-y divide-zinc-800 rounded-none border border-zinc-800">
          <div className="px-3 py-2.5">
            <Switch
              label="Kubernetes alerts"
              checked={k8s.enabled}
              disabled={!mutable || busy}
              onChange={(next) => saveK8s({ ...k8s, enabled: next }, next ? "Kubernetes alerts enabled" : "Kubernetes alerts off")}
            />
            <p className="mt-0.5 text-xs text-zinc-500">Master switch for every alert type below.</p>
          </div>
          {K8S_ROWS.map((row) => (
            <div key={row.key} className="px-3 py-2.5">
              <Switch
                label={row.label}
                checked={k8s[row.key]}
                disabled={!mutable || busy || !k8s.enabled}
                onChange={(next) => saveK8s({ ...k8s, [row.key]: next }, `${row.label} ${next ? "enabled" : "muted"}`)}
              />
              <p className="mt-0.5 text-xs text-zinc-500">{row.desc}</p>
            </div>
          ))}
        </div>

        <div className="space-y-2 rounded-none border border-zinc-800 p-3">
          <h5 className="text-xs font-semibold uppercase tracking-wide text-zinc-400">Muted clusters</h5>
          <p className="text-xs text-zinc-500">Clusters listed here never send desktop alerts.</p>
          {k8s.muted_clusters.length === 0 ? (
            <p className="text-xs text-zinc-500">No cluster is muted.</p>
          ) : (
            <ul className="divide-y divide-zinc-800 rounded-none border border-zinc-800">
              {k8s.muted_clusters.map((name) => (
                <li key={name} className="flex items-center justify-between gap-3 px-3 py-2">
                  <span className="truncate font-mono text-sm text-zinc-200">{name}</span>
                  <button
                    type="button"
                    disabled={!mutable || busy}
                    onClick={() => removeMuted(name)}
                    className="rounded-none px-2 py-1 text-xs text-rose-300 hover:bg-rose-500/10 disabled:opacity-40"
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className="flex gap-2">
            <select
              aria-label="Cluster to mute"
              value={addName}
              onChange={(e) => setAddName(e.target.value)}
              disabled={!mutable || busy || unmutedClusters.length === 0}
              className="h-9 min-w-0 flex-1 rounded-none border border-zinc-800 bg-zinc-950 px-3 text-sm text-white focus:border-blue-500 focus:outline-none disabled:opacity-60"
            >
              <option value="">{unmutedClusters.length === 0 ? "No clusters to mute" : "Choose a cluster…"}</option>
              {unmutedClusters.map((c) => (
                <option key={c.name} value={c.name}>
                  {c.name}
                </option>
              ))}
            </select>
            <button
              type="button"
              disabled={!mutable || busy || !addName}
              onClick={() => void addMuted()}
              className="h-9 rounded-none border border-zinc-700 bg-zinc-800 px-4 text-sm text-zinc-100 hover:bg-zinc-700 disabled:opacity-40"
            >
              Add
            </button>
          </div>
        </div>
      </div>

      {!prefs && (
        <p className="text-xs text-zinc-500">Loading notification settings…</p>
      )}
    </Card>
  );
};
