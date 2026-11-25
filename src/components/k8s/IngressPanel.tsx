import React, { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-shell";
import { api } from "../../lib/api";
import { INGRESS_ADDON_ID } from "../../lib/addons";
import type { IngressInfo, IngressUrl } from "../../types";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";

const POLL_MS = 15000;

export const IngressPanel: React.FC<{ cluster: string; onOpenAddons: () => void }> = ({
  cluster,
  onOpenAddons,
}) => {
  const toast = useToast();
  const [ingresses, setIngresses] = useState<IngressInfo[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  /** null = addon state unknown (its list request failed). */
  const [ingressReady, setIngressReady] = useState<boolean | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  /** Host whose expose/unexpose call is in flight. */
  const [busyHost, setBusyHost] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    const [list, addons] = await Promise.allSettled([
      api.listClusterIngresses(cluster),
      api.listClusterAddons(cluster),
    ]);
    if (list.status === "fulfilled") {
      setIngresses(list.value);
      setLoadError(null);
    } else {
      setLoadError(String(list.reason));
    }
    if (addons.status === "fulfilled") {
      setIngressReady(addons.value.find((a) => a.id === INGRESS_ADDON_ID)?.state === "ready");
    }
    setRefreshing(false);
  }, [cluster]);

  // Switching cluster drops the previous cluster's rows.
  useEffect(() => {
    setIngresses(null);
    setLoadError(null);
    setIngressReady(null);
  }, [cluster]);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => {
      if (!document.hidden) void refresh();
    }, POLL_MS);
    return () => window.clearInterval(id);
  }, [refresh]);

  const toggleExpose = async (u: IngressUrl) => {
    setBusyHost(u.host);
    try {
      await api.exposeIngressHost(cluster, u.host, !u.exposed);
      await refresh();
    } catch (e) {
      toast.error(u.exposed ? "Unexpose failed" : "Expose failed", String(e));
    } finally {
      setBusyHost(null);
    }
  };

  const openUrl = async (url: string) => {
    try {
      await open(url);
    } catch (e) {
      toast.error("Could not open the URL", String(e));
    }
  };

  // One row per host: an ingress can serve several hosts.
  const rows = (ingresses ?? []).flatMap((ing) => ing.urls.map((u) => ({ ing, u })));

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <p className="text-xs text-zinc-500">Hosts served by Ingress objects in this cluster.</p>
        <Button
          size="sm"
          variant="outline"
          className="rounded-none"
          isLoading={refreshing}
          onClick={() => void refresh()}
        >
          Refresh
        </Button>
      </div>

      {ingressReady === false && (
        <div className="flex flex-col gap-2 rounded-none border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-300 sm:flex-row sm:items-center sm:justify-between">
          <span>The ingress-nginx addon is not ready, so hosts cannot be routed yet.</span>
          <Button size="sm" variant="outline" className="shrink-0 rounded-none" onClick={onOpenAddons}>
            Open Addons
          </Button>
        </div>
      )}

      {loadError && (
        <div className="rounded-none border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-300 break-words">
          Could not load ingresses: {loadError}
        </div>
      )}

      {ingresses === null && !loadError && (
        <p className="text-sm text-zinc-500">Loading ingresses…</p>
      )}

      {ingresses !== null && rows.length === 0 && (
        <p className="rounded-none border border-zinc-800 px-4 py-6 text-center text-sm text-zinc-500">
          No ingresses in this cluster.
        </p>
      )}

      {rows.length > 0 && (
        <div className="divide-y divide-zinc-800 rounded-none border border-zinc-800 bg-zinc-900/90">
          {rows.map(({ ing, u }) => {
            const url = u.url;
            const busy = busyHost === u.host;
            return (
              <div
                key={`${ing.namespace}/${ing.name}/${u.host}`}
                className="flex flex-col gap-3 px-4 py-3 md:flex-row md:items-center md:justify-between"
              >
                <div className="min-w-0 flex-1 space-y-1.5">
                  <div className="flex flex-wrap items-center gap-2 text-xs">
                    <span className="font-mono text-zinc-400">
                      {ing.namespace}/{ing.name}
                    </span>
                    {ing.class && (
                      <span className="rounded-none border border-zinc-700 bg-zinc-800 px-1.5 py-0.5 font-mono text-[11px] text-zinc-300">
                        {ing.class}
                      </span>
                    )}
                    {ing.tls && (
                      <span className="rounded-none border border-emerald-500/30 bg-emerald-500/10 px-1.5 py-0.5 text-[11px] text-emerald-400">
                        TLS
                      </span>
                    )}
                    {u.exposed && (
                      <span className="rounded-none border border-sky-500/30 bg-sky-500/10 px-1.5 py-0.5 text-[11px] text-sky-400">
                        Exposed
                      </span>
                    )}
                  </div>
                  <p className="break-all font-mono text-sm text-white">{u.host || "*"}</p>
                  {url ? (
                    <p className="break-all font-mono text-xs text-zinc-400">{url}</p>
                  ) : u.note ? (
                    <p className="text-xs text-zinc-500">{u.note}</p>
                  ) : null}
                </div>

                <div className="flex shrink-0 flex-wrap gap-2">
                  {url && (
                    <Button
                      size="sm"
                      variant="outline"
                      className="rounded-none"
                      onClick={() => void openUrl(url)}
                    >
                      Open
                    </Button>
                  )}
                  <Button
                    size="sm"
                    variant={u.exposed ? "ghost" : "primary"}
                    className="rounded-none"
                    disabled={busy}
                    isLoading={busy}
                    onClick={() => void toggleExpose(u)}
                  >
                    {u.exposed ? "Unexpose" : "Expose via gateway"}
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
