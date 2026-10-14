import React from "react";
import { cn } from "../../lib/utils";

interface StatusBadgeProps {
  status: string;
  pulse?: boolean;
  className?: string;
}

export const StatusBadge: React.FC<StatusBadgeProps> = ({ status, pulse = false, className }) => {
  const s = status.toLowerCase();

  let colorClasses = "bg-zinc-800 text-zinc-300 border-zinc-700/60";
  let dotColor = "bg-zinc-400";
  let shouldPulse = pulse;

  if (s.includes("running") || s === "up") {
    colorClasses = "bg-emerald-500/10 text-emerald-400 border-emerald-500/30";
    dotColor = "bg-emerald-400";
    shouldPulse = true;
  } else if (s.includes("pause")) {
    colorClasses = "bg-amber-500/10 text-amber-400 border-amber-500/30";
    dotColor = "bg-amber-400";
  } else if (s.includes("exit") || s.includes("stop") || s === "dead") {
    colorClasses = "bg-zinc-800/80 text-zinc-400 border-zinc-700/40";
    dotColor = "bg-zinc-500";
  } else if (s.includes("restart") || s.includes("starting")) {
    colorClasses = "bg-blue-500/10 text-blue-400 border-blue-500/30";
    dotColor = "bg-blue-400";
    shouldPulse = true;
  } else if (s.includes("error") || s.includes("fail")) {
    colorClasses = "bg-rose-500/10 text-rose-400 border-rose-500/30";
    dotColor = "bg-rose-500";
  }

  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium border backdrop-blur-sm",
        colorClasses,
        className
      )}
    >
      <span className="relative flex h-2 w-2">
        {shouldPulse && (
          <span
            className={cn(
              "animate-ping absolute inline-flex h-full w-full rounded-full opacity-75",
              dotColor
            )}
          />
        )}
        <span className={cn("relative inline-flex rounded-full h-2 w-2", dotColor)} />
      </span>
      <span className="capitalize">{status}</span>
    </span>
  );
};
