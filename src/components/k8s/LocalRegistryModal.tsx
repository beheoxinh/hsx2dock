import { useEffect, useState } from "react";
import { Loader2, Package, Plug, Play, RefreshCw, Trash2, X } from "lucide-react";
import { Button } from "../ui/Button";
import { StatusBadge } from "../ui/StatusBadge";
import { useModal } from "../ui/ModalContext";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import type { KindCluster, LocalRegistryStatus } from "../../types";

interface LocalRegistryModalProps {
  clusters: KindCluster[];
  onClose: () => void;
}

/**
 * The shared `kind-registry` container on localhost:5001: start or remove it,
 * see which clusters use it and what it holds, and connect running clusters.
 */
export function LocalRegistryModal({ clusters, onClose }: LocalRegistryModalProps) {
  const modal = useModal();
  const toast = useToast();
  const [status, setStatus] = useState<LocalRegistryStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  /** The action in flight: "start", "remove" or `connect:<cluster>`. */
  const [busy, setBusy] = useState<string | null>(null);
  const [deleteData, setDeleteData] = useState(false);

  const load = async () => {
    try {
      setStatus(await api.localRegistryStatus());
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
  };

  useEffect(() => {
    void load();
  }, []);

  // Escape closes, but never while a request is in flight.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && busy === null) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const run = async (key: string, failTitle: string, action: () => Promise<unknown>, done?: string) => {
    if (busy) return;
    setBusy(key);
    try {
      await action();
      if (done) toast.success(done);
    } catch (e) {
      toast.error(failTitle, String(e));
    } finally {
      await load();
      setBusy(null);
    }
  };

  const removeRegistry = async () => {
    const ok = await modal.showConfirm(
      deleteData
        ? "Remove the local registry and delete every image pushed to it? Clusters connected to it lose those images. This cannot be undone."
        : "Remove the local registry container? Its images are kept, and Start brings the registry back with them.",
      "Remove Local Registry",
      "danger"
    );
    if (!ok) return;
    await run("remove", "Remove failed", () => api.removeLocalRegistry(deleteData), "Registry removed");
  };

  const connected = new Set(status?.connectedClusters ?? []);
  const toConnect = clusters.filter((c) => c.running && !connected.has(c.name));
  const registryState = status?.running ? "Running" : status?.exists ? "Stopped" : "Not created";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Local registry"
        className="flex max-h-[85vh] w-[min(640px,94vw)] flex-col rounded-none border border-zinc-800 bg-zinc-900 shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-zinc-800 px-4 py-3">
          <div className="flex min-w-0 items-center gap-2.5">
            <Package className="h-4 w-4 shrink-0 text-blue-400" />
            <div className="min-w-0">
              <p className="text-sm font-semibold text-white">Local registry</p>
              <p className="text-[11px] text-zinc-500">
                Shared by every cluster that connects; images stay on this machine.
              </p>
            </div>
          </div>
          <Button
            variant="ghost"
            size="icon"
            title="Close"
            aria-label="Close"
            onClick={onClose}
            disabled={busy !== null}
            className="h-7 w-7 rounded-none text-zinc-400"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-4 py-4">
          {loadError && (
            <div role="alert" className="border border-rose-500/30 bg-rose-500/10 px-3 py-2 font-mono text-xs text-rose-300">
              {loadError}
            </div>
          )}

          {!status && !loadError && (
            <p className="flex items-center gap-2 text-xs text-zinc-500">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              Reading registry status…
            </p>
          )}

          {status && (
            <>
              <div className="flex flex-wrap items-center justify-between gap-3 border border-zinc-800 bg-zinc-950 px-3 py-3">
                <div className="min-w-0 space-y-1.5">
                  <div className="flex items-center gap-2">
                    <StatusBadge status={registryState} />
                    <span className="font-mono text-xs text-zinc-300">{status.host}</span>
                  </div>
                  <p className="truncate font-mono text-[11px] text-zinc-500">{status.image}</p>
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    variant="ghost"
                    size="icon"
                    title="Refresh status"
                    aria-label="Refresh status"
                    disabled={busy !== null}
                    onClick={() => void load()}
                    className="h-8 w-8 rounded-none"
                  >
                    <RefreshCw className="h-3.5 w-3.5" />
                  </Button>
                  {!status.running && (
                    <Button
                      variant="primary"
                      size="sm"
                      className="gap-1.5 rounded-none"
                      isLoading={busy === "start"}
                      disabled={busy !== null}
                      onClick={() =>
                        run("start", "Could not start the registry", () => api.ensureLocalRegistry(), "Registry running")
                      }
                    >
                      <Play className="h-3.5 w-3.5" />
                      Start
                    </Button>
                  )}
                </div>
              </div>

              {status.exists && (
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <label className="flex items-center gap-2 text-xs text-zinc-400">
                    <input
                      type="checkbox"
                      checked={deleteData}
                      disabled={busy !== null}
                      onChange={(e) => setDeleteData(e.target.checked)}
                      className="h-3.5 w-3.5 accent-rose-500"
                    />
                    Also delete the image data
                  </label>
                  <Button
                    variant="outline"
                    size="sm"
                    className="gap-1.5 rounded-none border-rose-500/40 text-rose-300 hover:bg-rose-500/10"
                    isLoading={busy === "remove"}
                    disabled={busy !== null}
                    onClick={removeRegistry}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                    Remove registry
                  </Button>
                </div>
              )}

              <section className="space-y-2">
                <h4 className="text-[10px] uppercase tracking-wider text-zinc-500">Connected clusters</h4>
                {status.connectedClusters.length === 0 ? (
                  <p className="text-xs text-zinc-500">No cluster is connected yet.</p>
                ) : (
                  <div className="flex flex-wrap gap-1.5">
                    {status.connectedClusters.map((name) => (
                      <span
                        key={name}
                        className="border border-emerald-500/30 bg-emerald-500/10 px-2 py-0.5 font-mono text-[11px] text-emerald-300"
                      >
                        {name}
                      </span>
                    ))}
                  </div>
                )}

                {toConnect.length > 0 && (
                  <ul className="divide-y divide-zinc-800 border border-zinc-800">
                    {toConnect.map((c) => (
                      <li key={c.name} className="flex items-center justify-between gap-3 px-3 py-2">
                        <span className="font-mono text-xs text-zinc-200">{c.name}</span>
                        <Button
                          variant="outline"
                          size="sm"
                          className="gap-1.5 rounded-none"
                          isLoading={busy === `connect:${c.name}`}
                          disabled={busy !== null || !status.running}
                          title={
                            status.running
                              ? `Connect ${c.name} to the local registry`
                              : "Start the registry first"
                          }
                          onClick={() =>
                            run(
                              `connect:${c.name}`,
                              "Connect failed",
                              () => api.connectRegistryToCluster(c.name),
                              `${c.name} connected`
                            )
                          }
                        >
                          <Plug className="h-3.5 w-3.5" />
                          Connect
                        </Button>
                      </li>
                    ))}
                  </ul>
                )}
              </section>

              <section className="space-y-2">
                <h4 className="text-[10px] uppercase tracking-wider text-zinc-500">Repositories</h4>
                {status.repositories.length === 0 ? (
                  <p className="text-xs text-zinc-500">
                    Nothing pushed yet. Use Images → Push to local registry, then reference the image as{" "}
                    <span className="font-mono">localhost:5001/name:tag</span> in a connected cluster.
                  </p>
                ) : (
                  <ul className="max-h-48 divide-y divide-zinc-800 overflow-y-auto border border-zinc-800 font-mono text-xs text-zinc-300">
                    {status.repositories.map((repo) => (
                      <li key={repo} className="px-3 py-1.5">
                        {repo}
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
