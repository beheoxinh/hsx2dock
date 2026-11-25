import React, { useEffect } from "react";
import { X } from "lucide-react";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import { Button } from "../ui/Button";
import { cn } from "../../lib/utils";
import type { ComposeProjectStatus, HostPort } from "../../lib/composeProject";

export type BadgeTone = "neutral" | "info" | "warning" | "danger" | "success" | "accent" | "muted";

export const BADGE_TONE: Record<BadgeTone, string> = {
  neutral: "text-zinc-300 bg-zinc-800/80 border-zinc-700",
  info: "text-sky-300 bg-sky-500/10 border-sky-500/30",
  warning: "text-amber-300 bg-amber-500/10 border-amber-500/40",
  danger: "text-rose-300 bg-rose-500/10 border-rose-500/40",
  success: "text-emerald-300 bg-emerald-500/10 border-emerald-500/30",
  accent: "text-indigo-300 bg-indigo-500/10 border-indigo-500/30",
  muted: "text-zinc-500 bg-zinc-900 border-zinc-800",
};

export const STATUS_STYLE: Record<
  ComposeProjectStatus,
  { icon: string; box: string; label: string; dot: string }
> = {
  running: {
    icon: "text-emerald-400",
    box: "bg-emerald-500/10 border-emerald-500/40",
    label: "Running",
    dot: "bg-emerald-400",
  },
  partial: {
    icon: "text-amber-400",
    box: "bg-amber-500/10 border-amber-500/40",
    label: "Partially running",
    dot: "bg-amber-400",
  },
  stopped: {
    icon: "text-zinc-500",
    box: "bg-zinc-800/60 border-zinc-700",
    label: "Stopped",
    dot: "bg-zinc-500",
  },
  error: {
    icon: "text-rose-400",
    box: "bg-rose-500/10 border-rose-500/40",
    label: "Compose file error",
    dot: "bg-rose-500",
  },
};

/** Square icon badge; the hint is the tooltip and the accessible name. */
export const IconBadge: React.FC<{ hint: string; tone: BadgeTone; children: React.ReactNode }> = ({
  hint,
  tone,
  children,
}) => (
  <span
    role="img"
    aria-label={hint}
    title={hint}
    className={cn(
      "inline-flex h-6 w-6 shrink-0 cursor-help items-center justify-center border",
      BADGE_TONE[tone]
    )}
  >
    {children}
  </span>
);

/** Compact icon button; the title is also its accessible name. */
export const IconAction: React.FC<{
  title: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  className?: string;
  children: React.ReactNode;
}> = ({ title, onClick, disabled, danger, className, children }) => (
  <Button
    variant="outline"
    size="icon"
    onClick={(e) => {
      e.stopPropagation();
      onClick();
    }}
    disabled={disabled}
    title={title}
    aria-label={title}
    className={cn(
      "h-8 w-8 text-zinc-300",
      danger && "hover:text-rose-300 hover:border-rose-500/50",
      className
    )}
  >
    {children}
  </Button>
);

/** One cell of an info strip; borders between cells come from the parent. */
export const InfoCell: React.FC<{
  label: string;
  children: React.ReactNode;
  title?: string;
  className?: string;
}> = ({ label, children, title, className }) => (
  <div className={cn("min-w-0 px-3 py-2", className)} title={title}>
    <dt className="text-[10px] uppercase tracking-wider text-zinc-500">{label}</dt>
    <dd className="mt-0.5 font-mono text-xs text-zinc-200 min-w-0">{children}</dd>
  </div>
);

/** Open a URL in the desktop browser; failures are logged, never thrown. */
export function openUrl(url: string): void {
  openExternal(url).catch((err) => console.warn(`Could not open ${url}:`, err));
}

/** A published port as a link to http://localhost:PORT (tcp) or a plain chip. */
export const PortChip: React.FC<{ port: HostPort; small?: boolean }> = ({ port, small }) => {
  const chip = cn(
    "inline-flex items-center gap-1 border font-mono",
    small ? "px-1 py-0 text-[10px]" : "px-1.5 py-0.5 text-[10px]"
  );
  const body = (
    <>
      <span>{port.published}</span>
      <span className="text-zinc-600">→</span>
      <span className="text-zinc-400">
        {port.target}
        {port.protocol !== "tcp" ? `/${port.protocol}` : ""}
      </span>
    </>
  );
  if (port.protocol !== "tcp") {
    return (
      <span className={cn(chip, "border-zinc-700 bg-zinc-800/80 text-zinc-300")} title={`Host port ${port.published}/${port.protocol}`}>
        {body}
      </span>
    );
  }
  const url = `http://localhost:${port.published}`;
  return (
    <a
      href={url}
      target="_blank"
      rel="noreferrer"
      onClick={(e) => {
        // A target=_blank anchor does not leave the Tauri webview; hand the
        // URL to the desktop's opener instead.
        e.preventDefault();
        e.stopPropagation();
        openUrl(url);
      }}
      className={cn(
        chip,
        "border-sky-500/30 bg-sky-500/10 text-sky-300 hover:bg-sky-500/20 hover:border-sky-500/50 transition-colors"
      )}
      title={`Open ${url} (container port ${port.target})`}
    >
      {body}
    </a>
  );
};

/** Centered dialog with a title bar; Escape and the backdrop close it. */
export const DialogShell: React.FC<{
  title: string;
  subtitle?: string;
  icon?: React.ReactNode;
  onClose: () => void;
  footer?: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}> = ({ title, subtitle, icon, onClose, footer, className, children }) => {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/70 p-4" onClick={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onClick={(e) => e.stopPropagation()}
        className={cn("flex max-h-[85vh] w-full max-w-lg flex-col border border-zinc-800 bg-zinc-950 shadow-2xl", className)}
      >
        <div className="flex items-center gap-3 border-b border-zinc-800 px-4 py-3">
          {icon && (
            <span className="flex h-8 w-8 shrink-0 items-center justify-center border border-zinc-700 bg-zinc-900 text-zinc-300">
              {icon}
            </span>
          )}
          <div className="min-w-0 flex-1">
            <h2 className="truncate text-sm font-semibold text-white">{title}</h2>
            {subtitle && <p className="truncate text-xs text-zinc-500">{subtitle}</p>}
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="p-1 text-zinc-400 hover:bg-zinc-800 hover:text-white"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto p-4">{children}</div>
        {footer && (
          <div className="flex items-center justify-end gap-2 border-t border-zinc-800 px-4 py-3">{footer}</div>
        )}
      </div>
    </div>
  );
};

export const inputClass =
  "h-8 w-full border border-zinc-700 bg-zinc-900 px-2.5 text-xs text-zinc-100 placeholder-zinc-500 outline-none focus:border-blue-500";
