import React from "react";

/** A single-series inline sparkline (change over time) drawn as a thin line. */
export const ClusterSparkline: React.FC<{
  data: number[];
  color: string;
  max?: number;
  className?: string;
  /** Tailwind height class; the default suits a full-size chart slot. */
  heightClass?: string;
}> = ({ data, color, max, className, heightClass = "h-8" }) => {
  if (data.length < 2) {
    return <div className={`${heightClass} ${className ?? ""}`} aria-hidden="true" />;
  }
  const hi = max ?? Math.max(...data, 1);
  const lo = Math.min(...data, 0);
  const span = hi - lo || 1;
  const w = 100;
  const h = 32;
  const step = w / (data.length - 1);
  const pts = data.map((v, i) => {
    const x = i * step;
    const y = h - ((v - lo) / span) * h;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });
  return (
    <svg
      viewBox={`0 0 ${w} ${h}`}
      preserveAspectRatio="none"
      className={`${heightClass} w-full ${className ?? ""}`}
      aria-hidden="true"
    >
      <polyline
        points={`0,${h} ${pts.join(" ")} ${w},${h}`}
        fill={color}
        fillOpacity="0.12"
        stroke="none"
      />
      <polyline
        points={pts.join(" ")}
        fill="none"
        stroke={color}
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
};
