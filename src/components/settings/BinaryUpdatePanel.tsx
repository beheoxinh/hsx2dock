import React, { useEffect, useState } from "react";
import { AlertTriangle, ArrowRight, DownloadCloud, Loader2, RefreshCw } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { UpdateProgressPanel } from "./UpdateProgressPanel";
import {
  ago,
  bareVersion,
  binaryStatus,
  canApply,
  type BinaryStatusKind,
} from "../../lib/binaryUpdateModel";
import { checkBinary, dismissBinaryRun, installBinary, useBinaryUpdate } from "../../lib/binaryUpdates";
import type { UpdateTarget } from "../../types";

const BADGE: Record<BinaryStatusKind, string> = {
  updating: "border-sky-500/40 bg-sky-500/10 text-sky-300",
  checking: "border-zinc-700 bg-zinc-800/60 text-zinc-400",
  update: "border-amber-500/40 bg-amber-500/10 text-amber-300",
  missing: "border-zinc-700 bg-zinc-800/60 text-zinc-300",
  current: "border-emerald-500/40 bg-emerald-500/10 text-emerald-300",
  failed: "border-rose-500/40 bg-rose-500/10 text-rose-300",
  unknown: "border-zinc-700 bg-zinc-800/60 text-zinc-500",
};

export interface BinaryUpdatePanelProps {
  target: UpdateTarget;
  /** Display name: "Docker Engine", "Caddy", … */
  name: string;
  icon: React.ComponentType<{ className?: string }>;
  iconClass?: string;
  /** One quiet line under the versions (install path, what it is for). */
  description?: React.ReactNode;
  /** Install/update is impossible right now; the reason is shown instead. */
  blockedReason?: string | null;
  /** What applying does beyond replacing the file, shown before it starts. */
  impact?: string;
  /** Asked before installing; resolve false to cancel. */
  confirm?: (version: string | null, installed: boolean) => Promise<boolean>;
  /** Check (respecting the cache) when the panel first shows. Default true. */
  autoCheck?: boolean;
}

/**
 * The one update panel every bundled binary uses: name and status badge,
 * installed → latest, Check / Install / Update, and the live run (steps,
 * progress, log) underneath. State lives in the binaryUpdates store, so an
 * update keeps its progress while the page remounts or the engine restarts.
 */
export const BinaryUpdatePanel: React.FC<BinaryUpdatePanelProps> = ({
  target,
  name,
  icon: Icon,
  iconClass = "text-zinc-300",
  description,
  blockedReason,
  impact,
  confirm,
  autoCheck = true,
}) => {
  const toast = useToast();
  const s = useBinaryUpdate(target);
  const status = binaryStatus(s);
  const busy = s.installing || s.run?.status === "running";
  const [, tick] = useState(0);

  useEffect(() => {
    if (autoCheck) void checkBinary(target);
  }, [autoCheck, target]);

  // Keeps "checked 3 min ago" and the run's elapsed time moving.
  useEffect(() => {
    const t = window.setInterval(() => tick((n) => n + 1), busy ? 1000 : 30_000);
    return () => window.clearInterval(t);
  }, [busy]);

  const installed = bareVersion(s.info?.installed);
  const latest = bareVersion(s.info?.latest);
  const applicable = canApply(s) && !blockedReason;

  const onCheck = async () => {
    try {
      const info = await checkBinary(target, { force: true });
      if (!info) return;
      if (!info.installed) toast.info(`${name} is not installed`, `Latest release: ${bareVersion(info.latest)}`);
      else if (info.updateAvailable)
        toast.info(`${name} update available`, `${bareVersion(info.installed)} → ${bareVersion(info.latest)}`);
      else toast.success(`${name} is up to date`, `Version ${bareVersion(info.installed)}`);
    } catch (e) {
      toast.error(`${name}: update check failed`, String(e instanceof Error ? e.message : e));
    }
  };

  const onApply = async () => {
    if (confirm && !(await confirm(latest, !!installed))) return;
    try {
      const v = await installBinary(target);
      if (v !== null) toast.success(installed ? `${name} updated` : `${name} installed`, `Now on ${bareVersion(v)}`);
    } catch (e) {
      toast.error(`${name}: ${installed ? "update" : "install"} failed`, String(e instanceof Error ? e.message : e));
    }
  };

  const versionLine = !s.info ? (
    <span className="text-zinc-500">
      {s.checking ? "Asking the release channel…" : s.checkError ? s.checkError : "Not checked yet"}
    </span>
  ) : !installed ? (
    <span>
      <span className="text-zinc-500">not installed</span>
      <span className="text-zinc-600"> · latest </span>
      <span className="text-zinc-200">{latest}</span>
    </span>
  ) : s.info.updateAvailable ? (
    <span className="inline-flex items-center gap-1.5">
      <span className="text-zinc-400">{installed}</span>
      <ArrowRight className="h-3 w-3 text-amber-400" aria-label="to" />
      <span className="text-amber-200">{latest}</span>
    </span>
  ) : (
    <span className="text-zinc-300">{installed}</span>
  );

  return (
    <div className="border border-zinc-800 bg-zinc-950/60">
      <div className="flex items-center gap-3 px-3 py-2.5">
        <div className="flex h-8 w-8 shrink-0 items-center justify-center border border-zinc-800 bg-zinc-900">
          <Icon className={`h-4 w-4 ${iconClass}`} />
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-zinc-100">{name}</span>
            <span
              role="status"
              aria-live="polite"
              className={`inline-flex items-center gap-1 border px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider ${BADGE[status.kind]}`}
              title={status.kind === "failed" ? s.checkError ?? undefined : undefined}
            >
              {(status.kind === "updating" || status.kind === "checking") && (
                <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />
              )}
              {status.label}
            </span>
          </div>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-2 font-mono text-[11px]">
            {versionLine}
            {s.checkedAt > 0 && !busy && (
              <span className="font-sans text-zinc-600">
                · checked {ago(s.checkedAt, Date.now())}
                {s.info && s.checkError && " (last check failed)"}
              </span>
            )}
          </div>
          {description && <div className="mt-0.5 truncate text-[11px] text-zinc-500">{description}</div>}
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          <button
            type="button"
            onClick={onCheck}
            disabled={s.checking || busy}
            title="Check for a new release"
            aria-label={`Check for ${name} updates`}
            className="flex h-8 w-8 items-center justify-center border border-zinc-800 text-zinc-400 transition-colors hover:border-zinc-700 hover:text-zinc-100 disabled:opacity-40"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${s.checking ? "animate-spin" : ""}`} />
          </button>
          {(applicable || busy) && (
            <Button
              type="button"
              variant="primary"
              size="sm"
              disabled={busy}
              onClick={onApply}
              title={impact}
              className="gap-1.5 rounded-none"
            >
              {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <DownloadCloud className="h-3.5 w-3.5" />}
              <span>
                {busy ? (installed ? "Updating…" : "Installing…") : installed ? `Update to ${latest}` : `Install ${latest}`}
              </span>
            </Button>
          )}
        </div>
      </div>

      {blockedReason && canApply(s) && (
        <p className="flex items-center gap-1.5 border-t border-zinc-800 px-3 py-1.5 text-[11px] text-zinc-500">
          <AlertTriangle className="h-3 w-3 shrink-0 text-amber-400" />
          {blockedReason}
        </p>
      )}
      {impact && applicable && !busy && !s.run && (
        <p className="border-t border-zinc-800 px-3 py-1.5 text-[11px] text-zinc-500">{impact}</p>
      )}
      {s.installError && !s.run && (
        <p className="border-t border-rose-500/30 bg-rose-500/5 px-3 py-1.5 text-[11px] text-rose-300 break-words">
          {s.installError}
        </p>
      )}
      {s.run && <UpdateProgressPanel embedded run={s.run} onDismiss={() => dismissBinaryRun(target)} />}
    </div>
  );
};
