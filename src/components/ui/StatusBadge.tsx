import React from "react";
import { cn } from "../../lib/utils";
import type { HealthTone } from "../../lib/clusterHealth";

interface StatusBadgeProps {
  status: string;
  pulse?: boolean;
  className?: string;
  /**
   * Forces a colour instead of inferring one from `status`. Needed for badges
   * whose text carries a count rather than a state word — "0/3 Running" must
   * not read as healthy just because it contains "running".
   */
  tone?: HealthTone;
}

const TONES: Record<HealthTone, { color: string; dot: string }> = {
  neutral: { color: "bg-zinc-800 text-zinc-300 border-zinc-700/60", dot: "bg-zinc-400" },
  success: {
    color: "bg-emerald-500/10 text-emerald-400 border-emerald-500/30",
    dot: "bg-emerald-400",
  },
  warning: {
    color: "bg-amber-500/10 text-amber-400 border-amber-500/30",
    dot: "bg-amber-400",
  },
  danger: { color: "bg-rose-500/10 text-rose-400 border-rose-500/30", dot: "bg-rose-500" },
  info: { color: "bg-blue-500/10 text-blue-400 border-blue-500/30", dot: "bg-blue-400" },
};

export const StatusBadge = React.memo<StatusBadgeProps>(function StatusBadge({
  status,
  pulse = false,
  className,
  tone,
}) {
  const s = status.toLowerCase();

  let colorClasses = TONES.neutral.color;
  let dotColor = TONES.neutral.dot;
  let shouldPulse = pulse;

  if (tone) {
    colorClasses = TONES[tone].color;
    dotColor = TONES[tone].dot;
  } else if (s.includes("running") || s === "up") {
    colorClasses = TONES.success.color;
    dotColor = TONES.success.dot;
    shouldPulse = true;
  } else if (s.includes("pause")) {
    colorClasses = TONES.warning.color;
    dotColor = TONES.warning.dot;
  } else if (s.includes("exit") || s.includes("stop") || s === "dead") {
    colorClasses = "bg-zinc-800/80 text-zinc-400 border-zinc-700/40";
    dotColor = "bg-zinc-500";
  } else if (s.includes("restart") || s.includes("starting")) {
    colorClasses = TONES.info.color;
    dotColor = TONES.info.dot;
    shouldPulse = true;
  } else if (s.includes("error") || s.includes("fail")) {
    colorClasses = TONES.danger.color;
    dotColor = TONES.danger.dot;
  }

  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 px-2.5 py-1 rounded-none text-xs font-medium border",
        colorClasses,
        className
      )}
    >
      <span className="relative flex h-2 w-2">
        <span
          className={cn(
            "relative inline-flex rounded-full h-2 w-2",
            dotColor,
            shouldPulse && (dotColor.includes("emerald") ? "shadow-[0_0_8px_rgba(52,211,153,0.7)]" : "shadow-[0_0_6px_rgba(96,165,250,0.7)]")
          )}
        />
      </span>
      <span className="capitalize">{status}</span>
    </span>
  );
});
