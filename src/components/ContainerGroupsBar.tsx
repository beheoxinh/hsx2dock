import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  FolderOpen,
  FolderTree,
  Pencil,
  Plus,
  Trash2,
  Activity,
  Boxes,
  Cpu,
  HardDrive,
  Play,
  RotateCw,
  Square,
  FileText,
  Loader2,
  GripVertical,
  ArrowUpToLine,
  ArrowDownToLine,
  Search,
  X,
} from "lucide-react";
import { GroupLogsModal } from "./groups/GroupLogsModal";
import { Button } from "./ui/Button";
import { Switch } from "./ui/Switch";
import { useToast } from "./ui/Toast";
import { useModal } from "./ui/ModalContext";
import { ContainerCard, type ContainerAction } from "./ContainerCard";
import {
  useDraggable,
  useDropHandler,
  useDropTarget,
  useReorderTarget,
} from "./ContainerDragLayer";
import { api } from "../lib/api";
import { getContainerStore } from "../lib/containerStore";
import {
  clearGroup,
  isContainerBusy,
  isGroupBusy,
  markQueued,
  releaseGroup,
  requestGroup,
  settleGroup,
  useGroupOp,
} from "../lib/containerOps";
import { useStableCallback } from "../hooks/useStableCallback";
import { formatBytes } from "../lib/utils";
import {
  applySectionOrder,
  canonicalContainerIds,
  cardDropIndex,
  groupOf,
  moveIdTo,
  moveMember,
  reorderMember,
} from "../lib/containerGroups";
import type { ContainerGroup, ContainerItem, ContainerStats, GroupOp } from "../types";
import { useProgressiveList } from "../hooks/useProgressiveList";
import { useListSort } from "../hooks/useListSort";
import { sortItems } from "../lib/listSort";
import { SortControl } from "./ui/SortControl";

const GROUP_SORT_KEYS = ["custom", "name", "containers", "running"] as const;
type GroupSortKey = (typeof GROUP_SORT_KEYS)[number];
type GroupSection = "compose" | "user";
const GROUP_SORT_OPTIONS: { key: GroupSortKey; label: string }[] = [
  { key: "custom", label: "custom (drag)" },
  { key: "name", label: "name" },
  { key: "containers", label: "containers" },
  { key: "running", label: "running" },
];

type DetailsTab = "logs" | "inspect" | "stats" | "terminal";
type RouteLike = { domain: string; path?: string | null };

interface ContainerGroupsBarProps {
  /** Current container list — drives member resolution and the Isolated bucket. */
  containers: ContainerItem[];
  /** Saved groups, owned by the page so the flat list can filter by them. */
  groups: ContainerGroup[];
  /** Persists the whole group set (the page keeps the returned state). */
  onSaveGroups: (next: ContainerGroup[]) => Promise<void>;
  /** Live per-container stats from the page's stats tick. */
  statsMap?: Record<string, ContainerStats>;
  /** Disk usage per container. When omitted the bar polls for its own. */
  sizesMap?: Record<string, number>;
  /** Called after a bulk start/stop so the page re-fetches row states. */
  onChanged?: () => void;
  /** Opens the details drawer for a member card. */
  onOpenDetails: (
    id: string,
    tab?: "logs" | "inspect" | "stats" | "terminal",
    debug?: boolean
  ) => void;
  onEditContainer?: (id: string) => void;
  onRepairNetwork?: (container: ContainerItem) => void;
  /** Gateway routes per container id, precomputed by the page. */
  routesById?: Map<string, RouteLike[]>;
  /** Runs a lifecycle action on a member card. Must be stable. */
  onContainerAction: (container: ContainerItem, action: ContainerAction) => void;
  /**
   * The ungrouped container cards, rendered inside the Isolated bucket so they
   * are visibly *in* that group rather than floating below it. The page owns
   * them because it also renders compose project headers between them.
   */
  children?: React.ReactNode;
}

const EMPTY_MEMBERS: ContainerItem[] = [];
const noop = () => undefined;

const newId = () =>
  `g-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

/**
 * Stable id of the synthetic group that mirrors a compose project's containers.
 *
 * Compose owns its containers: the project starts and stops them, and its files
 * describe them, so they are shown as a group that cannot be renamed, deleted,
 * or dragged out of — only reordered inside, which is purely a display choice.
 */
const composeId = (project: string) => `compose:${project}`;

/** The synthetic group wrapping one compose project's containers. */
const composeGroup = (project: string, memberIds: string[], autoStart: boolean = false): ContainerGroup => ({
  id: composeId(project),
  name: project,
  container_ids: memberIds,
  auto_start: autoStart,
});

/**
 * The on-disk form of a compose project's remembered order.
 *
 * Its name is the synthetic id, not the project name: config validation
 * requires unique names, and a user group may legitimately be called "web"
 * while a compose project is too. The name is never rendered.
 */
const persistedComposeGroup = (
  project: string,
  memberIds: string[],
  autoStart: boolean = false
): ContainerGroup => ({
  id: composeId(project),
  name: composeId(project),
  container_ids: memberIds,
  auto_start: autoStart,
});

/** True for a saved entry that only carries a compose project's card order. */
const isComposeOrderEntry = (g: ContainerGroup) => g.id.startsWith("compose:");

/**
 * A card inside a group: identical to a list card, but compact and draggable.
 * Its handlers are bar-stable and take the container; the per-card bindings
 * are made here so the memo survives the parent's stats ticks.
 */
const MemberCard = React.memo<{
  c: ContainerItem;
  stats?: ContainerStats;
  sizeBytes?: number;
  /** Groups this card is rendered in — enables the in-place reorder preview. */
  ownGroupId: string;
  onAction: (c: ContainerItem, action: ContainerAction) => void;
  onOpenDetails: (id: string, tab?: DetailsTab, debug?: boolean) => void;
  onEditContainer?: (id: string) => void;
  onRepairNetwork?: (c: ContainerItem) => void;
  gatewayRoutes?: RouteLike[];
}>(function MemberCard({ c, stats, sizeBytes, ownGroupId, onAction, onOpenDetails, onEditContainer, onRepairNetwork, gatewayRoutes }) {
  const drag = useDraggable({
    id: c.id,
    label: c.name.replace(/^\//, ""),
    ownGroupId,
    composeOwned: !!c.compose_project,
  });
  const action = React.useCallback((a: ContainerAction) => onAction(c, a), [onAction, c]);
  const details = React.useCallback(
    (tab?: DetailsTab, debug?: boolean) => onOpenDetails(c.id, tab, debug),
    [onOpenDetails, c.id]
  );
  const edit = React.useMemo(
    () => (onEditContainer ? () => onEditContainer(c.id) : undefined),
    [onEditContainer, c.id]
  );
  const repair = React.useMemo(
    () => (onRepairNetwork ? () => onRepairNetwork(c) : undefined),
    [onRepairNetwork, c]
  );
  return (
    <ContainerCard
      container={c}
      stats={stats}
      sizeBytes={sizeBytes}
      dragHandle={drag}
      compact
      onAction={action}
      onOpenDetails={details}
      onEditContainer={edit}
      onRepairNetwork={repair}
      gatewayRoutes={gatewayRoutes}
    />
  );
});

/**
 * Start/stop/restart buttons of a group. While a pass runs, the clicked verb
 * becomes a spinner with live progress and the others are disabled, so a
 * 30-container start is visibly working instead of looking frozen.
 */
const GroupPowerButtons: React.FC<{
  name: string;
  op: GroupOp | undefined;
  busy: boolean;
  restart?: boolean;
  onBulk: (action: "start" | "stop" | "restart") => void;
}> = ({ name, op, busy, restart, onBulk }) => {
  const progress =
    op && op.total > 0 ? `${Math.min(op.completed, op.total)}/${op.total}` : null;
  const verbLabel: Record<string, string> = {
    start: "Starting",
    stop: "Stopping",
    restart: "Restarting",
  };
  if (op) {
    return (
      <span
        role="status"
        aria-live="polite"
        title={`${verbLabel[op.op] ?? "Working"} ${name}${progress ? ` — ${progress} done` : ""}`}
        className="inline-flex h-8 items-center gap-1.5 rounded-lg border border-blue-500/40 bg-blue-500/10 px-2.5 text-[11px] font-medium text-blue-300"
      >
        <Loader2 className="w-3.5 h-3.5 animate-spin" />
        <span className="whitespace-nowrap">
          {verbLabel[op.op] ?? "Working"}
          {progress ? ` ${progress}` : "…"}
        </span>
      </span>
    );
  }
  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        title="Start group"
        aria-label={`Start group ${name}`}
        disabled={busy}
        onClick={() => onBulk("start")}
        className="text-emerald-400 hover:text-emerald-300 hover:bg-emerald-500/10"
      >
        <Play className="w-3.5 h-3.5 fill-current" />
      </Button>
      <Button
        variant="ghost"
        size="icon"
        title="Stop group"
        aria-label={`Stop group ${name}`}
        disabled={busy}
        onClick={() => onBulk("stop")}
        className="text-rose-400 hover:text-rose-300 hover:bg-rose-500/10"
      >
        <Square className="w-3.5 h-3.5 fill-current" />
      </Button>
      {/* Compose projects have a restart verb of their own; plain groups get
          it per container from the card's own action. */}
      {restart && (
        <Button
          variant="ghost"
          size="icon"
          title="Restart project"
          aria-label={`Restart project ${name}`}
          disabled={busy}
          onClick={() => onBulk("restart")}
          className="text-blue-400 hover:text-blue-300 hover:bg-blue-500/10"
        >
          <RotateCw className="w-3.5 h-3.5" />
        </Button>
      )}
    </>
  );
};

/** The dashed area a card can be dropped into. */
const DropZone: React.FC<{ isOver: boolean; children: React.ReactNode }> = ({
  isOver,
  children,
}) => (
  <div
    className={`flex flex-col items-center justify-center gap-1 rounded-xl border border-dashed px-3 py-6 text-center transition-colors ${
      isOver
        ? "border-emerald-500/70 bg-emerald-500/5"
        : "border-zinc-700/70 bg-zinc-950/30"
    }`}
  >
    <FolderTree className="w-5 h-5 text-zinc-600" />
    {children}
  </div>
);

/** Inline-editable group name plus its member count. */
const GroupName: React.FC<{
  name: string;
  countLabel: string;
  editing: string | null;
  onChange: (name: string) => void;
  onCommit: () => void;
  onCancel: () => void;
}> = ({ name, countLabel, editing, onChange, onCommit, onCancel }) =>
  editing !== null ? (
    <input
      autoFocus
      value={editing}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") onCommit();
        if (e.key === "Escape") onCancel();
      }}
      onBlur={onCommit}
      aria-label="Group name"
      className="min-w-0 flex-1 rounded border border-zinc-700 bg-zinc-800/80 px-1.5 py-0.5 text-sm text-white focus:border-indigo-500 focus:outline-none"
    />
  ) : (
    <>
      <span className="truncate text-sm font-semibold text-white">{name}</span>
      <span className="shrink-0 text-[11px] text-zinc-500">{countLabel}</span>
    </>
  );

/**
 * Which group cards the user left expanded. Purely presentational, so it lives
 * in localStorage rather than the app config: it must survive a restart without
 * turning into another field the backend has to validate. Keyed by group id, so
 * a renamed group keeps its state; a deleted group's entry is dropped with it.
 */
const EXPANDED_STORAGE_KEY = "containerGroupsExpanded";
/** Synthetic id of the "Isolated" bucket. */
const ISOLATED_GROUP_ID = "__isolated__";

function loadExpandedGroups(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(EXPANDED_STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : {};
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out: Record<string, boolean> = {};
    for (const [id, open] of Object.entries(parsed)) {
      if (typeof open === "boolean") out[id] = open;
    }
    return out;
  } catch {
    return {};
  }
}

/** Isolated is shown above the other groups instead of below them. */
const ISOLATED_ON_TOP_KEY = "hsx2dock.isolatedOnTop";

function loadIsolatedOnTop(): boolean {
  try {
    return localStorage.getItem(ISOLATED_ON_TOP_KEY) === "1";
  } catch {
    return false;
  }
}

function persistIsolatedOnTop(onTop: boolean) {
  try {
    localStorage.setItem(ISOLATED_ON_TOP_KEY, onTop ? "1" : "0");
  } catch {
    /* storage disabled — the position just resets to the bottom */
  }
}

function persistExpandedGroups(next: Record<string, boolean>) {
  try {
    localStorage.setItem(EXPANDED_STORAGE_KEY, JSON.stringify(next));
  } catch {
    /* storage disabled — expansion memory is a convenience, not a requirement */
  }
}

/**
 * Container groups as full-width folder cards.
 *
 * A container belongs to exactly one group — like a file in one folder — so a
 * drop *moves* it out of whatever group held it before. Containers in no group
 * are shown under "Isolated", which is not a saved group: it is the heading for
 * everything unclaimed, and dropping onto it removes a container from its group.
 *
 * Groups persist in the app config (not on the engine).
 */
export const ContainerGroupsBar: React.FC<ContainerGroupsBarProps> = ({
  containers,
  groups,
  onSaveGroups,
  statsMap,
  sizesMap,
  onChanged,
  onOpenDetails,
  onEditContainer,
  onRepairNetwork,
  onContainerAction,
  routesById,
  children,
}) => {
  const toast = useToast();
  const modal = useModal();
  // Restored from the last session: a group the user opened stays open.
  const [expanded, setExpanded] = useState<Record<string, boolean>>(loadExpandedGroups);
  const [isolatedOnTop, setIsolatedOnTop] = useState(loadIsolatedOnTop);
  const toggleIsolatedPosition = useStableCallback(() =>
    setIsolatedOnTop((cur) => {
      persistIsolatedOnTop(!cur);
      return !cur;
    })
  );
  const [localSizes, setSizes] = useState<Record<string, number>>({});
  // The page owns the app-wide size cache (primed at startup); self-polling is
  // only the fallback when no map is supplied.
  const sizes = sizesMap ?? localSizes;
  // Inline rename: which card is being edited and the text being typed.
  const [editing, setEditing] = useState<{ id: string; name: string } | null>(
    null
  );
  const [saving, setSaving] = useState(false);
  const toggleGroupExpanded = useCallback((id: string) => {
    setExpanded((prev) => {
      const next = { ...prev, [id]: !prev[id] };
      persistExpandedGroups(next);
      return next;
    });
  }, []);

  const forgetGroupExpanded = useCallback((id: string) => {
    setExpanded((prev) => {
      if (!(id in prev)) return prev;
      const next = { ...prev };
      delete next[id];
      persistExpandedGroups(next);
      return next;
    });
  }, []);

  /** Force a card open/closed (a drop opens its target) and remember it. */
  const setGroupExpanded = useCallback((id: string, open: boolean) => {
    setExpanded((prev) => {
      if (!!prev[id] === open) return prev;
      const next = { ...prev, [id]: open };
      persistExpandedGroups(next);
      return next;
    });
  }, []);

  // Isolated starts expanded: its cards are the main list, so collapsing it
  // should be the deliberate action, not the default. Like the group cards,
  // the choice survives a restart.
  const isolatedOpen = expanded[ISOLATED_GROUP_ID] !== false;
  const toggleIsolated = () => toggleGroupExpanded(ISOLATED_GROUP_ID);
  // The logs modal remembers the group, not its members: membership is derived
  // live below, so containers moved in or out while it is open show up.
  const [logsGroupId, setLogsGroupId] = useState<string | null>(null);

  const loadSizes = useCallback(async () => {
    try {
      setSizes((await api.getContainerSizes()) ?? {});
    } catch {
      /* engine offline — pills fall back to 0 */
    }
  }, []);

  useEffect(() => {
    if (sizesMap) return;
    loadSizes();
    const t = setInterval(() => {
      if (document.hidden) return;
      void loadSizes();
      // The backend re-walks sizes at most every 5 min (each walk costs
      // dockerd seconds of CPU), so asking more often only returns the cache.
    }, 120_000);
    return () => clearInterval(t);
  }, [loadSizes, sizesMap]);

  const saveAll = async (next: ContainerGroup[]) => {
    // Invariant: a compose container belongs to its project card. Older saved
    // groups may still list one, and config validation rejects a container
    // claimed twice — so user groups are scrubbed here rather than failing the
    // save with a message the user cannot act on.
    const composeOwned = new Set(
      containers.filter((c) => c.compose_project).map((c) => c.id)
    );
    // Stored tokens can be short ids or names, so they are resolved before the
    // comparison; an unresolvable token is left alone (it is a stale entry the
    // member resolver already ignores).
    const tokenMap = canonicalContainerIds(containers);
    // A project that no longer exists leaves an order entry behind; drop it so
    // the config does not accumulate one per `compose up` ever run.
    const liveProjects = new Set(
      containers.map((c) => c.compose_project).filter(Boolean)
    );
    const cleaned = next
      .filter(
        (g) =>
          !isComposeOrderEntry(g) ||
          liveProjects.has(g.id.slice("compose:".length))
      )
      .map((g) =>
        isComposeOrderEntry(g)
          ? g
          : {
              ...g,
              container_ids: g.container_ids.filter((token) => {
                const id = tokenMap.get(token);
                return id ? !composeOwned.has(id) : true;
              }),
            }
      );
    await onSaveGroups(cleaned);
  };

  const toggleAutoStart = async (g: ContainerGroup) => {
    // Read at call time: two edits in quick succession must not both
    // start from the list this render closed over.
    const latest = getContainerStore().groups;
    try {
      await saveAll(
        latest.map((x) =>
          x.id === g.id ? { ...x, auto_start: !x.auto_start } : x
        )
      );
    } catch (e) {
      toast.error("Could not update auto-start", String(e));
    }
  };

  const toggleComposeAutoStart = async (g: ContainerGroup) => {
    // Read at call time: two edits in quick succession must not both
    // start from the list this render closed over.
    const latest = getContainerStore().groups;
    const project = g.id.slice("compose:".length);
    const existing = latest.find((x) => x.id === g.id);
    const nextAutoStart = !g.auto_start;
    const others = latest.filter((x) => x.id !== g.id);
    try {
      const nextGroup = existing
        ? { ...existing, auto_start: nextAutoStart }
        : persistedComposeGroup(project, g.container_ids, nextAutoStart);
      await saveAll([...others, nextGroup]);
    } catch (e) {
      toast.error("Could not update auto-start", String(e));
    }
  };

  const isolatedGroup = groups.find((x) => x.id === "__isolated__");
  const isolatedAutoStart = !!isolatedGroup?.auto_start;

  const toggleIsolatedAutoStart = async () => {
    // Read at call time: two edits in quick succession must not both
    // start from the list this render closed over.
    const latest = getContainerStore().groups;
    const nextAutoStart = !isolatedAutoStart;
    const existing = latest.find((x) => x.id === "__isolated__");
    const others = latest.filter((x) => x.id !== "__isolated__");
    try {
      const nextGroup: ContainerGroup = existing
        ? { ...existing, auto_start: nextAutoStart }
        : {
            id: "__isolated__",
            name: "__isolated__",
            container_ids: [],
            auto_start: nextAutoStart,
          };
      await saveAll([...others, nextGroup]);
    } catch (e) {
      toast.error("Could not update auto-start", String(e));
    }
  };

  /**
   * Starts/stops a group — saved, Isolated, or a compose project
   * (`compose:<project>`, which the backend runs through the compose CLI so
   * compose's own state stays in sync).
   *
   * The backend only schedules the pass and returns at once; per-container
   * progress arrives as `container-op` events (each card shows its own
   * queued/starting spinner) and the group's as `group-op` events.
   */
  const runBulk = useStableCallback(
    async (g: ContainerGroup, action: "start" | "stop" | "restart") => {
      if (action === "restart") return runComposeRestart(g);
      if (!requestGroup(g.id, action)) return;
      try {
        if (action === "start") await api.startContainerGroup(g.id);
        else await api.stopContainerGroup(g.id);
      } catch (e) {
        releaseGroup(g.id);
        toast.error(`Could not ${action} group`, String(e));
      }
    }
  );

  /**
   * Compose restart has no group pass in the backend: it is one compose CLI
   * call. Its members are marked queued here so their cards spin until the
   * call returns and the list shows them running again.
   */
  const runComposeRestart = async (g: ContainerGroup) => {
    const project = g.id.slice("compose:".length);
    if (!requestGroup(g.id, "restart")) return;
    const ids = (membersByGroupId.get(g.id) ?? [])
      .map((c) => c.id)
      .filter((id) => !isContainerBusy(id));
    markQueued(ids, "restart", g.id);
    try {
      await api.composeProjectAction(project, "restart");
      settleGroup(g.id);
    } catch (e) {
      clearGroup(g.id);
      toast.error("Could not restart project", String(e));
    } finally {
      releaseGroup(g.id);
      onChanged?.();
    }
  };

  const removeComposeProject = async (g: ContainerGroup) => {
    const project = g.id.slice("compose:".length);
    const members = membersOf(g);
    const confirmed = await modal.showConfirm(
      `This will stop and remove all ${members.length} containers belonging to this project (compose down).`,
      `Delete Compose Project "${project}"?`,
      "danger"
    );
    if (!confirmed) return;
    // The confirm dialog is open for a while: read the saved groups as they are
    // now, not as they were when the button was clicked.
    if (!requestGroup(g.id, "remove")) {
      toast.error("Project busy", "It is already being started, stopped or removed.");
      return;
    }
    markQueued(
      members.map((c) => c.id).filter((id) => !isContainerBusy(id)),
      "remove",
      g.id
    );
    try {
      const out = await api.composeProjectAction(project, "down");
      toast.success("Project deleted", out.trim() ? `${project}: ${out.trim()}` : project);
      // Clean up saved order entry if any
      await saveAll(getContainerStore().groups.filter((x) => x.id !== g.id));
      forgetGroupExpanded(g.id);
      onChanged?.();
    } catch (e) {
      // Fallback: remove containers directly if compose down fails
      try {
        await Promise.allSettled(members.map((c) => api.removeContainer(c.id, true)));
        await saveAll(getContainerStore().groups.filter((x) => x.id !== g.id));
        forgetGroupExpanded(g.id);
        toast.success("Project deleted", project);
        onChanged?.();
      } catch (err) {
        toast.error("Could not delete compose project", String(err));
      }
    } finally {
      settleGroup(g.id);
      releaseGroup(g.id);
    }
  };

  const removeGroup = async (g: ContainerGroup) => {
    const members = membersOf(g);
    const choice = await modal.showChoice(
      members.length === 0
        ? `This group is empty. Removing it will not affect any containers.`
        : `This group currently contains ${members.length} container${members.length === 1 ? "" : "s"}. Choose how you would like to proceed:`,
      members.length === 0
        ? [
            { value: "keep", label: "Delete Group", variant: "danger" },
          ]
        : [
            {
              value: "keep",
              label: "Delete Group Only",
              hint: `Keep ${members.length} container${members.length === 1 ? "" : "s"} and move them to Isolated`,
              variant: "secondary",
            },
            {
              value: "delete",
              label: "Delete Group and Containers",
              hint: "Permanently stop and delete all member containers from the Docker engine",
              variant: "danger",
            },
          ],
      `Delete "${g.name}"`,
      "danger"
    );
    if (!choice) return;
    if (isGroupBusy(g.id)) {
      toast.error("Group busy", `"${g.name}" is being started or stopped. Try again when it finishes.`);
      return;
    }
    try {
      if (choice === "delete") {
        requestGroup(g.id, "remove");
        markQueued(
          members.map((c) => c.id).filter((id) => !isContainerBusy(id)),
          "remove",
          g.id
        );
        const results = await Promise.allSettled(
          members.map((c) => api.removeContainer(c.id, true))
        ).finally(() => {
          settleGroup(g.id);
          releaseGroup(g.id);
        });
        const failed = results.filter((r) => r.status === "rejected").length;
        if (failed > 0) {
          toast.error(
            "Some containers survived",
            `${failed} of ${members.length} could not be deleted.`
          );
        } else {
          toast.success(
            "Containers deleted",
            `${members.length} removed from the engine.`
          );
        }
        onChanged?.();
      }
      // Latest saved groups: the confirm and the container removals above took time.
      await saveAll(getContainerStore().groups.filter((x) => x.id !== g.id));
      // The group is gone, so its remembered expansion must go too — a future
      // group that happens to reuse the id would otherwise open by surprise.
      forgetGroupExpanded(g.id);
      toast.success("Group deleted", g.name);
    } catch (e) {
      toast.error("Could not delete group", String(e));
    }
  };

  /** Creates an empty group immediately — members are added by dragging. */
  const createGroup = async () => {
    const base = "New group";
    let name = base;
    let n = 2;
    const taken = new Set(userGroups.map((g) => g.name.trim().toLowerCase()));
    while (taken.has(name.toLowerCase())) name = `${base} ${n++}`;
    const next: ContainerGroup = {
      id: newId(),
      name,
      container_ids: [],
      auto_start: false,
    };
    setSaving(true);
    try {
      await saveAll([...groups, next]);
      setEditing({ id: next.id, name });
    } catch (e) {
      toast.error("Could not create group", String(e));
    } finally {
      setSaving(false);
    }
  };

  /** Commits an inline rename; an empty name cancels instead of saving. */
  const commitRename = async () => {
    if (!editing) return;
    const name = editing.name.trim();
    const current = groups.find((g) => g.id === editing.id);
    setEditing(null);
    if (!current || !name || name === current.name) return;
    try {
      await saveAll(
        groups.map((x) => (x.id === current.id ? { ...x, name } : x))
      );
    } catch (e) {
      toast.error("Could not rename group", String(e));
    }
  };

  /** Token → container, built once per list instead of a scan per member. */
  const byToken = useMemo(() => {
    const m = new Map<string, ContainerItem>();
    for (const c of containers) {
      m.set(c.id, c);
      m.set(c.short_id, c);
      m.set(c.name, c);
    }
    return m;
  }, [containers]);
  const resolveMember = useCallback((id: string) => byToken.get(id), [byToken]);

  // kind node containers belong to the Kubernetes tab; they must never be
  // offered as group members or counted here.
  const visibleContainers = useMemo(
    () => containers.filter((c) => !c.kind_cluster),
    [containers]
  );
  const canonicalMap = useMemo(
    () => canonicalContainerIds(visibleContainers),
    [visibleContainers]
  );

  /** Resolved, de-duplicated members of a group (stale tokens are dropped). */
  const membersOf = useCallback(
    (g: ContainerGroup): ContainerItem[] => {
      const seen = new Set<string>();
      const out: ContainerItem[] = [];
      for (const token of g.container_ids) {
        const c = resolveMember(token);
        if (!c || seen.has(c.id)) continue;
        // A compose container belongs to its project card, even if an older
        // saved group still lists it — otherwise it would render twice.
        if (!isComposeOrderEntry(g) && c.compose_project) continue;
        seen.add(c.id);
        out.push(c);
      }
      return out;
    },
    [resolveMember]
  );

  /**
   * Compose projects shown as locked groups.
   *
   * A project is not a saved group: it is derived from the containers' labels,
   * so it appears and disappears with `docker compose up/down`. Its member order
   * is remembered in a saved group of the same synthetic id when the user
   * reorders it, which is why the saved groups are consulted for ordering.
   */
  const composeGroups = useMemo(() => {
    const byProject = new Map<string, ContainerItem[]>();
    for (const c of visibleContainers) {
      if (!c.compose_project) continue;
      const arr = byProject.get(c.compose_project);
      if (arr) arr.push(c);
      else byProject.set(c.compose_project, [c]);
    }
    const out: ContainerGroup[] = [];
    for (const [project, members] of byProject) {
      const saved = groups.find((g) => g.id === composeId(project));
      const order = new Map<string, number>();
      saved?.container_ids.forEach((token, i) => order.set(token, i));
      const sorted = [...members].sort(
        (a, b) =>
          (order.get(a.id) ?? Number.MAX_SAFE_INTEGER) -
          (order.get(b.id) ?? Number.MAX_SAFE_INTEGER)
      );
      out.push(composeGroup(project, sorted.map((c) => c.id), !!saved?.auto_start));
    }
    // Projects sort by name so the list does not reshuffle between polls.
    return out.sort((a, b) => a.name.localeCompare(b.name));
  }, [visibleContainers, groups]);

  // Everything not claimed by a user group or a compose project is "Isolated".
  // Compose containers are excluded explicitly: their project group is derived,
  // not saved, so `groupOf` cannot see it.
  const isolated = useMemo(() => {
    const claimed = new Set<string>();
    for (const g of groups) {
      for (const token of g.container_ids) claimed.add(canonicalMap.get(token) ?? token);
    }
    return visibleContainers.filter((c) => !c.compose_project && !claimed.has(c.id));
  }, [visibleContainers, groups, canonicalMap]);

  /**
   * Groups the user owns — compose order entries and the Isolated policy entry
   * are not rendered as cards.
   */
  const userGroups = useMemo(
    () => groups.filter((g) => !isComposeOrderEntry(g) && g.id !== ISOLATED_GROUP_ID),
    [groups]
  );

  /** Precomputed member map ensuring stable array identities per group across re-renders */
  // A group whose members did not change keeps its previous array, so its
  // memoised card is not re-rendered because some other container changed.
  const membersCache = React.useRef(new Map<string, ContainerItem[]>());
  const membersByGroupId = useMemo(() => {
    const map = new Map<string, ContainerItem[]>();
    for (const g of [...composeGroups, ...userGroups]) {
      const next = membersOf(g);
      const prev = membersCache.current.get(g.id);
      map.set(
        g.id,
        prev && prev.length === next.length && prev.every((c, i) => c === next[i])
          ? prev
          : next
      );
    }
    membersCache.current = map;
    return map;
  }, [composeGroups, userGroups, membersOf]);

  // Group cards are ordered by the toolbar choice; compose projects stay above
  // user groups. Members inside a group keep their dragged order.
  const [groupSort, setGroupSort] = useListSort("containerGroups", GROUP_SORT_KEYS, {
    key: "custom",
    dir: "asc",
  });
  const [sortedComposeGroups, sortedUserGroups] = useMemo(() => {
    // Custom order is the saved list's order (set by dragging); a compose
    // project never dragged has no saved entry and goes last, A→Z.
    const position = new Map(groups.map((g, i) => [g.id, i]));
    const getters = {
      name: (g: ContainerGroup) => g.name,
      custom: (g: ContainerGroup) => position.get(g.id),
      containers: (g: ContainerGroup) => membersByGroupId.get(g.id)?.length ?? 0,
      running: (g: ContainerGroup) =>
        (membersByGroupId.get(g.id) ?? []).filter((c) => c.state === "running").length,
    };
    const byName = (g: ContainerGroup) => g.name;
    return [
      sortItems(composeGroups, getters, groupSort, byName),
      sortItems(userGroups, getters, groupSort, byName),
    ];
  }, [composeGroups, userGroups, groups, membersByGroupId, groupSort]);

  // Group name search; the container search above filters the cards inside.
  const [groupQuery, setGroupQuery] = useState("");
  const groupNeedle = groupQuery.trim().toLowerCase();
  const shownComposeGroups = useMemo(
    () =>
      groupNeedle
        ? sortedComposeGroups.filter((g) => g.name.toLowerCase().includes(groupNeedle))
        : sortedComposeGroups,
    [sortedComposeGroups, groupNeedle]
  );
  const shownUserGroups = useMemo(
    () =>
      groupNeedle
        ? sortedUserGroups.filter((g) => g.name.toLowerCase().includes(groupNeedle))
        : sortedUserGroups,
    [sortedUserGroups, groupNeedle]
  );

  /**
   * Saves a section's dragged order. Compose projects get an order entry if
   * they had none; the list switches to the custom order so the result shows.
   */
  const commitGroupOrder = useStableCallback(async (section: GroupSection, orderedIds: string[]) => {
    const latest = getContainerStore().groups;
    const saved = new Map(latest.map((g) => [g.id, g]));
    const source = new Map((section === "compose" ? composeGroups : userGroups).map((g) => [g.id, g]));
    const ordered = orderedIds.flatMap((id) => {
      const entry = saved.get(id);
      if (entry) return [entry];
      const g = source.get(id);
      if (!g) return [];
      return [section === "compose" ? persistedComposeGroup(g.name, g.container_ids, g.auto_start) : g];
    });
    setGroupSort({ key: "custom", dir: "asc" });
    try {
      await saveAll(applySectionOrder(latest, ordered));
    } catch (err) {
      toast.error("Could not save group order", String(err));
    }
  });

  // Group cards are dragged by their grip with plain pointer events (HTML5
  // drag-and-drop is unreliable in WebKitGTK), separately from the container
  // drag layer: a group only moves within its own section.
  const composeSectionRef = React.useRef<HTMLDivElement | null>(null);
  const userSectionRef = React.useRef<HTMLDivElement | null>(null);
  const [groupDrag, setGroupDrag] = useState<{ id: string; section: GroupSection; index: number } | null>(null);
  const startGroupDrag = useStableCallback((id: string, section: GroupSection, e: React.PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const ids = (section === "compose" ? sortedComposeGroups : sortedUserGroups).map((g) => g.id);
    const root = (section === "compose" ? composeSectionRef : userSectionRef).current;
    const startY = e.clientY;
    let active = false;
    let index: number | null = null;
    const rects = () =>
      ids.map(
        (gid) =>
          root?.querySelector<HTMLElement>(`[data-group-card="${CSS.escape(gid)}"]`)?.getBoundingClientRect() ?? {
            top: 0,
            height: 0,
          }
      );
    const onMove = (ev: PointerEvent) => {
      if (!active) {
        if (Math.abs(ev.clientY - startY) < 5) return;
        active = true;
        document.body.style.cursor = "grabbing";
      }
      const next = cardDropIndex(ids, rects(), ev.clientY, id);
      index = next;
      setGroupDrag((cur) => (cur && cur.id === id && cur.index === next ? cur : { id, section, index: next }));
    };
    const end = (commit: boolean) => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("blur", onCancel);
      window.removeEventListener("keydown", onKey);
      document.body.style.cursor = "";
      setGroupDrag(null);
      if (!commit || !active || index === null) return;
      const next = moveIdTo(ids, id, index);
      if (next.some((x, i) => x !== ids[i])) void commitGroupOrder(section, next);
    };
    const onUp = () => end(true);
    const onCancel = () => end(false);
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") end(false);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("blur", onCancel);
    window.addEventListener("keydown", onKey);
  });
  const gripCompose = useStableCallback((id: string, e: React.PointerEvent) => startGroupDrag(id, "compose", e));
  const gripUser = useStableCallback((id: string, e: React.PointerEvent) => startGroupDrag(id, "user", e));
  const gripBlocked = groupNeedle
    ? "Clear the group search to reorder groups"
    : null;

  /** Drop line position inside `section`: the id it sits before, or "end". */
  const dropMarker = (section: GroupSection, list: ContainerGroup[]): string | null => {
    if (!groupDrag || groupDrag.section !== section) return null;
    const ids = list.map((g) => g.id);
    const next = moveIdTo(ids, groupDrag.id, groupDrag.index);
    if (!next.some((x, i) => x !== ids[i])) return null;
    const without = ids.filter((x) => x !== groupDrag.id);
    return without[groupDrag.index] ?? "end";
  };

  /**
   * Moves a container into `groupId`, or out of every group when `groupId` is
   * null. A container lives in one group only, so this is a move, not an
   * append.
   */
  const moveInto = useStableCallback(
    async (
      groupId: string | null,
      payload: { id: string; label: string; lockedToGroupId?: string },
      preview?: { beforeKey: string | null; index: number; unchanged: boolean } | null
    ) => {
      const target = containers.find(
        (x) =>
          x.id === payload.id ||
          x.short_id === payload.id ||
          x.name === payload.id
      );
      if (!target) return;
      // Read at call time: two edits in quick succession must not both
      // start from the list this render closed over.
      const latest = getContainerStore().groups;
      const map = canonicalContainerIds(containers);

      // A compose member is owned by its project. Reordering inside it is a
      // display preference we remember; moving it out would leave the project
      // half-managed, so it is refused rather than silently ignored.
      if (target.compose_project) {
        const ownId = composeId(target.compose_project);
        if (groupId !== ownId) {
          toast.error(
            "Managed by compose",
            `${target.name.replace(/^\//, "")} belongs to project "${target.compose_project}" and cannot leave it.`
          );
          return;
        }
        if (!preview || preview.unchanged) return;
        const current = composeGroups.find((g) => g.id === ownId);
        const ordered = current?.container_ids ?? [];
        const without = ordered.filter((id) => id !== target.id);
        const at = Math.max(0, Math.min(preview.index, without.length));
        const nextIds = [
          ...without.slice(0, at),
          target.id,
          ...without.slice(at),
        ];
        try {
          // The synthetic compose group is persisted alongside real ones; it is
          // not shown as an editable card, it only carries the member order.
          const others = latest.filter((g) => g.id !== ownId);
          await saveAll([
            ...others,
            persistedComposeGroup(target.compose_project, nextIds),
          ]);
        } catch (err) {
          toast.error("Could not reorder project", String(err));
        }
        return;
      }

      // A compose project group is not a destination for loose containers.
      if (groupId && groupId.startsWith("compose:")) {
        toast.error(
          "Project group is read-only",
          "Compose containers are managed by their project."
        );
        return;
      }

      const from = groupOf(latest, map, target.id);
      if ((from?.id ?? null) === groupId) {
        // Same group: this is a reorder, not a move.
        if (groupId && preview && !preview.unchanged) {
          const current = latest.find((g) => g.id === groupId);
          if (current) {
            const ordered = current.container_ids
              .map((token) => map.get(token))
              .filter((id): id is string => !!id);
            const without = ordered.filter((id) => id !== target.id);
            const at = Math.max(0, Math.min(preview.index, without.length));
            const nextIds = [
              ...without.slice(0, at),
              target.id,
              ...without.slice(at),
            ];
            try {
              await saveAll(
                reorderMember(latest, map, target.id, groupId, nextIds)
              );
            } catch (err) {
              toast.error("Could not reorder group", String(err));
            }
          }
        }
        if (groupId) setGroupExpanded(groupId, true);
        return;
      }
      try {
        await saveAll(moveMember(latest, map, target.id, groupId));
        // Show the result of the drop: a collapsed group would otherwise
        // swallow the card with no visible feedback.
        if (groupId) setGroupExpanded(groupId, true);
        const label = target.name.replace(/^\//, "");
        const dest = groupId
          ? latest.find((g) => g.id === groupId)?.name ?? "group"
          : "Isolated";
        toast.success("Container moved", `${label} → ${dest}`);
      } catch (err) {
        toast.error("Could not move container", String(err));
      }
    }
  );

  // The drag layer reports where the drop landed; this component owns the move.
  useDropHandler(moveInto);

  // Card handlers take the group, so one stable function serves every card
  // and the memoised GroupCards only re-render when their own data changes.
  const onComposeAutoStart = useStableCallback((g: ContainerGroup) => void toggleComposeAutoStart(g));
  const onUserAutoStart = useStableCallback((g: ContainerGroup) => void toggleAutoStart(g));
  const onRemoveCompose = useStableCallback((g: ContainerGroup) => void removeComposeProject(g));
  const onRemoveGroup = useStableCallback((g: ContainerGroup) => void removeGroup(g));
  const openLogs = useStableCallback((g: ContainerGroup) => setLogsGroupId(g.id));
  const logsTarget = useMemo(() => {
    if (!logsGroupId) return null;
    const g = [...composeGroups, ...userGroups].find((x) => x.id === logsGroupId);
    return g ? { name: g.name, containers: membersByGroupId.get(g.id) ?? [] } : null;
  }, [logsGroupId, composeGroups, userGroups, membersByGroupId]);
  const onRenameStart = useStableCallback((g: ContainerGroup) => setEditing({ id: g.id, name: g.name }));
  const onRenameChange = useStableCallback((g: ContainerGroup, name: string) => setEditing({ id: g.id, name }));
  const onRenameCommit = useStableCallback(() => void commitRename());
  const onRenameCancel = useStableCallback(() => setEditing(null));
  const runIsolatedBulk = useStableCallback((action: "start" | "stop") =>
    void runBulk(
      { id: ISOLATED_GROUP_ID, name: "Isolated", container_ids: [], auto_start: false },
      action
    )
  );

  const isolatedCard = (
    <IsolatedCard
      isolated={isolated}
      isOpen={isolatedOpen}
      autoStart={isolatedAutoStart}
      onTop={isolatedOnTop}
      onToggle={toggleIsolated}
      onAutoStart={toggleIsolatedAutoStart}
      onMove={toggleIsolatedPosition}
      onBulk={runIsolatedBulk}
    >
      {children}
    </IsolatedCard>
  );

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2 text-zinc-400">
          <FolderTree className="w-4 h-4 shrink-0" />
          <span className="shrink-0 text-xs font-semibold uppercase tracking-wider">
            Groups
          </span>
          <span className="truncate text-[11px] text-zinc-600">
            drag a card into a group to move it · drag a group's grip to reorder groups
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-2">
        <div className="relative w-48">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-zinc-500" />
          <input
            type="text"
            value={groupQuery}
            onChange={(e) => setGroupQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") setGroupQuery("");
            }}
            placeholder="Search groups..."
            aria-label="Search groups by name"
            className="h-9 w-full rounded-none border border-zinc-800 bg-zinc-900/90 pl-8 pr-7 text-xs text-white placeholder-zinc-500 focus:border-blue-500 focus:outline-none"
          />
          {groupQuery && (
            <button
              type="button"
              onClick={() => setGroupQuery("")}
              title="Clear group search"
              aria-label="Clear group search"
              className="absolute right-1.5 top-1/2 -translate-y-1/2 p-0.5 text-zinc-500 hover:text-zinc-200"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
        <SortControl value={groupSort} options={GROUP_SORT_OPTIONS} onChange={setGroupSort} label="Sort groups" />
        <Button
          variant="outline"
          size="sm"
          className="shrink-0 gap-1.5 border-emerald-500/60 bg-emerald-500/10 text-xs text-emerald-300 hover:border-emerald-400 hover:bg-emerald-500/20 hover:text-emerald-200"
          onClick={createGroup}
          disabled={saving}
          title="Create an empty group"
        >
          <Plus className="w-3.5 h-3.5" />
          New group
        </Button>
        </div>
      </div>

      {/* Full-width stack: a group card holds its member cards, so it spans the
          list instead of sitting in a narrow grid cell. Compose projects come
          first — they are the containers the user is least likely to touch. */}
      <div className="space-y-3">
        {isolatedOnTop && isolatedCard}

        {groupNeedle && shownComposeGroups.length === 0 && shownUserGroups.length === 0 && (
          <p className="border border-dashed border-zinc-800 px-3 py-2 text-xs text-zinc-500">
            No group matches "{groupQuery.trim()}".
          </p>
        )}

        {shownComposeGroups.length > 0 && (
        <div ref={composeSectionRef} className="space-y-3">
        {shownComposeGroups.map((g) => (
          <React.Fragment key={g.id}>
          {dropMarker("compose", shownComposeGroups) === g.id && <GroupDropLine />}
          <GroupCard
            group={g}
            onGripDown={shownComposeGroups.length > 1 ? gripCompose : undefined}
            gripBlocked={gripBlocked}
            dragging={groupDrag?.id === g.id}
            members={membersByGroupId.get(g.id) ?? EMPTY_MEMBERS}
            isOpen={!!expanded[g.id]}
            editing={null}
            statsMap={statsMap}
            sizesMap={sizes}
            compose
            onToggle={toggleGroupExpanded}
            onRenameStart={noop}
            onRenameChange={noop}
            onRenameCommit={noop}
            onRenameCancel={noop}
            onAutoStart={onComposeAutoStart}
            onBulk={runBulk}
            onDelete={onRemoveCompose}
            onOpenLogs={openLogs}
            onOpenDetails={onOpenDetails}
            onEditContainer={onEditContainer}
            onRepairNetwork={onRepairNetwork}
            onContainerAction={onContainerAction}
            routesById={routesById}
          />
          </React.Fragment>
        ))}
        {dropMarker("compose", shownComposeGroups) === "end" && <GroupDropLine />}
        </div>
        )}

        {shownUserGroups.length > 0 && (
        <div ref={userSectionRef} className="space-y-3">
        {shownUserGroups.map((g) => (
          <React.Fragment key={g.id}>
          {dropMarker("user", shownUserGroups) === g.id && <GroupDropLine />}
          <GroupCard
            group={g}
            onGripDown={shownUserGroups.length > 1 ? gripUser : undefined}
            gripBlocked={gripBlocked}
            dragging={groupDrag?.id === g.id}
            members={membersByGroupId.get(g.id) ?? EMPTY_MEMBERS}
            isOpen={!!expanded[g.id]}
            editing={editing?.id === g.id ? editing.name : null}
            statsMap={statsMap}
            sizesMap={sizes}
            onToggle={toggleGroupExpanded}
            onRenameStart={onRenameStart}
            onRenameChange={onRenameChange}
            onRenameCommit={onRenameCommit}
            onRenameCancel={onRenameCancel}
            onAutoStart={onUserAutoStart}
            onBulk={runBulk}
            onDelete={onRemoveGroup}
            onOpenLogs={openLogs}
            onOpenDetails={onOpenDetails}
            onEditContainer={onEditContainer}
            onRepairNetwork={onRepairNetwork}
            onContainerAction={onContainerAction}
            routesById={routesById}
          />
          </React.Fragment>
        ))}
        {dropMarker("user", shownUserGroups) === "end" && <GroupDropLine />}
        </div>
        )}

        {!isolatedOnTop && isolatedCard}
      </div>

      {logsTarget && (
        <GroupLogsModal
          isOpen={true}
          onClose={() => setLogsGroupId(null)}
          groupName={logsTarget.name}
          containers={logsTarget.containers}
        />
      )}
    </div>
  );
};

interface GroupCardProps {
  group: ContainerGroup;
  members: ContainerItem[];
  aggregate?: { mem: number; cpu: number; disk: number };
  isOpen: boolean;
  editing: string | null;
  statsMap?: Record<string, ContainerStats>;
  sizesMap?: Record<string, number>;
  /**
   * A compose project group: its membership is owned by the project, so it
   * cannot be renamed, deleted, auto-started, or have cards dragged out.
   */
  compose?: boolean;
  // Every handler is bar-stable and receives the group, so the memo holds.
  onToggle: (id: string) => void;
  onRenameStart: (g: ContainerGroup) => void;
  onRenameChange: (g: ContainerGroup, name: string) => void;
  onRenameCommit: () => void;
  onRenameCancel: () => void;
  onAutoStart: (g: ContainerGroup) => void;
  onBulk: (g: ContainerGroup, action: "start" | "stop" | "restart") => void;
  onDelete: (g: ContainerGroup) => void;
  onOpenLogs?: (g: ContainerGroup) => void;
  onOpenDetails: (id: string, tab?: DetailsTab, debug?: boolean) => void;
  onEditContainer?: (id: string) => void;
  onRepairNetwork?: (container: ContainerItem) => void;
  onContainerAction: (container: ContainerItem, action: ContainerAction) => void;
  routesById?: Map<string, RouteLike[]>;
  /** Starts dragging this card to reorder groups; absent when it cannot move. */
  onGripDown?: (id: string, e: React.PointerEvent) => void;
  /** Why reordering is off right now (a search is active), shown on the grip. */
  gripBlocked?: string | null;
  /** This card is the one being dragged. */
  dragging?: boolean;
}

/** Where a dragged group card would land. */
const GroupDropLine = () => <div className="h-0.5 bg-emerald-400/80 shadow-[0_0_8px] shadow-emerald-400/50" />;

const GroupCard = React.memo<GroupCardProps>(function GroupCard({
  group: g,
  members,
  aggregate: propAggregate,
  isOpen,
  editing,
  statsMap,
  sizesMap,
  compose = false,
  onToggle: onToggleId,
  onRenameStart: onRenameStartG,
  onRenameChange: onRenameChangeG,
  onRenameCommit,
  onRenameCancel,
  onAutoStart: onAutoStartG,
  onBulk: onBulkG,
  onDelete: onDeleteG,
  onOpenLogs: onOpenLogsG,
  onOpenDetails,
  onEditContainer,
  onRepairNetwork,
  onContainerAction,
  routesById,
  onGripDown,
  gripBlocked,
  dragging = false,
}) {
  const onToggle = () => onToggleId(g.id);
  const onRenameStart = () => onRenameStartG(g);
  const onRenameChange = (name: string) => onRenameChangeG(g, name);
  const onAutoStart = () => onAutoStartG(g);
  const onBulk = (action: "start" | "stop" | "restart") => onBulkG(g, action);
  const onDelete = () => onDeleteG(g);
  const onOpenLogs = onOpenLogsG ? () => onOpenLogsG(g) : undefined;
  // Live progress of a start/stop pass on this group (backend events).
  const groupOp = useGroupOp(g.id);
  const busy = !!groupOp;
  const aggregate = React.useMemo(() => {
    if (propAggregate) return propAggregate;
    let mem = 0;
    let cpu = 0;
    let disk = 0;
    for (const c of members) {
      const s = statsMap?.[c.id];
      if (s) {
        mem += s.mem_usage ?? 0;
        cpu += s.cpu_percent ?? 0;
      }
      disk += sizesMap?.[c.id] ?? 0;
    }
    return { mem, cpu, disk };
  }, [propAggregate, members, statsMap, sizesMap]);
  const drop = useDropTarget(g.id);
  const hasMembers = members.length > 0;
  const running = members.filter((c) => c.state === "running").length;

  // Compose containers may only be reordered inside their own project, so a
  // drop from one onto any other card is refused — do not paint the accepting
  // highlight for a drop that will be rejected.
  const acceptsDrop = !drop.payload
    ? false
    : compose
      ? drop.payload.composeOwned === true && drop.payload.ownGroupId === g.id
      : drop.payload.composeOwned !== true;
  const isOver = drop.isOver && acceptsDrop;

  // Members are read for reorder hit-testing, which the drag layer calls on
  // every pointermove — keep the accessors stable so they do not re-register.
  const stackRef = React.useRef<HTMLDivElement | null>(null);
  const orderedIds = React.useMemo(() => members.map((c) => c.id), [members]);
  // A group of 100+ members mounted every card (with live stats) on open.
  // Render pages as the user scrolls; reorder hit-testing still sees the full
  // order, and a drop below the last mounted card lands before the next one.
  const progressive = useProgressiveList(members, g.id, 40);
  const getElements = React.useCallback(
    () =>
      stackRef.current
        ? Array.from(
            stackRef.current.querySelectorAll<HTMLElement>("[data-container-card]")
          )
        : [],
    []
  );
  const getOrderedIds = React.useCallback(() => orderedIds, [orderedIds]);
  const reorder = useReorderTarget(g.id, getElements, getOrderedIds);

  return (
    <div
      ref={drop.ref}
      data-group-card={g.id}
      className={`rounded-2xl border bg-zinc-900/90 shadow-lg transition-colors duration-150 ${
        dragging ? "opacity-50 " : ""
      }${
        isOver
          ? "border-emerald-500/70 bg-emerald-500/5 ring-1 ring-emerald-500/40"
          : compose
            ? "border-sky-500/25"
            : "border-zinc-800/80"
      }`}
      title={
        compose
          ? `Compose project — managed by docker compose`
          : g.auto_start
            ? "Auto-starts with the engine"
            : "Manual start only"
      }
    >
      <div className="flex items-center gap-2 px-4 pt-3 pb-2">
        {onGripDown && (
          <button
            type="button"
            data-no-drag
            onPointerDown={gripBlocked ? undefined : (e) => onGripDown(g.id, e)}
            disabled={!!gripBlocked}
            title={gripBlocked ?? "Drag to reorder groups"}
            aria-label={`Reorder group ${g.name}`}
            style={{ touchAction: "none" }}
            className="-ml-2 shrink-0 p-0.5 text-zinc-600 hover:text-zinc-300 enabled:cursor-grab disabled:opacity-40"
          >
            <GripVertical className="w-4 h-4" />
          </button>
        )}
        {hasMembers ? (
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={isOpen}
            aria-label={isOpen ? `Collapse ${g.name}` : `Expand ${g.name}`}
            className="flex min-w-0 flex-1 items-center gap-2 rounded-lg text-left focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-indigo-500"
          >
            <ChevronDown
              className={`w-4 h-4 shrink-0 text-zinc-500 transition-transform duration-200 ${
                isOpen ? "rotate-0" : "-rotate-90"
              }`}
            />
            <FolderOpen className="w-4 h-4 shrink-0 text-indigo-400" />
            <GroupName
              name={g.name}
              countLabel={`${running}/${members.length} running`}
              editing={editing}
              onChange={onRenameChange}
              onCommit={onRenameCommit}
              onCancel={onRenameCancel}
            />
          </button>
        ) : (
          /* Empty group: no chevron, no actions — just the name and the drop
             zone below, which is the only thing it can do yet. */
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <FolderTree className="w-4 h-4 shrink-0 text-indigo-400/70" />
            <GroupName
              name={g.name}
              countLabel="empty"
              editing={editing}
              onChange={onRenameChange}
              onCommit={onRenameCommit}
              onCancel={onRenameCancel}
            />
          </div>
        )}

        {/* Compose projects are derived from container labels, not saved
            groups: the badge marks why their card has no rename/delete. */}
        {compose && (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-none border border-sky-500/40 bg-sky-500/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-sky-300">
            <Boxes className="w-3 h-3" />
            compose
          </span>
        )}

        {/* Rename, auto-start and delete apply to every group, empty or not —
            an empty group still has a name and a startup policy to configure.
            Start/stop are hidden until there is something to act on. */}
        <div className="flex shrink-0 items-center gap-1">
          {/* Auto-start is a policy, not an action, so it leads the row as a
              labelled switch — an icon button here read as "run once".
              Compose projects start with their project, not the engine. */}
          <Switch
            checked={g.auto_start}
            onChange={onAutoStart}
            label="Auto-start"
            title={
              g.auto_start
                ? "Auto-start on — starts with the engine"
                : "Auto-start off — manual start only"
            }
            className="mr-1.5"
          />
          {hasMembers && (
            <GroupPowerButtons
              name={g.name}
              op={groupOp}
              busy={busy}
              restart={compose}
              onBulk={onBulk}
            />
          )}
          {hasMembers && onOpenLogs && (
            <Button
              variant="ghost"
              size="icon"
              title="Group logs"
              aria-label={`View group logs for ${g.name}`}
              onClick={onOpenLogs}
              className="text-zinc-400 hover:text-indigo-300 hover:bg-indigo-500/10"
            >
              <FileText className="w-3.5 h-3.5" />
            </Button>
          )}
          {!compose && (
            <Button
              variant="ghost"
              size="icon"
              title="Rename group"
              aria-label={`Rename group ${g.name}`}
              onClick={onRenameStart}
              className="text-zinc-400 hover:text-white"
            >
              <Pencil className="w-3.5 h-3.5" />
            </Button>
          )}
          <Button
            variant="ghost"
            size="icon"
            title={compose ? "Delete compose project" : "Delete group"}
            aria-label={compose ? `Delete compose project ${g.name}` : `Delete group ${g.name}`}
            disabled={busy}
            onClick={onDelete}
            className="text-zinc-400 hover:text-rose-400"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </Button>
        </div>
      </div>

      {hasMembers && (
        <div className="flex flex-wrap items-center gap-1.5 px-4 pb-2">
          <span className="inline-flex items-center gap-1 rounded-lg border border-zinc-700/50 bg-zinc-800/60 px-2 py-0.5 font-mono text-[11px] text-emerald-300">
            <Activity className="w-3 h-3 text-emerald-400" />
            {formatBytes(aggregate.mem)}
          </span>
          <span className="inline-flex items-center gap-1 rounded-lg border border-zinc-700/50 bg-zinc-800/60 px-2 py-0.5 font-mono text-[11px] text-blue-300">
            <Cpu className="w-3 h-3 text-blue-400" />
            {aggregate.cpu.toFixed(1)}%
          </span>
          <span className="inline-flex items-center gap-1 rounded-lg border border-zinc-700/50 bg-zinc-800/60 px-2 py-0.5 font-mono text-[11px] text-amber-300">
            <HardDrive className="w-3 h-3 text-amber-400" />
            {formatBytes(aggregate.disk)}
          </span>
        </div>
      )}

      <div className="px-4 pb-3">
        {hasMembers ? (
          isOpen ? (
            <div ref={stackRef} className="space-y-2">
              {progressive.visible.map((c) => (
                <React.Fragment key={c.id}>
                  {/* Where the dragged card would land. Rendered from live
                      pointer position, so the user sees the order change
                      before releasing. */}
                  {reorder && !reorder.unchanged && reorder.beforeKey === c.id && (
                    <div className="h-0.5 rounded-none bg-indigo-400/80 shadow-[0_0_8px] shadow-indigo-400/50" />
                  )}
                  <MemberCard
                    c={c}
                    stats={statsMap?.[c.id]}
                    sizeBytes={sizesMap?.[c.id]}
                    ownGroupId={g.id}
                    onAction={onContainerAction}
                    onOpenDetails={onOpenDetails}
                    onEditContainer={onEditContainer}
                    onRepairNetwork={onRepairNetwork}
                    gatewayRoutes={routesById?.get(c.id)}
                  />
                </React.Fragment>
              ))}
              {reorder && !reorder.unchanged && reorder.beforeKey === null && (
                <div className="h-0.5 rounded-none bg-indigo-400/80 shadow-[0_0_8px] shadow-indigo-400/50" />
              )}
              {progressive.hasMore && (
                <div ref={progressive.sentinelRef} className="h-8" aria-hidden="true" />
              )}
            </div>
          ) : (
            /* Collapsed: a one-line summary, not the drop hint — the group
               already has members, so it is not waiting for a first drop. */
            <button
              type="button"
              onClick={onToggle}
              className="flex w-full items-center justify-between gap-2 rounded-xl border border-zinc-800/80 bg-zinc-950/40 px-3 py-2 text-left transition-colors hover:border-zinc-700/70 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-indigo-500"
            >
              <span className="truncate text-xs text-zinc-400">
                {members
                  .slice(0, 3)
                  .map((c) => c.name.replace(/^\//, ""))
                  .join(", ")}
                {members.length > 3 ? ` +${members.length - 3} more` : ""}
              </span>
              <span className="shrink-0 text-[11px] text-zinc-600">
                {isOver ? "Release to move here" : "Expand"}
              </span>
            </button>
          )
        ) : (
          <DropZone isOver={isOver}>
            <p className="text-xs text-zinc-500">
              Drop container cards here to add them
            </p>
          </DropZone>
        )}
      </div>
    </div>
  );
});

interface IsolatedCardProps {
  isolated: ContainerItem[];
  isOpen: boolean;
  autoStart: boolean;
  /** Shown above the other groups rather than below. */
  onTop: boolean;
  onToggle: () => void;
  onAutoStart: () => void;
  /** Flips Isolated between the top and the bottom of the group list. */
  onMove: () => void;
  onBulk: (action: "start" | "stop") => void;
  /** The ungrouped container cards, rendered inside this bucket. */
  children?: React.ReactNode;
}

/**
 * The bucket for containers with no group.
 *
 * It is not a saved group — it cannot be renamed or deleted, and it has no
 * auto-start policy. It is the home of every container that belongs to no
 * user group, and its body renders those cards directly so they are visibly
 * *inside* it. Dropping onto it takes a container out of whichever group
 * held it.
 */
const IsolatedCard: React.FC<IsolatedCardProps> = ({
  isolated,
  isOpen,
  autoStart,
  onTop,
  onToggle,
  onAutoStart,
  onMove,
  onBulk,
  children,
}) => {
  const drop = useDropTarget(null);
  const groupOp = useGroupOp(ISOLATED_GROUP_ID);
  const hasMembers = isolated.length > 0;
  const running = isolated.filter((c) => c.state === "running").length;

  return (
    <div
      ref={drop.ref}
      data-group-card="__isolated__"
      className={`rounded-2xl border bg-zinc-900/60 shadow-lg transition-colors duration-150 ${
        drop.isOver && drop.payload?.composeOwned !== true
          ? "border-emerald-500/70 bg-emerald-500/5 ring-1 ring-emerald-500/40"
          : "border-zinc-800/80"
      }`}
      title="Containers that are not in any group"
    >
      <div className="flex items-center gap-2 px-4 pt-3 pb-2">
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={isOpen}
          aria-label={isOpen ? "Collapse Isolated" : "Expand Isolated"}
          className="flex min-w-0 flex-1 items-center gap-2 rounded-lg text-left focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-indigo-500"
        >
          <ChevronDown
            className={`w-4 h-4 shrink-0 text-zinc-500 transition-transform duration-200 ${
              isOpen ? "rotate-0" : "-rotate-90"
            }`}
          />
          <FolderTree className="w-4 h-4 shrink-0 text-zinc-400" />
          <span className="truncate text-sm font-semibold text-zinc-300">
            Isolated
          </span>
          <span className="shrink-0 text-[11px] text-zinc-500">
            {hasMembers
              ? `${running}/${isolated.length} running`
              : "every container is grouped"}
          </span>
        </button>

        <div className="flex shrink-0 items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            onClick={onMove}
            title={onTop ? "Shown above the groups — click to move it below" : "Shown below the groups — click to move it to the top"}
            aria-label={onTop ? "Move Isolated below the groups" : "Move Isolated above the groups"}
            className="mr-1 text-zinc-400 hover:text-white"
          >
            {onTop ? <ArrowDownToLine className="w-3.5 h-3.5" /> : <ArrowUpToLine className="w-3.5 h-3.5" />}
          </Button>
          <Switch
            checked={autoStart}
            onChange={onAutoStart}
            label="Auto-start"
            title={
              autoStart
                ? "Auto-start on — starts with the engine"
                : "Auto-start off — manual start only"
            }
            className="mr-1.5"
          />
          {hasMembers && (
            <GroupPowerButtons
              name="Isolated"
              op={groupOp}
              busy={!!groupOp}
              onBulk={(action) => action !== "restart" && onBulk(action)}
            />
          )}
        </div>
      </div>

      <div className="px-4 pb-3">
        {hasMembers && isOpen ? (
          <div className="space-y-3">
            {children ?? (
              <p className="px-1 text-xs text-zinc-500">
                No ungrouped containers to show
              </p>
            )}
          </div>
        ) : (
          <DropZone isOver={drop.isOver}>
            <p className="text-xs text-zinc-500">
              {hasMembers
                ? "Expand to see the containers that are not in any group"
                : "Containers created outside a group appear here"}
            </p>
            <p className="text-[11px] text-zinc-600">
              Drop a card here to take it out of its group
            </p>
          </DropZone>
        )}
      </div>
    </div>
  );
};
