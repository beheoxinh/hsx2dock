import React, { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, disposeListener } from "../../lib/api";
import { Skeleton } from "../ui/Skeleton";
import { cn, formatRelativeTime } from "../../lib/utils";
import {
  ALERTS_REFRESH_MS,
  alertKindLabel,
  alertTooltip,
  highestSeverity,
  sortAlerts,
} from "../../lib/clusterAlerts";
import type { ClusterAlert, ClusterAlertsEvent } from "../../types";

interface Snapshot {
  cluster: string;
  alerts: ClusterAlert[];
}

/**
 * Active incident alerts for one kind cluster. Refreshes on an interval and
 * applies pushed `cluster-alerts` events for this cluster.
 */
export const AlertsPanel: React.FC<{ cluster: string }> = ({ cluster }) => {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [failed, setFailed] = useState<{ cluster: string; message: string } | null>(null);

  useEffect(() => {
    let cancelled = false;
    let inFlight = false;

    const load = () => {
      if (inFlight || document.visibilityState === "hidden") return;
      inFlight = true;
      api
        .listClusterAlerts(cluster)
        .then((alerts) => {
          if (!cancelled) setSnapshot({ cluster, alerts });
        })
        .catch((e: unknown) => {
          if (!cancelled) setFailed({ cluster, message: String(e) });
        })
        .finally(() => {
          inFlight = false;
        });
    };

    load();
    const timer = window.setInterval(load, ALERTS_REFRESH_MS);
    const onVisible = () => {
      if (document.visibilityState === "visible") load();
    };
    document.addEventListener("visibilitychange", onVisible);

    const unlisten = disposeListener(
      listen<ClusterAlertsEvent>("cluster-alerts", (e) => {
        if (cancelled || e.payload.cluster !== cluster) return;
        setSnapshot({ cluster, alerts: e.payload.alerts });
      })
    );

    return () => {
      cancelled = true;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
      unlisten();
    };
  }, [cluster]);

  const current = snapshot?.cluster === cluster ? snapshot.alerts : null;
  const error = failed?.cluster === cluster ? failed.message : null;
  const sorted = useMemo(() => (current ? sortAlerts(current) : []), [current]);

  if (current === null && error === null) {
    return (
      <div className="space-y-2">
        <Skeleton className="h-12 w-full rounded-none" />
        <Skeleton className="h-12 w-full rounded-none" />
      </div>
    );
  }

  return (
    <div className="rounded-none border border-zinc-800 bg-zinc-950">
      <div className="flex items-center justify-between border-b border-zinc-800 px-3 py-2">
        <h3 className="text-sm font-medium text-white">Alerts</h3>
        <span className="text-xs tabular-nums text-zinc-500">{sorted.length} active</span>
      </div>

      {error && (
        <div className="border-b border-rose-900/60 bg-rose-950/30 px-3 py-2 text-xs text-rose-300">
          Could not load alerts: {error}
        </div>
      )}

      {sorted.length === 0 ? (
        <div className="px-4 py-6 text-center text-sm text-zinc-500">No active alerts.</div>
      ) : (
        <ul className="divide-y divide-zinc-800">
          {sorted.map((a) => {
            const isError = a.severity === "error";
            return (
              <li key={a.key} className="flex items-center gap-3 px-3 py-2 text-sm">
                <span
                  role="img"
                  aria-label={isError ? "Error" : "Warning"}
                  title={isError ? "Error" : "Warning"}
                  className={cn(
                    "h-2 w-2 shrink-0 rounded-none",
                    isError ? "bg-rose-500" : "bg-amber-400"
                  )}
                />
                <span className="w-32 shrink-0 truncate font-medium text-white">
                  {alertKindLabel(a.kind)}
                </span>
                <span className="min-w-0 max-w-[16rem] shrink truncate font-mono text-xs text-zinc-300">
                  {a.object}
                </span>
                <span className="w-28 shrink-0 truncate text-xs text-zinc-500">
                  {a.namespace ?? "cluster-wide"}
                </span>
                <span className="min-w-0 flex-1 truncate text-xs text-zinc-400" title={a.message}>
                  {a.message}
                </span>
                <span className="shrink-0 text-xs tabular-nums text-zinc-500">
                  {formatRelativeTime(a.since)}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
};

/** Compact count badge for cluster cards. Renders nothing when there are no alerts. */
export const AlertBadge: React.FC<{ alerts: readonly ClusterAlert[] }> = ({ alerts }) => {
  if (alerts.length === 0) return null;
  const severity = highestSeverity(alerts);
  const isError = severity === "error";
  const label = `${alerts.length} active alert${alerts.length === 1 ? "" : "s"}`;
  return (
    <span
      role="img"
      aria-label={label}
      title={alertTooltip(alerts)}
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1.5 rounded-none border px-2 text-xs font-medium tabular-nums",
        isError
          ? "border-rose-700/70 bg-rose-950/60 text-rose-300"
          : "border-amber-700/70 bg-amber-950/50 text-amber-300"
      )}
    >
      <span className={cn("h-1.5 w-1.5 rounded-none", isError ? "bg-rose-500" : "bg-amber-400")} />
      {alerts.length}
    </span>
  );
};
