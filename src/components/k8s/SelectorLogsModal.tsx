import React, { useEffect, useMemo, useRef, useState } from "react";
import { Copy, FileText, Loader2, RefreshCw, X } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import { mergeLogChunk, nextSince } from "../../lib/logMerge";
import type { LogBuffer } from "../../lib/logMerge";
import { parseSelectorLogLine, podColorIndex, sinceTimeFromSeconds } from "../../lib/k8sManifest";

const LINE_CAP = 5000;
const FOLLOW_MS = 3000;
const TAIL_OPTIONS = [100, 200, 500, 1000] as const;

/** Full class names so Tailwind keeps them. Each pod gets one by name. */
const PALETTE = [
  { text: "text-sky-300", chip: "border-sky-500/40 bg-sky-500/10 text-sky-300", dot: "bg-sky-400" },
  { text: "text-emerald-300", chip: "border-emerald-500/40 bg-emerald-500/10 text-emerald-300", dot: "bg-emerald-400" },
  { text: "text-amber-300", chip: "border-amber-500/40 bg-amber-500/10 text-amber-300", dot: "bg-amber-400" },
  { text: "text-fuchsia-300", chip: "border-fuchsia-500/40 bg-fuchsia-500/10 text-fuchsia-300", dot: "bg-fuchsia-400" },
  { text: "text-rose-300", chip: "border-rose-500/40 bg-rose-500/10 text-rose-300", dot: "bg-rose-400" },
  { text: "text-cyan-300", chip: "border-cyan-500/40 bg-cyan-500/10 text-cyan-300", dot: "bg-cyan-400" },
  { text: "text-lime-300", chip: "border-lime-500/40 bg-lime-500/10 text-lime-300", dot: "bg-lime-400" },
  { text: "text-violet-300", chip: "border-violet-500/40 bg-violet-500/10 text-violet-300", dot: "bg-violet-400" },
] as const;

interface Query {
  selector?: string;
  workloadKind?: string;
  workloadName?: string;
}

const pill = (on: boolean) =>
  `h-7 rounded-lg border px-2.5 text-[11px] transition-colors ${
    on
      ? "border-indigo-500/50 bg-indigo-500/15 text-indigo-200"
      : "border-zinc-800 bg-zinc-950 text-zinc-500 hover:text-zinc-300"
  }`;

/**
 * Live logs of every pod behind a label selector, or behind a workload's
 * selector. Follow mode polls with `sinceTime` and merges the new lines; a
 * generation counter drops any response that belongs to an older query.
 */
export function SelectorLogsModal({
  cluster,
  namespace,
  selector,
  workloadKind,
  workloadName,
  title,
  onClose,
}: {
  cluster: string;
  namespace: string;
  selector?: string;
  workloadKind?: string;
  workloadName?: string;
  title?: string;
  onClose: () => void;
}) {
  const toast = useToast();
  // A plain selector can be edited; a workload resolves its own selector.
  const editable = !workloadKind;
  const [query, setQuery] = useState<Query>(() =>
    workloadKind ? { workloadKind, workloadName } : { selector }
  );
  const [selectorDraft, setSelectorDraft] = useState(selector ?? "");
  const [tail, setTail] = useState<number>(200);
  const [follow, setFollow] = useState(true);
  const [textFilter, setTextFilter] = useState("");
  const [hiddenPods, setHiddenPods] = useState<Set<string>>(() => new Set());
  const [showTs, setShowTs] = useState(true);
  const [wrap, setWrap] = useState(false);
  const [buffer, setBuffer] = useState<LogBuffer | undefined>(undefined);
  const [resolved, setResolved] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const genRef = useRef(0);
  const bufferRef = useRef<LogBuffer | undefined>(undefined);
  const inflightRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  // True while the user is at (or near) the bottom; follow mode only
  // auto-scrolls in that case.
  const stickRef = useRef(true);

  const commitBuffer = (b: LogBuffer | undefined) => {
    bufferRef.current = b;
    setBuffer(b);
  };

  /**
   * One fetch. Fresh (`incremental` false) replaces the buffer with a tail;
   * incremental asks only for lines since the newest second shown.
   */
  const load = (incremental: boolean) => {
    if (incremental && inflightRef.current) return;
    const gen = genRef.current;
    const prev = incremental ? bufferRef.current : undefined;
    const since = nextSince(prev);
    inflightRef.current = true;
    setLoading(true);
    const opts =
      since !== undefined
        ? { ...query, sinceTime: sinceTimeFromSeconds(since) }
        : { ...query, tail };
    api
      .getSelectorLogs(cluster, namespace, opts)
      .then((r) => {
        if (gen !== genRef.current) return;
        setResolved(r.selector);
        const next = mergeLogChunk(prev, r.lines, LINE_CAP);
        if (next !== prev) commitBuffer(next);
        setError(null);
      })
      .catch((e) => {
        if (gen === genRef.current) setError(String(e));
      })
      .finally(() => {
        if (gen === genRef.current) {
          inflightRef.current = false;
          setLoading(false);
        }
      });
  };

  // New query, tail size, or cluster/namespace: start over with a fresh tail.
  useEffect(() => {
    genRef.current++;
    inflightRef.current = false;
    stickRef.current = true;
    commitBuffer(undefined);
    setResolved("");
    load(false);
    return () => {
      genRef.current++;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cluster, namespace, query, tail]);

  // Follow: incremental refresh every few seconds while the tab is visible.
  useEffect(() => {
    if (!follow) return;
    const t = window.setInterval(() => {
      if (!document.hidden) load(true);
    }, FOLLOW_MS);
    return () => window.clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [follow, cluster, namespace, query, tail]);

  // Escape closes the modal.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const rows = useMemo(
    () =>
      (buffer?.lines ?? []).map((raw, i) => ({ i, raw, ...parseSelectorLogLine(raw) })),
    [buffer]
  );

  const pods = useMemo(() => {
    const seen = new Set<string>();
    for (const r of rows) if (r.pod) seen.add(r.pod);
    return Array.from(seen);
  }, [rows]);

  const needle = textFilter.trim().toLowerCase();
  const visible = useMemo(
    () =>
      rows.filter((r) => {
        if (r.pod && hiddenPods.has(r.pod)) return false;
        return !needle || r.raw.toLowerCase().includes(needle);
      }),
    [rows, hiddenPods, needle]
  );

  // Follow mode keeps the newest line in view unless the user scrolled up.
  useEffect(() => {
    const el = scrollRef.current;
    if (follow && stickRef.current && el) el.scrollTop = el.scrollHeight;
  }, [visible, follow]);

  const onScroll = (e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
  };

  const toggleFollow = () => {
    if (!follow) stickRef.current = true;
    setFollow((f) => !f);
  };

  const applySelector = () => {
    const next = selectorDraft.trim();
    if (!next) {
      setError("Enter a label selector, e.g. app=web");
      return;
    }
    setError(null);
    setQuery({ selector: next });
  };

  const togglePod = (pod: string) =>
    setHiddenPods((prev) => {
      const next = new Set(prev);
      if (next.has(pod)) next.delete(pod);
      else next.add(pod);
      return next;
    });

  const copyAll = async () => {
    const lines = bufferRef.current?.lines ?? [];
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      toast.success("Logs copied", `${lines.length} lines`);
    } catch (e) {
      toast.error("Copy failed", String(e));
    }
  };

  const heading = title ?? (workloadKind ? `${workloadKind}/${workloadName}` : `selector ${query.selector ?? ""}`);
  const total = buffer?.lines.length ?? 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={heading}
        className="flex h-[88vh] w-[min(1400px,94vw)] flex-col rounded-2xl border border-zinc-800 bg-zinc-900 shadow-2xl"
      >
        <div className="flex items-center gap-2 border-b border-zinc-800 px-4 py-3">
          <FileText className="h-4 w-4 shrink-0 text-zinc-400" />
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-white">{heading}</p>
            <p className="truncate text-[11px] text-zinc-500">
              {cluster} · namespace {namespace}
              {resolved && (
                <>
                  {" · selector "}
                  <span className="font-mono text-zinc-400">{resolved}</span>
                </>
              )}
            </p>
          </div>
          <div className="ml-auto flex items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              title="Refresh now"
              aria-label="Refresh now"
              onClick={() => load(true)}
              className="h-7 w-7 text-zinc-400 hover:text-white"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${loading ? "animate-spin" : ""}`} />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              title="Copy all lines"
              aria-label="Copy all lines"
              onClick={copyAll}
              disabled={total === 0}
              className="h-7 w-7 text-zinc-400 hover:text-white"
            >
              <Copy className="h-3.5 w-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              title="Close"
              aria-label="Close logs"
              onClick={onClose}
              className="h-7 w-7 text-zinc-400 hover:text-white"
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
        </div>

        <div className="space-y-2 border-b border-zinc-800/80 px-4 py-2.5">
          <div className="flex flex-wrap items-center gap-2">
            {editable && (
              <form
                className="flex items-center gap-1.5"
                onSubmit={(e) => {
                  e.preventDefault();
                  applySelector();
                }}
              >
                <input
                  value={selectorDraft}
                  onChange={(e) => setSelectorDraft(e.target.value)}
                  placeholder="label selector, e.g. app=web"
                  aria-label="Label selector"
                  spellCheck={false}
                  className="h-7 w-64 rounded-lg border border-zinc-800 bg-zinc-950 px-2 font-mono text-[11px] text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-indigo-500"
                />
                <Button type="submit" variant="outline" size="sm" className="h-7 px-2.5 text-[11px]">
                  Load
                </Button>
              </form>
            )}
            <label className="flex items-center gap-1.5 text-[11px] text-zinc-500">
              Tail
              <select
                value={tail}
                onChange={(e) => setTail(Number(e.target.value))}
                aria-label="Tail lines"
                className="h-7 rounded-lg border border-zinc-800 bg-zinc-950 px-2 text-[11px] text-zinc-200 focus:outline-none focus:border-indigo-500"
              >
                {TAIL_OPTIONS.map((n) => (
                  <option key={n} value={n}>
                    {n}
                  </option>
                ))}
              </select>
            </label>
            <button
              type="button"
              aria-pressed={follow}
              onClick={toggleFollow}
              className={pill(follow)}
            >
              {follow ? "Following" : "Paused"}
            </button>
            <button
              type="button"
              aria-pressed={showTs}
              onClick={() => setShowTs((v) => !v)}
              className={pill(showTs)}
            >
              Timestamps
            </button>
            <button
              type="button"
              aria-pressed={wrap}
              onClick={() => setWrap((v) => !v)}
              className={pill(wrap)}
            >
              Wrap
            </button>
            <input
              value={textFilter}
              onChange={(e) => setTextFilter(e.target.value)}
              placeholder="Filter text…"
              aria-label="Filter text"
              className="ml-auto h-7 w-56 rounded-lg border border-zinc-800 bg-zinc-950 px-2 text-[11px] text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-indigo-500"
            />
            <span className="text-[11px] tabular-nums text-zinc-500">
              {visible.length} / {total} lines
            </span>
            {loading && <Loader2 className="h-3.5 w-3.5 animate-spin text-zinc-500" />}
          </div>

          {pods.length > 0 && (
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="text-[10px] uppercase tracking-wider text-zinc-500">Pods</span>
              {pods.map((pod) => {
                const color = PALETTE[podColorIndex(pod, PALETTE.length)];
                const on = !hiddenPods.has(pod);
                return (
                  <button
                    key={pod}
                    type="button"
                    aria-pressed={on}
                    onClick={() => togglePod(pod)}
                    title={on ? `Hide ${pod}` : `Show ${pod}`}
                    className={`inline-flex max-w-72 items-center gap-1.5 rounded-full border px-2 py-0.5 font-mono text-[10px] transition-opacity ${
                      on ? color.chip : "border-zinc-800 text-zinc-600 opacity-50 line-through"
                    }`}
                  >
                    <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${color.dot}`} />
                    <span className="truncate">{pod}</span>
                  </button>
                );
              })}
              {hiddenPods.size > 0 && (
                <button
                  type="button"
                  onClick={() => setHiddenPods(new Set())}
                  className="text-[11px] text-indigo-300 hover:text-indigo-200"
                >
                  show all
                </button>
              )}
            </div>
          )}
        </div>

        {error && (
          <div
            role="alert"
            className="mx-4 mt-3 whitespace-pre-wrap break-words rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 font-mono text-xs text-rose-300"
          >
            {error}
          </div>
        )}

        <div
          ref={scrollRef}
          onScroll={onScroll}
          className="min-h-0 flex-1 overflow-auto bg-zinc-950/60 px-4 py-3 font-mono text-[11px] leading-relaxed select-text"
        >
          {buffer === undefined && loading ? (
            <p className="text-zinc-500">Loading logs…</p>
          ) : total === 0 ? (
            <p className="text-zinc-500">(no log lines)</p>
          ) : visible.length === 0 ? (
            <p className="text-zinc-500">No lines match the current filter.</p>
          ) : (
            visible.map((r) => {
              const color = r.pod ? PALETTE[podColorIndex(r.pod, PALETTE.length)] : null;
              return (
                <div
                  key={r.i}
                  className={`flex gap-2 ${wrap ? "whitespace-pre-wrap break-all" : "whitespace-pre"}`}
                >
                  {r.pod && (
                    <span
                      className={`shrink-0 ${color?.text ?? ""}`}
                      title={r.container ? `${r.pod} / ${r.container}` : r.pod}
                    >
                      {r.pod}
                    </span>
                  )}
                  {showTs && r.ts && <span className="shrink-0 text-zinc-600">{r.ts}</span>}
                  <span className="min-w-0 text-zinc-300">{r.message}</span>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}
