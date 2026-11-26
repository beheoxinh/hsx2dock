import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AlertTriangle, CheckCircle2, FileDiff, Loader2, Play, RefreshCw, RotateCcw, Save, XCircle } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import { diffCounts, diffLines, withContext } from "../../lib/textDiff";
import { locateComposeError } from "../../lib/yamlLocate";
import { cn } from "../../lib/utils";
import type { ComposeValidation } from "../../types";
import { YamlEditor, type YamlEditorHandle } from "./YamlEditor";

/** How often the open file is checked for changes made outside the editor. */
const POLL_MS = 3000;

/**
 * The compose file in an editor: validate with `compose config` (unsaved
 * text goes through stdin), review the diff, save (refused when the file
 * changed on disk meanwhile or does not validate), optionally apply.
 * While visible, the file is polled: clean editors reload silently, dirty
 * ones get a conflict banner.
 */
export const ConfigTab: React.FC<{
  filePath: string;
  busy: boolean;
  onSaved: () => void;
  onApply: () => void;
  onDirtyChange: (dirty: boolean) => void;
  /** False while another drawer tab is shown: the editor stays mounted but must not poll. */
  active?: boolean;
}> = ({ filePath, busy, onSaved, onApply, onDirtyChange, active = true }) => {
  const toast = useToast();
  const [original, setOriginal] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [validation, setValidation] = useState<{ text: string; result: ComposeValidation } | null>(null);
  const [checking, setChecking] = useState(false);
  const [saving, setSaving] = useState(false);
  const [showDiff, setShowDiff] = useState(false);
  // Disk text that replaced the baseline while the editor holds unsaved edits.
  const [diskConflict, setDiskConflict] = useState<string | null>(null);
  const [reloadedNote, setReloadedNote] = useState(false);
  const editorRef = useRef<YamlEditorHandle>(null);
  // Mirrors of the state for the poll loop, which must see the latest text
  // rather than the values from the render that started it.
  const draftRef = useRef("");
  const originalRef = useRef<string | null>(null);
  const diskRef = useRef<string | null>(null);

  const applyLoaded = useCallback((text: string) => {
    draftRef.current = text;
    originalRef.current = text;
    diskRef.current = text;
    setOriginal(text);
    setDraft(text);
    setValidation(null);
    setDiskConflict(null);
  }, []);

  const load = useCallback(async () => {
    try {
      applyLoaded(await api.composeReadFile(filePath));
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
  }, [filePath, applyLoaded]);

  useEffect(() => {
    void load();
  }, [load]);

  const onEdit = (text: string) => {
    draftRef.current = text;
    setDraft(text);
  };

  // Poll the file while the tab is visible and selected; stop when hidden,
  // deselected or unmounted. (The drawer keeps this editor mounted on other tabs.)
  useEffect(() => {
    let timer: number | null = null;
    let reading = false;
    const tick = async () => {
      if (reading || originalRef.current === null) return;
      reading = true;
      try {
        const disk = await api.composeReadFile(filePath);
        if (disk === diskRef.current) return;
        diskRef.current = disk;
        if (disk === originalRef.current) {
          // Back at the saved text (e.g. restored outside): nothing to resolve.
          setDiskConflict(null);
        } else if (draftRef.current === originalRef.current) {
          applyLoaded(disk);
          setReloadedNote(true);
        } else {
          setDiskConflict(disk);
        }
      } catch {
        // A missing or unreadable file is often a transient state during another
        // program's save; the next tick retries.
      } finally {
        reading = false;
      }
    };
    const stop = () => {
      if (timer !== null) window.clearInterval(timer);
      timer = null;
    };
    const start = () => {
      if (timer === null) timer = window.setInterval(() => void tick(), POLL_MS);
    };
    const onVisibility = () => {
      if (active && document.visibilityState === "visible") {
        void tick();
        start();
      } else {
        stop();
      }
    };
    document.addEventListener("visibilitychange", onVisibility);
    onVisibility();
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stop();
    };
  }, [filePath, applyLoaded, active]);

  useEffect(() => {
    if (!reloadedNote) return;
    const t = window.setTimeout(() => setReloadedNote(false), 3000);
    return () => window.clearTimeout(t);
  }, [reloadedNote]);

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
      // Our own write is not an external change: record it as the disk state.
      diskRef.current = draft;
      originalRef.current = draft;
      setOriginal(draft);
      setDiskConflict(null);
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
  const errorLine =
    result && !result.ok && result.error ? locateComposeError(result.error, validation!.text)?.line ?? null : null;

  const goToLine = (line: number) => {
    setShowDiff(false);
    // The editor is hidden rather than unmounted while the diff shows, so it
    // is only visible again after this frame.
    requestAnimationFrame(() => editorRef.current?.goToLine(line));
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
        <Button
          size="sm"
          variant="ghost"
          onClick={() => onEdit(original)}
          disabled={!dirty}
          title="Discard unsaved changes"
        >
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

      {diskConflict !== null && (
        <div className="flex flex-wrap items-center gap-2 border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
          <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
          <span className="mr-auto">File changed on disk. Your unsaved edits are kept until you choose.</span>
          <Button
            size="sm"
            variant="outline"
            title="Replace your unsaved edits with the text now on disk"
            onClick={() => {
              applyLoaded(diskConflict);
              setReloadedNote(true);
            }}
          >
            Reload (discard mine)
          </Button>
          <Button
            size="sm"
            variant="outline"
            title="Keep your edits. The disk text becomes the save baseline, so the next Save overwrites it"
            onClick={() => {
              originalRef.current = diskConflict;
              setOriginal(diskConflict);
              setDiskConflict(null);
            }}
          >
            Keep mine
          </Button>
        </div>
      )}

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

      {showDiff && dirty && (
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
      )}

      {/* Kept mounted while the diff shows, so undo history and scroll survive. */}
      <div className={cn("relative min-h-0 flex-1 border border-zinc-800 bg-zinc-950", showDiff && dirty && "hidden")}>
        <YamlEditor
          ref={editorRef}
          value={draft}
          onChange={onEdit}
          onSave={() => {
            if (dirty) void save(false);
          }}
          errorLine={errorLine}
          errorMessage={result && !result.ok ? result.error ?? "" : ""}
          ariaLabel="Compose file"
        />
      </div>
      <p className="flex flex-wrap items-center gap-x-1 text-[10px] text-zinc-600">
        <span>{filePath} · Tab indents · Ctrl+F finds · Ctrl+S saves · {dirty ? "unsaved changes" : "saved"}</span>
        {reloadedNote && <span className="text-emerald-400">· Reloaded from disk</span>}
      </p>
    </div>
  );
};
