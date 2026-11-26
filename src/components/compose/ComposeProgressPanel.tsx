import React, { useEffect, useRef, useState } from "react";
import { CheckCircle2, ChevronDown, ChevronUp, Loader2, X, XCircle } from "lucide-react";
import { cn } from "../../lib/utils";

export interface ComposeOp {
  id: string;
  project: string;
  title: string;
  status: "running" | "ok" | "error";
  lines: string[];
  error?: string;
  started: number;
  finished?: number;
}

function elapsed(op: ComposeOp): string {
  const ms = (op.finished ?? Date.now()) - op.started;
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
}

const OpRow: React.FC<{ op: ComposeOp; open: boolean; onToggle: () => void; onDismiss: () => void }> = ({
  op,
  open,
  onToggle,
  onDismiss,
}) => {
  const logRef = useRef<HTMLPreElement>(null);
  useEffect(() => {
    if (open && logRef.current) logRef.current.scrollTop = logRef.current.scrollHeight;
  }, [open, op.lines.length]);
  const last = op.error ?? op.lines[op.lines.length - 1] ?? "Starting…";
  return (
    <div className="border-b border-zinc-800 last:border-b-0">
      <div className="flex items-center gap-2 px-3 py-2">
        {op.status === "running" ? (
          <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-sky-400" aria-label="Running" />
        ) : op.status === "ok" ? (
          <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-400" aria-label="Done" />
        ) : (
          <XCircle className="h-3.5 w-3.5 shrink-0 text-rose-400" aria-label="Failed" />
        )}
        <button type="button" onClick={onToggle} className="min-w-0 flex-1 text-left">
          <div className="flex items-baseline gap-2">
            <span className="truncate text-xs font-medium text-zinc-100">{op.title}</span>
            <span className="shrink-0 font-mono text-[10px] text-zinc-500">{elapsed(op)}</span>
          </div>
          <p className={cn("truncate font-mono text-[10px]", op.status === "error" ? "text-rose-300" : "text-zinc-500")}>
            {last}
          </p>
        </button>
        <button
          type="button"
          onClick={onToggle}
          title={open ? "Hide output" : "Show output"}
          aria-label={open ? "Hide output" : "Show output"}
          className="p-1 text-zinc-500 hover:text-zinc-200"
        >
          {open ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronUp className="h-3.5 w-3.5" />}
        </button>
        {op.status !== "running" && (
          <button
            type="button"
            onClick={onDismiss}
            title="Dismiss"
            aria-label="Dismiss"
            className="p-1 text-zinc-500 hover:text-zinc-200"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>
      {open && (
        <pre
          ref={logRef}
          className="max-h-56 overflow-auto whitespace-pre-wrap break-all border-t border-zinc-800 bg-zinc-950 px-3 py-2 font-mono text-[10px] leading-4 text-zinc-400"
        >
          {op.lines.length ? op.lines.join("\n") : "No output yet."}
          {op.error && `\n${op.error}`}
        </pre>
      )}
    </div>
  );
};

/**
 * Compose operations of this session, docked bottom-right: a spinner while
 * `compose` runs, its live output on demand, and the result once it ends.
 */
export const ComposeProgressPanel: React.FC<{
  ops: ComposeOp[];
  onDismiss: (id: string) => void;
  onClear: () => void;
}> = ({ ops, onDismiss, onClear }) => {
  const [collapsed, setCollapsed] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [, tick] = useState(0);
  const running = ops.filter((o) => o.status === "running").length;

  // Keep the elapsed time moving while something runs.
  useEffect(() => {
    if (!running) return;
    const t = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, [running]);

  if (ops.length === 0) return null;
  const failed = ops.filter((o) => o.status === "error").length;

  return (
    <div className="fixed bottom-4 right-4 z-40 w-[26rem] max-w-[calc(100vw-2rem)] border border-zinc-700 bg-zinc-900 shadow-2xl">
      <div className="flex items-center gap-2 border-b border-zinc-800 bg-zinc-900/80 px-3 py-1.5">
        {running ? (
          <Loader2 className="h-3.5 w-3.5 animate-spin text-sky-400" aria-hidden="true" />
        ) : failed ? (
          <XCircle className="h-3.5 w-3.5 text-rose-400" aria-hidden="true" />
        ) : (
          <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400" aria-hidden="true" />
        )}
        <span className="flex-1 text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
          Compose operations
          {running > 0 && <span className="ml-1 normal-case text-sky-300">· {running} running</span>}
        </span>
        {ops.some((o) => o.status !== "running") && (
          <button type="button" onClick={onClear} className="text-[11px] text-zinc-500 hover:text-zinc-200">
            Clear finished
          </button>
        )}
        <button
          type="button"
          onClick={() => setCollapsed((v) => !v)}
          title={collapsed ? "Expand" : "Collapse"}
          aria-label={collapsed ? "Expand" : "Collapse"}
          className="p-1 text-zinc-500 hover:text-zinc-200"
        >
          {collapsed ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
        </button>
      </div>
      {!collapsed && (
        <div className="max-h-[50vh] overflow-y-auto">
          {[...ops].reverse().map((op) => (
            <OpRow
              key={op.id}
              op={op}
              open={open === op.id}
              onToggle={() => setOpen((cur) => (cur === op.id ? null : op.id))}
              onDismiss={() => onDismiss(op.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
};
