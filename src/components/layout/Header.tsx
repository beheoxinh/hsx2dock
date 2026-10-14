import React from "react";
import { Power, RefreshCw } from "lucide-react";
import { Button } from "../ui/Button";
import { StatusBadge } from "../ui/StatusBadge";
import type { DaemonState } from "../../types";

interface HeaderProps {
  daemonState: DaemonState;
  onToggleEngine: () => void;
  isToggling?: boolean;
  onRefresh?: () => void;
}

export const Header: React.FC<HeaderProps> = ({
  daemonState,
  onToggleEngine,
  isToggling = false,
  onRefresh,
}) => {
  const isRunning = daemonState === "Running";
  const stateStr = typeof daemonState === "string" ? daemonState : "Error";

  return (
    <header className="h-16 px-6 bg-zinc-950/60 backdrop-blur-xl border-b border-zinc-800/80 flex items-center justify-between select-none z-10">
      <div className="flex items-center gap-4">
        <div className="flex items-center gap-2.5">
          <span className="text-xs font-medium text-zinc-400">Docker Daemon:</span>
          <StatusBadge status={stateStr} pulse={isRunning} />
        </div>
      </div>

      <div className="flex items-center gap-3">
        {onRefresh && (
          <Button
            variant="ghost"
            size="icon"
            onClick={onRefresh}
            title="Refresh Data"
            className="text-zinc-400 hover:text-white"
          >
            <RefreshCw className="w-4 h-4" />
          </Button>
        )}

        <Button
          variant={isRunning ? "danger" : "primary"}
          size="sm"
          onClick={onToggleEngine}
          isLoading={isToggling}
          className="gap-2 font-medium shadow-lg"
        >
          <Power className="w-3.5 h-3.5" />
          <span>{isRunning ? "Stop Daemon" : "Start Daemon"}</span>
        </Button>
      </div>
    </header>
  );
};
