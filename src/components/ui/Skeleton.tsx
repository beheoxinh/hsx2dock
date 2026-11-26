import React from "react";
import { cn } from "../../lib/utils";

/**
 * Base placeholder block. Every skeleton in the app is built from this so the
 * pulse stays in step across a page, and `prefers-reduced-motion` users get a
 * static block instead of a shimmer (see `index.css`).
 */
export const Skeleton: React.FC<React.HTMLAttributes<HTMLDivElement> & { pulse?: boolean }> = ({
  className,
  pulse = true,
  ...props
}) => (
  <div
    aria-hidden="true"
    className={cn("skeleton rounded-md bg-zinc-800/70", pulse && "animate-pulse", className)}
    {...props}
  />
);

/** A rounded, borderless placeholder shaped like a titled block of text. */
export const SkeletonText: React.FC<{
  lines?: number;
  className?: string;
  lineClassName?: string;
}> = ({ lines = 2, className, lineClassName }) => (
  <div className={cn("space-y-2", className)} aria-hidden="true">
    {Array.from({ length: lines }, (_, i) => (
      <Skeleton
        key={i}
        className={cn(i === lines - 1 ? "h-3 w-2/3" : "h-3 w-full", lineClassName)}
      />
    ))}
  </div>
);

/**
 * A card-shaped placeholder for row lists — images, volumes, networks, routes.
 * `dense` matches the tighter list cards used inside panels.
 */
export const ListRowSkeleton: React.FC<{
  dense?: boolean;
  /** Number of trailing action buttons to reserve. */
  actions?: number;
  className?: string;
}> = ({ dense = false, actions = 3, className }) => (
  <div
    aria-hidden="true"
    className={cn(
      "bg-zinc-900/60 backdrop-blur-xl border border-zinc-800/80 rounded-2xl shadow-xl",
      dense ? "p-3" : "p-5",
      className
    )}
  >
    <div className="flex items-center gap-4">
      <Skeleton className={cn("rounded-xl shrink-0", dense ? "h-9 w-9" : "h-11 w-11")} />
      <div className="min-w-0 flex-1 space-y-2">
        <Skeleton className="h-3.5 w-1/3 min-w-[120px]" />
        <Skeleton className="h-3 w-1/2 min-w-[160px]" />
      </div>
      <div className="flex items-center gap-1.5 shrink-0">
        {Array.from({ length: actions }, (_, i) => (
          <Skeleton key={i} className="h-8 w-8 rounded-lg" />
        ))}
      </div>
    </div>
    {!dense && (
      <div className="mt-4 flex items-center gap-2">
        <Skeleton className="h-5 w-20 rounded-md" />
        <Skeleton className="h-5 w-20 rounded-md" />
        <Skeleton className="h-5 w-24 rounded-md" />
      </div>
    )}
  </div>
);

/** `rows` stacked list placeholders, wrapped in the page's usual spacing. */
export const SkeletonList: React.FC<{
  rows?: number;
  dense?: boolean;
  actions?: number;
  className?: string;
}> = ({ rows = 4, dense, actions, className }) => (
  <div className={cn("space-y-3", className)} aria-hidden="true">
    {Array.from({ length: rows }, (_, i) => (
      <ListRowSkeleton key={i} dense={dense} actions={actions} />
    ))}
  </div>
);

/** Metric tile used by the dashboard's summary row. */
export const StatTileSkeleton: React.FC<{ className?: string }> = ({ className }) => (
  <div
    aria-hidden="true"
    className={cn(
      "bg-zinc-900/60 backdrop-blur-xl border border-zinc-800/80 rounded-2xl p-5 space-y-3",
      className
    )}
  >
    <Skeleton className="h-3 w-24" />
    <Skeleton className="h-7 w-32" />
    <Skeleton className="h-2 w-full rounded-none" />
  </div>
);

/**
 * Chart-shaped placeholder for the dashboard's live resource panels.
 *
 * Mirrors the AreaChart layout (fixed y-axis gutter, dashed grid, plot area)
 * so the card keeps its exact height when real samples replace it — the
 * chart appears in place instead of the panel jumping to a different size.
 */
export const ChartSkeleton: React.FC<{
  /** Caption centred over the plot, e.g. "Collecting samples…". */
  hint?: string;
  className?: string;
  /** false = a still placeholder. A pulse repaints the webview every frame
   *  (~10% CPU on WebKitGTK), so it must only run while data is on its way. */
  pulse?: boolean;
}> = ({ hint, className, pulse = true }) => {
  // Several charts can render at once; a shared gradient id would collide.
  const fillId = React.useId();
  return (
    <div aria-hidden="true" className={cn("flex h-full w-full gap-2", className)}>
      <div className="flex w-11 shrink-0 flex-col justify-between py-[2px]">
        {Array.from({ length: 3 }, (_, i) => (
          <Skeleton key={i} pulse={pulse} className={cn("h-2", i === 2 ? "w-3" : "w-8")} />
        ))}
      </div>
      <div className="relative min-w-0 flex-1 overflow-hidden">
        <div className="absolute inset-0 flex flex-col justify-between py-[2px]">
          {Array.from({ length: 5 }, (_, i) => (
            <div
              key={i}
              className={cn(
                "h-px w-full",
                i === 4
                  ? "bg-zinc-800"
                  : i === 0
                    ? "bg-zinc-800/50"
                    : "border-t border-dashed border-zinc-800/50"
              )}
            />
          ))}
        </div>
        {/* A soft wave that pulses where the live line will be drawn. */}
        <svg
          viewBox="0 0 100 40"
          preserveAspectRatio="none"
          className={cn("absolute inset-x-0 bottom-0 h-3/4 w-full text-zinc-700/40", pulse && "animate-pulse")}
        >
          <defs>
            <linearGradient id={fillId} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="currentColor" stopOpacity="0.55" />
              <stop offset="100%" stopColor="currentColor" stopOpacity="0.05" />
            </linearGradient>
          </defs>
          <path
            d="M0,30 C8,28 14,20 22,23 C30,26 34,12 42,15 C50,18 56,26 64,22 C72,18 78,8 86,12 C92,15 96,20 100,17 L100,40 L0,40 Z"
            fill={`url(#${fillId})`}
          />
          <path
            d="M0,30 C8,28 14,20 22,23 C30,26 34,12 42,15 C50,18 56,26 64,22 C72,18 78,8 86,12 C92,15 96,20 100,17"
            fill="none"
            stroke="currentColor"
            strokeWidth="1"
            strokeOpacity="0.8"
          />
        </svg>
        {hint && (
          <span className="absolute inset-0 flex items-center justify-center text-[11px] text-zinc-500">
            {hint}
          </span>
        )}
      </div>
    </div>
  );
};

/** Donut-shaped placeholder matching `DonutChart`'s ring + legend layout. */
export const DonutSkeleton: React.FC<{ className?: string }> = ({ className }) => (
  <div
    aria-hidden="true"
    className={cn("flex h-full min-h-0 items-center gap-3", className)}
  >
    <div className="relative aspect-square h-full max-h-[150px] shrink-0">
      <div className="absolute inset-0 animate-pulse rounded-full border-[12px] border-zinc-800/80" />
      <div className="absolute inset-0 flex flex-col items-center justify-center gap-1">
        <Skeleton className="h-3 w-12" />
        <Skeleton className="h-2 w-8" />
      </div>
    </div>
    <div className="min-w-0 flex-1 space-y-2">
      {Array.from({ length: 4 }, (_, i) => (
        <div key={i} className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <Skeleton className="h-1.5 w-1.5 rounded-full" />
            <Skeleton className="h-2.5 w-14" />
          </div>
          <Skeleton className="h-2.5 w-20" />
        </div>
      ))}
    </div>
  </div>
);

/**
 * Label/value rows for the dashboard's spec panels and compact tool lists.
 * `compact` matches the dense binary-tool rows; the default matches the
 * two-column spec table.
 */
export const KeyValueSkeleton: React.FC<{
  rows?: number;
  compact?: boolean;
  className?: string;
}> = ({ rows = 4, compact = false, className }) => (
  <div aria-hidden="true" className={cn(compact ? "space-y-1" : "space-y-2", className)}>
    {Array.from({ length: rows }, (_, i) => (
      <div
        key={i}
        className={cn(
          "flex items-center",
          compact ? "gap-2 rounded-lg px-2.5 py-1.5" : "justify-between gap-3"
        )}
      >
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {compact && <Skeleton className="h-4 w-4 shrink-0 rounded" />}
          <div className="min-w-0 flex-1 space-y-1">
            <Skeleton
              className={cn("h-2.5", i % 2 === 0 ? "w-2/5" : "w-1/3")}
            />
            {compact && <Skeleton className="h-2 w-3/5" />}
          </div>
        </div>
        <Skeleton className="h-2.5 w-14 shrink-0" />
      </div>
    ))}
  </div>
);
