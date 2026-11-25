import React, { useEffect, useState } from "react";
import { Bell } from "lucide-react";
import { Card } from "../ui/Card";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";
import { cn } from "../../lib/utils";
import type { NotificationPrefs } from "../../types";

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

/**
 * Desktop-notification preferences. Each toggle saves immediately (the tray
 * menu reads the same config, so the two stay in sync), and saving always
 * re-reads the config first so it can never clobber the Settings page form.
 */
export const NotificationsCard: React.FC = () => {
  const toast = useToast();
  const [prefs, setPrefs] = useState<NotificationPrefs | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api
      .getAppConfig()
      .then((cfg) => setPrefs({ ...DEFAULTS, ...(cfg.notifications ?? {}) }))
      .catch(() => setPrefs(DEFAULTS));
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
                  "relative w-10 h-5.5 rounded-full border transition-colors shrink-0",
                  on
                    ? "bg-emerald-500/30 border-emerald-500/50"
                    : "bg-zinc-800 border-zinc-700"
                )}
                style={{ height: "1.375rem" }}
              >
                <span
                  className={cn(
                    "absolute top-1/2 -translate-y-1/2 w-4 h-4 rounded-full transition-all",
                    on ? "left-[1.3rem] bg-emerald-400" : "left-0.5 bg-zinc-500"
                  )}
                />
              </span>
            </button>
          );
        })}
      </div>

      {!prefs && (
        <p className="text-xs text-zinc-500">Loading notification settings…</p>
      )}
    </Card>
  );
};
