import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Search,
  Plus,
  Layers,
} from "lucide-react";
import { Button } from "../components/ui/Button";
import { EngineGate } from "../components/ui/EngineGate";
import { useToast } from "../components/ui/Toast";
import { useModal } from "../components/ui/ModalContext";
import { ContainerGroupsBar } from "../components/ContainerGroupsBar";
import { RepairContainerNetworkModal } from "../components/containers/RepairContainerNetworkModal";
import { ContainerGroupsSkeleton } from "../components/containers/ContainerSkeletons";
import {
  ContainerCard,
  type ContainerAction,
} from "../components/ContainerCard";
import { useDraggable } from "../components/ContainerDragLayer";
import {
  canonicalContainerIds,
  reconcileStats,
  routesByContainer,
} from "../lib/containerGroups";
import { api, disposeListener, events } from "../lib/api";
import { isContainerBusy, trackContainerOp } from "../lib/containerOps";
import { useStableCallback } from "../hooks/useStableCallback";
import {
  primeContainerStore,
  refreshContainers,
  saveContainerGroups,
  useContainerStore,
} from "../lib/containerStore";
import type { ContainerGroup, ContainerItem, ContainerStats, DaemonState } from "../types";
import { useProgressiveList } from "../hooks/useProgressiveList";

interface ContainersPageProps {
  onOpenDetails: (
    id: string,
    tab?: "logs" | "inspect" | "stats" | "terminal",
    debug?: boolean,
    edit?: boolean
  ) => void;
  onOpenWizard: () => void;
  onEditContainer?: (id: string) => void;
  /** Engine state — the page renders an EngineGate panel unless Running. */
  daemonState?: DaemonState;
}

type DetailsTab = "logs" | "inspect" | "stats" | "terminal";
type GatewayRouteLike = { domain: string; target: string; path?: string | null; container_name?: string | null };

/**
 * A flat-list card that can be dragged into a group. The drag handle has to
 * come from a hook, so the card is wrapped rather than built inline in the
 * list's map callback.
 *
 * Every handler it receives is page-stable and takes the container, so the
 * memo holds across stats ticks; the per-card bindings are made here.
 */
const DraggableContainerCard = React.memo<{
  container: ContainerItem;
  stats?: ContainerStats;
  sizeBytes?: number;
  busy: boolean;
  onAction: (c: ContainerItem, action: ContainerAction) => void;
  onOpenDetails: (id: string, tab?: DetailsTab, debug?: boolean) => void;
  onEditContainer?: (id: string) => void;
  onRepairNetwork?: (c: ContainerItem) => void;
  gatewayRoutes?: GatewayRouteLike[];
}>(function DraggableContainerCard({
  container,
  stats,
  sizeBytes,
  busy,
  onAction,
  onOpenDetails,
  onEditContainer,
  onRepairNetwork,
  gatewayRoutes,
}) {
  const dragHandle = useDraggable({
    id: container.id,
    label: container.name.replace(/^\//, ""),
    composeOwned: !!container.compose_project,
  });
  const action = useCallback((a: ContainerAction) => onAction(container, a), [onAction, container]);
  const details = useCallback(
    (tab?: DetailsTab, debug?: boolean) => onOpenDetails(container.id, tab, debug),
    [onOpenDetails, container.id]
  );
  const edit = useMemo(
    () => (onEditContainer ? () => onEditContainer(container.id) : undefined),
    [onEditContainer, container.id]
  );
  const repair = useMemo(
    () => (onRepairNetwork ? () => onRepairNetwork(container) : undefined),
    [onRepairNetwork, container]
  );
  return (
    <ContainerCard
      container={container}
      stats={stats}
      sizeBytes={sizeBytes}
      busy={busy}
      dragHandle={dragHandle}
      onAction={action}
      onOpenDetails={details}
      onEditContainer={edit}
      onRepairNetwork={repair}
      gatewayRoutes={gatewayRoutes}
    />
  );
});

const NO_ROUTES = new Map<string, never[]>();

export const ContainersPage: React.FC<ContainersPageProps> = ({
  onOpenDetails,
  onOpenWizard,
  onEditContainer,
  daemonState = "Running",
}) => {
  // Containers, sizes, groups and gateway routes come from the app-wide cache
  // that is primed when the app starts, so this screen paints instantly
  // instead of showing an empty shell while Docker answers.
  const store = useContainerStore();
  const containers = store.containers;
  const sizesMap = store.sizes;
  const groups = store.groups;
  const gatewayRoutes = store.gatewayRoutes;
  const loading = !store.ready;
  const [statsMap, setStatsMap] = useState<Record<string, ContainerStats>>({});
  const [search, setSearch] = useState("");
  const toast = useToast();
  const modal = useModal();
  // Groups live in the shared store so the flat list can hide containers
  // already placed in one; the group bar edits and persists the same array.

  const [repairTarget, setRepairTarget] = useState<ContainerItem | null>(null);

  const saveGroups = useStableCallback(async (next: ContainerGroup[]) => {
    await saveContainerGroups(next);
  });

  const loadContainers = useStableCallback(async () => {
    // Refreshes the shared cache; every mounted view of it re-renders.
    await refreshContainers();
  });

  useEffect(() => {
    // Usually a no-op: the app primed this cache on startup. It still covers a
    // cold deep-link into the tab and the case where the engine was stopped
    // when the app opened.
    void primeContainerStore();

    // Each tick carries every running container, so the map is replaced (a
    // removed container drops out) while unchanged entries keep their
    // identity and their cards stay memoised.
    return disposeListener(
      events.onContainerStatsTick((stats) => {
        setStatsMap((prev) => reconcileStats(prev, stats ?? {}));
      })
    );
  }, []);

  const handleAction = useStableCallback(async (c: ContainerItem, action: ContainerAction) => {
    const cName = c.name.replace(/^\//, "");
    if (isContainerBusy(c.id)) return;
    try {
      if (action === "remove") {
        const confirmed = await modal.showConfirm(
          `Are you sure you want to remove container '${cName}'?\n\nThis action cannot be undone.`,
          "Remove Container",
          "danger"
        );
        if (!confirmed) return;
      }
      // The card swaps its buttons for a spinner while this runs and until
      // the live list shows the new state.
      await trackContainerOp(c.id, action, async () => {
        switch (action) {
          case "start":
            // `docker start` refuses a paused container; resume it instead.
            return c.state === "paused"
              ? api.unpauseContainer(c.id)
              : api.startContainer(c.id);
          case "stop":
            return api.stopContainer(c.id);
          case "restart":
            return api.restartContainer(c.id);
          case "pause":
            return api.pauseContainer(c.id);
          case "remove":
            return api.removeContainer(c.id, true);
        }
      });
    } catch (e) {
      toast.error("Operation failed", String(e));
    }
  });

  const openRepair = useStableCallback((c: ContainerItem) => setRepairTarget(c));
  const openDetails = useStableCallback(
    (id: string, tab?: DetailsTab, debug?: boolean) => onOpenDetails(id, tab, debug)
  );
  const editContainer = useStableCallback((id: string) => onEditContainer?.(id));

  // Matching routes per container, computed once per list/route change rather
  // than once per card on every render.
  const routesRef = useRef<Map<string, GatewayRouteLike[]>>(NO_ROUTES);
  const routesById = useMemo(() => {
    const next = routesByContainer(
      containers,
      // A disabled route is not served; its chip would link to nothing.
      (gatewayRoutes as GatewayRouteLike[]).filter((r) => (r as { enabled?: boolean }).enabled !== false),
      routesRef.current
    );
    routesRef.current = next;
    return next;
  }, [containers, gatewayRoutes]);

  // Kind node containers are managed entirely on the Kubernetes tab, so they
  // are hidden from this generic container list.
  const query = search.trim().toLowerCase();
  const searchMatches = useMemo(
    () =>
      containers.filter(
        (c) =>
          !c.kind_cluster &&
          (!query ||
            c.name.toLowerCase().includes(query) ||
            c.image.toLowerCase().includes(query) ||
            c.id.toLowerCase().includes(query))
      ),
    [containers, query]
  );

  // Containers placed in a user group live inside that group's card above, and
  // compose containers live inside their project's card, so the flat list only
  // shows what is still ungrouped ("Isolated") — moving a card into a group
  // removes it from here, like a file leaving a folder.
  // Membership is resolved against the full list so a search cannot make a
  // grouped container look ungrouped.
  const grouped = useMemo(() => {
    const map = canonicalContainerIds(containers.filter((c) => !c.kind_cluster));
    const ids = new Set<string>();
    for (const g of groups) {
      for (const token of g.container_ids) ids.add(map.get(token) ?? token);
    }
    return ids;
  }, [containers, groups]);
  const filteredContainers = useMemo(
    () => searchMatches.filter((c) => !c.compose_project && !grouped.has(c.id)),
    [searchMatches, grouped]
  );
  const allGrouped = searchMatches.length > 0 && filteredContainers.length === 0;
  const progressive = useProgressiveList(filteredContainers, search);

  // Engine off → designed feature panel instead of fetch errors.
  if (daemonState !== "Running") {
    return (
      <EngineGate state={daemonState} resource="containers">
        {null}
      </EngineGate>
    );
  }



  return (
    <div className="p-8 space-y-6 max-w-7xl mx-auto h-full flex flex-col">
      {/* Action Bar */}
      <div className="flex flex-col sm:flex-row sm:flex-wrap sm:items-center justify-between gap-4">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">Containers</h2>
          <p className="text-sm text-zinc-400 mt-1">
            Manage instances, stream live performance telemetry, and launch shells
          </p>
        </div>

        <div className="flex items-center gap-3">
          <div className="relative w-64">
            <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
            <input
              type="text"
              placeholder="Search containers..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="w-full h-9 pl-9 pr-4 rounded-xl bg-zinc-900 border border-zinc-800 text-sm text-white placeholder-zinc-400 focus:outline-none focus:border-blue-500 transition-colors"
            />
          </div>

          <Button variant="primary" size="sm" onClick={onOpenWizard} className="gap-2">
            <Plus className="w-4 h-4" />
            <span>Create Container</span>
          </Button>
        </div>
      </div>

      {/* Groups. The ungrouped containers are rendered as children so they sit
          *inside* the Isolated bucket rather than floating below it. This whole
          block scrolls as one region now that the list lives inside it. */}
      <div className="flex-1 overflow-y-auto pb-8">
        {!store.ready ? (
          <ContainerGroupsSkeleton />
        ) : (
        <ContainerGroupsBar
        containers={containers}
        groups={groups}
        onSaveGroups={saveGroups}
        statsMap={statsMap}
        sizesMap={sizesMap}
        onChanged={loadContainers}
        onOpenDetails={openDetails}
        onEditContainer={onEditContainer ? editContainer : undefined}
        onRepairNetwork={openRepair}
        onContainerAction={handleAction}
        routesById={routesById}
      >
        {/* Ungrouped containers — the body of the Isolated bucket. */}
        <div className="space-y-3">
          {/* Plain rows: a motion wrapper per card cost a layout animation
              for every container on every list change. */}
          {progressive.visible.map((c) => (
            <DraggableContainerCard
              key={c.id}
              container={c}
              stats={statsMap[c.id]}
              sizeBytes={sizesMap[c.id]}
              busy={false}
              onAction={handleAction}
              onOpenDetails={openDetails}
              onEditContainer={onEditContainer ? editContainer : undefined}
              onRepairNetwork={openRepair}
              gatewayRoutes={routesById.get(c.id)}
            />
          ))}
          {progressive.hasMore && (
            <div ref={progressive.sentinelRef} className="h-8" aria-hidden="true" />
          )}

          {!loading && filteredContainers.length === 0 && (
            <div className="py-16 text-center text-zinc-400">
              <Layers className="w-10 h-10 mx-auto text-zinc-500 mb-3" />
              <p className="font-medium text-white">
                {allGrouped ? "All matching containers are in groups" : "No containers matched"}
              </p>
              <p className="text-xs text-zinc-400 mt-1">
                {allGrouped
                  ? "Open a group card above to reach them"
                  : search
                    ? "Try searching with a different keyword"
                    : "Get started by launching a new container image"}
              </p>
            </div>
          )}
        </div>
        </ContainerGroupsBar>
        )}
      </div>

      {/* Repair Container Network Modal */}
      <RepairContainerNetworkModal
        isOpen={!!repairTarget}
        container={repairTarget}
        onClose={() => setRepairTarget(null)}
        onSuccess={loadContainers}
      />
    </div>
  );
};
