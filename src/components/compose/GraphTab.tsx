import React, { useMemo } from "react";
import { dependencyLayers, type ComposeServiceRow } from "../../lib/composeProject";
import { cn } from "../../lib/utils";

const NODE_W = 170;
const NODE_H = 46;
const COL_GAP = 70;
const ROW_GAP = 18;
const PAD = 16;

function tone(row: ComposeServiceRow): { stroke: string; fill: string; text: string; label: string } {
  if (row.containers.length === 0)
    return { stroke: "#3f3f46", fill: "#18181b", text: "#71717a", label: row.optional ? "not created (profile)" : "not created" };
  if (row.running === row.containers.length)
    return { stroke: "#10b981", fill: "#022c22", text: "#d1fae5", label: `running ${row.running}` };
  if (row.running > 0)
    return { stroke: "#f59e0b", fill: "#2a1a03", text: "#fde68a", label: `${row.running}/${row.containers.length} running` };
  return { stroke: "#52525b", fill: "#18181b", text: "#a1a1aa", label: "stopped" };
}

/**
 * depends_on as a left-to-right graph: dependencies on the left, the
 * services that need them to their right. Node colour is the service state.
 */
export const GraphTab: React.FC<{ rows: ComposeServiceRow[]; onSelect?: (row: ComposeServiceRow) => void }> = ({
  rows,
  onSelect,
}) => {
  const layout = useMemo(() => {
    const specs = rows.map((r) => r.spec).filter((s): s is NonNullable<typeof s> => s !== null);
    const layers = dependencyLayers(specs);
    const columns: ComposeServiceRow[][] = [];
    for (const row of rows) {
      const layer = layers.get(row.name) ?? 0;
      (columns[layer] ??= []).push(row);
    }
    const pos = new Map<string, { x: number; y: number }>();
    columns.forEach((col, ci) =>
      col.forEach((row, ri) =>
        pos.set(row.name, { x: PAD + ci * (NODE_W + COL_GAP), y: PAD + ri * (NODE_H + ROW_GAP) })
      )
    );
    const tallest = Math.max(1, ...columns.map((c) => c?.length ?? 0));
    const width = PAD * 2 + Math.max(1, columns.length) * NODE_W + Math.max(0, columns.length - 1) * COL_GAP;
    const height = PAD * 2 + tallest * NODE_H + (tallest - 1) * ROW_GAP;
    const edges: { from: string; to: string; key: string }[] = [];
    for (const row of rows) {
      for (const dep of row.spec?.depends_on ?? []) {
        if (pos.has(dep)) edges.push({ from: row.name, to: dep, key: `${row.name}->${dep}` });
      }
    }
    return { pos, width, height, edges };
  }, [rows]);

  if (rows.length === 0) return <p className="p-6 text-center text-sm text-zinc-500">No services.</p>;
  const hasEdges = layout.edges.length > 0;

  return (
    <div className="space-y-2">
      <p className="text-[11px] text-zinc-500">
        {hasEdges
          ? "Arrows point from a service to what it depends on (depends_on). Compose starts the left column first."
          : "No depends_on between these services; they start in parallel."}
      </p>
      <div className="overflow-auto border border-zinc-800 bg-zinc-950/60">
        <svg
          width={layout.width}
          height={layout.height}
          viewBox={`0 0 ${layout.width} ${layout.height}`}
          role="img"
          aria-label="Service dependency graph"
        >
          <defs>
            <marker id="compose-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
              <path d="M 0 0 L 10 5 L 0 10 z" fill="#71717a" />
            </marker>
          </defs>
          {layout.edges.map((e) => {
            const a = layout.pos.get(e.from)!;
            const b = layout.pos.get(e.to)!;
            const x1 = a.x;
            const y1 = a.y + NODE_H / 2;
            const x2 = b.x + NODE_W;
            const y2 = b.y + NODE_H / 2;
            const mid = (x1 + x2) / 2;
            return (
              <path
                key={e.key}
                d={`M ${x1} ${y1} C ${mid} ${y1}, ${mid} ${y2}, ${x2} ${y2}`}
                fill="none"
                stroke="#52525b"
                strokeWidth={1.5}
                markerEnd="url(#compose-arrow)"
              />
            );
          })}
          {rows.map((row) => {
            const p = layout.pos.get(row.name);
            if (!p) return null;
            const t = tone(row);
            return (
              <g
                key={row.name}
                transform={`translate(${p.x} ${p.y})`}
                className={cn(onSelect && "cursor-pointer")}
                onClick={() => onSelect?.(row)}
              >
                <title>{`${row.name} — ${t.label}${row.spec?.image ? `\n${row.spec.image}` : ""}`}</title>
                <rect width={NODE_W} height={NODE_H} fill={t.fill} stroke={t.stroke} strokeWidth={1.5} />
                <text x={10} y={19} fill={t.text} fontSize={12} fontWeight={600} fontFamily="ui-sans-serif, system-ui">
                  {row.name.length > 20 ? `${row.name.slice(0, 19)}…` : row.name}
                </text>
                <text x={10} y={35} fill="#71717a" fontSize={10} fontFamily="ui-monospace, monospace">
                  {t.label}
                </text>
              </g>
            );
          })}
        </svg>
      </div>
    </div>
  );
};
