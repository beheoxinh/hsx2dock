import React, { useEffect, useMemo, useRef, useState } from "react";

export interface ChartSeries {
  name: string;
  color: string;
  data: number[];
}

interface AreaChartProps {
  series: ChartSeries[];
  formatValue?: (v: number) => string;
  /** Fixed upper bound (e.g. 100 for percentages). Omitted = auto from data. */
  yMax?: number;
  emptyHint?: string;
  /** Shown bottom-right of the plot, e.g. "last 3 min". */
  timeHint?: string;
}

/** Width reserved for the y-axis labels, and the gap next to them. */
const AXIS_W = 44;
const GAP = 8;
const PAD = 4;

/**
 * Fills whatever box its parent gives it — both axes are measured, so the
 * chart shrinks with the window instead of forcing the page to scroll.
 */
export const AreaChart: React.FC<AreaChartProps> = ({
  series,
  formatValue,
  yMax,
  emptyHint = "No samples yet",
  timeHint,
}) => {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ w: 480, h: 150 });
  const [hoverIdx, setHoverIdx] = useState<number | null>(null);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const measure = () => {
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (w > 0 && h > 0) setBox({ w, h });
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);

  const plotW = Math.max(40, box.w - AXIS_W - GAP);
  const plotH = Math.max(40, box.h);
  const defsId = React.useId();

  const fmt = formatValue ?? ((v: number) => v.toFixed(1));
  const pointCount = Math.max(0, ...series.map((s) => s.data.length));
  const hasData = pointCount >= 2;

  const max = useMemo(() => {
    if (yMax !== undefined) return yMax;
    const m = Math.max(0, ...series.flatMap((s) => s.data));
    return m > 0 ? m * 1.15 : 1;
  }, [series, yMax]);

  const innerW = Math.max(1, plotW - PAD * 2);
  const innerH = Math.max(1, plotH - PAD * 2);
  const xFor = (i: number) =>
    PAD + (pointCount <= 1 ? innerW : (i / (pointCount - 1)) * innerW);
  const yFor = (v: number) =>
    PAD + innerH - (max <= 0 ? 0 : Math.min(v, max) / max) * innerH;

  const onMove = (e: React.MouseEvent<SVGSVGElement>) => {
    if (!hasData) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const t = (e.clientX - rect.left - PAD) / Math.max(1, innerW);
    setHoverIdx(Math.min(pointCount - 1, Math.max(0, Math.round(t * (pointCount - 1)))));
  };

  const hoverValues =
    hoverIdx === null
      ? []
      : series
          .filter((s) => s.data.length)
          .map((s) => ({
            name: s.name,
            color: s.color,
            value: s.data[Math.min(hoverIdx, s.data.length - 1)] ?? 0,
          }));

  return (
    <div ref={wrapRef} className="flex h-full w-full gap-2">
      <div
        className="flex shrink-0 flex-col justify-between py-[2px] text-right text-[10px] leading-none text-zinc-500"
        style={{ width: AXIS_W }}
      >
        <span>{fmt(max)}</span>
        <span>{fmt(max / 2)}</span>
        <span>0</span>
      </div>

      <div className="relative min-w-0 flex-1">
        <svg
          width={plotW}
          height={plotH}
          viewBox={`0 0 ${plotW} ${plotH}`}
          onMouseMove={onMove}
          onMouseLeave={() => setHoverIdx(null)}
          className="block cursor-crosshair"
        >
          <defs>
            {series.map((s, si) => (
              <linearGradient key={si} id={`${defsId}-${si}`} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={s.color} stopOpacity="0.40" />
                <stop offset="100%" stopColor={s.color} stopOpacity="0.02" />
              </linearGradient>
            ))}
          </defs>

          {[0, 0.25, 0.5, 0.75, 1].map((g) => {
            const y = PAD + innerH * (1 - g);
            return (
              <line
                key={g}
                x1={PAD}
                x2={plotW - PAD}
                y1={y}
                y2={y}
                stroke="rgba(63,63,70,0.55)"
                strokeWidth={1}
                strokeDasharray={g === 0 ? undefined : "3 4"}
              />
            );
          })}

          {hasData &&
            series.map((s, si) => {
              const pts = s.data.map((v, i) => `${xFor(i)},${yFor(v)}`);
              const line = pts.map((p, i) => `${i === 0 ? "M" : "L"}${p}`).join(" ");
              const area = `${line} L${xFor(s.data.length - 1)},${PAD + innerH} L${xFor(
                0
              )},${PAD + innerH} Z`;
              return (
                <g key={s.name}>
                  <path d={area} fill={`url(#${defsId}-${si})`} />
                  <path
                    d={line}
                    fill="none"
                    stroke={s.color}
                    strokeWidth={1.75}
                    strokeLinejoin="round"
                    strokeLinecap="round"
                  />
                </g>
              );
            })}

          {hoverIdx !== null && hasData && (
            <>
              <line
                x1={xFor(hoverIdx)}
                x2={xFor(hoverIdx)}
                y1={PAD}
                y2={PAD + innerH}
                stroke="rgba(161,161,170,0.6)"
                strokeWidth={1}
                strokeDasharray="3 3"
              />
              {series.map((s) =>
                s.data.length ? (
                  <circle
                    key={s.name}
                    cx={xFor(hoverIdx)}
                    cy={yFor(s.data[Math.min(hoverIdx, s.data.length - 1)] ?? 0)}
                    r={3}
                    fill={s.color}
                    stroke="#09090b"
                    strokeWidth={1.5}
                  />
                ) : null
              )}
            </>
          )}
        </svg>

        {!hasData && (
          <div className="absolute inset-0 flex items-center justify-center text-center text-[11px] text-zinc-600">
            {emptyHint}
          </div>
        )}

        {hoverIdx !== null && hasData && (
          <div className="pointer-events-none absolute right-1 top-1 z-10 rounded-lg border border-zinc-700/70 bg-zinc-950/95 px-2.5 py-1.5 text-xs shadow-xl">
            <div className="mb-1 text-[10px] uppercase tracking-wider text-zinc-500">
              {pointCount - 1 - hoverIdx === 0
                ? "now"
                : `-${pointCount - 1 - hoverIdx} samples`}
            </div>
            {hoverValues.map((v) => (
              <div key={v.name} className="flex items-center gap-2">
                <span
                  className="h-1.5 w-1.5 rounded-full"
                  style={{ backgroundColor: v.color }}
                />
                <span className="text-zinc-400">{v.name}</span>
                <span className="ml-auto font-medium tabular-nums text-zinc-100">
                  {fmt(v.value)}
                </span>
              </div>
            ))}
          </div>
        )}

        {timeHint && hoverIdx === null && hasData && (
          <div className="pointer-events-none absolute bottom-0 right-1 text-[10px] text-zinc-600">
            {timeHint}
          </div>
        )}
      </div>
    </div>
  );
};
