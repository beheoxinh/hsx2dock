import React, { useEffect, useMemo, useState } from "react";
import { CheckCircle2, FileCode2, GitCompare, Rocket, X } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import {
  QUICK_DEPLOY_DEFAULTS,
  buildQuickDeploy,
  classifyDiff,
  diffStats,
  validateQuickDeploy,
} from "../../lib/k8sManifest";
import type { DiffLineKind, QuickDeployInput, QuickServiceType } from "../../lib/k8sManifest";

/** Same limit the backend enforces on `kubectl apply` input. */
const MAX_BYTES = 90 * 1024;
const encoder = new TextEncoder();
const byteLength = (s: string): number => encoder.encode(s).length;

const INPUT =
  "h-8 w-full rounded-lg border border-zinc-800 bg-zinc-950 px-2 text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-indigo-500";

const DIFF_LINE: Record<DiffLineKind, string> = {
  add: "bg-emerald-500/10 text-emerald-300",
  del: "bg-rose-500/10 text-rose-300",
  hunk: "bg-cyan-500/5 text-cyan-300",
  header: "font-semibold text-zinc-400",
  meta: "text-zinc-500",
  context: "text-zinc-500",
};

type Tab = "yaml" | "quick";

interface DiffResult {
  /** The YAML that was diffed; a different current text makes the diff stale. */
  source: string;
  text: string;
  changed: boolean;
}

/** Parses a number input; an empty field becomes NaN so validation flags it. */
const parseNum = (s: string): number => (s.trim() === "" ? NaN : Number(s));
const numText = (n: number): string => (Number.isNaN(n) ? "" : String(n));

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block space-y-1">
      <span className="text-[10px] uppercase tracking-wider text-zinc-500">{label}</span>
      {children}
    </label>
  );
}

/**
 * Apply raw YAML to a cluster (with a server-side dry-run diff first), or
 * generate a Deployment (+ Service) from a small form. Nothing is applied
 * without the user pressing Apply or Deploy.
 */
export function YamlApplyModal({
  cluster,
  initialYaml = "",
  initialTab = "yaml",
  title,
  onClose,
  onApplied,
}: {
  cluster: string;
  initialYaml?: string;
  initialTab?: "yaml" | "quick";
  title?: string;
  onClose: () => void;
  onApplied?: () => void;
}) {
  const toast = useToast();
  const [tab, setTab] = useState<Tab>(initialTab);
  const [yaml, setYaml] = useState(initialYaml);
  const [diff, setDiff] = useState<DiffResult | null>(null);
  const [busy, setBusy] = useState<"diff" | "apply" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [output, setOutput] = useState<string | null>(null);
  const [form, setForm] = useState<QuickDeployInput>(QUICK_DEPLOY_DEFAULTS);

  const bytes = byteLength(yaml);
  const overLimit = bytes > MAX_BYTES;
  const diffStale = diff !== null && diff.source !== yaml;
  const diffLines = useMemo(() => (diff ? classifyDiff(diff.text) : []), [diff]);
  const stats = useMemo(() => (diff ? diffStats(diff.text) : null), [diff]);
  const formErrors = useMemo(() => validateQuickDeploy(form), [form]);
  const preview = useMemo(() => buildQuickDeploy(form), [form]);

  // Escape closes, but never while a request is in flight.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && busy === null) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const setField = <K extends keyof QuickDeployInput>(key: K, value: QuickDeployInput[K]) =>
    setForm((f) => ({ ...f, [key]: value }));

  const runDiff = async () => {
    if (busy) return;
    if (!yaml.trim()) return setError("Nothing to diff: the YAML is empty.");
    if (overLimit) return setError("The YAML is over the 90 KiB limit.");
    const source = yaml;
    setBusy("diff");
    setError(null);
    try {
      const r = await api.diffClusterYaml(cluster, source);
      setDiff({ source, text: r.diff, changed: r.changed });
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const runApply = async (text: string) => {
    if (busy) return;
    if (!text.trim()) return setError("Nothing to apply: the YAML is empty.");
    if (byteLength(text) > MAX_BYTES) return setError("The YAML is over the 90 KiB limit.");
    setBusy("apply");
    setError(null);
    try {
      const out = await api.applyClusterYaml(cluster, text);
      setOutput(out.trim() || "applied");
      setDiff(null);
      toast.success("Applied", out.trim().split("\n")[0] || undefined);
      onApplied?.();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  // Tab inserts two spaces instead of moving focus out of the editor.
  const onEditorKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Tab" || e.shiftKey) return;
    e.preventDefault();
    const el = e.currentTarget;
    const start = el.selectionStart;
    const end = el.selectionEnd;
    setYaml(yaml.slice(0, start) + "  " + yaml.slice(end));
    requestAnimationFrame(() => {
      el.selectionStart = el.selectionEnd = start + 2;
    });
  };

  const openQuickInEditor = () => {
    setYaml(preview);
    setDiff(null);
    setError(null);
    setTab("yaml");
  };

  const tabButton = (id: Tab, label: string) => (
    <button
      type="button"
      role="tab"
      aria-selected={tab === id}
      onClick={() => setTab(id)}
      className={`px-3 py-2 text-xs font-medium rounded-t-lg transition-colors ${
        tab === id
          ? "text-white border-b-2 border-indigo-400 bg-zinc-800/40"
          : "text-zinc-400 hover:text-zinc-200"
      }`}
    >
      {label}
    </button>
  );

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title ?? "Apply YAML"}
        className="flex h-[85vh] w-[min(1000px,94vw)] flex-col rounded-xl border border-zinc-800 bg-zinc-900 shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-zinc-800 px-4 py-3">
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-white">{title ?? "Apply YAML"}</p>
            <p className="text-[11px] text-zinc-500">
              cluster <span className="font-mono">{cluster}</span> · server-side dry-run diff before apply
            </p>
          </div>
          <Button
            variant="ghost"
            size="icon"
            title="Close"
            aria-label="Close"
            onClick={onClose}
            className="h-7 w-7 text-zinc-400"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        <div role="tablist" className="flex items-center gap-1 border-b border-zinc-800/60 px-4 pt-2">
          {tabButton("yaml", "YAML")}
          {tabButton("quick", "Quick deploy")}
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto p-4">
          {tab === "yaml" ? (
            <>
              <textarea
                value={yaml}
                onChange={(e) => setYaml(e.target.value)}
                onKeyDown={onEditorKeyDown}
                spellCheck={false}
                aria-label="Manifest YAML"
                placeholder="apiVersion: v1&#10;kind: ConfigMap&#10;…"
                className="min-h-[220px] flex-1 w-full resize-none rounded-lg border border-zinc-800 bg-zinc-950 p-3 font-mono text-[12px] leading-relaxed text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-indigo-500"
              />
              <div className="flex flex-wrap items-center gap-2">
                <span
                  className={`text-[11px] font-mono tabular-nums ${
                    overLimit ? "text-rose-400" : "text-zinc-500"
                  }`}
                >
                  {(bytes / 1024).toFixed(1)} / 90 KiB
                </span>
                <div className="ml-auto flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    isLoading={busy === "diff"}
                    disabled={busy !== null || !yaml.trim() || overLimit}
                    onClick={runDiff}
                  >
                    <GitCompare className="h-3.5 w-3.5" />
                    Diff
                  </Button>
                  <Button
                    variant="primary"
                    size="sm"
                    isLoading={busy === "apply"}
                    disabled={busy !== null || !yaml.trim() || overLimit}
                    onClick={() => runApply(yaml)}
                  >
                    <CheckCircle2 className="h-3.5 w-3.5" />
                    Apply
                  </Button>
                </div>
              </div>
            </>
          ) : (
            <div className="grid gap-4 lg:grid-cols-[minmax(0,320px)_minmax(0,1fr)]">
              <div className="space-y-3">
                <Field label="Name">
                  <input
                    className={INPUT}
                    value={form.name}
                    placeholder="my-app"
                    onChange={(e) => setField("name", e.target.value)}
                  />
                </Field>
                <Field label="Image">
                  <input
                    className={INPUT}
                    value={form.image}
                    placeholder="nginx:1.27"
                    onChange={(e) => setField("image", e.target.value)}
                  />
                </Field>
                <p className="text-[11px] leading-relaxed text-amber-300/80">
                  The image must be pullable by the cluster, or loaded into it first (kind:{" "}
                  <span className="font-mono">kind load docker-image &lt;image&gt; --name {cluster}</span>
                  ).
                </p>
                <div className="grid grid-cols-2 gap-3">
                  <Field label="Namespace">
                    <input
                      className={INPUT}
                      value={form.namespace}
                      onChange={(e) => setField("namespace", e.target.value)}
                    />
                  </Field>
                  <Field label="Replicas">
                    <input
                      className={INPUT}
                      type="number"
                      min={0}
                      max={50}
                      value={numText(form.replicas)}
                      onChange={(e) => setField("replicas", parseNum(e.target.value))}
                    />
                  </Field>
                  <Field label="Container port">
                    <input
                      className={INPUT}
                      type="number"
                      min={0}
                      max={65535}
                      value={numText(form.containerPort)}
                      onChange={(e) => setField("containerPort", parseNum(e.target.value))}
                    />
                  </Field>
                  <Field label="Service">
                    <select
                      className={INPUT}
                      value={form.serviceType}
                      onChange={(e) => setField("serviceType", e.target.value as QuickServiceType)}
                    >
                      <option value="none">none</option>
                      <option value="ClusterIP">ClusterIP</option>
                      <option value="NodePort">NodePort</option>
                    </select>
                  </Field>
                </div>
                {form.serviceType === "NodePort" && (
                  <Field label="NodePort (30000–32767, 0 = automatic)">
                    <input
                      className={INPUT}
                      type="number"
                      value={numText(form.nodePort)}
                      onChange={(e) => setField("nodePort", parseNum(e.target.value))}
                    />
                  </Field>
                )}
                <Field label="Env (KEY=value per line)">
                  <textarea
                    className="w-full resize-none rounded-lg border border-zinc-800 bg-zinc-950 p-2 font-mono text-[11px] text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-indigo-500"
                    rows={4}
                    spellCheck={false}
                    value={form.env}
                    placeholder={"LOG_LEVEL=info\nMODE=prod"}
                    onChange={(e) => setField("env", e.target.value)}
                  />
                </Field>
                {formErrors.length > 0 && (
                  <ul className="space-y-0.5 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-300">
                    {formErrors.map((msg) => (
                      <li key={msg}>• {msg}</li>
                    ))}
                  </ul>
                )}
              </div>

              <div className="flex min-h-0 flex-col gap-2">
                <span className="text-[10px] uppercase tracking-wider text-zinc-500">
                  Generated YAML
                </span>
                <pre className="min-h-[200px] flex-1 overflow-auto rounded-lg border border-zinc-800 bg-zinc-950 p-3 font-mono text-[11px] leading-relaxed text-zinc-300 select-text">
                  {preview}
                </pre>
                <div className="flex items-center justify-end gap-2">
                  <Button variant="outline" size="sm" disabled={busy !== null} onClick={openQuickInEditor}>
                    <FileCode2 className="h-3.5 w-3.5" />
                    Open in YAML editor
                  </Button>
                  <Button
                    variant="primary"
                    size="sm"
                    isLoading={busy === "apply"}
                    disabled={busy !== null || formErrors.length > 0}
                    onClick={() => runApply(preview)}
                  >
                    <Rocket className="h-3.5 w-3.5" />
                    Deploy
                  </Button>
                </div>
              </div>
            </div>
          )}

          {error && (
            <div
              role="alert"
              className="whitespace-pre-wrap break-words rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 font-mono text-xs text-rose-300"
            >
              {error}
            </div>
          )}

          {diff && (
            <div className="rounded-lg border border-zinc-800 bg-zinc-950/60">
              <div className="flex flex-wrap items-center gap-3 border-b border-zinc-800 px-3 py-2 text-[11px]">
                <span className="font-semibold text-zinc-300">Diff vs cluster</span>
                {stats && (
                  <span className="font-mono">
                    <span className="text-emerald-400">+{stats.added}</span>{" "}
                    <span className="text-rose-400">-{stats.removed}</span>
                  </span>
                )}
                {diffStale && (
                  <span className="rounded border border-amber-500/30 bg-amber-500/10 px-1.5 py-0.5 text-amber-300">
                    stale: the YAML changed since this diff; run Diff again
                  </span>
                )}
              </div>
              {!diff.changed || diff.text.trim() === "" ? (
                <p className="px-3 py-3 text-xs text-zinc-500">
                  No changes: the cluster already matches this YAML.
                </p>
              ) : (
                <pre className="max-h-72 overflow-auto py-2 font-mono text-[11px] leading-relaxed select-text">
                  {diffLines.map((l, i) => (
                    <div key={i} className={`whitespace-pre px-3 ${DIFF_LINE[l.kind]}`}>
                      {l.text || " "}
                    </div>
                  ))}
                </pre>
              )}
            </div>
          )}

          {output && (
            <div className="rounded-lg border border-emerald-500/30 bg-emerald-500/5">
              <div className="border-b border-emerald-500/20 px-3 py-2 text-[11px] font-semibold text-emerald-300">
                kubectl output
              </div>
              <pre className="max-h-40 overflow-auto whitespace-pre-wrap px-3 py-2 font-mono text-[11px] text-zinc-300 select-text">
                {output}
              </pre>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
