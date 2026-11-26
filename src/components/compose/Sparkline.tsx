import React from "react";

/**
 * Tiny inline SVG line of recent samples. Scales to its own maximum (floor 0)
 * so a flat idle line still reads as flat. Needs two samples to draw a line.
 */
export const Sparkline: React.FC<{
  values: number[];
  label: string;
  width?: number;
  height?: number;
  className?: string;
}> = ({ values, label, width = 56, height = 16, className }) => {
  if (values.length < 2) {
    return <span className={className} aria-hidden="true" />;
  }
  const max = Math.max(...values, 0);
  const step = width / (values.length - 1);
  // Half a pixel of padding keeps the stroke of the top sample inside the box.
  const points = values
    .map((v, i) => {
      const x = i * step;
      const y = max > 0 ? height - 0.5 - (v / max) * (height - 1) : height - 0.5;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  const last = values[values.length - 1];
  return (
    <svg
      role="img"
      aria-label={`${label}: last ${last.toFixed(1)}, peak ${max.toFixed(1)} over ${values.length} samples`}
      viewBox={`0 0 ${width} ${height}`}
      width={width}
      height={height}
      className={className}
    >
      <polyline points={points} fill="none" stroke="currentColor" strokeWidth={1} strokeLinejoin="round" />
    </svg>
  );
};
