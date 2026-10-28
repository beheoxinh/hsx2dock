import React from "react";
import { cn } from "../../lib/utils";

/**
 * Base placeholder block. Every skeleton in the app is built from this so the
 * pulse stays in step across a page, and `prefers-reduced-motion` users get a
 * static block instead of a shimmer (see `index.css`).
 */
export const Skeleton: React.FC<React.HTMLAttributes<HTMLDivElement>> = ({
  className,
  ...props
}) => (
  <div
    aria-hidden="true"
    className={cn("skeleton animate-pulse rounded-md bg-zinc-800/70", className)}
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
    <Skeleton className="h-2 w-full rounded-full" />
  </div>
);
