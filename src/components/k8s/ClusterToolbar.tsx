import React from "react";
import { Loader2, Play, Search, Square, X } from "lucide-react";
import { cn } from "../../lib/utils";
import {
  HEALTH_LABEL,
  SORT_KEYS,
  STATUS_FILTERS,
  type SortKey,
  type StatusFilter,
} from "../../lib/k8sClusterList";
import { Button } from "../ui/Button";

const STATUS_CHIP_LABEL: Record<StatusFilter, string> = {
  all: "All",
  running: "Running",
  stopped: "Stopped",
  degraded: HEALTH_LABEL.degraded,
};

const SORT_LABEL: Record<SortKey, string> = {
  name: "Name",
  status: "Status",
  created: "Created",
  cpu: "CPU",
  memory: "Memory",
};

export interface BulkProgress {
  action: "start" | "stop";
  done: number;
  total: number;
  failed: number;
}

export interface ClusterToolbarProps {
  query: string;
  onQuery: (q: string) => void;
  status: StatusFilter;
  onStatus: (s: StatusFilter) => void;
  counts: Record<StatusFilter, number>;
  tag: string | null;
  onTag: (t: string | null) => void;
  tags: string[];
  sort: SortKey;
  onSort: (s: SortKey) => void;
  /** Visible cards / total clusters, shown when a filter hides some. */
  visibleCount: number;
  totalCount: number;
  /** Clusters that Start all / Stop all would act on. */
  startableCount: number;
  stoppableCount: number;
  onBulkStart: () => void;
  onBulkStop: () => void;
  bulkProgress: BulkProgress | null;
  disabled: boolean;
}

const chipClass = (active: boolean) =>
  cn(
    "inline-flex items-center gap-1.5 h-6 px-2.5 rounded-none border text-[11px] font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-blue-500",
    active
      ? "bg-blue-500/15 border-blue-500/40 text-blue-200"
      : "bg-zinc-900 border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-700"
  );

/**
 * One compact row above the list: search, status and tag chips, then
 * Start all / Stop all and sort.
 */
export const ClusterToolbar: React.FC<ClusterToolbarProps> = (p) => {
  const running = p.bulkProgress !== null;

  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
      <div className="relative w-full sm:w-56">
        <Search className="w-3.5 h-3.5 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400 pointer-events-none" />
        <input
          type="text"
          aria-label="Search clusters by name or tag"
          placeholder="Search by name or tag…"
          value={p.query}
          onChange={(e) => p.onQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape" && p.query) p.onQuery("");
          }}
          className="w-full h-8 pl-8 pr-8 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-white placeholder-zinc-400 focus:outline-none focus:border-blue-500"
        />
        {p.query && (
          <button
            type="button"
            aria-label="Clear search"
            title="Clear search"
            onClick={() => p.onQuery("")}
            className="absolute right-2 top-1/2 -translate-y-1/2 p-1 rounded text-zinc-500 hover:text-white outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Filter clusters by status">
        {STATUS_FILTERS.map((s) => (
          <button
            key={s}
            type="button"
            aria-pressed={p.status === s}
            onClick={() => p.onStatus(s)}
            className={chipClass(p.status === s)}
          >
            <span>{STATUS_CHIP_LABEL[s]}</span>
            <span className="font-mono text-[10px] text-zinc-500">{p.counts[s]}</span>
          </button>
        ))}
      </div>

      {p.tags.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="mx-1 h-4 w-px bg-zinc-800" aria-hidden="true" />
          <span className="text-[11px] text-zinc-500">Tags</span>
          {p.tags.map((t) => (
            <button
              key={t}
              type="button"
              aria-pressed={p.tag === t}
              title={`Show clusters tagged ${t}`}
              onClick={() => p.onTag(p.tag === t ? null : t)}
              className={chipClass(p.tag === t)}
            >
              {t}
            </button>
          ))}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2 text-xs text-zinc-400 sm:ml-auto">
        <Button
          variant="outline"
          size="sm"
          onClick={p.onBulkStart}
          disabled={p.disabled || running || p.startableCount === 0}
          title={
            p.startableCount === 0
              ? "Every cluster is already running"
              : `Start ${p.startableCount} stopped cluster${p.startableCount === 1 ? "" : "s"}`
          }
          className="gap-1.5 h-7"
        >
          <Play className="w-3.5 h-3.5" />
          <span>Start all</span>
        </Button>
        <Button
          variant="outline"
          size="sm"
          onClick={p.onBulkStop}
          disabled={p.disabled || running || p.stoppableCount === 0}
          title={
            p.stoppableCount === 0
              ? "No cluster is running"
              : `Stop ${p.stoppableCount} running cluster${p.stoppableCount === 1 ? "" : "s"} (data is kept)`
          }
          className="gap-1.5 h-7"
        >
          <Square className="w-3.5 h-3.5" />
          <span>Stop all</span>
        </Button>

        {p.bulkProgress && (
          <span
            role="status"
            aria-live="polite"
            className="inline-flex items-center gap-1.5 font-mono text-[11px] text-zinc-300"
          >
            <Loader2 className="w-3.5 h-3.5 animate-spin" aria-hidden="true" />
            {p.bulkProgress.action === "start" ? "Starting" : "Stopping"} {p.bulkProgress.done}/
            {p.bulkProgress.total}
            {p.bulkProgress.failed > 0 && (
              <span className="text-rose-400"> · {p.bulkProgress.failed} failed</span>
            )}
          </span>
        )}

        <div className="flex items-center gap-1.5">
          <label htmlFor="cluster-sort" className="text-[11px] text-zinc-500">
            Sort
          </label>
          <select
            id="cluster-sort"
            value={p.sort}
            onChange={(e) => p.onSort(e.target.value as SortKey)}
            className="h-7 px-2 rounded-lg bg-zinc-900 border border-zinc-800 text-xs text-zinc-200 focus:outline-none focus:border-blue-500"
          >
            {SORT_KEYS.map((k) => (
              <option key={k} value={k}>
                {SORT_LABEL[k]}
              </option>
            ))}
          </select>
        </div>
      </div>
    </div>
  );
};
