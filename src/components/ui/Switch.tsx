import React from "react";
import { cn } from "../../lib/utils";

/**
 * Small accessible toggle. A `<button role="switch">` so the keyboard and
 * screen readers get the same behaviour as a native checkbox, with an optional
 * inline label on its right.
 */
export const Switch: React.FC<{
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  /** Accessible name; also the tooltip when `title` is omitted. */
  label: string;
  title?: string;
  className?: string;
}> = ({ checked, onChange, disabled, label, title, className }) => (
  <button
    type="button"
    role="switch"
    aria-checked={checked}
    aria-label={label}
    title={title ?? label}
    disabled={disabled}
    onClick={() => onChange(!checked)}
    className={cn(
      "group inline-flex items-center gap-2 rounded-none focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/60 disabled:cursor-not-allowed disabled:opacity-40",
      className
    )}
  >
    <span
      className={cn(
        "relative inline-flex h-4 w-7 shrink-0 items-center rounded-none border transition-colors",
        checked
          ? "border-emerald-400/70 bg-emerald-500/80"
          : "border-zinc-600 bg-zinc-700/60 group-hover:bg-zinc-600/70"
      )}
    >
      <span
        className={cn(
          "h-3 w-3 rounded-none bg-white shadow transition-transform",
          checked ? "translate-x-3" : "translate-x-0.5"
        )}
      />
    </span>
    <span className="text-xs text-zinc-400 group-hover:text-zinc-200">{label}</span>
  </button>
);
