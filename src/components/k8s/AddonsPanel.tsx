import React, { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-shell";
import { ChevronDown, ChevronUp, Copy } from "lucide-react";
import { api, disposeListener } from "../../lib/api";
import {
  DASHBOARD_ADDON_ID,
  addonPollIntervalMs,
  addonStateTone,
  isAddonBusy,
  toHttpsUrl,
} from "../../lib/addons";
import type { AddonProgressPayload, AddonStatus } from "../../types";
import { Button } from "../ui/Button";
import { StatusBadge } from "../ui/StatusBadge";
import { useModal } from "../ui/ModalContext";
import { useToast } from "../ui/Toast";

/** Rolling log kept per addon. */
const LOG_LIMIT = 50;
const DASHBOARD_NAMESPACE = "kubernetes-dashboard";
const DASHBOARD_SERVICE = "kubernetes-dashboard";

/** Copy of a record without one key. */
function withoutKey<T>(record: Record<string, T>, key: string): Record<string, T> {
  const next = { ...record };
  delete next[key];
  return next;
}

export const AddonsPanel: React.FC<{ cluster: string }> = ({ cluster }) => {
  const toast = useToast();
  const modal = useModal();

  const [addons, setAddons] = useState<AddonStatus[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [pollMs, setPollMs] = useState(15000);
  const [lastLine, setLastLine] = useState<Record<string, string>>({});
  const [logs, setLogs] = useState<Record<string, string[]>>({});
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [opBusy, setOpBusy] = useState<Record<string, boolean>>({});
  const [token, setToken] = useState<string | null>(null);
  const [tokenBusy, setTokenBusy] = useState(false);
  const [opening, setOpening] = useState(false);
  const loadingRef = useRef(false);

  const load = useCallback(async () => {
    loadingRef.current = true;
    try {
      const list = await api.listClusterAddons(cluster);
      setAddons(list);
      setLoadError(null);
      setPollMs(addonPollIntervalMs(list));
    } catch (e) {
      setLoadError(String(e));
    } finally {
      loadingRef.current = false;
    }
  }, [cluster]);

  // Switching cluster drops everything that belonged to the previous one.
  useEffect(() => {
    setAddons(null);
    setLoadError(null);
    setLastLine({});
    setLogs({});
    setExpanded({});
    setErrors({});
    setOpBusy({});
    setToken(null);
  }, [cluster]);

  // Poll with a setTimeout chain; the delay follows the busy state.
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      if (!document.hidden && !loadingRef.current) await load();
      if (!cancelled) timer = setTimeout(tick, pollMs);
    };
    void tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [load, pollMs]);

  // Refresh as soon as the window becomes visible again.
  useEffect(() => {
    const onVisible = () => {
      if (!document.hidden) void load();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => document.removeEventListener("visibilitychange", onVisible);
  }, [load]);

  // Live progress from install/uninstall runs.
  useEffect(
    () =>
      disposeListener(
        listen<AddonProgressPayload>("addon-progress", (e) => {
          const p = e.payload;
          if (p.cluster !== cluster) return;
          setLastLine((prev) => ({ ...prev, [p.addon]: p.line }));
          setLogs((prev) => ({
            ...prev,
            [p.addon]: [...(prev[p.addon] ?? []), p.line].slice(-LOG_LIMIT),
          }));
          const err = p.error;
          if (err) {
            setErrors((prev) => ({ ...prev, [p.addon]: err }));
            toast.error(`${p.addon} failed`, err);
          }
          if (p.done) void load();
        })
      ),
    [cluster, load, toast]
  );

  const runOp = async (a: AddonStatus, install: boolean) => {
    setOpBusy((prev) => ({ ...prev, [a.id]: true }));
    setErrors((prev) => withoutKey(prev, a.id));
    setLogs((prev) => ({ ...prev, [a.id]: [] }));
    setLastLine((prev) => withoutKey(prev, a.id));
    try {
      if (install) await api.installClusterAddon(cluster, a.id);
      else await api.uninstallClusterAddon(cluster, a.id);
      await load();
    } catch (e) {
      toast.error(`${install ? "Install" : "Uninstall"} ${a.name} failed`, String(e));
    } finally {
      setOpBusy((prev) => ({ ...prev, [a.id]: false }));
    }
  };

  const uninstall = async (a: AddonStatus) => {
    const ok = await modal.showConfirm(
      `Uninstall ${a.name} from cluster ${cluster}?`,
      "Uninstall addon",
      "danger"
    );
    if (ok) await runOp(a, false);
  };

  const getToken = async () => {
    setTokenBusy(true);
    try {
      setToken(await api.dashboardToken(cluster));
    } catch (e) {
      toast.error("Could not get the dashboard token", String(e));
    } finally {
      setTokenBusy(false);
    }
  };

  const copyToken = async () => {
    if (!token) return;
    try {
      await navigator.clipboard.writeText(token);
      toast.success("Token copied");
    } catch (e) {
      toast.error("Copy failed", String(e));
    }
  };

  const openDashboard = async () => {
    setOpening(true);
    try {
      const forwards = await api.listPortForwards(cluster);
      const existing = forwards.find(
        (f) =>
          f.kind === "svc" &&
          f.name === DASHBOARD_SERVICE &&
          f.namespace === DASHBOARD_NAMESPACE &&
          f.url
      );
      const info =
        existing ??
        (await api.startPortForward({
          cluster,
          namespace: DASHBOARD_NAMESPACE,
          kind: "svc",
          name: DASHBOARD_SERVICE,
          remotePort: 443,
          localPort: 0,
          autoStart: false,
        }));
      await open(toHttpsUrl(info.url));
    } catch (e) {
      toast.error("Could not open the dashboard", String(e));
    } finally {
      setOpening(false);
    }
  };

  return (
    <div className="space-y-3">
      {loadError && (
        <div className="rounded-none border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-300 break-words">
          Could not load addons: {loadError}
        </div>
      )}

      {addons === null && !loadError && (
        <p className="text-sm text-zinc-500">Loading addons…</p>
      )}

      {addons !== null && addons.length === 0 && (
        <p className="rounded-none border border-zinc-800 px-4 py-6 text-center text-sm text-zinc-500">
          No addons are available for this cluster.
        </p>
      )}

      {addons !== null && addons.length > 0 && (
        <div className="divide-y divide-zinc-800 rounded-none border border-zinc-800 bg-zinc-900/90">
          {addons.map((a) => {
            const busy = isAddonBusy(a.state) || !!opBusy[a.id];
            const canInstall = a.state === "not-installed" || a.state === "error";
            const canUninstall =
              a.state === "ready" || a.state === "error" || a.state === "progressing";
            const isDashboard = a.id === DASHBOARD_ADDON_ID && a.state === "ready";
            const log = logs[a.id] ?? [];
            const isOpen = !!expanded[a.id];

            return (
              <div key={a.id} className="space-y-2 px-4 py-3">
                <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-semibold text-white">{a.name}</span>
                      <span className="font-mono text-xs text-zinc-500">{a.version}</span>
                      <StatusBadge
                        status={a.state}
                        tone={addonStateTone(a.state)}
                        className="rounded-none"
                      />
                    </div>
                    <p className="text-xs leading-relaxed text-zinc-400">{a.description}</p>
                    {a.message && <p className="text-xs text-zinc-500">{a.message}</p>}
                    {a.detail && <p className="break-all font-mono text-xs text-zinc-500">{a.detail}</p>}
                    {busy && lastLine[a.id] && (
                      <p className="truncate font-mono text-xs text-sky-400">{lastLine[a.id]}</p>
                    )}
                    {errors[a.id] && (
                      <p className="break-words text-xs text-rose-400">{errors[a.id]}</p>
                    )}
                  </div>

                  <div className="flex shrink-0 flex-wrap items-center gap-2">
                    {log.length > 0 && (
                      <Button
                        size="sm"
                        variant="ghost"
                        className="rounded-none"
                        onClick={() => setExpanded((prev) => ({ ...prev, [a.id]: !isOpen }))}
                      >
                        {isOpen ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
                        Log
                      </Button>
                    )}
                    {canInstall && (
                      <Button
                        size="sm"
                        variant="primary"
                        className="rounded-none"
                        disabled={busy}
                        isLoading={!!opBusy[a.id]}
                        onClick={() => void runOp(a, true)}
                      >
                        Install
                      </Button>
                    )}
                    {canUninstall && (
                      <Button
                        size="sm"
                        variant="danger"
                        className="rounded-none"
                        disabled={busy}
                        isLoading={!!opBusy[a.id]}
                        onClick={() => void uninstall(a)}
                      >
                        Uninstall
                      </Button>
                    )}
                  </div>
                </div>

                {isDashboard && (
                  <div className="space-y-2">
                    <div className="flex flex-wrap gap-2">
                      <Button
                        size="sm"
                        variant="outline"
                        className="rounded-none"
                        isLoading={tokenBusy}
                        onClick={() => void getToken()}
                      >
                        Get token
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        className="rounded-none"
                        isLoading={opening}
                        onClick={() => void openDashboard()}
                      >
                        Open
                      </Button>
                    </div>
                    {token && (
                      <div className="flex items-stretch border border-zinc-800 bg-zinc-950">
                        <textarea
                          readOnly
                          rows={3}
                          value={token}
                          className="min-w-0 flex-1 resize-none rounded-none bg-transparent px-3 py-2 font-mono text-xs text-zinc-200 outline-none"
                        />
                        <Button
                          size="sm"
                          variant="ghost"
                          className="self-stretch rounded-none"
                          onClick={() => void copyToken()}
                        >
                          <Copy className="h-3.5 w-3.5" />
                          Copy
                        </Button>
                      </div>
                    )}
                  </div>
                )}

                {isOpen && log.length > 0 && (
                  <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-none border border-zinc-800 bg-zinc-950 p-2 font-mono text-[11px] text-zinc-400">
                    {log.join("\n")}
                  </pre>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
