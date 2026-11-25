import React, { useCallback, useEffect, useRef, useState } from "react";
import { Loader2, Plus, RefreshCw, RotateCcw, Search, Trash2, X } from "lucide-react";
import { api } from "../../lib/api";
import {
  QUICK_REPOS,
  cleanValuesText,
  currentRevision,
  defaultChartRef,
  errorMessage,
  helmStatusTone,
  isValidReleaseName,
  shortUpdated,
} from "../../lib/helm";
import { cn } from "../../lib/utils";
import type { HelmChart, HelmRelease, HelmReleaseDetail, HelmRepo } from "../../types";
import { Button } from "../ui/Button";
import { useModal } from "../ui/ModalContext";
import { StatusBadge } from "../ui/StatusBadge";
import { useToast } from "../ui/Toast";

type OpResult<T> = { ok: true; value: T } | { ok: false; error: string };
type ReleaseRef = { namespace: string; name: string };
type DetailTab = "values" | "all" | "manifest" | "notes" | "history";

const DETAIL_TABS: { id: DetailTab; label: string }[] = [
  { id: "values", label: "Values" },
  { id: "all", label: "All values" },
  { id: "manifest", label: "Manifest" },
  { id: "notes", label: "Notes" },
  { id: "history", label: "History" },
];

const INPUT_CLASS =
  "h-9 w-full px-3 rounded-none bg-zinc-950 border border-zinc-800 text-sm text-white placeholder-zinc-600 focus:outline-none focus:border-indigo-500 disabled:opacity-60";
const TEXTAREA_CLASS =
  "w-full px-3 py-2 rounded-none bg-zinc-950 border border-zinc-800 font-mono text-xs leading-relaxed text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 disabled:opacity-60";

/**
 * One Helm operation at a time. `run` refuses (returns null) while another
 * operation holds the flag, so two Helm calls can never overlap even when a
 * click slips past the disabled buttons.
 */
function useHelmOps() {
  const [busy, setBusy] = useState<string | null>(null);
  const busyRef = useRef<string | null>(null);

  const run = useCallback(async <T,>(label: string, fn: () => Promise<T>): Promise<OpResult<T> | null> => {
    if (busyRef.current !== null) return null;
    busyRef.current = label;
    setBusy(label);
    try {
      return { ok: true, value: await fn() };
    } catch (e) {
      return { ok: false, error: errorMessage(e) };
    } finally {
      busyRef.current = null;
      setBusy(null);
    }
  }, []);

  return { busy, run };
}

const ErrorBox: React.FC<{ message: string }> = ({ message }) => (
  <div className="rounded-none border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-300 whitespace-pre-wrap break-words">
    {message}
  </div>
);

const Pre: React.FC<{ text: string }> = ({ text }) => (
  <pre className="max-h-[28rem] overflow-auto whitespace-pre rounded-none border border-zinc-800 bg-zinc-950 p-3 font-mono text-xs leading-relaxed text-zinc-300">
    {text.trim() ? text : "(empty)"}
  </pre>
);

/** Helm release or revision status rendered with the shared badge tone. */
const HelmStatus: React.FC<{ status: string }> = ({ status }) => (
  <StatusBadge status={status || "unknown"} tone={helmStatusTone(status)} />
);

// ---------------------------------------------------------------------------
// Upgrade editor
// ---------------------------------------------------------------------------

interface UpgradeFormProps {
  cluster: string;
  release: HelmRelease;
  initialValues: string;
  busy: string | null;
  run: <T>(label: string, fn: () => Promise<T>) => Promise<OpResult<T> | null>;
  onCancel: () => void;
  onDone: (message: string) => void;
}

const UpgradeForm: React.FC<UpgradeFormProps> = ({
  cluster,
  release,
  initialValues,
  busy,
  run,
  onCancel,
  onDone,
}) => {
  const [chart, setChart] = useState(() => defaultChartRef(release.chart));
  const [version, setVersion] = useState("");
  const [values, setValues] = useState(() => cleanValuesText(initialValues));
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    const ref = chart.trim();
    if (!ref) {
      setError("Chart is required, for example bitnami/redis.");
      return;
    }
    const r = await run("upgrade", () =>
      api.helmInstall({
        cluster,
        namespace: release.namespace,
        name: release.name,
        chart: ref,
        version: version.trim() || null,
        values: values.trim() ? values : null,
        upgrade: true,
      })
    );
    if (!r) return;
    if (!r.ok) {
      setError(r.error);
      return;
    }
    onDone(r.value);
  };

  return (
    <div className="space-y-3 border-b border-zinc-800 bg-zinc-950/40 px-4 py-4">
      <div className="text-sm font-medium text-white">Upgrade {release.name}</div>
      <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
        <label className="block">
          <span className="mb-1 block text-xs text-zinc-400">Chart (repo/chart)</span>
          <input
            type="text"
            value={chart}
            disabled={!!busy}
            onChange={(e) => setChart(e.target.value)}
            placeholder="bitnami/redis"
            className={INPUT_CLASS}
          />
        </label>
        <label className="block">
          <span className="mb-1 block text-xs text-zinc-400">Version (optional)</span>
          <input
            type="text"
            value={version}
            disabled={!!busy}
            onChange={(e) => setVersion(e.target.value)}
            placeholder="latest"
            className={INPUT_CLASS}
          />
        </label>
      </div>
      <label className="block">
        <span className="mb-1 block text-xs text-zinc-400">Values (YAML, pre-filled with the current overrides)</span>
        <textarea
          value={values}
          disabled={!!busy}
          onChange={(e) => setValues(e.target.value)}
          rows={12}
          spellCheck={false}
          className={TEXTAREA_CLASS}
        />
      </label>
      {error && <ErrorBox message={error} />}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-[11px] text-zinc-500">Upgrades can take up to 10 minutes.</p>
        <div className="flex gap-2">
          <Button variant="ghost" size="sm" className="rounded-none" onClick={onCancel} disabled={!!busy}>
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            className="rounded-none"
            onClick={() => void submit()}
            disabled={!!busy}
            isLoading={busy === "upgrade"}
          >
            {busy === "upgrade" ? "Upgrading…" : "Upgrade"}
          </Button>
        </div>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Install dialog
// ---------------------------------------------------------------------------

interface InstallDialogProps {
  cluster: string;
  busy: string | null;
  run: <T>(label: string, fn: () => Promise<T>) => Promise<OpResult<T> | null>;
  onClose: () => void;
  onInstalled: () => Promise<void>;
}

const InstallDialog: React.FC<InstallDialogProps> = ({ cluster, busy, run, onClose, onInstalled }) => {
  const toast = useToast();
  const modal = useModal();

  const [repos, setRepos] = useState<HelmRepo[] | null>(null);
  const [repoError, setRepoError] = useState<string | null>(null);
  const [repoName, setRepoName] = useState("");
  const [repoUrl, setRepoUrl] = useState("");

  const [query, setQuery] = useState("");
  const [results, setResults] = useState<HelmChart[] | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);

  const [picked, setPicked] = useState<HelmChart | null>(null);
  const [valuesText, setValuesText] = useState("");
  const [valuesError, setValuesError] = useState<string | null>(null);
  const [releaseName, setReleaseName] = useState("");
  const [namespace, setNamespace] = useState("default");
  const [version, setVersion] = useState("");
  const [installError, setInstallError] = useState<string | null>(null);

  const loadRepos = useCallback(async () => {
    const r = await run("repos", () => api.helmRepoList());
    if (!r) return;
    if (r.ok) {
      setRepos(r.value);
      setRepoError(null);
    } else {
      setRepoError(r.error);
    }
  }, [run]);

  useEffect(() => {
    void loadRepos();
  }, [loadRepos]);

  const addRepo = async (name: string, url: string) => {
    const n = name.trim();
    const u = url.trim();
    if (!n || !u) {
      setRepoError("Repository name and URL are required.");
      return;
    }
    const r = await run("repo-add", () => api.helmRepoAdd(n, u));
    if (!r) return;
    if (!r.ok) {
      setRepoError(r.error);
      return;
    }
    setRepoError(null);
    setRepoName("");
    setRepoUrl("");
    toast.success("Repository added", n);
    await loadRepos();
  };

  const removeRepo = async (name: string) => {
    const ok = await modal.showConfirm(
      `Remove the Helm repository "${name}"? Charts from it will no longer be searchable until it is added back.`,
      "Remove repository",
      "warning"
    );
    if (!ok) return;
    const r = await run("repo-remove", () => api.helmRepoRemove(name));
    if (!r) return;
    if (!r.ok) {
      setRepoError(r.error);
      return;
    }
    toast.success("Repository removed", name);
    await loadRepos();
  };

  const updateRepos = async () => {
    const r = await run("repo-update", () => api.helmRepoUpdate());
    if (!r) return;
    if (!r.ok) {
      setRepoError(r.error);
      return;
    }
    setRepoError(null);
    toast.success("Repositories updated", r.value.trim() || undefined);
  };

  const search = async () => {
    const q = query.trim();
    if (!q) return;
    const r = await run("search", () => api.helmSearch(q));
    if (!r) return;
    if (r.ok) {
      setResults(r.value);
      setSearchError(null);
    } else {
      setResults(null);
      setSearchError(r.error);
    }
  };

  const pick = async (chart: HelmChart) => {
    setPicked(chart);
    setVersion(chart.version);
    setReleaseName("");
    setNamespace("default");
    setValuesText("");
    setValuesError(null);
    setInstallError(null);
    const r = await run("values", () => api.helmShowValues(chart.name, chart.version || undefined));
    if (!r) return;
    if (r.ok) setValuesText(r.value);
    else setValuesError(r.error);
  };

  const trimmedName = releaseName.trim();
  const nameInvalid = trimmedName !== "" && !isValidReleaseName(trimmedName);
  const canInstall =
    !!picked && isValidReleaseName(trimmedName) && namespace.trim() !== "" && !busy;

  const install = async () => {
    if (!picked || !canInstall) return;
    const r = await run("install", () =>
      api.helmInstall({
        cluster,
        namespace: namespace.trim(),
        name: trimmedName,
        chart: picked.name,
        version: version.trim() || null,
        values: valuesText.trim() ? valuesText : null,
        upgrade: false,
      })
    );
    if (!r) return;
    if (!r.ok) {
      setInstallError(r.error);
      return;
    }
    toast.success("Chart installed", r.value);
    await onInstalled();
  };

  const repoNames = new Set((repos ?? []).map((r) => r.name));

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={() => !busy && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="helm-install-title"
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[90vh] w-full max-w-3xl flex-col overflow-hidden rounded-none border border-zinc-800 bg-zinc-900 shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-zinc-800 px-5 py-4">
          <div>
            <span id="helm-install-title" className="text-sm font-semibold text-white">
              Install Helm chart
            </span>
            <p className="mt-0.5 text-[11px] text-zinc-500">
              The first Helm call may download the helm binary.
            </p>
            {busy === "install" && (
              <p className="mt-0.5 text-[11px] text-amber-400">Installing can take up to 10 minutes.</p>
            )}
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={!!busy}
            aria-label="Close install dialog"
            title="Close"
            className="rounded-none text-zinc-400 outline-none hover:text-white focus-visible:ring-2 focus-visible:ring-blue-500 disabled:opacity-50"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="flex-1 space-y-6 overflow-y-auto px-5 py-4">
          {/* Repositories */}
          <section className="space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-400">Repositories</h3>
              <div className="flex flex-wrap gap-2">
                {QUICK_REPOS.map((q) => (
                  <Button
                    key={q.name}
                    variant="outline"
                    size="sm"
                    className="rounded-none gap-1.5"
                    disabled={!!busy || repoNames.has(q.name)}
                    onClick={() => void addRepo(q.name, q.url)}
                    title={q.url}
                  >
                    <Plus className="h-3.5 w-3.5" />
                    {q.name}
                  </Button>
                ))}
                <Button
                  variant="secondary"
                  size="sm"
                  className="rounded-none gap-1.5"
                  disabled={!!busy}
                  isLoading={busy === "repo-update"}
                  onClick={() => void updateRepos()}
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                  Update repos
                </Button>
              </div>
            </div>

            {repoError && <ErrorBox message={repoError} />}

            <div className="divide-y divide-zinc-800 rounded-none border border-zinc-800">
              {repos === null && (
                <div className="flex items-center gap-2 px-4 py-3 text-xs text-zinc-500">
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  Loading repositories…
                </div>
              )}
              {repos !== null && repos.length === 0 && (
                <div className="px-4 py-3 text-xs text-zinc-500">No repositories yet. Add one below or use a quick add.</div>
              )}
              {(repos ?? []).map((r) => (
                <div key={r.name} className="flex items-stretch divide-x divide-zinc-800">
                  <div className="w-40 shrink-0 px-3 py-2 font-mono text-xs text-white">{r.name}</div>
                  <div className="min-w-0 flex-1 truncate px-3 py-2 text-xs text-zinc-400" title={r.url}>
                    {r.url}
                  </div>
                  <div className="flex items-center px-2">
                    <Button
                      variant="ghost"
                      size="sm"
                      className="rounded-none gap-1.5 text-rose-300 hover:text-rose-200"
                      disabled={!!busy}
                      onClick={() => void removeRepo(r.name)}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                      Remove
                    </Button>
                  </div>
                </div>
              ))}
            </div>

            <form
              className="grid grid-cols-1 gap-2 md:grid-cols-[12rem_1fr_auto]"
              onSubmit={(e) => {
                e.preventDefault();
                void addRepo(repoName, repoUrl);
              }}
            >
              <input
                type="text"
                aria-label="Repository name"
                value={repoName}
                disabled={!!busy}
                onChange={(e) => setRepoName(e.target.value)}
                placeholder="name"
                className={INPUT_CLASS}
              />
              <input
                type="text"
                aria-label="Repository URL"
                value={repoUrl}
                disabled={!!busy}
                onChange={(e) => setRepoUrl(e.target.value)}
                placeholder="https://charts.example.com"
                className={INPUT_CLASS}
              />
              <Button
                type="submit"
                variant="secondary"
                size="md"
                className="rounded-none"
                disabled={!!busy || !repoName.trim() || !repoUrl.trim()}
                isLoading={busy === "repo-add"}
              >
                Add repo
              </Button>
            </form>
          </section>

          {/* Search */}
          <section className="space-y-3">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-400">Search charts</h3>
            <form
              className="flex gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                void search();
              }}
            >
              <input
                type="text"
                aria-label="Chart search"
                value={query}
                disabled={!!busy}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="redis, nginx, postgres…"
                className={INPUT_CLASS}
              />
              <Button
                type="submit"
                variant="secondary"
                size="md"
                className="rounded-none gap-2"
                disabled={!!busy || !query.trim()}
                isLoading={busy === "search"}
              >
                <Search className="h-4 w-4" />
                Search
              </Button>
            </form>

            {searchError && <ErrorBox message={searchError} />}

            {results !== null && (
              <div className="max-h-64 divide-y divide-zinc-800 overflow-y-auto rounded-none border border-zinc-800">
                {results.length === 0 && (
                  <div className="px-4 py-3 text-xs text-zinc-500">
                    No charts match. Add the repository that publishes it first.
                  </div>
                )}
                {results.map((c) => (
                  <button
                    key={`${c.name}@${c.version}`}
                    type="button"
                    disabled={!!busy}
                    onClick={() => void pick(c)}
                    className={cn(
                      "flex w-full items-stretch divide-x divide-zinc-800 rounded-none text-left hover:bg-zinc-800/40 disabled:pointer-events-none disabled:opacity-60",
                      picked?.name === c.name && picked.version === c.version && "bg-zinc-800/60"
                    )}
                  >
                    <div className="w-56 shrink-0 truncate px-3 py-2 font-mono text-xs text-white">{c.name}</div>
                    <div className="w-24 shrink-0 px-3 py-2 font-mono text-xs text-zinc-300">{c.version}</div>
                    <div className="w-24 shrink-0 truncate px-3 py-2 text-xs text-zinc-400">{c.app_version}</div>
                    <div className="min-w-0 flex-1 truncate px-3 py-2 text-xs text-zinc-500" title={c.description}>
                      {c.description}
                    </div>
                  </button>
                ))}
              </div>
            )}
          </section>

          {/* Configure and install */}
          {picked && (
            <section className="space-y-3 border-t border-zinc-800 pt-5">
              <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-400">
                Configure {picked.name} {picked.version}
              </h3>

              <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
                <label className="block">
                  <span className="mb-1 block text-xs text-zinc-400">Release name (required)</span>
                  <input
                    type="text"
                    value={releaseName}
                    disabled={!!busy}
                    onChange={(e) => setReleaseName(e.target.value)}
                    placeholder="my-redis"
                    className={cn(INPUT_CLASS, nameInvalid && "border-rose-500/60")}
                  />
                  {nameInvalid && (
                    <span className="mt-1 block text-[11px] text-rose-300">
                      Lowercase letters, digits and '-', start with a letter, at most 53 characters.
                    </span>
                  )}
                </label>
                <label className="block">
                  <span className="mb-1 block text-xs text-zinc-400">Namespace</span>
                  <input
                    type="text"
                    value={namespace}
                    disabled={!!busy}
                    onChange={(e) => setNamespace(e.target.value)}
                    className={INPUT_CLASS}
                  />
                </label>
                <label className="block">
                  <span className="mb-1 block text-xs text-zinc-400">Version (optional)</span>
                  <input
                    type="text"
                    value={version}
                    disabled={!!busy}
                    onChange={(e) => setVersion(e.target.value)}
                    className={INPUT_CLASS}
                  />
                </label>
              </div>

              <label className="block">
                <span className="mb-1 block text-xs text-zinc-400">
                  Values (YAML){busy === "values" && " · loading chart defaults…"}
                </span>
                <textarea
                  value={valuesText}
                  disabled={!!busy}
                  onChange={(e) => setValuesText(e.target.value)}
                  rows={12}
                  spellCheck={false}
                  className={TEXTAREA_CLASS}
                />
              </label>
              {valuesError && <ErrorBox message={`Could not load default values: ${valuesError}`} />}
              {installError && <ErrorBox message={installError} />}

              <div className="flex flex-wrap items-center justify-between gap-3">
                <Button
                  variant="ghost"
                  size="sm"
                  className="rounded-none"
                  disabled={!!busy}
                  onClick={() => setPicked(null)}
                >
                  Back to search
                </Button>
                <div className="flex items-center gap-3">
                  {busy === "install" && (
                    <span className="text-[11px] text-zinc-500">Installing can take up to 10 minutes.</span>
                  )}
                  <Button
                    variant="primary"
                    size="md"
                    className="rounded-none"
                    disabled={!canInstall}
                    isLoading={busy === "install"}
                    onClick={() => void install()}
                  >
                    {busy === "install" ? "Installing…" : "Install"}
                  </Button>
                </div>
              </div>
            </section>
          )}
        </div>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

export const HelmPanel: React.FC<{ cluster: string }> = ({ cluster }) => {
  const toast = useToast();
  const modal = useModal();
  const { busy, run } = useHelmOps();

  const clusterRef = useRef(cluster);
  useEffect(() => {
    clusterRef.current = cluster;
  }, [cluster]);

  const [releases, setReleases] = useState<HelmRelease[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [selected, setSelected] = useState<ReleaseRef | null>(null);
  const [detail, setDetail] = useState<HelmReleaseDetail | null>(null);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [tab, setTab] = useState<DetailTab>("values");
  const [upgradeOpen, setUpgradeOpen] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [installOpen, setInstallOpen] = useState(false);

  const refreshReleases = useCallback(async () => {
    const c = cluster;
    const r = await run("list", () => api.helmListReleases(c));
    if (!r || clusterRef.current !== c) return;
    if (r.ok) {
      setReleases(r.value);
      setListError(null);
    } else {
      setListError(r.error);
    }
  }, [cluster, run]);

  const loadDetail = useCallback(
    async (ref: ReleaseRef) => {
      const c = cluster;
      const r = await run("detail", () => api.helmReleaseDetail(c, ref.namespace, ref.name));
      if (!r || clusterRef.current !== c) return;
      if (r.ok) {
        setDetail(r.value);
        setDetailError(null);
      } else {
        setDetail(null);
        setDetailError(r.error);
      }
    },
    [cluster, run]
  );

  // Reset everything when the cluster changes, then load its releases.
  useEffect(() => {
    setReleases(null);
    setListError(null);
    setSelected(null);
    setDetail(null);
    setDetailError(null);
    setTab("values");
    setUpgradeOpen(false);
    setActionError(null);
    setInstallOpen(false);
    void refreshReleases();
  }, [refreshReleases]);

  const openRelease = (rel: HelmRelease) => {
    const ref = { namespace: rel.namespace, name: rel.name };
    setSelected(ref);
    setDetail(null);
    setDetailError(null);
    setTab("values");
    setUpgradeOpen(false);
    setActionError(null);
    void loadDetail(ref);
  };

  const closeDetail = () => {
    setSelected(null);
    setDetail(null);
    setDetailError(null);
    setUpgradeOpen(false);
    setActionError(null);
  };

  const current = selected
    ? (releases ?? []).find((r) => r.namespace === selected.namespace && r.name === selected.name) ?? null
    : null;

  const uninstall = async (ref: ReleaseRef) => {
    const ok = await modal.showConfirm(
      `Uninstall "${ref.name}" from namespace "${ref.namespace}" in ${cluster}? Its resources will be deleted.`,
      "Uninstall release",
      "danger"
    );
    if (!ok) return;
    const c = cluster;
    const r = await run("uninstall", () => api.helmUninstall(c, ref.namespace, ref.name));
    if (!r) return;
    if (!r.ok) {
      setActionError(r.error);
      return;
    }
    toast.success("Release uninstalled", `${ref.name} removed from ${ref.namespace}`);
    closeDetail();
    await refreshReleases();
  };

  const rollback = async (ref: ReleaseRef, revision: number) => {
    const ok = await modal.showConfirm(
      `Roll back "${ref.name}" to revision ${revision}? Resources will be changed to match that revision.`,
      "Roll back release",
      "warning"
    );
    if (!ok) return;
    const c = cluster;
    const r = await run("rollback", () => api.helmRollback(c, ref.namespace, ref.name, revision));
    if (!r) return;
    if (!r.ok) {
      setActionError(r.error);
      return;
    }
    setActionError(null);
    toast.success("Release rolled back", `${ref.name} is back at revision ${revision}`);
    await loadDetail(ref);
    await refreshReleases();
  };

  const onUpgraded = async (ref: ReleaseRef, message: string) => {
    setUpgradeOpen(false);
    setActionError(null);
    toast.success("Release upgraded", message);
    await loadDetail(ref);
    await refreshReleases();
  };

  const listBusy = busy === "list";

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold text-white">Helm releases</h2>
          <p className="text-[11px] text-zinc-500">
            {releases ? `${releases.length} release${releases.length === 1 ? "" : "s"}` : "Loading…"} in {cluster}
          </p>
        </div>
        <div className="flex gap-2">
          <Button
            variant="secondary"
            size="sm"
            className="rounded-none gap-2"
            disabled={!!busy}
            isLoading={listBusy}
            onClick={() => void refreshReleases()}
          >
            <RefreshCw className="h-3.5 w-3.5" />
            Refresh
          </Button>
          <Button
            variant="primary"
            size="sm"
            className="rounded-none gap-2"
            disabled={!!busy}
            onClick={() => setInstallOpen(true)}
          >
            <Plus className="h-3.5 w-3.5" />
            Install chart
          </Button>
        </div>
      </div>

      {listError && <ErrorBox message={listError} />}

      <div className="rounded-none border border-zinc-800 bg-zinc-900/90">
        {releases === null && !listError && (
          <div className="flex items-center gap-2 px-4 py-6 text-sm text-zinc-500">
            <Loader2 className="h-4 w-4 animate-spin" />
            Loading releases…
          </div>
        )}
        {releases !== null && releases.length === 0 && (
          <div className="px-4 py-6 text-sm text-zinc-500">
            No Helm releases in this cluster. Install a chart to get started.
          </div>
        )}
        {releases !== null && releases.length > 0 && (
          <ul className="divide-y divide-zinc-800">
            {releases.map((rel) => {
              const isSelected = selected?.namespace === rel.namespace && selected?.name === rel.name;
              return (
                <li key={`${rel.namespace}/${rel.name}`}>
                  <button
                    type="button"
                    disabled={!!busy}
                    onClick={() => openRelease(rel)}
                    className={cn(
                      "flex w-full items-stretch divide-x divide-zinc-800 rounded-none text-left hover:bg-zinc-800/40 disabled:pointer-events-none disabled:opacity-60",
                      isSelected && "bg-zinc-800/60"
                    )}
                  >
                    <div className="min-w-0 flex-1 px-4 py-3">
                      <div className="truncate text-sm font-medium text-white">{rel.name}</div>
                      <div className="truncate font-mono text-[11px] text-zinc-500">{rel.namespace}</div>
                    </div>
                    <div className="w-52 shrink-0 px-4 py-3">
                      <div className="truncate font-mono text-xs text-zinc-300" title={rel.chart}>
                        {rel.chart}
                      </div>
                      <div className="truncate text-[11px] text-zinc-500">app {rel.app_version || "-"}</div>
                    </div>
                    <div className="w-20 shrink-0 px-4 py-3 font-mono text-xs text-zinc-300">rev {rel.revision}</div>
                    <div className="flex w-40 shrink-0 items-center px-4 py-3">
                      <HelmStatus status={rel.status} />
                    </div>
                    <div
                      className="w-44 shrink-0 truncate px-4 py-3 text-xs text-zinc-400"
                      title={rel.updated}
                    >
                      {shortUpdated(rel.updated) || "-"}
                    </div>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>

      {selected && (
        <div className="rounded-none border border-zinc-800 bg-zinc-900/90">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-zinc-800 px-4 py-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <span className="truncate text-sm font-semibold text-white">{selected.name}</span>
                {current && <HelmStatus status={current.status} />}
              </div>
              <div className="truncate font-mono text-[11px] text-zinc-500">
                {selected.namespace}
                {current ? ` · ${current.chart}` : ""}
              </div>
            </div>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                size="sm"
                className="rounded-none"
                disabled={!!busy || !detail || !current}
                onClick={() => {
                  setActionError(null);
                  setUpgradeOpen(true);
                }}
              >
                Upgrade
              </Button>
              <Button
                variant="danger"
                size="sm"
                className="rounded-none gap-1.5"
                disabled={!!busy}
                isLoading={busy === "uninstall"}
                onClick={() => void uninstall(selected)}
              >
                <Trash2 className="h-3.5 w-3.5" />
                Uninstall
              </Button>
              <Button variant="ghost" size="sm" className="rounded-none gap-1.5" onClick={closeDetail}>
                <X className="h-3.5 w-3.5" />
                Close
              </Button>
            </div>
          </div>

          <div className="flex divide-x divide-zinc-800 overflow-x-auto border-b border-zinc-800">
            {DETAIL_TABS.map((t) => (
              <button
                key={t.id}
                type="button"
                onClick={() => setTab(t.id)}
                aria-pressed={tab === t.id}
                className={cn(
                  "shrink-0 rounded-none px-4 py-2 text-xs font-medium transition-colors",
                  tab === t.id ? "bg-zinc-800/70 text-white" : "text-zinc-400 hover:bg-zinc-800/40 hover:text-zinc-200"
                )}
              >
                {t.label}
              </button>
            ))}
          </div>

          {upgradeOpen && current && detail && (
            <UpgradeForm
              key={`${selected.namespace}/${selected.name}`}
              cluster={cluster}
              release={current}
              initialValues={detail.values}
              busy={busy}
              run={run}
              onCancel={() => setUpgradeOpen(false)}
              onDone={(message) => void onUpgraded(selected, message)}
            />
          )}

          <div className="space-y-3 px-4 py-4">
            {actionError && <ErrorBox message={actionError} />}
            {detailError && <ErrorBox message={detailError} />}
            {!detail && !detailError && (
              <div className="flex items-center gap-2 text-sm text-zinc-500">
                <Loader2 className="h-4 w-4 animate-spin" />
                Loading release details…
              </div>
            )}
            {detail && tab === "values" && <Pre text={detail.values} />}
            {detail && tab === "all" && <Pre text={detail.allValues} />}
            {detail && tab === "manifest" && <Pre text={detail.manifest} />}
            {detail && tab === "notes" && <Pre text={detail.notes} />}
            {detail && tab === "history" && (
              <HistoryTable
                history={detail.history}
                busy={busy}
                onRollback={(revision) => void rollback(selected, revision)}
              />
            )}
          </div>
        </div>
      )}

      {installOpen && (
        <InstallDialog
          cluster={cluster}
          busy={busy}
          run={run}
          onClose={() => setInstallOpen(false)}
          onInstalled={async () => {
            setInstallOpen(false);
            await refreshReleases();
          }}
        />
      )}
    </div>
  );
};

interface HistoryTableProps {
  history: HelmReleaseDetail["history"];
  busy: string | null;
  onRollback: (revision: number) => void;
}

const HistoryTable: React.FC<HistoryTableProps> = ({ history, busy, onRollback }) => {
  const current = currentRevision(history);
  const rows = [...history].sort((a, b) => b.revision - a.revision);

  if (rows.length === 0) {
    return <div className="text-sm text-zinc-500">No revisions recorded.</div>;
  }

  return (
    <div className="divide-y divide-zinc-800 overflow-x-auto rounded-none border border-zinc-800">
      {rows.map((h) => (
        <div key={h.revision} className="flex min-w-[46rem] items-stretch divide-x divide-zinc-800 text-sm">
          <div className="w-16 shrink-0 px-3 py-2.5 font-mono text-white">#{h.revision}</div>
          <div className="w-40 shrink-0 px-3 py-2.5 text-xs text-zinc-400">{shortUpdated(h.updated) || "-"}</div>
          <div className="flex w-32 shrink-0 items-center px-3 py-2.5">
            <HelmStatus status={h.status} />
          </div>
          <div className="w-44 shrink-0 truncate px-3 py-2.5 font-mono text-xs text-zinc-300" title={h.chart}>
            {h.chart}
          </div>
          <div className="min-w-0 flex-1 truncate px-3 py-2.5 text-xs text-zinc-500" title={h.description}>
            {h.description}
          </div>
          <div className="flex w-32 shrink-0 items-center justify-end px-2 py-1.5">
            {h.revision < current && (
              <Button
                variant="outline"
                size="sm"
                className="rounded-none gap-1.5"
                disabled={!!busy}
                onClick={() => onRollback(h.revision)}
              >
                <RotateCcw className="h-3.5 w-3.5" />
                Rollback
              </Button>
            )}
          </div>
        </div>
      ))}
    </div>
  );
};
