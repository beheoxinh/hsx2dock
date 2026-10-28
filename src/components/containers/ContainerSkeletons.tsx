import React from "react";
import { Card } from "../ui/Card";
import { Skeleton } from "../ui/Skeleton";

/**
 * Loading placeholder for one container card.
 *
 * Mirrors the real `ContainerCard` layout (identity block, metric chips,
 * network + port row, action buttons) so the list does not jump when the data
 * lands — the skeleton is the same size and shape as what replaces it.
 */
export const ContainerCardSkeleton: React.FC<{ compact?: boolean }> = ({
  compact = false,
}) => (
  <Card
    aria-hidden="true"
    className={`flex flex-col lg:flex-row lg:items-center justify-between ${
      compact ? "gap-3 p-3" : "gap-5 p-4"
    }`}
  >
    <div
      className={`flex items-start min-w-0 flex-1 lg:max-w-[340px] xl:max-w-[420px] ${
        compact ? "gap-2.5" : "gap-3.5"
      }`}
    >
      <Skeleton
        className={`shrink-0 rounded-xl ${compact ? "h-8 w-8" : "h-9 w-9"}`}
      />
      <div className="min-w-0 flex-1 space-y-2">
        <div className="flex items-center gap-2">
          <Skeleton className="h-2.5 w-2.5 rounded-full shrink-0" />
          <Skeleton className="h-3.5 w-32 max-w-[55%]" />
          <Skeleton className="h-3 w-14 shrink-0" />
        </div>
        <Skeleton className="h-3 w-2/3" />
      </div>
    </div>

    <div className="flex flex-col gap-1.5 flex-1 min-w-0 lg:px-2">
      <div className="flex items-center gap-2 flex-wrap">
        <Skeleton className="h-5 w-24 rounded-md" />
        <Skeleton className="h-5 w-24 rounded-md" />
        <Skeleton className="h-5 w-24 rounded-md" />
      </div>
      <div className="flex items-center gap-1.5 flex-wrap">
        <Skeleton className="h-5 w-20 rounded-md" />
        <Skeleton className="h-5 w-24 rounded-md" />
      </div>
    </div>

    <div className="flex items-center gap-1.5 shrink-0 self-end lg:self-center">
      {Array.from({ length: 5 }, (_, i) => (
        <Skeleton key={i} className="h-8 w-8 rounded-lg" />
      ))}
    </div>
  </Card>
);

/** Placeholder for a group / compose folder card, header plus member rows. */
export const GroupCardSkeleton: React.FC<{
  members?: number;
  compactMembers?: boolean;
}> = ({ members = 2, compactMembers = true }) => (
  <Card aria-hidden="true" className="p-4 space-y-3">
    <div className="flex items-center gap-3">
      <Skeleton className="h-4 w-4 rounded shrink-0" />
      <Skeleton className="h-8 w-8 rounded-lg shrink-0" />
      <div className="min-w-0 flex-1 space-y-2">
        <Skeleton className="h-3.5 w-40 max-w-[60%]" />
        <Skeleton className="h-3 w-64 max-w-full" />
      </div>
      <Skeleton className="h-7 w-24 rounded-lg shrink-0" />
      <Skeleton className="h-7 w-7 rounded-lg shrink-0" />
    </div>
    {members > 0 && (
      <div className="space-y-2 pl-1 sm:pl-7">
        {Array.from({ length: members }, (_, i) => (
          <ContainerCardSkeleton key={i} compact={compactMembers} />
        ))}
      </div>
    )}
  </Card>
);

/**
 * Whole Containers screen body while the first snapshot is in flight:
 * compose folders, then a user group, then the isolated bucket.
 */
export const ContainerGroupsSkeleton: React.FC = () => (
  <div className="space-y-3" aria-busy="true" aria-label="Loading containers">
    <GroupCardSkeleton members={2} />
    <GroupCardSkeleton members={1} />
    <Card aria-hidden="true" className="p-4 space-y-3">
      <div className="flex items-center gap-3">
        <Skeleton className="h-4 w-4 rounded shrink-0" />
        <Skeleton className="h-8 w-8 rounded-lg shrink-0" />
        <div className="min-w-0 flex-1 space-y-2">
          <Skeleton className="h-3.5 w-32" />
          <Skeleton className="h-3 w-56 max-w-full" />
        </div>
        <Skeleton className="h-7 w-20 rounded-lg shrink-0" />
      </div>
      <div className="space-y-2 pl-1 sm:pl-7">
        <ContainerCardSkeleton compact />
        <ContainerCardSkeleton compact />
        <ContainerCardSkeleton compact />
      </div>
    </Card>
  </div>
);
