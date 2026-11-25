import React from "react";

interface Item {
  key: string;
  label: string;
  sublabel?: string;
  value: number;
  display?: string;
  color: string;
}

interface BarListProps {
  items: Item[];
  max?: number;
  emptyHint?: string;
}

export const BarList: React.FC<BarListProps> = ({
  items,
  max,
  emptyHint = "No data",
}) => {
  const data = items
    .filter((i) => i.value > 0 || i.display)
    .sort((a, b) => b.value - a.value)
    .slice(0, 6);

  const maxVal = max ?? Math.max(0, ...data.map((d) => d.value));
  return (
    <div className="space-y-3">
      {data.map((d) => {
        const pct = maxVal === 0 ? 0 : (d.value / maxVal) * 100;
        return (
          <div key={d.key} className="flex items-center gap-3">
            <div className="min-w-0 flex-1">
              <div className="mb-1 flex items-center justify-between text-xs">
                <div className="min-w-0">
                  <div className="truncate text-zinc-200">{d.label}</div>
                  {d.sublabel && (
                    <div className="mt-0.5 truncate text-[10px] text-zinc-500">
                      {d.sublabel}
                    </div>
                  )}
                </div>
                <span className="ml-3 shrink-0 tabular-nums text-zinc-400">
                  {d.display ?? `${d.value.toFixed(1)}%`}
                </span>
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-none bg-zinc-800">
                <div
                  className="h-1.5 rounded-none"
                  style={{ width: `${Math.max(5, pct)}%`, backgroundColor: d.color }}
                />
              </div>
            </div>
          </div>
        );
      })}
      {data.length === 0 && (
        <div className="text-xs text-zinc-600">{emptyHint}</div>
      )}
    </div>
  );
};
