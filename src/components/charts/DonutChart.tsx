import React, { useEffect, useRef, useState } from "react";

interface Segment {
  label: string;
  value: number;
  color: string;
}

interface DonutChartProps {
  segments: Segment[];
  total: number;
  centerLabel?: string;
}

const STROKE_W = 12;

export const DonutChart: React.FC<DonutChartProps> = ({
  segments,
  total,
  centerLabel,
}) => {
  const ref = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState(120);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => {
      const h = el.clientHeight;
      if (h > 0) {
        const s = Math.min(h, 150);
        setSize(s);
      }
    };
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);

  const r = (size - STROKE_W) / 2;
  const C = 2 * Math.PI * r;
  let acc = 0;

  const filtered = segments.filter((s) => s.value > 0);
  const hasData = filtered.length > 0 && total > 0;

  const formatBytesForDisplay = (bytes: number) => {
    if (!bytes || bytes <= 0) return "0 B";
    const k = 1024;
    const sizes = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    const v = bytes / Math.pow(k, i);
    return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${sizes[i]}`;
  };

  return (
    <div ref={ref} className="flex h-full min-h-0 items-center gap-3">
      <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
        <g transform={`rotate(-90 ${size / 2} ${size / 2})`}>
          <circle
            cx={size / 2}
            cy={size / 2}
            r={r}
            stroke="rgba(39,39,42,0.9)"
            strokeWidth={STROKE_W}
            fill="none"
          />
          {hasData &&
            filtered.map((s, i) => {
              const len = (s.value / total) * C;
              const dash = `${len} ${C - len}`;
              const el = (
                <circle
                  key={i}
                  cx={size / 2}
                  cy={size / 2}
                  r={r}
                  stroke={s.color}
                  strokeWidth={STROKE_W}
                  fill="none"
                  strokeDasharray={dash}
                  strokeDashoffset={-acc}
                  strokeLinecap="round"
                />
              );
              acc += len;
              return el;
            })}
        </g>
        <text
          x={size / 2}
          y={size / 2 - (centerLabel ? 6 : 0)}
          dominantBaseline="middle"
          textAnchor="middle"
          fontSize={centerLabel ? "13" : "15"}
          className="fill-zinc-100 font-semibold tabular-nums"
        >
          {hasData ? formatBytesForDisplay(total) : "0 B"}
        </text>
        {centerLabel && (
          <text
            x={size / 2}
            y={size / 2 + 12}
            dominantBaseline="middle"
            textAnchor="middle"
            fontSize="9"
            className="fill-zinc-500 uppercase tracking-wider"
          >
            {centerLabel}
          </text>
        )}
      </svg>
      <div className="flex-1 space-y-1.5">
        {filtered.map((s) => (
          <div key={s.label} className="flex items-center justify-between text-xs">
            <div className="flex items-center gap-2">
              <span className="h-1.5 w-1.5 rounded-full" style={{ backgroundColor: s.color }} />
              <span className="text-zinc-400">{s.label}</span>
            </div>
            <span className="tabular-nums text-zinc-200">
              {Math.round((s.value / Math.max(1, total)) * 100)}% / {formatBytesForDisplay(s.value)}
            </span>
          </div>
        ))}
        {!hasData && <div className="text-xs text-zinc-600">No data</div>}
      </div>
    </div>
  );
};
