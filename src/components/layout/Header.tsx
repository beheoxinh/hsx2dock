import React from "react";
import { Power, RefreshCw, Sun, Moon } from "lucide-react";
import { useState, useEffect } from "react";
import { Button } from "../ui/Button";
import { StatusBadge } from "../ui/StatusBadge";
import { summarizeClusters } from "../../lib/clusterHealth";
import { api } from "../../lib/api";
import type { DaemonState, KindCluster } from "../../types";

interface HeaderProps {
  daemonState: DaemonState;
  onToggleEngine: () => void;
  isToggling?: boolean;
  onRefresh?: () => void;
  /** Opens the Kubernetes page from the cluster status badge. */
  onOpenClusters?: () => void;
}

export const Header: React.FC<HeaderProps> = ({
  daemonState,
  onToggleEngine,
  isToggling = false,
  onRefresh,
  onOpenClusters,
}) => {
  const isRunning = daemonState === "Running";
  const stateStr = typeof daemonState === "string" ? daemonState : "Error";
  const [isDark, setIsDark] = useState(true);
  const [clusters, setClusters] = useState<KindCluster[]>([]);

  useEffect(() => {
    setIsDark(document.documentElement.classList.contains("dark"));
  }, []);

  // Cluster status next to the daemon badge. Clusters can only change while
  // the engine is up, so the poll is idle otherwise.
  useEffect(() => {
    if (!isRunning) {
      setClusters([]);
      return;
    }
    let disposed = false;
    const load = () => {
      api
        .listKindClusters()
        .then((list) => {
          if (!disposed) setClusters(list);
        })
        .catch(() => {
          /* engine went away mid-poll — keep the last snapshot */
        });
    };
    load();
    const timer = window.setInterval(load, 10_000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [isRunning]);

  const health = summarizeClusters(clusters, isRunning);

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
      <div className="flex items-center gap-5">
        <div className="flex items-center gap-2.5">
          <span className="text-xs font-medium text-zinc-400">Docker Daemon:</span>
          <StatusBadge status={stateStr} pulse={isRunning} />
        </div>

        <div className="flex items-center gap-2.5">
          <span className="text-xs font-medium text-zinc-400">K8s Clusters:</span>
          <button
            type="button"
            onClick={onOpenClusters}
            disabled={!onOpenClusters}
            title={health.detail}
            aria-label={`Kubernetes clusters: ${health.clusters.label}`}
            className="rounded-full transition-opacity hover:opacity-80 disabled:cursor-default disabled:hover:opacity-100"
          >
            <StatusBadge
              status={health.clusters.label}
              tone={health.clusters.tone}
              pulse={health.clusters.pulse}
            />
          </button>

          {/* Per-cluster health only appears once there is something to report. */}
          {health.pods && (
            <span title={`${health.pods.label} across all clusters`}>
              <StatusBadge status={health.pods.label} tone={health.pods.tone} />
            </span>
          )}
          {health.api && (
            <span title="Whether each running cluster's API server answers /readyz">
              <StatusBadge status={health.api.label} tone={health.api.tone} />
            </span>
          )}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-3">
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
