import React, { useEffect, useRef } from "react";
import {
  CheckCircle2,
  Circle,
  Loader2,
  Terminal,
  X,
  XCircle,
} from "lucide-react";
import type { RunStatus, UpdateRun } from "../../lib/binaryUpdateModel";

export type { RunStatus, UpdateRun } from "../../lib/binaryUpdateModel";

function elapsed(run: UpdateRun): string {
  const ms = (run.finishedAt ?? Date.now()) - run.startedAt;
  const s = Math.max(0, Math.round(ms / 1000));
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
}

const STATUS_COPY: Record<RunStatus, string> = {
  running: "Updating",
  done: "Complete",
  error: "Failed",
};

const STATUS_STYLE: Record<RunStatus, string> = {
  running: "bg-sky-500/10 text-sky-300 border-sky-500/30",
  done: "bg-emerald-500/10 text-emerald-300 border-emerald-500/30",
  error: "bg-red-500/10 text-red-300 border-red-500/30",
};

function StepIcon({
  index,
  active,
  status,
}: {
  index: number;
  active: number;
  status: RunStatus;
}) {
  if (status === "error" && index === active) {
    return <XCircle className="w-4 h-4 text-red-400 shrink-0" />;
  }
  if (status === "done" || index < active) {
    return <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0" />;
  }
  if (index === active) {
    return <Loader2 className="w-4 h-4 text-sky-400 animate-spin shrink-0" />;
  }
  return <Circle className="w-4 h-4 text-zinc-600 shrink-0" />;
}

/**
 * Step checklist + live log for one update run. Every binary updater renders
 * the same panel (through BinaryUpdatePanel), so the process looks identical
 * whether it is the engine, CLI, compose, caddy or kind being replaced.
 */
export const UpdateProgressPanel: React.FC<{
  run: UpdateRun;
  onDismiss?: () => void;
  className?: string;
  /** Rendered inside a BinaryUpdatePanel: no outer frame, a divider on top. */
  embedded?: boolean;
}> = ({ run, onDismiss, className, embedded = false }) => {
  const logRef = useRef<HTMLDivElement>(null);
  const terminal = run.status !== "running";
  const active = run.step ?? 0;
  const percent = Math.max(
    0,
    Math.min(100, Math.round((run.fraction ?? 0) * 100))
  );

  useEffect(() => {
    if (logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [run.logs.length, run.status]);

  return (
    <div
      className={`overflow-hidden ${
        embedded ? "border-t border-zinc-800 bg-zinc-950/40" : "border border-zinc-800 bg-zinc-950/70"
      } ${className ?? ""}`}
    >
      <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-800/80">
        <Terminal className="w-4 h-4 text-zinc-400 shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-sm font-medium text-zinc-100 truncate">
              {run.version ? `v${run.version.replace(/^v/, "")}` : "Update run"}
            </span>
            <span
              className={`text-[10px] font-semibold uppercase tracking-wider px-1.5 py-0.5 border ${
                STATUS_STYLE[run.status]
              }`}
              role="status"
              aria-live="polite"
            >
              {STATUS_COPY[run.status]}
            </span>
            {run.status === "running" && run.fraction != null && (
              <span className="text-[11px] font-mono text-zinc-400">
                {percent}%
              </span>
            )}
            <span className="text-[11px] font-mono text-zinc-500">{elapsed(run)}</span>
          </div>
          <p className="text-[11px] text-zinc-500 mt-0.5 truncate">
            {run.error ?? run.detail ?? run.label ?? "Waiting for the first step…"}
          </p>
        </div>
        {terminal && onDismiss && (
          <button
            type="button"
            onClick={onDismiss}
            aria-label="Dismiss update log"
            className="p-1 text-zinc-500 hover:text-zinc-200 hover:bg-zinc-800 transition-colors shrink-0"
          >
            <X className="w-4 h-4" />
          </button>
        )}
      </div>

      <div
        className="h-1 bg-zinc-900"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-label="Update progress"
      >
        <div
          className={`h-full transition-[width] duration-300 ease-out ${
            run.status === "error"
              ? "bg-red-500"
              : run.status === "done"
                ? "bg-emerald-500"
                : "bg-sky-500"
          }`}
          style={{ width: `${run.status === "done" ? 100 : percent}%` }}
        />
      </div>

      {run.plan.length > 0 && (
        <ol className="px-4 py-3 grid grid-cols-1 sm:grid-cols-2 gap-x-4 gap-y-1.5">
          {run.plan.map((step, index) => (
            <li
              key={`${index}-${step}`}
              className={`flex items-center gap-2 text-xs ${
                index === active && run.status !== "done"
                  ? "text-zinc-100"
                  : index < active || run.status === "done"
                    ? "text-zinc-400"
                    : "text-zinc-600"
              }`}
            >
              <StepIcon index={index} active={active} status={run.status} />
              <span className="truncate">{step}</span>
            </li>
          ))}
        </ol>
      )}

      <div
        ref={logRef}
        className="mx-4 mb-3 max-h-44 overflow-y-auto bg-black/60 border border-zinc-800/80 px-3 py-2 font-mono text-[11px] leading-relaxed select-text"
        aria-label="Update log"
      >
        {run.logs.length === 0 ? (
          <p className="text-zinc-600">Waiting for the first step…</p>
        ) : (
          run.logs.map((line, index) => (
            <div key={`${line.at}-${index}`} className="flex gap-2">
              <span className="text-zinc-600 shrink-0">{line.at}</span>
              <span className="text-zinc-300 break-all">{line.text}</span>
            </div>
          ))
        )}
      </div>

      {run.status === "error" && run.error && (
        <div className="mx-4 mb-3 border border-red-500/30 bg-red-500/5 px-3 py-2">
          <p className="text-xs text-red-300 break-words">{run.error}</p>
        </div>
      )}
    </div>
  );
};
