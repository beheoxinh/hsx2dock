import React, { useEffect, useMemo, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
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
import {
  ContainerCard,
  type ContainerAction,
} from "../components/ContainerCard";
import { useDraggable } from "../components/ContainerDragLayer";
import { canonicalContainerIds, groupOf } from "../lib/containerGroups";
import { api, events } from "../lib/api";
import type { ContainerGroup, ContainerItem, ContainerStats, DaemonState } from "../types";

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

/**
 * A flat-list card that can be dragged into a group. The drag handle has to
 * come from a hook, so the card is wrapped rather than built inline in the
 * list's map callback.
 */
const DraggableContainerCard: React.FC<{
  container: ContainerItem;
  stats?: ContainerStats;
  sizeBytes?: number;
  busy: boolean;
  onAction: (action: ContainerAction) => void;
  onOpenDetails: (tab?: "logs" | "inspect" | "stats" | "terminal", debug?: boolean) => void;
  onEditContainer?: () => void;
}> = ({ container, stats, sizeBytes, busy, onAction, onOpenDetails, onEditContainer }) => {
  const dragHandle = useDraggable({
    id: container.id,
    label: container.name.replace(/^\//, ""),
    composeOwned: !!container.compose_project,
  });
  return (
    <ContainerCard
      container={container}
      stats={stats}
      sizeBytes={sizeBytes}
      busy={busy}
      dragHandle={dragHandle}
      onAction={onAction}
      onOpenDetails={onOpenDetails}
      onEditContainer={onEditContainer}
    />
  );
};

export const ContainersPage: React.FC<ContainersPageProps> = ({
  onOpenDetails,
  onOpenWizard,
  onEditContainer,
  daemonState = "Running",
}) => {
  const [containers, setContainers] = useState<ContainerItem[]>([]);
  const [statsMap, setStatsMap] = useState<Record<string, ContainerStats>>({});
  const [sizesMap, setSizesMap] = useState<Record<string, number>>({});
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(true);
  const [actionLoading, setActionLoading] = useState<Record<string, boolean>>({});
  const toast = useToast();
  const modal = useModal();
  // Groups live here so the flat list can hide containers already placed in
  // one; the group bar edits and persists the same array.
  const [groups, setGroups] = useState<ContainerGroup[]>([]);

  const saveGroups = async (next: ContainerGroup[]) => {
    await api.saveContainerGroups(next);
    setGroups(next);
  };

  const loadContainers = async () => {
    try {
      setLoading(true);
      const [data, sizes] = await Promise.allSettled([
        api.listContainers(true),
        api.getContainerSizes(),
      ]);
      if (data.status === "fulfilled") setContainers(data.value);
      if (sizes.status === "fulfilled") setSizesMap(sizes.value ?? {});
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadContainers();
    api
      .listContainerGroups()
      .then(setGroups)
      .catch((e) => console.error(e));

    let unlistenStats: (() => void) | undefined;
    let unlistenContainers: (() => void) | undefined;

    events.onContainerStatsTick((stats) => {
      setStatsMap((prev) => ({ ...prev, ...stats }));
    }).then((u) => (unlistenStats = u));

    events.onContainersTick((updatedList) => {
      setContainers(updatedList);
      setLoading(false);
    }).then((u) => (unlistenContainers = u));

    return () => {
      if (unlistenStats) unlistenStats();
      if (unlistenContainers) unlistenContainers();
    };
  }, []);

  const handleAction = async (
    c: ContainerItem,
    action: "start" | "stop" | "restart" | "pause" | "unpause" | "clone" | "remove"
  ) => {
    const cName = c.name.replace(/^\//, "");
    try {
      setActionLoading((prev) => ({ ...prev, [c.id]: true }));
      if (action === "start") {
        await api.startContainer(c.id);
        toast.success("Container started", `${cName} is now running.`);
      } else if (action === "stop") {
        await api.stopContainer(c.id);
        toast.info("Container stopped", `${cName} has been stopped.`);
      } else if (action === "restart") {
        await api.restartContainer(c.id);
        toast.success("Container restarted", `${cName} rebooted successfully.`);
      } else if (action === "pause") {
        await api.pauseContainer(c.id);
        toast.info("Container paused", `${cName} has been paused.`);
      } else if (action === "unpause") {
        await api.unpauseContainer(c.id);
        toast.success("Container unpaused", `${cName} resumed.`);
      } else if (action === "clone") {
        const details = await api.inspectContainer(c.id);
        const newName = `${cName}-clone-${Math.floor(Math.random() * 1000)}`;
        const envVars = (details.env || []).map((e) => {
          const idx = e.indexOf("=");
          return idx !== -1
            ? { key: e.slice(0, idx), value: e.slice(idx + 1) }
            : { key: e, value: "" };
        });
        await api.runContainer({
          image: details.image,
          name: newName,
          env: envVars,
          command: details.command ? details.command.split(" ") : undefined,
          restart_policy: "unless-stopped",
        });
        toast.success("Container cloned", `Created duplicate container: ${newName}`);
      } else if (action === "remove") {
        const confirmed = await modal.showConfirm(`Are you sure you want to remove container '${cName}'?\n\nThis action cannot be undone.`, "Remove Container", "danger");
        if (!confirmed) return;
        await api.removeContainer(c.id, true);
        toast.success("Container removed", `${cName} was deleted.`);
      }
      await loadContainers();
    } catch (e) {
      toast.error("Operation failed", String(e));
    } finally {
      setActionLoading((prev) => ({ ...prev, [c.id]: false }));
    }
  };

  // Kind node containers are managed entirely on the Kubernetes tab, so they
  // are hidden from this generic container list.
  const searchMatches = containers.filter(
    (c) =>
      !c.kind_cluster &&
      (c.name.toLowerCase().includes(search.toLowerCase()) ||
        c.image.toLowerCase().includes(search.toLowerCase()) ||
        c.id.toLowerCase().includes(search.toLowerCase()))
  );

  // Containers placed in a user group live inside that group's card above, and
  // compose containers live inside their project's card, so the flat list only
  // shows what is still ungrouped ("Isolated") — moving a card into a group
  // removes it from here, like a file leaving a folder.
  // Membership is resolved against the full list so a search cannot make a
  // grouped container look ungrouped.
  const groupsMap = useMemo(
    () => canonicalContainerIds(containers.filter((c) => !c.kind_cluster)),
    [containers]
  );
  const filteredContainers = searchMatches.filter(
    (c) => !c.compose_project && !groupOf(groups, groupsMap, c.id)
  );
  const allGrouped = searchMatches.length > 0 && filteredContainers.length === 0;

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
        <ContainerGroupsBar
        containers={containers}
        groups={groups}
        onSaveGroups={saveGroups}
        statsMap={statsMap}
        onChanged={loadContainers}
        onOpenDetails={onOpenDetails}
        onEditContainer={onEditContainer}
        onContainerAction={handleAction}
        busyMap={actionLoading}
      >
        {/* Ungrouped containers — the body of the Isolated bucket. */}
        <div className="space-y-3">
          <AnimatePresence>
            {filteredContainers.map((c) => {
              const stats = statsMap[c.id];
              const isBusy = !!actionLoading[c.id];

              return (
                <motion.div
                  key={c.id}
                  layout
                  initial={{ opacity: 0, y: 10 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, scale: 0.98 }}
                  transition={{ duration: 0.2 }}
                >
                  <DraggableContainerCard
                    container={c}
                    stats={stats}
                    sizeBytes={sizesMap[c.id]}
                    busy={isBusy}
                    onAction={(action) => handleAction(c, action)}
                    onOpenDetails={(tab, debug) => onOpenDetails(c.id, tab, debug)}
                    onEditContainer={onEditContainer ? () => onEditContainer(c.id) : undefined}
                  />
                </motion.div>
              );
            })}
          </AnimatePresence>

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
      </div>
    </div>
  );
};
