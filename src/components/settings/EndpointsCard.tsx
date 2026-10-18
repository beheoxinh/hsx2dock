import React, { useEffect, useState } from "react";
import {
  CheckCircle2,
  Cloud,
  HardDrive,
  Loader2,
  Pencil,
  Plug,
  Plus,
  Server,
  Trash2,
  XCircle,
} from "lucide-react";
import { Card } from "../ui/Card";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { useModal } from "../ui/ModalContext";
import { api } from "../../lib/api";
import type { EndpointInput, EndpointStatus } from "../../types";

const KIND_LABEL: Record<string, string> = {
  local: "Bundled",
  tcp: "TCP",
  ssh: "SSH",
};

const KIND_PLACEHOLDER: Record<string, string> = {
  tcp: "host:2375",
  ssh: "user@host",
};

interface DraftState {
  id: string | null;
  name: string;
  kind: string;
  uri: string;
}

const EMPTY_DRAFT: DraftState = { id: null, name: "", kind: "tcp", uri: "" };

/**
 * Lists the local bundled engine plus every saved remote, and is the only
 * place an endpoint is added, tested or switched. Switching is refused unless
 * the target answers, so the app never ends up pointed at a dead daemon.
 */
export const EndpointsCard: React.FC = () => {
  const toast = useToast();
  const modal = useModal();

  const [endpoints, setEndpoints] = useState<EndpointStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [draft, setDraft] = useState<DraftState | null>(null);
  const [testing, setTesting] = useState(false);
  const [tested, setTested] = useState<EndpointStatus | null>(null);
  const [switching, setSwitching] = useState<string | null>(null);

  const load = async () => {
    try {
      setLoading(true);
      setEndpoints(await api.listEndpoints());
    } catch (e) {
      toast.error("Could not list endpoints", String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const openNew = () => {
    setDraft({ ...EMPTY_DRAFT });
    setTested(null);
  };

  const openEdit = (ep: EndpointStatus) => {
    setDraft({ id: ep.id, name: ep.name, kind: ep.kind, uri: ep.uri });
    setTested(null);
  };

  const payload = (d: DraftState): EndpointInput => ({
    id: d.id,
    name: d.name.trim(),
    kind: d.kind,
    uri: d.uri.trim(),
  });

  const handleTest = async () => {
    if (!draft) return;
    try {
      setTesting(true);
      setTested(null);
      const res = await api.testEndpoint(payload(draft));
      setTested(res);
      toast.success("Endpoint reachable", `Docker ${res.version ?? "unknown"}`);
    } catch (e) {
      setTested({
        id: "",
        name: draft.name,
        kind: draft.kind,
        uri: draft.uri,
        is_active: false,
        reachable: false,
        version: null,
        latency_ms: null,
        error: String(e),
      });
      toast.error("Endpoint unreachable", String(e));
    } finally {
      setTesting(false);
    }
  };

  const handleSave = async () => {
    if (!draft) return;
    if (!draft.name.trim()) {
      toast.error("Name is required");
      return;
    }
    try {
      await api.saveEndpoint(payload(draft));
      toast.success("Endpoint saved", draft.name.trim());
      setDraft(null);
      setTested(null);
      await load();
    } catch (e) {
      toast.error("Could not save endpoint", String(e));
    }
  };

  const handleRemove = async (ep: EndpointStatus) => {
    const ok = await modal.showConfirm(
      `Remove the endpoint "${ep.name}"?\n\nThe daemon itself is not touched.`,
      "Remove Endpoint",
      "danger"
    );
    if (!ok) return;
    try {
      await api.removeEndpoint(ep.id);
      toast.success("Endpoint removed", ep.name);
      await load();
    } catch (e) {
      toast.error("Could not remove endpoint", String(e));
    }
  };

  const handleActivate = async (ep: EndpointStatus) => {
    try {
      setSwitching(ep.id);
      await api.setActiveEndpoint(ep.id);
      toast.success(
        "Endpoint active",
        ep.kind === "local" ? "Using the bundled engine" : ep.name
      );
      await load();
    } catch (e) {
      // Never fall back silently: the previous endpoint stays active.
      toast.error("Could not switch endpoint", String(e));
    } finally {
      setSwitching(null);
    }
  };

  return (
    <Card className="p-6 space-y-4">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h3 className="font-semibold text-white flex items-center gap-2">
            <Server className="w-4 h-4 text-indigo-400" />
            Engine Endpoints
          </h3>
          <p className="text-xs text-zinc-500 mt-1">
            Manage the bundled engine and remote Docker daemons (TCP or SSH).
            Engine lifecycle controls stay disabled while a remote is active.
          </p>
        </div>
        <Button variant="secondary" size="sm" onClick={openNew}>
          <Plus className="w-4 h-4" />
          Add endpoint
        </Button>
      </div>

      {loading ? (
        <div className="flex items-center gap-2 py-3 text-xs text-zinc-500">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Checking endpoints…
        </div>
      ) : (
        <div className="space-y-2">
          {endpoints.map((ep) => {
            const busy = switching === ep.id;
            return (
              <div
                key={ep.id}
                className={`flex items-center justify-between gap-3 rounded-lg border px-3 py-2.5 ${
                  ep.is_active
                    ? "border-cyan-500/40 bg-cyan-500/5"
                    : "border-zinc-800 bg-zinc-900/40"
                }`}
              >
                <div className="flex min-w-0 items-center gap-3">
                  <div
                    className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-zinc-900/70 ${
                      ep.kind === "local" ? "text-emerald-400" : "text-indigo-400"
                    }`}
                  >
                    {ep.kind === "local" ? (
                      <HardDrive className="h-4 w-4" />
                    ) : (
                      <Cloud className="h-4 w-4" />
                    )}
                  </div>
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-sm text-zinc-100">{ep.name}</span>
                      <span className="shrink-0 rounded border border-zinc-700/70 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-zinc-400">
                        {KIND_LABEL[ep.kind] ?? ep.kind}
                      </span>
                      {ep.is_active && (
                        <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wide text-cyan-400">
                          active
                        </span>
                      )}
                    </div>
                    <div className="truncate font-mono text-[11px] text-zinc-500">
                      {ep.uri}
                    </div>
                  </div>
                </div>

                <div className="flex shrink-0 items-center gap-2">
                  {ep.kind !== "local" && ep.is_active && (
                    <span className="flex items-center gap-1 text-[11px]">
                      {ep.reachable ? (
                        <>
                          <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400" />
                          <span className="text-emerald-400">
                            {ep.version ?? "up"}
                            {ep.latency_ms !== null ? ` · ${ep.latency_ms}ms` : ""}
                          </span>
                        </>
                      ) : (
                        <>
                          <XCircle className="h-3.5 w-3.5 text-rose-400" />
                          <span className="text-rose-400">unreachable</span>
                        </>
                      )}
                    </span>
                  )}

                  {!ep.is_active && (
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={busy}
                      onClick={() => handleActivate(ep)}
                    >
                      {busy ? (
                        <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      ) : (
                        <Plug className="w-3.5 h-3.5" />
                      )}
                      Use
                    </Button>
                  )}

                  {ep.kind !== "local" && (
                    <>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => openEdit(ep)}
                        title="Edit"
                        className="text-zinc-500 hover:text-zinc-100"
                      >
                        <Pencil className="w-3.5 h-3.5" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => handleRemove(ep)}
                        title="Remove"
                        className="text-zinc-500 hover:text-rose-400"
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </Button>
                    </>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}

      {draft && (
        <div className="space-y-3 rounded-lg border border-zinc-800 bg-zinc-900/30 p-3">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-zinc-400">Name</label>
              <input
                type="text"
                autoFocus
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="Prod swarm"
                className="h-9 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-2.5 text-sm text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
              />
            </div>
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-zinc-400">Kind</label>
              <select
                value={draft.kind}
                onChange={(e) => setDraft({ ...draft, kind: e.target.value })}
                className="h-9 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-2.5 text-sm text-white outline-none focus:border-zinc-700"
              >
                <option value="tcp">TCP (unix socket over http)</option>
                <option value="ssh">SSH</option>
              </select>
            </div>
          </div>

          <div className="space-y-1.5">
            <label className="text-xs font-medium text-zinc-400">Address</label>
            <input
              type="text"
              value={draft.uri}
              onChange={(e) => {
                setDraft({ ...draft, uri: e.target.value });
                setTested(null);
              }}
              placeholder={KIND_PLACEHOLDER[draft.kind] ?? "host:2375"}
              className="h-9 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-2.5 font-mono text-xs text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
            />
            <p className="text-[11px] text-zinc-600">
              {draft.kind === "tcp"
                ? "The daemon must expose its API. Port 2375 defaults in when omitted."
                : "Uses your ssh agent and ~/.ssh keys; no password prompt."}
            </p>
          </div>

          {tested && (
            <div
              className={`flex items-center gap-2 text-[11px] ${
                tested.reachable ? "text-emerald-400" : "text-rose-400"
              }`}
            >
              {tested.reachable ? (
                <>
                  <CheckCircle2 className="h-3.5 w-3.5" />
                  Docker {tested.version ?? "unknown"}
                  {tested.latency_ms !== null ? ` · ${tested.latency_ms}ms` : ""}
                </>
              ) : (
                <>
                  <XCircle className="h-3.5 w-3.5" />
                  <span className="break-all">{tested.error}</span>
                </>
              )}
            </div>
          )}

          <div className="flex justify-end gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setDraft(null);
                setTested(null);
              }}
            >
              Cancel
            </Button>
            <Button variant="secondary" size="sm" onClick={handleTest} disabled={testing}>
              {testing ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <Plug className="w-3.5 h-3.5" />
              )}
              Test
            </Button>
            <Button
              variant="primary"
              size="sm"
              onClick={handleSave}
              disabled={!tested?.reachable}
              title={tested?.reachable ? "Save this endpoint" : "Test the connection first"}
            >
              Save
            </Button>
          </div>
        </div>
      )}
    </Card>
  );
};
