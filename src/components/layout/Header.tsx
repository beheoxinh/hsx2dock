import React from "react";
import { Power, RefreshCw, Sun, Moon } from "lucide-react";
import { useState, useEffect } from "react";
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
  const [isDark, setIsDark] = useState(true);

  useEffect(() => {
    setIsDark(document.documentElement.classList.contains("dark"));
  }, []);

  const toggleTheme = () => {
    const nextDark = !isDark;
    setIsDark(nextDark);
    if (nextDark) {
      document.documentElement.classList.add("dark");
      document.documentElement.classList.remove("light");
      localStorage.setItem("theme", "dark");
    } else {
      document.documentElement.classList.remove("dark");
      document.documentElement.classList.add("light");
      localStorage.setItem("theme", "light");
    }
  };

  return (
    <header className="h-16 px-6 bg-zinc-950/60 backdrop-blur-xl border-b border-zinc-800/80 flex items-center justify-between select-none z-10">
      <div className="flex items-center gap-4">
        <div className="flex items-center gap-2.5">
          <span className="text-xs font-medium text-zinc-400">Docker Daemon:</span>
          <StatusBadge status={stateStr} pulse={isRunning} />
        </div>
      </div>

      <div className="flex items-center gap-3">
        <Button
          variant="ghost"
          size="icon"
          onClick={toggleTheme}
          title={isDark ? "Switch to Light Mode" : "Switch to Dark Mode"}
          className="text-zinc-400 hover:text-white"
        >
          {isDark ? <Sun className="w-4 h-4 text-amber-400" /> : <Moon className="w-4 h-4 text-indigo-400" />}
        </Button>

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
