import React, { useEffect, useState } from "react";
import { Link2, Loader2, X } from "lucide-react";
import { api } from "../../lib/api";
import type { K8sServiceInfo, KindCluster, ServiceLink, ServiceLinkCheck } from "../../types";
import { Button } from "../ui/Button";
import { useModal } from "../ui/ModalContext";
import { useToast } from "../ui/Toast";

const rowKey = (l: Pick<ServiceLink, "namespace" | "name">) => `${l.namespace}/${l.name}`;

const errText = (e: unknown): string => {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
};

const fieldClass =
  "h-9 w-full px-3 rounded-none bg-zinc-950 border border-zinc-800 text-sm text-white placeholder-zinc-600 focus:outline-none focus:border-blue-500 disabled:opacity-60";

interface CheckState {
  results?: ServiceLinkCheck[];
  error?: string;
}

/** Service links on one cluster: list, per-row port check and unlink, plus a link dialog. */
export const ServiceLinksPanel: React.FC<{ cluster: string }> = ({ cluster }) => {
  const toast = useToast();
  const modal = useModal();
  const [links, setLinks] = useState<ServiceLink[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [refreshKey, setRefreshKey] = useState(0);
  const [checks, setChecks] = useState<Record<string, CheckState>>({});
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setLoadError(null);
    api
      .listServiceLinks(cluster)
      .then((list) => {
        if (alive) setLinks(list);
      })
      .catch((e) => {
        if (alive) {
          setLinks([]);
          setLoadError(errText(e));
        }
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [cluster, refreshKey]);

  // Check results belong to one cluster's rows; drop them when the cluster or list changes.
  useEffect(() => {
    setChecks({});
  }, [cluster, refreshKey]);

  const runCheck = async (l: ServiceLink) => {
    const key = rowKey(l);
    setBusyKey(key);
    try {
      const results = await api.checkServiceLink(cluster, l.namespace, l.name);
      setChecks((prev) => ({ ...prev, [key]: { results } }));
    } catch (e) {
      setChecks((prev) => ({ ...prev, [key]: { error: errText(e) } }));
    } finally {
      setBusyKey(null);
    }
  };

  const runUnlink = async (l: ServiceLink) => {
    const ok = await modal.showConfirm(
      `Unlink ${l.namespace}/${l.name} (source ${l.source}) from ${cluster}?`,
      "Unlink service",
      "danger"
    );
    if (!ok) return;
    const key = rowKey(l);
    setBusyKey(key);
    try {
      await api.unlinkClusterService(cluster, l.namespace, l.name);
      toast.success("Service unlinked", key);
      setRefreshKey((k) => k + 1);
    } catch (e) {
      toast.error("Unlink failed", errText(e));
    } finally {
      setBusyKey(null);
    }
  };

  return (
    <section className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-white">Service links</h3>
          <p className="text-[11px] text-zinc-500">
            Services from other running clusters, exposed on this cluster through a local link.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="rounded-none"
            onClick={() => setRefreshKey((k) => k + 1)}
            disabled={loading}
          >
            Refresh
          </Button>
          <Button
            variant="primary"
            size="sm"
            className="rounded-none gap-2"
            onClick={() => setDialogOpen(true)}
          >
            <Link2 className="w-3.5 h-3.5" />
            Link from another cluster…
          </Button>
        </div>
      </div>

      <div className="border border-zinc-800 bg-zinc-900 rounded-none divide-y divide-zinc-800 overflow-x-auto">
        {loading && links.length === 0 ? (
          <div className="flex items-center gap-2 px-4 py-3 text-xs text-zinc-500">
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
            Loading service links…
          </div>
        ) : loadError ? (
          <div className="px-4 py-3 text-xs text-rose-400">Could not load service links: {loadError}</div>
        ) : links.length === 0 ? (
          <div className="px-4 py-6 text-center text-xs text-zinc-500">
            No service links on this cluster. Use "Link from another cluster…" to expose a NodePort or
            LoadBalancer service from a running cluster.
          </div>
        ) : (
          links.map((l) => {
            const key = rowKey(l);
            const state = checks[key];
            const busy = busyKey === key;
            return (
              <div key={key} className="min-w-[720px]">
                <div className="grid grid-cols-[minmax(0,1.1fr)_minmax(0,1.3fr)_minmax(0,1fr)_minmax(0,1.3fr)_auto_auto] items-stretch divide-x divide-zinc-800">
                  <div className="min-w-0 px-4 py-2.5 flex flex-col justify-center">
                    <span className="text-[10px] uppercase tracking-wide text-zinc-500">Local service</span>
                    <span className="text-sm text-white truncate" title={key}>
                      <span className="text-zinc-500">{l.namespace}/</span>
                      {l.name}
                    </span>
                  </div>
                  <div className="min-w-0 px-4 py-2.5 flex flex-col justify-center">
                    <span className="text-[10px] uppercase tracking-wide text-zinc-500">Source</span>
                    <span className="text-xs font-mono text-zinc-300 truncate" title={l.source}>
                      {l.source}
                    </span>
                  </div>
                  <div className="min-w-0 px-4 py-2.5 flex flex-col justify-center">
                    <span className="text-[10px] uppercase tracking-wide text-zinc-500">Address</span>
                    <span className="text-xs font-mono text-zinc-300 truncate" title={l.address}>
                      {l.address || "—"}
                    </span>
                  </div>
                  <div className="min-w-0 px-4 py-2.5 flex flex-col justify-center">
                    <span className="text-[10px] uppercase tracking-wide text-zinc-500">Ports</span>
                    {l.ports.length === 0 ? (
                      <span className="text-xs text-zinc-500">—</span>
                    ) : (
                      <span className="text-xs font-mono text-zinc-300 truncate">
                        {l.ports
                          .map(
                            (p) =>
                              `${p.name ? `${p.name} ` : ""}${p.port} → ${p.remotePort}, ${p.protocol}`
                          )
                          .join(" · ")}
                      </span>
                    )}
                  </div>
                  <div className="px-4 py-2.5 flex flex-col justify-center">
                    <span className="text-[10px] uppercase tracking-wide text-zinc-500">Mode</span>
                    <span className="inline-flex w-fit rounded-none border border-zinc-700 px-1.5 py-0.5 text-[11px] text-zinc-300">
                      {l.mode || "—"}
                    </span>
                  </div>
                  <div className="px-3 py-2.5 flex items-center gap-2">
                    <Button
                      variant="ghost"
                      size="sm"
                      className="rounded-none"
                      onClick={() => runCheck(l)}
                      disabled={busy}
                    >
                      {busy ? (
                        <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      ) : null}
                      Check
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      className="rounded-none text-rose-300 border-rose-500/30 hover:bg-rose-500/10 hover:text-rose-200"
                      onClick={() => runUnlink(l)}
                      disabled={busy}
                    >
                      Unlink
                    </Button>
                  </div>
                </div>

                {state && (
                  <div className="px-4 py-2 bg-zinc-950/60 space-y-1">
                    {state.error && (
                      <p className="text-xs text-rose-400">Check failed: {state.error}</p>
                    )}
                    {state.results && state.results.length === 0 && (
                      <p className="text-xs text-zinc-500">No ports to check.</p>
                    )}
                    {state.results?.map((r, i) => (
                      <div
                        key={`${r.port}-${i}`}
                        className={`flex flex-wrap items-baseline gap-x-3 text-xs ${
                          r.ok ? "text-emerald-400" : "text-rose-400"
                        }`}
                      >
                        <span className="font-mono">:{r.port}</span>
                        <span className="font-medium">{r.ok ? "OK" : "Failed"}</span>
                        <span className="text-zinc-400 break-words min-w-0">{r.message}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>

      {dialogOpen && (
        <LinkServiceDialog
          cluster={cluster}
          onClose={() => setDialogOpen(false)}
          onLinked={() => {
            setDialogOpen(false);
            setRefreshKey((k) => k + 1);
          }}
        />
      )}
    </section>
  );
};

interface LinkServiceDialogProps {
  /** The target cluster the link is created on. */
  cluster: string;
  onClose: () => void;
  /** Called after a successful link; the dialog is expected to close. */
  onLinked: () => void;
}

/** Pick a service on another running cluster and link it into `cluster`. */
export const LinkServiceDialog: React.FC<LinkServiceDialogProps> = ({ cluster, onClose, onLinked }) => {
  const toast = useToast();
  const [clusters, setClusters] = useState<KindCluster[] | null>(null);
  const [clustersError, setClustersError] = useState<string | null>(null);
  const [sourceCluster, setSourceCluster] = useState("");
  const [sourceNamespace, setSourceNamespace] = useState("default");
  const [services, setServices] = useState<K8sServiceInfo[] | null>(null);
  const [servicesLoading, setServicesLoading] = useState(false);
  const [servicesError, setServicesError] = useState<string | null>(null);
  const [sourceService, setSourceService] = useState("");
  const [targetNamespace, setTargetNamespace] = useState("default");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api
      .listKindClusters()
      .then((list) => {
        if (alive) setClusters(list.filter((c) => c.running === true && c.name !== cluster));
      })
      .catch((e) => {
        if (alive) setClustersError(errText(e));
      });
    return () => {
      alive = false;
    };
  }, [cluster]);

  useEffect(() => {
    setSourceService("");
    setServices(null);
    setServicesError(null);
    if (!sourceCluster) {
      setServicesLoading(false);
      return;
    }
    let alive = true;
    setServicesLoading(true);
    api
      .getClusterServices(sourceCluster)
      .then((list) => {
        if (alive) setServices(list);
      })
      .catch((e) => {
        if (alive) setServicesError(errText(e));
      })
      .finally(() => {
        if (alive) setServicesLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [sourceCluster]);

  const ns = sourceNamespace.trim();
  const namespaceServices = services?.filter((s) => s.namespace === ns) ?? [];
  // Free text when the service list failed to load, or the namespace has none to pick from.
  const useFreeText = servicesError !== null || (services !== null && namespaceServices.length === 0);
  const canSubmit =
    !busy &&
    sourceCluster !== "" &&
    ns !== "" &&
    sourceService.trim() !== "" &&
    targetNamespace.trim() !== "";

  const submit = async () => {
    setBusy(true);
    setSubmitError(null);
    const svc = sourceService.trim();
    try {
      await api.linkClusterService({
        sourceCluster,
        sourceNamespace: ns,
        sourceService: svc,
        targetCluster: cluster,
        targetNamespace: targetNamespace.trim(),
        name: name.trim() || null,
      });
      toast.success("Service linked", `${sourceCluster}/${ns}/${svc} → ${cluster}`);
      onLinked();
    } catch (e) {
      setSubmitError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape" && !busy) {
      e.preventDefault();
      onClose();
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={() => !busy && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="link-service-title"
        onKeyDown={onKeyDown}
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-lg max-h-[90vh] overflow-y-auto rounded-none border border-zinc-800 bg-zinc-900 shadow-2xl"
      >
        <div className="flex items-center justify-between px-5 py-4 border-b border-zinc-800">
          <div>
            <span id="link-service-title" className="text-sm font-semibold text-white">
              Link service into {cluster}
            </span>
            <p className="text-[11px] text-zinc-500 mt-0.5">
              Exposes a service from another running cluster on this one.
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            aria-label="Close link dialog"
            title="Close"
            className="text-zinc-400 hover:text-white outline-none focus-visible:ring-2 focus-visible:ring-blue-500 disabled:opacity-50"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="px-5 py-4 space-y-4">
          <div>
            <label htmlFor="link-source-cluster" className="block text-xs text-zinc-400 mb-1">
              Source cluster
            </label>
            <select
              id="link-source-cluster"
              value={sourceCluster}
              onChange={(e) => setSourceCluster(e.target.value)}
              disabled={busy || clusters === null || clusters.length === 0}
              className={fieldClass}
            >
              <option value="">
                {clusters === null && !clustersError ? "Loading clusters…" : "Select a cluster"}
              </option>
              {(clusters ?? []).map((c) => (
                <option key={c.name} value={c.name}>
                  {c.name}
                </option>
              ))}
            </select>
            {clustersError && (
              <p className="text-[11px] text-rose-400 mt-1">Could not load clusters: {clustersError}</p>
            )}
            {clusters !== null && clusters.length === 0 && !clustersError && (
              <p className="text-[11px] text-amber-400 mt-1">
                No other running cluster. Start another cluster to link its services here.
              </p>
            )}
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label htmlFor="link-source-namespace" className="block text-xs text-zinc-400 mb-1">
                Source namespace
              </label>
              <input
                id="link-source-namespace"
                type="text"
                value={sourceNamespace}
                onChange={(e) => {
                  setSourceNamespace(e.target.value);
                  setSourceService("");
                }}
                disabled={busy}
                className={fieldClass}
              />
            </div>
            <div>
              <label htmlFor="link-target-namespace" className="block text-xs text-zinc-400 mb-1">
                Target namespace
              </label>
              <input
                id="link-target-namespace"
                type="text"
                value={targetNamespace}
                onChange={(e) => setTargetNamespace(e.target.value)}
                disabled={busy}
                className={fieldClass}
              />
            </div>
          </div>

          <div>
            <label htmlFor="link-source-service" className="block text-xs text-zinc-400 mb-1">
              Source service
            </label>
            {!sourceCluster ? (
              <select id="link-source-service" disabled className={fieldClass}>
                <option>Choose a source cluster first</option>
              </select>
            ) : servicesLoading ? (
              <select id="link-source-service" disabled className={fieldClass}>
                <option>Loading services…</option>
              </select>
            ) : useFreeText ? (
              <input
                id="link-source-service"
                type="text"
                value={sourceService}
                onChange={(e) => setSourceService(e.target.value)}
                placeholder="service name"
                disabled={busy}
                className={fieldClass}
              />
            ) : (
              <select
                id="link-source-service"
                value={sourceService}
                onChange={(e) => setSourceService(e.target.value)}
                disabled={busy}
                className={fieldClass}
              >
                <option value="">Select a service</option>
                {namespaceServices.map((s) => (
                  <option key={s.name} value={s.name}>
                    {s.name} ({s.type})
                  </option>
                ))}
              </select>
            )}
            <p className="text-[11px] text-zinc-600 mt-1">
              {servicesError
                ? `Could not load services (${servicesError}). Type the service name instead. `
                : sourceCluster && services !== null && namespaceServices.length === 0
                  ? `No services in namespace "${ns}". Type the service name instead. `
                  : ""}
              Only NodePort or LoadBalancer services can be linked; the backend rejects other types.
            </p>
          </div>

          <div>
            <label htmlFor="link-name" className="block text-xs text-zinc-400 mb-1">
              Local name <span className="text-zinc-600">(optional, defaults to the service name)</span>
            </label>
            <input
              id="link-name"
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              disabled={busy}
              className={fieldClass}
            />
          </div>

          {submitError && (
            <div className="border border-rose-500/30 bg-rose-500/10 rounded-none px-3 py-2 text-xs text-rose-300 break-words">
              {submitError}
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 px-5 py-4 border-t border-zinc-800">
          <Button variant="ghost" size="sm" className="rounded-none" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            className="rounded-none gap-2"
            onClick={submit}
            disabled={!canSubmit}
          >
            {busy && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            Link service
          </Button>
        </div>
      </div>
    </div>
  );
};
