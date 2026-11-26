import React, { useCallback, useEffect, useRef, useState } from "react";
import { AlertCircle, Loader2 } from "lucide-react";
import { api } from "../../lib/api";
import { cn } from "../../lib/utils";
import type { HealthDetails as Health, HealthRun } from "../../types";

/** Refresh period while the details stay open. */
const POLL_MS = 10000;
/** Recent runs shown before "show all". */
const RUNS_SHOWN = 5;

function fmtMs(ms: number): string {
  if (!ms) return "—";
  return ms < 1000 ? `${ms}ms` : `${Number((ms / 1000).toFixed(1))}s`;
}

function fmtTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}

/** The healthcheck command without the CMD / CMD-SHELL prefix Docker stores. */
function fmtTest(test: string[]): string {
  if (test.length === 0) return "—";
  const [first, ...rest] = test;
  return first === "CMD-SHELL" || first === "CMD" ? rest.join(" ") : test.join(" ");
}

const STATUS_TONE: Record<string, string> = {
  healthy: "text-emerald-300",
  unhealthy: "text-rose-300",
  starting: "text-sky-300",
};

const RunRow: React.FC<{ run: HealthRun }> = ({ run }) => (
  <li className="border border-zinc-800 bg-zinc-950">
    <div className="flex items-center gap-3 border-b border-zinc-800 px-2 py-1 font-mono text-[11px] text-zinc-400">
      <span className="truncate">{fmtTime(run.start)}</span>
      <span className={cn("ml-auto shrink-0", run.exit_code === 0 ? "text-emerald-300" : "text-rose-300")}>
        exit {run.exit_code}
      </span>
    </div>
    <pre className="max-h-32 overflow-auto whitespace-pre-wrap break-all p-2 font-mono text-[11px] text-zinc-300">
      {run.output.trim() || "(no output)"}
    </pre>
  </li>
);

/**
 * Healthcheck of one container: status, streak, command, timings and the
 * recent runs. Reloads every POLL_MS while mounted (the row is expanded).
 */
export const HealthDetails: React.FC<{ containerId: string }> = ({ containerId }) => {
  const [data, setData] = useState<Health | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);
  // Container id of the request in flight: a slow `docker inspect` must not stack polls.
  const pending = useRef<string | null>(null);

  const load = useCallback(async () => {
    if (pending.current === containerId) return;
    pending.current = containerId;
    try {
      setData(await api.containerHealth(containerId));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      if (pending.current === containerId) pending.current = null;
    }
  }, [containerId]);

  useEffect(() => {
    void load();
    const t = window.setInterval(() => {
      if (!document.hidden) void load();
    }, POLL_MS);
    return () => window.clearInterval(t);
  }, [load]);

  if (!data) {
    return error ? (
      <p className="flex items-center gap-1.5 text-[11px] text-rose-300">
        <AlertCircle className="h-3 w-3" aria-hidden="true" />
        {error}
      </p>
    ) : (
      <p className="flex items-center gap-1.5 text-[11px] text-zinc-500">
        <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />
        Loading healthcheck…
      </p>
    );
  }

  const runs = data.runs ?? [];
  const shown = showAll ? runs : runs.slice(0, RUNS_SHOWN);

  return (
    <div className="flex flex-col gap-2 text-xs">
      {error && <p className="text-[11px] text-rose-300">Refresh failed: {error}</p>}
      <dl className="grid grid-cols-2 border border-zinc-800 sm:grid-cols-5 sm:divide-x sm:divide-zinc-800">
        <div className="px-3 py-1.5">
          <dt className="text-[10px] uppercase tracking-wider text-zinc-500">Status</dt>
          <dd className={cn("font-mono", STATUS_TONE[data.status] ?? "text-zinc-400")}>{data.status || "none"}</dd>
        </div>
        <div className="px-3 py-1.5">
          <dt className="text-[10px] uppercase tracking-wider text-zinc-500">Failing streak</dt>
          <dd className="font-mono text-zinc-200">{data.failing_streak}</dd>
        </div>
        <div className="px-3 py-1.5">
          <dt className="text-[10px] uppercase tracking-wider text-zinc-500">Interval</dt>
          <dd className="font-mono text-zinc-200">{fmtMs(data.interval_ms)}</dd>
        </div>
        <div className="px-3 py-1.5">
          <dt className="text-[10px] uppercase tracking-wider text-zinc-500">Timeout</dt>
          <dd className="font-mono text-zinc-200">{fmtMs(data.timeout_ms)}</dd>
        </div>
        <div className="px-3 py-1.5">
          <dt className="text-[10px] uppercase tracking-wider text-zinc-500">Retries</dt>
          <dd className="font-mono text-zinc-200">{data.retries}</dd>
        </div>
      </dl>
      <div>
        <p className="mb-1 text-[10px] uppercase tracking-wider text-zinc-500">Test command</p>
        <pre className="overflow-x-auto border border-zinc-800 bg-zinc-950 p-2 font-mono text-[11px] text-zinc-300">
          {fmtTest(data.test ?? [])}
        </pre>
      </div>
      <div>
        <div className="mb-1 flex items-center justify-between">
          <p className="text-[10px] uppercase tracking-wider text-zinc-500">Recent runs (newest first)</p>
          {runs.length > RUNS_SHOWN && (
            <button
              type="button"
              onClick={() => setShowAll((v) => !v)}
              className="text-[11px] text-indigo-300 hover:text-indigo-200"
            >
              {showAll ? "Show fewer" : `Show all (${runs.length})`}
            </button>
          )}
        </div>
        {runs.length === 0 ? (
          <p className="text-[11px] text-zinc-500">No runs recorded yet.</p>
        ) : (
          <ul className="flex flex-col gap-1">
            {shown.map((r, i) => (
              <RunRow key={`${r.start}-${i}`} run={r} />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
};
