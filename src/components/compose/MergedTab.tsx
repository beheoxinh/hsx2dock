import React, { useCallback, useEffect, useMemo, useState } from "react";
import { AlertCircle, Copy, Loader2, RefreshCw } from "lucide-react";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import { IconAction } from "./composeUi";

/**
 * Read-only `compose config` output: what the project really runs, with the
 * override file and the `.env` values already applied.
 */
export const MergedTab: React.FC<{ filePath: string }> = ({ filePath }) => {
  const toast = useToast();
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setText(await api.composeMergedConfig(filePath));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [filePath]);

  useEffect(() => {
    void load();
  }, [load]);

  const lines = useMemo(() => (text ?? "").split("\n"), [text]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text ?? "");
      toast.success("Merged config copied");
    } catch (e) {
      toast.error("Copy failed", String(e));
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="flex items-center gap-2">
        <p className="min-w-0 flex-1 text-[11px] text-zinc-500">
          What compose actually runs: override file and .env applied.
        </p>
        <IconAction title="Copy merged config" onClick={() => void copy()} disabled={!text}>
          <Copy className="h-4 w-4" />
        </IconAction>
        <IconAction title="Reload merged config" onClick={() => void load()} disabled={loading}>
          {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
        </IconAction>
      </div>

      {error && (
        <p className="flex items-center gap-1.5 whitespace-pre-wrap border border-rose-500/40 bg-rose-500/10 px-3 py-1.5 font-mono text-[11px] text-rose-200">
          <AlertCircle className="h-3 w-3 shrink-0" aria-hidden="true" />
          {error}
        </p>
      )}

      {text !== null && (
        <div className="min-h-0 flex-1 overflow-auto border border-zinc-800 bg-zinc-950 py-2 font-mono text-xs">
          {lines.map((line, i) => (
            <div key={i} className="flex">
              <span className="w-12 shrink-0 select-none pr-3 text-right text-zinc-600" aria-hidden="true">
                {i + 1}
              </span>
              <span className="whitespace-pre pr-4 text-zinc-200">{line || " "}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
};
