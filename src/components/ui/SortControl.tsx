import { ArrowDownWideNarrow, ArrowUpNarrowWide } from "lucide-react";
import type { SortState } from "../../lib/listSort";

/** Sort key select plus an ascending/descending toggle, for list toolbars. */
export function SortControl<K extends string>({
  value,
  options,
  onChange,
  label = "Sort",
}: {
  value: SortState<K>;
  options: readonly { key: K; label: string }[];
  onChange: (next: SortState<K>) => void;
  label?: string;
}) {
  const asc = value.dir === "asc";
  return (
    <div className="flex items-center h-9 shrink-0 border border-zinc-800 bg-zinc-900/90">
      <select
        value={value.key}
        onChange={(e) => onChange({ ...value, key: e.target.value as K })}
        aria-label={label}
        title={label}
        className="h-full bg-transparent pl-2 pr-1 text-xs text-zinc-300 focus:outline-none"
      >
        {options.map((o) => (
          <option key={o.key} value={o.key} className="bg-zinc-900">
            Sort: {o.label}
          </option>
        ))}
      </select>
      <button
        type="button"
        onClick={() => onChange({ ...value, dir: asc ? "desc" : "asc" })}
        title={asc ? "Ascending (click for descending)" : "Descending (click for ascending)"}
        aria-label={asc ? "Sort ascending" : "Sort descending"}
        className="h-full px-2 border-l border-zinc-800 text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800"
      >
        {asc ? <ArrowUpNarrowWide className="w-3.5 h-3.5" /> : <ArrowDownWideNarrow className="w-3.5 h-3.5" />}
      </button>
    </div>
  );
}
