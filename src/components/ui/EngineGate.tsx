import React, { useState } from "react";
import { Power, Loader2, AlertTriangle, PackageOpen } from "lucide-react";
import { Button } from "./Button";
import { useToast } from "./Toast";
import { api } from "../../lib/api";
import type { DaemonState } from "../../types";

/**
 * Gate for every page that talks to Docker. Renders its children only while
 * the engine is `Running`; otherwise it replaces the page with a designed
 * "engine is off" panel (start button, spinner, or error) so a stopped engine
 * reads as a feature state instead of a wall of failed fetches.
 */
export const EngineGate: React.FC<{
  state: DaemonState;
  /** Noun shown in the copy, e.g. "containers", "clusters". */
  resource?: string;
  children: React.ReactNode;
}> = ({ state, resource = "resources", children }) => {
  const toast = useToast();
  const [toggling, setToggling] = useState(false);

  if (state === "Running") return <>{children}</>;

  const transient =
    state === "Starting" || state === "Stopping" || state === "Installing";
  const notInstalled = state === "NotInstalled";
  const error = typeof state === "object" ? state.Error : null;

  const start = async () => {
    if (toggling) return;
    try {
      setToggling(true);
      await api.toggleEngine();
    } catch (e) {
      toast.error("Could not start the engine", String(e));
    } finally {
      setToggling(false);
    }
  };

  const title = transient
    ? state === "Installing"
      ? "Installing the Docker engine…"
      : state === "Stopping"
        ? "Stopping the Docker engine…"
        : "Starting the Docker engine…"
    : notInstalled
      ? "Docker engine is not installed"
      : error
        ? "Docker engine failed"
        : "Docker engine is stopped";

  const body = transient
    ? "This page opens automatically once the engine is ready."
    : notInstalled
      ? "The bundled rootless engine has not been installed yet — finish the setup on the Dashboard, then come back."
      : `The ${resource} on this page live in the app's bundled rootless engine. Start it to continue.`;

  return (
    <div className="h-full flex items-center justify-center p-8">
      <div className="max-w-md w-full rounded-2xl border border-zinc-800 bg-zinc-900/60 p-8 text-center space-y-4">
        <div
          className={`w-12 h-12 rounded-xl border flex items-center justify-center mx-auto ${
            error
              ? "bg-rose-500/10 border-rose-500/30"
              : "bg-amber-500/10 border-amber-500/30"
          }`}
        >
          {transient ? (
            <Loader2 className="w-6 h-6 text-amber-400 animate-spin" />
          ) : error ? (
            <AlertTriangle className="w-6 h-6 text-rose-400" />
          ) : notInstalled ? (
            <PackageOpen className="w-6 h-6 text-amber-400" />
          ) : (
            <Power className="w-6 h-6 text-amber-400" />
          )}
        </div>

        <h3 className="text-base font-semibold text-white">{title}</h3>
        <p className="text-sm text-zinc-400 leading-relaxed">{body}</p>

        {error && (
          <p className="text-xs text-rose-400 font-mono break-all text-left rounded-lg bg-rose-500/5 border border-rose-500/20 px-3 py-2">
            {error}
          </p>
        )}

        {!transient && !notInstalled && (
          <Button
            variant="primary"
            onClick={start}
            disabled={toggling}
            className="gap-2 mx-auto"
          >
            {toggling ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <Power className="w-4 h-4" />
            )}
            Start Docker engine
          </Button>
        )}
      </div>
    </div>
  );
};
