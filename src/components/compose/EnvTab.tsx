import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Eye, EyeOff, FileText, Loader2, Play, Plus, RefreshCw, RotateCcw, Save, Table2, Trash2 } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import { duplicateKeys, isSecretKey, isValidEnvKey, parseEnv, serializeEnv, type EnvRow } from "../../lib/dotenv";
import { cn } from "../../lib/utils";
import { inputClass } from "./composeUi";

/** A row with an id that survives edits, deletes and inserts. */
type KeyedRow = EnvRow & { uid: number };
let nextUid = 1;
const keyed = (rows: EnvRow[]): KeyedRow[] => rows.map((r) => ({ ...r, uid: nextUid++ }));

/**
 * The project's env file as an editable key/value table (comments and other
 * lines are kept as they are), or as raw text. Secret-looking values are
 * hidden until revealed.
 */
export const EnvTab: React.FC<{
  filePath: string;
  busy: boolean;
  onSaved: () => void;
  onApply: () => void;
  onDirtyChange: (dirty: boolean) => void;
}> = ({ filePath, busy, onSaved, onApply, onDirtyChange }) => {
  const toast = useToast();
  const [envName, setEnvName] = useState(".env");
  const [nameDraft, setNameDraft] = useState(".env");
  const [path, setPath] = useState("");
  const [original, setOriginal] = useState<string | null>(null);
  const [exists, setExists] = useState(false);
  const [rows, setRows] = useState<KeyedRow[]>([]);
  const [raw, setRaw] = useState("");
  const [mode, setMode] = useState<"table" | "raw">("table");
  const [revealed, setRevealed] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const env = await api.composeReadEnv(filePath, envName);
      setPath(env.path);
      setExists(env.exists);
      setOriginal(env.content);
      setRows(keyed(parseEnv(env.content)));
      setRaw(env.content);
      setRevealed(new Set());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [filePath, envName]);

  useEffect(() => {
    void load();
  }, [load]);

  const current = mode === "table" ? serializeEnv(rows) : raw;
  const dirty = original !== null && current !== original && !(original === "" && current === "");
  useEffect(() => onDirtyChange(dirty), [dirty, onDirtyChange]);

  const dups = useMemo(() => duplicateKeys(rows), [rows]);
  const invalid = rows.some((r) => r.kind === "var" && !isValidEnvKey(r.key));

  const switchMode = (next: "table" | "raw") => {
    if (next === mode) return;
    if (next === "raw") setRaw(serializeEnv(rows));
    else setRows(keyed(parseEnv(raw)));
    setMode(next);
  };

  const update = (uid: number, patch: Partial<{ key: string; value: string }>) =>
    setRows((prev) => prev.map((r) => (r.uid === uid && r.kind === "var" ? { ...r, ...patch } : r)));

  const save = async (apply: boolean) => {
    if (original === null) return;
    if (mode === "table" && invalid) {
      toast.error("Not saved", "Variable names may contain letters, digits, _ . - and must not start with a digit.");
      return;
    }
    setSaving(true);
    try {
      await api.composeSaveEnv(filePath, envName, current, exists ? original : null);
      setOriginal(current);
      setExists(true);
      toast.success("Env file saved", path);
      onSaved();
      if (apply) onApply();
    } catch (e) {
      toast.error("Save failed", String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex items-center gap-1">
          <input
            value={nameDraft}
            onChange={(e) => setNameDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !dirty) setEnvName(nameDraft.trim() || ".env");
            }}
            onBlur={() => {
              if (!dirty) setEnvName(nameDraft.trim() || ".env");
              else setNameDraft(envName);
            }}
            title={dirty ? "Save or revert before opening another env file" : "Env file, relative to the project folder"}
            disabled={dirty}
            className={cn(inputClass, "w-44 font-mono")}
            aria-label="Env file name"
          />
        </div>
        <div className="flex border border-zinc-700">
          <button
            type="button"
            onClick={() => switchMode("table")}
            className={cn("flex items-center gap-1 px-2 py-1 text-xs", mode === "table" ? "bg-zinc-800 text-white" : "text-zinc-400")}
          >
            <Table2 className="h-3.5 w-3.5" /> Table
          </button>
          <button
            type="button"
            onClick={() => switchMode("raw")}
            className={cn("flex items-center gap-1 border-l border-zinc-700 px-2 py-1 text-xs", mode === "raw" ? "bg-zinc-800 text-white" : "text-zinc-400")}
          >
            <FileText className="h-3.5 w-3.5" /> Raw
          </button>
        </div>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            setRows(keyed(parseEnv(original ?? "")));
            setRevealed(new Set());
            setRaw(original ?? "");
          }}
          disabled={!dirty}
        >
          <RotateCcw className="h-3.5 w-3.5" /> Revert
        </Button>
        <Button size="sm" variant="ghost" onClick={() => void load()} disabled={dirty}>
          <RefreshCw className="h-3.5 w-3.5" /> Reload
        </Button>
        <div className="ml-auto flex gap-2">
          <Button size="sm" onClick={() => void save(false)} disabled={!dirty || saving}>
            {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />}
            Save
          </Button>
          <Button
            size="sm"
            variant="primary"
            onClick={() => void save(true)}
            disabled={!dirty || saving || busy}
            title="Save, then docker compose up -d so containers pick up the new values"
          >
            <Play className="h-3.5 w-3.5" /> Save &amp; apply
          </Button>
        </div>
      </div>

      {error ? (
        <p className="border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">{error}</p>
      ) : loading ? (
        <div className="flex items-center gap-2 p-6 text-sm text-zinc-500">
          <Loader2 className="h-4 w-4 animate-spin" /> Loading…
        </div>
      ) : mode === "raw" ? (
        <textarea
          value={raw}
          onChange={(e) => setRaw(e.target.value)}
          spellCheck={false}
          aria-label="Env file"
          placeholder="KEY=value"
          className="min-h-0 flex-1 resize-none border border-zinc-800 bg-zinc-950 p-3 font-mono text-[11px] leading-5 text-zinc-100 outline-none"
        />
      ) : (
        <div className="min-h-0 flex-1 overflow-auto border border-zinc-800">
          {!exists && rows.length === 0 && (
            <p className="px-3 py-4 text-xs text-zinc-500">
              {envName} does not exist yet. Add a variable and save to create it.
            </p>
          )}
          {rows.map((r) =>
            r.kind === "raw" ? (
              <div key={r.uid} className="truncate border-b border-zinc-800/60 px-3 py-1 font-mono text-[11px] text-zinc-600" title={r.text}>
                {r.text || " "}
              </div>
            ) : (
              <div key={r.uid} className="flex items-center gap-2 border-b border-zinc-800/60 px-2 py-1">
                <input
                  value={r.key}
                  onChange={(e) => update(r.uid, { key: e.target.value })}
                  aria-label="Variable name"
                  className={cn(
                    inputClass,
                    "h-7 w-56 shrink-0 font-mono",
                    (!isValidEnvKey(r.key) || dups.has(r.key)) && "border-amber-500/60"
                  )}
                  title={dups.has(r.key) ? "Defined more than once; the last one wins" : undefined}
                />
                <span className="text-zinc-600">=</span>
                <input
                  value={r.value}
                  type={isSecretKey(r.key) && !revealed.has(r.uid) ? "password" : "text"}
                  onChange={(e) => update(r.uid, { value: e.target.value })}
                  aria-label={`Value of ${r.key}`}
                  className={cn(inputClass, "h-7 min-w-0 flex-1 font-mono")}
                />
                {isSecretKey(r.key) && (
                  <button
                    type="button"
                    onClick={() =>
                      setRevealed((prev) => {
                        const next = new Set(prev);
                        if (next.has(r.uid)) next.delete(r.uid);
                        else next.add(r.uid);
                        return next;
                      })
                    }
                    title={revealed.has(r.uid) ? "Hide value" : "Show value"}
                    aria-label={revealed.has(r.uid) ? "Hide value" : "Show value"}
                    className="p-1 text-zinc-500 hover:text-zinc-200"
                  >
                    {revealed.has(r.uid) ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                  </button>
                )}
                <button
                  type="button"
                  onClick={() => setRows((prev) => prev.filter((x) => x.uid !== r.uid))}
                  title={`Remove ${r.key}`}
                  aria-label={`Remove ${r.key}`}
                  className="p-1 text-zinc-500 hover:text-rose-300"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
              </div>
            )
          )}
          <div className="p-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() => setRows((prev) => [...prev, { kind: "var", key: "", value: "", exported: false, uid: nextUid++ }])}
            >
              <Plus className="h-3.5 w-3.5" /> Add variable
            </Button>
          </div>
        </div>
      )}
      <p className="text-[10px] text-zinc-600">
        {path || envName} · values are used for <code>${"{VAR}"}</code> interpolation in the compose file; recreate
        containers (Save &amp; apply) for changes to take effect.
      </p>
    </div>
  );
};
