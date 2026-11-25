import React, { useEffect, useMemo, useState } from "react";
import { AreaChart } from "../charts/AreaChart";
import { Button } from "../ui/Button";
import { Skeleton } from "../ui/Skeleton";
import { api } from "../../lib/api";
import { cn, formatRelativeTime } from "../../lib/utils";
import {
  HISTORY_RANGES,
  HISTORY_REFRESH_MS,
  recentHistory,
} from "../../lib/clusterHistory";
import type { HistoryRange, HistorySample } from "../../types";

const EMPTY_HINT = "No samples yet. The app records one sample per minute while it runs.";

const COLOR_CPU = "#38bdf8"; // sky-400
const COLOR_MEM = "#a78bfa"; // violet-400
const COLOR_RUNNING = "#34d399"; // emerald-400
const COLOR_TOTAL = "#a1a1aa"; // zinc-400

interface Loaded {
  key: string;
  samples: HistorySample[];
}

interface Failed {
  key: string;
  message: string;
}

/**
 * CPU, memory and pod history for one kind cluster, with a range switcher.
 * Results that arrive after the cluster or range changed are ignored.
 */
export const HistoryPanel: React.FC<{ cluster: string }> = ({ cluster }) => {
  const [range, setRange] = useState<HistoryRange>("1h");
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [failed, setFailed] = useState<Failed | null>(null);

  const key = `${cluster}\u0000${range}`;

  useEffect(() => {
    let cancelled = false;
    let inFlight = false;

    const load = () => {
      if (inFlight || document.visibilityState === "hidden") return;
      inFlight = true;
      api
        .clusterHistory(cluster, range)
        .then((samples) => {
          if (!cancelled) setLoaded({ key, samples });
        })
        .catch((e: unknown) => {
          if (!cancelled) setFailed({ key, message: String(e) });
        })
        .finally(() => {
          inFlight = false;
        });
    };

    load();
    const timer = window.setInterval(load, HISTORY_REFRESH_MS);
    const onVisible = () => {
      if (document.visibilityState === "visible") load();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [cluster, range, key]);

  const samples = loaded?.key === key ? loaded.samples : null;
  const error = failed?.key === key ? failed.message : null;

  const recent = useMemo(() => (samples ? recentHistory(samples) : null), [samples]);
  const latest = recent && recent.length > 0 ? recent[recent.length - 1] : null;

  const cpuData = recent?.map((s) => s.cpuPercent) ?? [];
  const memData = recent?.map((s) => s.memMb) ?? [];
  const runningData = recent?.map((s) => s.runningPods) ?? [];
  const totalData = recent?.map((s) => s.pods) ?? [];

  const isLoading = samples === null && error === null;
  const isEmpty = recent !== null && recent.length === 0;

  return (
    <div className="rounded-none border border-zinc-800 bg-zinc-950">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-zinc-800 px-3 py-2">
        <h3 className="text-sm font-medium text-white">History</h3>
        <div
          role="group"
          aria-label="History range"
          className="inline-flex border border-zinc-800"
        >
          {HISTORY_RANGES.map((r) => {
            const active = r === range;
            return (
              <Button
                key={r}
                size="sm"
                variant="ghost"
                aria-pressed={active}
                onClick={() => setRange(r)}
                className={cn(
                  "h-7 rounded-none border-0 px-3 text-xs",
                  active ? "bg-zinc-800 text-white" : "text-zinc-500"
                )}
              >
                {r}
              </Button>
            );
          })}
        </div>
      </div>

      {error && (
        <div className="border-b border-rose-900/60 bg-rose-950/30 px-3 py-2 text-xs text-rose-300">
          Could not load history: {error}
        </div>
      )}

      {isLoading && (
        <div className="grid grid-cols-1 divide-y divide-zinc-800 md:grid-cols-3 md:divide-x md:divide-y-0">
          {[0, 1, 2].map((i) => (
            <div key={i} className="p-3">
              <Skeleton className="mb-2 h-3 w-16 rounded-none" />
              <Skeleton className="h-48 w-full rounded-none" />
            </div>
          ))}
        </div>
      )}

      {isEmpty && (
        <div className="px-4 py-8 text-center text-sm text-zinc-500">{EMPTY_HINT}</div>
      )}

      {recent && recent.length > 0 && (
        <>
          <div className="grid grid-cols-1 divide-y divide-zinc-800 md:grid-cols-3 md:divide-x md:divide-y-0">
            <ChartCell title="CPU" value={latest ? `${latest.cpuPercent.toFixed(1)}%` : "-"}>
              <AreaChart
                series={[{ name: "CPU", color: COLOR_CPU, data: cpuData }]}
                yMax={100}
                formatValue={(v) => `${v.toFixed(1)}%`}
                timeHint={range}
              />
            </ChartCell>
            <ChartCell
              title="Memory"
              value={latest ? `${Math.round(latest.memMb)} MiB` : "-"}
            >
              <AreaChart
                series={[{ name: "Memory", color: COLOR_MEM, data: memData }]}
                formatValue={(v) => `${Math.round(v)} MiB`}
                timeHint={range}
              />
            </ChartCell>
            <ChartCell
              title="Pods"
              value={latest ? `${latest.runningPods} / ${latest.pods}` : "-"}
            >
              <AreaChart
                series={[
                  { name: "Running", color: COLOR_RUNNING, data: runningData },
                  { name: "Total", color: COLOR_TOTAL, data: totalData },
                ]}
                formatValue={(v) => String(Math.round(v))}
                timeHint={range}
              />
            </ChartCell>
          </div>

          {latest && (
            <div className="grid grid-cols-2 divide-x divide-zinc-800 border-t border-zinc-800 sm:grid-cols-4">
              <SummaryCell label="CPU" value={`${latest.cpuPercent.toFixed(1)}%`} />
              <SummaryCell label="Memory" value={`${Math.round(latest.memMb)} MiB`} />
              <SummaryCell
                label="Pods running / total"
                value={`${latest.runningPods} / ${latest.pods}`}
              />
              <SummaryCell label="Sampled" value={formatRelativeTime(latest.t)} />
            </div>
          )}
        </>
      )}
    </div>
  );
};

const ChartCell: React.FC<{
  title: string;
  value: string;
  children: React.ReactNode;
}> = ({ title, value, children }) => (
  <div className="min-w-0 p-3">
    <div className="mb-2 flex items-baseline justify-between gap-2">
      <span className="text-xs font-medium text-white">{title}</span>
      <span className="text-[11px] tabular-nums text-zinc-400">{value}</span>
    </div>
    <div className="h-48">{children}</div>
  </div>
);

const SummaryCell: React.FC<{ label: string; value: string }> = ({ label, value }) => (
  <div className="min-w-0 px-3 py-2">
    <div className="truncate text-[11px] uppercase tracking-wider text-zinc-500">{label}</div>
    <div className="truncate text-sm font-medium tabular-nums text-white">{value}</div>
  </div>
);
