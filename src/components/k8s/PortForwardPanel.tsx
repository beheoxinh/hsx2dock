import React, { useCallback, useEffect, useRef, useState } from "react";
import { Copy, ExternalLink, Square } from "lucide-react";
import { open } from "@tauri-apps/plugin-shell";
import { Button } from "../ui/Button";
import { Skeleton } from "../ui/Skeleton";
import { StatusBadge } from "../ui/StatusBadge";
import { useToast } from "../ui/Toast";
import { useModal, type ModalChoice } from "../ui/ModalContext";
import { api } from "../../lib/api";
import type { PortForwardInfo } from "../../types";
import { forwardStatusTone } from "../../lib/portForward";

const POLL_MS = 5000;

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** Lists the port-forwards of one cluster, with copy, open and stop actions. */
export const PortForwardPanel: React.FC<{ cluster: string }> = ({ cluster }) => {
  const toast = useToast();
  const modal = useModal();
  const [items, setItems] = useState<PortForwardInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stoppingId, setStoppingId] = useState<string | null>(null);

  // One list request at a time. A refresh requested while one is in flight
  // runs again once it settles, so an action's result is never skipped.
  const inFlight = useRef(false);
  const rerun = useRef(false);
  const activeCluster = useRef(cluster);
  useEffect(() => {
    activeCluster.current = cluster;
  }, [cluster]);

  const load = useCallback(async () => {
    if (inFlight.current) {
      rerun.current = true;
      return;
    }
    inFlight.current = true;
    try {
      do {
        rerun.current = false;
        const target = cluster;
        try {
          const list = await api.listPortForwards(target);
          if (activeCluster.current === target) {
            setItems(list);
            setError(null);
          }
        } catch (e) {
          if (activeCluster.current === target) setError(errorText(e));
        }
      } while (rerun.current);
    } finally {
      inFlight.current = false;
    }
  }, [cluster]);

  // First load for this cluster, then poll while the window is visible.
  useEffect(() => {
    setItems(null);
    setError(null);
    void load();
    const timer = window.setInterval(() => {
      if (document.visibilityState === "hidden") return;
      void load();
    }, POLL_MS);
    const onVisible = () => {
      if (document.visibilityState === "visible") void load();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [load]);

  const copyUrl = async (info: PortForwardInfo) => {
    try {
      await navigator.clipboard.writeText(info.url);
      toast.success("URL copied", info.url);
    } catch (e) {
      toast.error("Copy failed", errorText(e));
    }
  };

  const openUrl = async (info: PortForwardInfo) => {
    try {
      await open(info.url);
    } catch (e) {
      toast.error("Could not open URL", errorText(e));
    }
  };

  const stop = async (info: PortForwardInfo) => {
    const target = `${info.kind}/${info.namespace}/${info.name}:${info.remotePort}`;
    const choices: ModalChoice[] = [
      {
        value: "stop",
        label: "Stop",
        hint: info.autoStart
          ? "Stops the forward now. It stays saved and starts again with the cluster."
          : "Stops the forward now.",
      },
    ];
    if (info.autoStart) {
      choices.push({
        value: "forget",
        label: "Stop and forget",
        variant: "danger",
        hint: "Stops the forward and removes its saved auto-start entry.",
      });
    }
    const choice = await modal.showChoice(
      `Stop the port-forward ${target}?`,
      choices,
      "Stop port-forward",
      "warning"
    );
    if (choice !== "stop" && choice !== "forget") return;

    setStoppingId(info.id);
    try {
      await api.stopPortForward(info.id, choice === "forget");
      toast.info(choice === "forget" ? "Port-forward stopped and forgotten" : "Port-forward stopped");
    } catch (e) {
      toast.error("Could not stop port-forward", errorText(e));
    } finally {
      setStoppingId(null);
      await load();
    }
  };

  const loading = items === null && error === null;

  return (
    <section className="space-y-3" aria-label="Port-forwards">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold text-white">Port-forwards</h3>
        {items && <span className="text-xs text-zinc-500">{plural(items.length, "forward")}</span>}
      </div>

      {error && (
        <div
          role="alert"
          className="rounded-none border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-300 break-words"
        >
          {error}
        </div>
      )}

      {loading && (
        <div className="rounded-none border border-zinc-800 bg-zinc-900/60 divide-y divide-zinc-800" aria-hidden="true">
          {[0, 1].map((i) => (
            <div key={i} className="flex items-center gap-3 px-4 py-3">
              <Skeleton className="h-5 w-20 rounded-none" />
              <div className="min-w-0 flex-1 space-y-2">
                <Skeleton className="h-3 w-2/5 rounded-none" />
                <Skeleton className="h-2.5 w-3/5 rounded-none" />
              </div>
            </div>
          ))}
        </div>
      )}

      {items && items.length === 0 && (
        <div className="rounded-none border border-zinc-800 bg-zinc-900/60 px-4 py-6 text-center text-sm text-zinc-500">
          No port-forwards for this cluster. Use Forward on a service or pod.
        </div>
      )}

      {items && items.length > 0 && (
        <ul className="rounded-none border border-zinc-800 bg-zinc-900/60 divide-y divide-zinc-800">
          {items.map((info) => {
            const target = `${info.kind}/${info.namespace}/${info.name}:${info.remotePort}`;
            return (
              <li key={info.id} className="flex items-center gap-3 px-4 py-3 min-w-0">
                <StatusBadge
                  status={info.status}
                  tone={forwardStatusTone(info.status)}
                  className="rounded-none shrink-0"
                />
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="font-mono text-sm text-white truncate" title={target}>
                      {target}
                    </span>
                    {info.autoStart && (
                      <span className="shrink-0 rounded-none border border-sky-500/30 bg-sky-500/10 px-1.5 py-0.5 text-[11px] text-sky-400">
                        auto-start
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-3 min-w-0 text-xs text-zinc-500">
                    <a
                      href={info.url}
                      onClick={(e) => {
                        e.preventDefault();
                        void openUrl(info);
                      }}
                      className="font-mono text-zinc-400 hover:text-white truncate"
                      title={info.url}
                    >
                      {info.url}
                    </a>
                    <span className="shrink-0">{plural(info.restarts, "restart")}</span>
                  </div>
                  {info.message && (
                    <p
                      className={`text-xs truncate mt-0.5 ${
                        info.status === "error" ? "text-rose-400" : "text-zinc-500"
                      }`}
                      title={info.message}
                    >
                      {info.message}
                    </p>
                  )}
                </div>

                <div className="flex items-center gap-1 shrink-0">
                  <Button
                    variant="ghost"
                    size="icon"
                    className="rounded-none"
                    title="Copy URL"
                    aria-label="Copy URL"
                    onClick={() => void copyUrl(info)}
                  >
                    <Copy className="w-4 h-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="rounded-none"
                    title="Open in browser"
                    aria-label="Open in browser"
                    onClick={() => void openUrl(info)}
                  >
                    <ExternalLink className="w-4 h-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="rounded-none hover:text-rose-400"
                    title="Stop port-forward"
                    aria-label="Stop port-forward"
                    disabled={stoppingId === info.id}
                    onClick={() => void stop(info)}
                  >
                    <Square className="w-4 h-4" />
                  </Button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
};
