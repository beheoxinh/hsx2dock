import React, { useEffect, useState } from "react";
import {
  CheckCircle2,
  KeyRound,
  Loader2,
  Pencil,
  Plug,
  Plus,
  ShieldAlert,
  Star,
  Trash2,
  XCircle,
} from "lucide-react";
import { Card } from "../ui/Card";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { useModal } from "../ui/ModalContext";
import { api } from "../../lib/api";
import type { RegistryInput, RegistryStatus, RegistryTestResult } from "../../types";

/** One-click presets for the registries people actually use. */
const PRESETS: { label: string; host: string; hint?: string }[] = [
  { label: "Docker Hub", host: "docker.io" },
  { label: "GitHub ghcr", host: "ghcr.io" },
  { label: "GitLab.com", host: "registry.gitlab.com" },
  { label: "GitLab self-hosted", host: "registry.example.com", hint: "your GitLab's registry host" },
  { label: "Quay", host: "quay.io" },
  { label: "Harbor", host: "harbor.example.com", hint: "your Harbor host[:port]" },
];

interface DraftState {
  id: string | null;
  name: string;
  host: string;
  username: string;
  /** Empty on edit = keep the stored secret. */
  password: string;
  insecure: boolean;
}

const EMPTY_DRAFT: DraftState = {
  id: null,
  name: "",
  host: "",
  username: "",
  password: "",
  insecure: false,
};

/**
 * Registry login manager. Any host that speaks the Docker Registry v2 API
 * works — Docker Hub, ghcr.io, GitLab, a personal Harbor, or a community
 * registry. Saved logins are attached automatically to pull/push operations
 * whose image reference points at that host.
 */
export const RegistriesCard: React.FC = () => {
  const toast = useToast();
  const modal = useModal();
  const [registries, setRegistries] = useState<RegistryStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [draft, setDraft] = useState<DraftState | null>(null);
  const [testing, setTesting] = useState(false);
  const [tested, setTested] = useState<RegistryTestResult | null>(null);
  /** Id of the login whose default toggle is in flight, so its star can spin. */
  const [busyId, setBusyId] = useState<string | null>(null);

  const load = async () => {
    try {
      setRegistries(await api.listRegistries());
    } catch (e) {
      toast.error("Could not list registry logins", String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const openNew = (presetHost?: string, presetLabel?: string) => {
    setDraft({
      ...EMPTY_DRAFT,
      host: presetHost ?? "",
      name: presetLabel ?? "",
    });
    setTested(null);
  };

  const openEdit = (reg: RegistryStatus) => {
    setDraft({
      id: reg.id,
      name: reg.name,
      host: reg.host,
      username: reg.username,
      password: "",
      insecure: reg.insecure,
    });
    setTested(null);
  };

  const payload = (d: DraftState): RegistryInput => ({
    id: d.id,
    name: d.name.trim(),
    host: d.host.trim(),
    username: d.username.trim(),
    password: d.password,
    insecure: d.insecure,
  });

  const validateDraft = (): string | null => {
    if (!draft) return "No draft";
    if (!draft.host.trim()) return "Registry host is required";
    if (!draft.username.trim()) return "Username is required";
    if (!draft.password.trim() && !draft.id)
      return "Password or access token is required";
    return null;
  };

  const handleTest = async () => {
    if (!draft) return;
    const problem = validateDraft();
    if (problem) {
      toast.error(problem);
      return;
    }
    try {
      setTesting(true);
      setTested(null);
      const res = await api.testRegistryAuth(payload(draft));
      setTested(res);
      if (res.ok) {
        toast.success("Login verified", res.message);
      } else {
        toast.error("Registry refused the login", res.message);
      }
    } catch (e) {
      setTested({ ok: false, message: String(e), latency_ms: 0 });
      toast.error("Could not test the login", String(e));
    } finally {
      setTesting(false);
    }
  };

  const handleSave = async () => {
    if (!draft) return;
    const problem = validateDraft();
    if (problem) {
      toast.error(problem);
      return;
    }
    if (!tested?.ok) {
      toast.error("Test the login first", "A login is only saved after it verifies.");
      return;
    }
    try {
      await api.saveRegistryAuth(payload(draft));
      toast.success("Registry login saved", draft.host.trim());
      setDraft(null);
      setTested(null);
      await load();
    } catch (e) {
      toast.error("Could not save the login", String(e));
    }
  };

  const handleRemove = async (reg: RegistryStatus) => {
    const ok = await modal.showConfirm(
      `Remove the saved login for "${reg.host}" (user ${reg.username})?\n\nImages from this registry will fall back to anonymous pulls.`,
      "Remove Registry Login",
      "danger"
    );
    if (!ok) return;
    try {
      await api.removeRegistryAuth(reg.id);
      toast.success("Login removed", reg.host);
      await load();
    } catch (e) {
      toast.error("Could not remove the login", String(e));
    }
  };

  const handleMakeDefault = async (reg: RegistryStatus) => {
    if (reg.is_default) return;
    setBusyId(reg.id);
    try {
      await api.setDefaultRegistry(reg.id);
      toast.success("Default registry set", `New pushes go to ${reg.host}`);
      await load();
    } catch (e) {
      toast.error("Could not set the default registry", String(e));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <Card className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <div>
          <h3 className="flex items-center gap-2 text-sm font-semibold text-white">
            <KeyRound className="w-4 h-4 text-amber-400" />
            Registry Logins
          </h3>
          <p className="text-xs text-zinc-500 mt-1">
            Sign in to Docker Hub, GitHub, GitLab, Harbor, or any custom registry.
            Saved logins are used automatically when pulling or pushing images
            from that host. Star a login to make it the default push target.
          </p>
        </div>
        <Button variant="secondary" size="sm" onClick={() => openNew()} className="shrink-0 whitespace-nowrap">
          <Plus className="w-4 h-4" />
          Add login
        </Button>
      </div>

      {/* Preset quick-add chips */}
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-[11px] text-zinc-600">Quick add:</span>
        {PRESETS.map((p) => (
          <button
            key={p.label}
            type="button"
            onClick={() => openNew(p.host, p.label)}
            title={p.hint}
            className="rounded-md border border-zinc-800 bg-zinc-900/50 px-2 py-0.5 text-[11px] text-zinc-400 hover:border-zinc-700 hover:text-zinc-200 transition-colors"
          >
            {p.label}
          </button>
        ))}
      </div>

      {loading ? (
        <div className="flex items-center gap-2 py-3 text-xs text-zinc-500">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Loading registry logins…
        </div>
      ) : registries.length === 0 && !draft ? (
        <p className="text-xs text-zinc-600 py-2">
          No registry logins saved yet. Public images still pull anonymously.
        </p>
      ) : (
        <div className="space-y-2">
          {registries.map((reg) => (
            <div
              key={reg.id}
              className="flex items-center justify-between gap-3 rounded-lg border border-zinc-800 bg-zinc-900/40 px-3 py-2.5"
            >
              <div className="flex min-w-0 items-center gap-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-zinc-900/70 text-amber-400">
                  <KeyRound className="h-4 w-4" />
                </div>
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="truncate text-sm text-zinc-100">{reg.name}</span>
                    {reg.is_default && (
                      <span className="flex shrink-0 items-center gap-1 rounded border border-emerald-500/30 bg-emerald-500/5 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-emerald-400">
                        <Star className="h-3 w-3" />
                        default
                      </span>
                    )}
                    {reg.insecure && (
                      <span className="flex shrink-0 items-center gap-1 rounded border border-amber-500/30 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-amber-400">
                        <ShieldAlert className="h-3 w-3" />
                        insecure TLS
                      </span>
                    )}
                  </div>
                  <div className="truncate font-mono text-[11px] text-zinc-500">
                    {reg.host} · {reg.username}
                  </div>
                </div>
              </div>

              <div className="flex shrink-0 items-center gap-1">
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => handleMakeDefault(reg)}
                  disabled={reg.is_default || busyId === reg.id}
                  title={
                    reg.is_default
                      ? "New pushes already default to this registry"
                      : "Make this the default push target"
                  }
                  className={
                    reg.is_default
                      ? "text-emerald-400"
                      : "text-zinc-500 hover:text-emerald-400"
                  }
                >
                  <Star className={`w-3.5 h-3.5 ${reg.is_default ? "fill-current" : ""}`} />
                </Button>
                {reg.has_password ? (
                  <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400" />
                ) : (
                  <XCircle className="h-3.5 w-3.5 text-zinc-600" />
                )}
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => openEdit(reg)}
                  title="Edit"
                  className="text-zinc-500 hover:text-zinc-100"
                >
                  <Pencil className="w-3.5 h-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => handleRemove(reg)}
                  title="Remove"
                  className="text-zinc-500 hover:text-rose-400"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}

      {draft && (
        <div className="space-y-3 rounded-lg border border-zinc-800 bg-zinc-900/30 p-3">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-zinc-400">Display name</label>
              <input
                type="text"
                autoFocus
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="GitHub ghcr"
                className="h-9 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-2.5 text-sm text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
              />
            </div>
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-zinc-400">Registry host</label>
              <input
                type="text"
                list="registry-host-presets"
                value={draft.host}
                onChange={(e) => {
                  setDraft({ ...draft, host: e.target.value });
                  setTested(null);
                }}
                placeholder="ghcr.io"
                className="h-9 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-2.5 font-mono text-xs text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
              />
              <datalist id="registry-host-presets">
                {PRESETS.map((p) => (
                  <option key={p.host} value={p.host} />
                ))}
                <option value="registry.example.com" />
              </datalist>
              <p className="text-[11px] text-zinc-600">
                Bare host (and port if needed) — no https:// prefix. Personal
                Harbor often lives at <span className="font-mono">harbor.host:8443</span>.
              </p>
            </div>
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-zinc-400">Username</label>
              <input
                type="text"
                value={draft.username}
                onChange={(e) => {
                  setDraft({ ...draft, username: e.target.value });
                  setTested(null);
                }}
                placeholder="your-user or robot$ci"
                className="h-9 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-2.5 text-sm text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
              />
            </div>
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-zinc-400">
                Password / access token
              </label>
              <input
                type="password"
                value={draft.password}
                onChange={(e) => {
                  setDraft({ ...draft, password: e.target.value });
                  setTested(null);
                }}
                placeholder={draft.id ? "•••••••• (unchanged if blank)" : "token or password"}
                className="h-9 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-2.5 text-sm text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
              />
              <p className="text-[11px] text-zinc-600">
                ghcr/GitLab need a token (write:packages / read_registry scope);
                Harbor uses an account or robot password.
              </p>
            </div>
          </div>

          <label className="flex items-center gap-2 text-xs text-zinc-400 cursor-pointer select-none">
            <input
              type="checkbox"
              checked={draft.insecure}
              onChange={(e) => {
                setDraft({ ...draft, insecure: e.target.checked });
                setTested(null);
              }}
              className="rounded border-zinc-700 bg-zinc-900"
            />
            Allow self-signed certificates or plain HTTP
            <span className="text-zinc-600">(personal Harbor behind a LAN cert)</span>
          </label>

          {tested && (
            <div
              className={`rounded-lg border px-3 py-2 text-xs ${
                tested.ok
                  ? "border-emerald-500/30 bg-emerald-500/5 text-emerald-400"
                  : "border-rose-500/30 bg-rose-500/5 text-rose-400"
              }`}
            >
              {tested.message}
              {tested.latency_ms > 0 && (
                <span className="opacity-70"> · {tested.latency_ms}ms</span>
              )}
            </div>
          )}

          <div className="flex items-center justify-end gap-2 pt-1">
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
              disabled={!tested?.ok}
              title={tested?.ok ? "Save this login" : "Test the login first"}
            >
              Save
            </Button>
          </div>
        </div>
      )}
    </Card>
  );
};
