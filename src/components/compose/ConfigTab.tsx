import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CheckCircle2, FileDiff, Loader2, Play, RefreshCw, RotateCcw, Save, XCircle } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import { diffCounts, diffLines, withContext } from "../../lib/textDiff";
import { locateComposeError } from "../../lib/yamlLocate";
import { cn } from "../../lib/utils";
import type { ComposeValidation } from "../../types";

/**
 * The compose file in an editor: validate with `compose config` (unsaved
 * text goes through stdin), review the diff, save (refused when the file
 * changed on disk meanwhile or does not validate), optionally apply.
 */
export const ConfigTab: React.FC<{
  filePath: string;
  busy: boolean;
  onSaved: () => void;
  onApply: () => void;
  onDirtyChange: (dirty: boolean) => void;
}> = ({ filePath, busy, onSaved, onApply, onDirtyChange }) => {
  const toast = useToast();
  const [original, setOriginal] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [validation, setValidation] = useState<{ text: string; result: ComposeValidation } | null>(null);
  const [checking, setChecking] = useState(false);
  const [saving, setSaving] = useState(false);
  const [showDiff, setShowDiff] = useState(false);
  const editorRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    try {
      const text = await api.composeReadFile(filePath);
      setOriginal(text);
      setDraft(text);
      setLoadError(null);
      setValidation(null);
    } catch (e) {
      setLoadError(String(e));
    }
  }, [filePath]);

  useEffect(() => {
    void load();
  }, [load]);

  const dirty = original !== null && draft !== original;
  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);

  const diff = useMemo(() => (dirty && original !== null ? diffLines(original, draft) : []), [dirty, original, draft]);
  const counts = diffCounts(diff);
  const stale = validation !== null && validation.text !== draft;

  const validate = async (): Promise<ComposeValidation | null> => {
    setChecking(true);
    try {
      const result = await api.composeValidate(filePath, draft);
      setValidation({ text: draft, result });
      return result;
    } catch (e) {
      toast.error("Validation failed to run", String(e));
      return null;
    } finally {
      setChecking(false);
    }
  };

  const save = async (apply: boolean) => {
    if (original === null) return;
    const check = validation && !stale ? validation.result : await validate();
    if (!check?.ok) {
      toast.error("Not saved", "Fix the validation error first.");
      return;
    }
    setSaving(true);
    try {
      await api.composeSaveFile(filePath, draft, original);
      setOriginal(draft);
      setShowDiff(false);
      toast.success("Compose file saved", filePath);
      onSaved();
      if (apply) onApply();
    } catch (e) {
      toast.error("Save failed", String(e));
    } finally {
      setSaving(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Tab") {
      // Two spaces instead of moving focus: YAML is indented with spaces.
      e.preventDefault();
      const el = e.currentTarget;
      const { selectionStart: s, selectionEnd: end } = el;
      const next = draft.slice(0, s) + "  " + draft.slice(end);
      setDraft(next);
      requestAnimationFrame(() => {
        el.selectionStart = el.selectionEnd = s + 2;
      });
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      if (dirty) void save(false);
    }
  };

  if (loadError) {
    return (
      <div className="space-y-3 p-4">
        <p className="border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">{loadError}</p>
        <Button size="sm" onClick={() => void load()}>
          <RefreshCw className="h-3.5 w-3.5" /> Retry
        </Button>
      </div>
    );
  }
  if (original === null) {
    return (
      <div className="flex items-center gap-2 p-6 text-sm text-zinc-500">
        <Loader2 className="h-4 w-4 animate-spin" /> Loading compose file…
      </div>
    );
  }

  const result = validation?.result;
  const lineCount = draft.split("\n").length;
  const errorLine =
    result && !result.ok && result.error ? locateComposeError(result.error, validation!.text)?.line ?? null : null;

  const goToLine = (line: number) => {
    setShowDiff(false);
    requestAnimationFrame(() => {
      const el = editorRef.current;
      if (!el) return;
      const lines = el.value.split("\n");
      const n = Math.min(Math.max(line, 1), lines.length);
      const start = lines.slice(0, n - 1).reduce((sum, l) => sum + l.length + 1, 0);
      el.focus();
      el.setSelectionRange(start, start + lines[n - 1].length);
      // 20px per line (leading-5); keep a few lines of context above.
      el.scrollTop = Math.max(0, (n - 4) * 20);
    });
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="outline" onClick={() => void validate()} disabled={checking}>
          {checking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <CheckCircle2 className="h-3.5 w-3.5" />}
          Validate
        </Button>
        <Button
          size="sm"
          variant={showDiff ? "secondary" : "outline"}
          onClick={() => setShowDiff((v) => !v)}
          disabled={!dirty}
          title="Show what saving will change"
        >
          <FileDiff className="h-3.5 w-3.5" />
          Diff
          {dirty && (
            <span className="font-mono text-[10px]">
              <span className="text-emerald-400">+{counts.added}</span> <span className="text-rose-400">−{counts.removed}</span>
            </span>
          )}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => setDraft(original)} disabled={!dirty} title="Discard unsaved changes">
          <RotateCcw className="h-3.5 w-3.5" /> Revert
        </Button>
        <Button size="sm" variant="ghost" onClick={() => void load()} disabled={dirty} title={dirty ? "Revert first" : "Reload from disk"}>
          <RefreshCw className="h-3.5 w-3.5" /> Reload
        </Button>
        <div className="ml-auto flex gap-2">
          <Button size="sm" onClick={() => void save(false)} disabled={!dirty || saving}>
            {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />}
            Save
          </Button>
          <Button size="sm" variant="primary" onClick={() => void save(true)} disabled={!dirty || saving || busy} title="Save, then docker compose up -d">
            <Play className="h-3.5 w-3.5" />
            Save &amp; apply
          </Button>
        </div>
      </div>

      {result && (
        <div
          className={cn(
            "flex items-start gap-2 border px-3 py-2 text-xs",
            result.ok ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-200" : "border-rose-500/40 bg-rose-500/10 text-rose-200",
            stale && "opacity-60"
          )}
        >
          {result.ok ? <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0" /> : <XCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />}
          <span className="min-w-0 whitespace-pre-wrap break-words font-mono">
            {result.ok
              ? `Valid — ${result.model?.services.length ?? 0} services${result.model?.profiles.length ? `, profiles: ${result.model.profiles.join(", ")}` : ""}`
              : result.error}
            {stale && " (edited since)"}
          </span>
          {errorLine !== null && (
            <button
              type="button"
              onClick={() => goToLine(errorLine)}
              className="ml-auto shrink-0 border border-rose-500/40 px-2 py-0.5 text-[11px] hover:bg-rose-500/20"
            >
              Go to line {errorLine}
            </button>
          )}
        </div>
      )}

      {showDiff && dirty ? (
        <div className="min-h-0 flex-1 overflow-auto border border-zinc-800 bg-zinc-950 font-mono text-[11px] leading-5">
          {withContext(diff, 3).map((l, i) =>
            l === null ? (
              <div key={`gap-${i}`} className="bg-zinc-900/80 px-3 text-zinc-600">
                ⋯
              </div>
            ) : (
              <div
                key={i}
                className={cn(
                  "flex whitespace-pre",
                  l.kind === "add" && "bg-emerald-500/10 text-emerald-200",
                  l.kind === "del" && "bg-rose-500/10 text-rose-200",
                  l.kind === "same" && "text-zinc-400"
                )}
              >
                <span className="w-10 shrink-0 select-none pr-2 text-right text-zinc-600">{l.oldNo ?? ""}</span>
                <span className="w-10 shrink-0 select-none pr-2 text-right text-zinc-600">{l.newNo ?? ""}</span>
                <span className="w-4 shrink-0 select-none">{l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}</span>
                <span>{l.text}</span>
              </div>
            )
          )}
        </div>
      ) : (
        <div className="relative flex min-h-0 flex-1 border border-zinc-800 bg-zinc-950">
          <div
            ref={gutterRef}
            aria-hidden="true"
            className="select-none overflow-hidden border-r border-zinc-800 bg-zinc-900/50 px-2 py-2 text-right font-mono text-[11px] leading-5 text-zinc-600"
          >
            {Array.from({ length: lineCount }, (_, i) => (
              <div key={i} className={cn(errorLine === i + 1 && "bg-rose-500/30 text-rose-200")}>
                {i + 1}
              </div>
            ))}
          </div>
          <textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            ref={editorRef}
            onScroll={(e) => {
              if (gutterRef.current) gutterRef.current.scrollTop = e.currentTarget.scrollTop;
            }}
            spellCheck={false}
            aria-label="Compose file"
            className="min-h-0 flex-1 resize-none overflow-auto whitespace-pre bg-transparent px-3 py-2 font-mono text-[11px] leading-5 text-zinc-100 outline-none"
          />
        </div>
      )}
      <p className="text-[10px] text-zinc-600">
        {filePath} · Tab inserts two spaces · Ctrl+S saves · {dirty ? "unsaved changes" : "saved"}
      </p>
    </div>
  );
};
