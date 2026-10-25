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
} from "lucide-react";
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
import { formatBytes } from "../lib/utils";
import {
  canonicalContainerIds,
  groupOf,
  moveMember,
  reorderMember,
} from "../lib/containerGroups";
import type { ContainerGroup, ContainerItem, ContainerStats } from "../types";

interface ContainerGroupsBarProps {
  /** Current container list — drives member resolution and the Isolated bucket. */
  containers: ContainerItem[];
  /** Saved groups, owned by the page so the flat list can filter by them. */
  groups: ContainerGroup[];
  /** Persists the whole group set (the page keeps the returned state). */
  onSaveGroups: (next: ContainerGroup[]) => Promise<void>;
  /** Live per-container stats from the page's stats tick. */
  statsMap?: Record<string, ContainerStats>;
  /** Called after a bulk start/stop so the page re-fetches row states. */
  onChanged?: () => void;
  /** Opens the details drawer for a member card. */
  onOpenDetails: (
    id: string,
    tab?: "logs" | "inspect" | "stats" | "terminal",
    debug?: boolean
  ) => void;
  onEditContainer?: (id: string) => void;
  /** Runs a lifecycle action on a member card. */
  onContainerAction: (container: ContainerItem, action: ContainerAction) => void;
  /** Containers with an action in flight, so their buttons disable. */
  busyMap?: Record<string, boolean>;
  /**
   * The ungrouped container cards, rendered inside the Isolated bucket so they
   * are visibly *in* that group rather than floating below it. The page owns
   * them because it also renders compose project headers between them.
   */
  children?: React.ReactNode;
}

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
const composeGroup = (project: string, memberIds: string[]): ContainerGroup => ({
  id: composeId(project),
  name: project,
  container_ids: memberIds,
  auto_start: false,
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
  memberIds: string[]
): ContainerGroup => ({
  id: composeId(project),
  name: composeId(project),
  container_ids: memberIds,
  auto_start: false,
});

/** True for a saved entry that only carries a compose project's card order. */
const isComposeOrderEntry = (g: ContainerGroup) => g.id.startsWith("compose:");

/** A card inside a group: identical to a list card, but compact and draggable. */
const MemberCard: React.FC<{
  c: ContainerItem;
  stats?: ContainerStats;
  busy: boolean;
  /** Groups this card is rendered in — enables the in-place reorder preview. */
  ownGroupId: string;
  onAction: (action: ContainerAction) => void;
  onOpenDetails: (tab?: "logs" | "inspect" | "stats" | "terminal", debug?: boolean) => void;
  onEditContainer?: () => void;
}> = ({ c, stats, busy, ownGroupId, onAction, onOpenDetails, onEditContainer }) => {
  const drag = useDraggable({
    id: c.id,
    label: c.name.replace(/^\//, ""),
    ownGroupId,
    composeOwned: !!c.compose_project,
  });
  return (
    <ContainerCard
      container={c}
      stats={stats}
      busy={busy}
      dragHandle={drag}
      compact
      onAction={onAction}
      onOpenDetails={onOpenDetails}
      onEditContainer={onEditContainer}
    />
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
  onChanged,
  onOpenDetails,
  onEditContainer,
  onContainerAction,
  busyMap,
  children,
}) => {
  const toast = useToast();
  const modal = useModal();
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [sizes, setSizes] = useState<Record<string, number>>({});
  // Inline rename: which card is being edited and the text being typed.
  const [editing, setEditing] = useState<{ id: string; name: string } | null>(
    null
  );
  const [saving, setSaving] = useState(false);
  // Isolated starts expanded: its cards are the main list, so collapsing it
  // should be the deliberate action, not the default.
  const [isolatedOpen, setIsolatedOpen] = useState(true);
  const toggleIsolated = () => setIsolatedOpen((v) => !v);

  const loadSizes = useCallback(async () => {
    try {
      setSizes((await api.getContainerSizes()) ?? {});
    } catch {
      /* engine offline — pills fall back to 0 */
    }
  }, []);

  useEffect(() => {
    loadSizes();
    const t = setInterval(loadSizes, 30_000);
    return () => clearInterval(t);
  }, [loadSizes]);

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
    try {
      await saveAll(
        groups.map((x) =>
          x.id === g.id ? { ...x, auto_start: !x.auto_start } : x
        )
      );
    } catch (e) {
      toast.error("Could not update auto-start", String(e));
    }
  };

  const runBulk = async (g: ContainerGroup, action: "start" | "stop") => {
    setBusy((b) => ({ ...b, [g.id]: true }));
    try {
      const msg =
        action === "start"
          ? await api.startContainerGroup(g.id)
          : await api.stopContainerGroup(g.id);
      toast.success(`Group ${action}`, `${g.name}: ${msg}`);
      onChanged?.();
    } catch (e) {
      toast.error(`Could not ${action} group`, String(e));
    } finally {
      setBusy((b) => ({ ...b, [g.id]: false }));
    }
  };

  /**
   * Starts/stops a compose project.
   *
   * A project group is derived, not saved, so the group command cannot address
   * it. It goes through the compose CLI instead, which is the only thing that
   * knows a project's services — stopping the containers directly would leave
   * compose's own state out of sync.
   */
  const runComposeBulk = async (
    g: ContainerGroup,
    action: "start" | "stop" | "restart"
  ) => {
    const project = g.id.slice("compose:".length);
    setBusy((b) => ({ ...b, [g.id]: true }));
    try {
      const out = await api.composeProjectAction(project, action);
      const past =
        action === "stop" ? "stopped" : action === "start" ? "started" : "restarted";
      toast.success(
        `Project ${past}`,
        out.trim() ? `${project}: ${out.trim()}` : project
      );
      onChanged?.();
    } catch (e) {
      toast.error(`Could not ${action} project`, String(e));
    } finally {
      setBusy((b) => ({ ...b, [g.id]: false }));
    }
  };

  const removeGroup = async (g: ContainerGroup) => {
    const members = membersOf(g);
    const choice = await modal.showChoice(
      members.length === 0
        ? `Delete group "${g.name}"?`
        : `Delete group "${g.name}" and its ${members.length} container${members.length === 1 ? "" : "s"}?`,
      members.length === 0
        ? [
            { value: "keep", label: "Delete group", variant: "danger" },
          ]
        : [
            {
              value: "keep",
              label: "Group only",
              hint: `Release ${members.length} container${members.length === 1 ? "" : "s"} to Isolated`,
              variant: "secondary",
            },
            {
              value: "delete",
              label: "Group + containers",
              hint: "Delete them from the engine",
              variant: "danger",
            },
          ],
      "Delete group",
      "danger"
    );
    if (!choice) return;
    try {
      if (choice === "delete") {
        const results = await Promise.allSettled(
          members.map((c) => api.removeContainer(c.id, true))
        );
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
      await saveAll(groups.filter((x) => x.id !== g.id));
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

  const resolveMember = useCallback(
    (id: string) =>
      containers.find(
        (x) => x.id === id || x.short_id === id || x.name === id
      ),
    [containers]
  );

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
  const membersOf = (g: ContainerGroup): ContainerItem[] => {
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
  };

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
      out.push(composeGroup(project, sorted.map((c) => c.id)));
    }
    // Projects sort by name so the list does not reshuffle between polls.
    return out.sort((a, b) => a.name.localeCompare(b.name));
  }, [visibleContainers, groups]);

  // Everything not claimed by a user group or a compose project is "Isolated".
  // Compose containers are excluded explicitly: their project group is derived,
  // not saved, so `groupOf` cannot see it.
  const isolated = visibleContainers.filter(
    (c) => !c.compose_project && !groupOf(groups, canonicalMap, c.id)
  );

  /** Groups the user owns — compose order entries are not rendered as cards. */
  const userGroups = groups.filter((g) => !isComposeOrderEntry(g));

  /**
   * Moves a container into `groupId`, or out of every group when `groupId` is
   * null. A container lives in one group only, so this is a move, not an
   * append.
   */
  const moveInto = useCallback(
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
          const others = groups.filter((g) => g.id !== ownId);
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

      const from = groupOf(groups, map, target.id);
      if ((from?.id ?? null) === groupId) {
        // Same group: this is a reorder, not a move.
        if (groupId && preview && !preview.unchanged) {
          const current = groups.find((g) => g.id === groupId);
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
                reorderMember(groups, map, target.id, groupId, nextIds)
              );
            } catch (err) {
              toast.error("Could not reorder group", String(err));
            }
          }
        }
        if (groupId) setExpanded((e) => ({ ...e, [groupId]: true }));
        return;
      }
      try {
        await saveAll(moveMember(groups, map, target.id, groupId));
        // Show the result of the drop: a collapsed group would otherwise
        // swallow the card with no visible feedback.
        if (groupId) setExpanded((e) => ({ ...e, [groupId]: true }));
        const label = target.name.replace(/^\//, "");
        const dest = groupId
          ? groups.find((g) => g.id === groupId)?.name ?? "group"
          : "Isolated";
        toast.success("Container moved", `${label} → ${dest}`);
      } catch (err) {
        toast.error("Could not move container", String(err));
      }
    },
    [containers, groups, composeGroups, onSaveGroups, toast]
  );

  // The drag layer reports where the drop landed; this component owns the move.
  useDropHandler(moveInto);

  /** Aggregated live stats across the group's members. */
  const aggregate = (g: ContainerGroup) => {
    let mem = 0;
    let cpu = 0;
    let disk = 0;
    for (const id of g.container_ids) {
      const cid = resolveMember(id)?.id ?? id;
      const s = statsMap?.[cid];
      if (s) {
        mem += s.mem_usage ?? 0;
        cpu += s.cpu_percent ?? 0;
      }
      disk += sizes[cid] ?? 0;
    }
    return { mem, cpu, disk };
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2 text-zinc-400">
          <FolderTree className="w-4 h-4 shrink-0" />
          <span className="shrink-0 text-xs font-semibold uppercase tracking-wider">
            Groups
          </span>
          <span className="truncate text-[11px] text-zinc-600">
            drag a card into a group to move it · one group per container
          </span>
        </div>
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

      {/* Full-width stack: a group card holds its member cards, so it spans the
          list instead of sitting in a narrow grid cell. Compose projects come
          first — they are the containers the user is least likely to touch. */}
      <div className="space-y-3">
        {composeGroups.map((g) => (
          <GroupCard
            key={g.id}
            group={g}
            members={membersOf(g)}
            aggregate={aggregate(g)}
            busy={!!busy[g.id]}
            isOpen={!!expanded[g.id]}
            editing={null}
            statsMap={statsMap}
            busyMap={busyMap}
            compose
            onToggle={() => setExpanded((e) => ({ ...e, [g.id]: !e[g.id] }))}
            onRenameStart={() => undefined}
            onRenameChange={() => undefined}
            onRenameCommit={() => undefined}
            onRenameCancel={() => undefined}
            onAutoStart={() => undefined}
            onBulk={(action) => void runComposeBulk(g, action)}
            onRestart={() => void runComposeBulk(g, "restart")}
            onDelete={() => undefined}
            onOpenDetails={onOpenDetails}
            onEditContainer={onEditContainer}
            onContainerAction={onContainerAction}
          />
        ))}

        {userGroups.map((g) => (
          <GroupCard
            key={g.id}
            group={g}
            members={membersOf(g)}
            aggregate={aggregate(g)}
            busy={!!busy[g.id]}
            isOpen={!!expanded[g.id]}
            editing={editing?.id === g.id ? editing.name : null}
            statsMap={statsMap}
            busyMap={busyMap}
            onToggle={() => setExpanded((e) => ({ ...e, [g.id]: !e[g.id] }))}
            onRenameStart={() => setEditing({ id: g.id, name: g.name })}
            onRenameChange={(name) => setEditing({ id: g.id, name })}
            onRenameCommit={() => void commitRename()}
            onRenameCancel={() => setEditing(null)}
            onAutoStart={() => void toggleAutoStart(g)}
            onBulk={(action) => void runBulk(g, action)}
            onRestart={() => undefined}
            onDelete={() => void removeGroup(g)}
            onOpenDetails={onOpenDetails}
            onEditContainer={onEditContainer}
            onContainerAction={onContainerAction}
          />
        ))}

        <IsolatedCard isolated={isolated} isOpen={isolatedOpen} onToggle={toggleIsolated}>
          {children}
        </IsolatedCard>
      </div>
    </div>
  );
};

interface GroupCardProps {
  group: ContainerGroup;
  members: ContainerItem[];
  aggregate: { mem: number; cpu: number; disk: number };
  busy: boolean;
  isOpen: boolean;
  editing: string | null;
  statsMap?: Record<string, ContainerStats>;
  busyMap?: Record<string, boolean>;
  /**
   * A compose project group: its membership is owned by the project, so it
   * cannot be renamed, deleted, auto-started, or have cards dragged out.
   */
  compose?: boolean;
  onToggle: () => void;
  onRenameStart: () => void;
  onRenameChange: (name: string) => void;
  onRenameCommit: () => void;
  onRenameCancel: () => void;
  onAutoStart: () => void;
  onBulk: (action: "start" | "stop") => void;
  /** Only used by compose project cards, which have a restart verb. */
  onRestart: () => void;
  onDelete: () => void;
  onOpenDetails: (
    id: string,
    tab?: "logs" | "inspect" | "stats" | "terminal",
    debug?: boolean
  ) => void;
  onEditContainer?: (id: string) => void;
  onContainerAction: (container: ContainerItem, action: ContainerAction) => void;
}

const GroupCard: React.FC<GroupCardProps> = ({
  group: g,
  members,
  aggregate,
  busy,
  isOpen,
  editing,
  statsMap,
  busyMap,
  compose = false,
  onToggle,
  onRenameStart,
  onRenameChange,
  onRenameCommit,
  onRenameCancel,
  onAutoStart,
  onBulk,
  onRestart,
  onDelete,
  onOpenDetails,
  onEditContainer,
  onContainerAction,
}) => {
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
      className={`rounded-2xl border bg-zinc-900/60 backdrop-blur-xl shadow-xl transition-colors duration-150 ${
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
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full border border-sky-500/40 bg-sky-500/10 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-sky-300">
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
          {!compose && (
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
          )}
          {hasMembers && (
            <>
              <Button
                variant="ghost"
                size="icon"
                title="Start group"
                aria-label={`Start group ${g.name}`}
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
                aria-label={`Stop group ${g.name}`}
                disabled={busy}
                onClick={() => onBulk("stop")}
                className="text-rose-400 hover:text-rose-300 hover:bg-rose-500/10"
              >
                <Square className="w-3.5 h-3.5 fill-current" />
              </Button>
              {/* Compose projects have a restart verb of their own; plain
                  groups get it per container from the card's own action. */}
              {compose && (
                <Button
                  variant="ghost"
                  size="icon"
                  title="Restart project"
                  aria-label={`Restart project ${g.name}`}
                  disabled={busy}
                  onClick={() => onRestart()}
                  className="text-blue-400 hover:text-blue-300 hover:bg-blue-500/10"
                >
                  <RotateCw className="w-3.5 h-3.5" />
                </Button>
              )}
            </>
          )}
          {!compose && (
            <>
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
              <Button
                variant="ghost"
                size="icon"
                title="Delete group"
                aria-label={`Delete group ${g.name}`}
                onClick={onDelete}
                className="text-zinc-400 hover:text-rose-400"
              >
                <Trash2 className="w-3.5 h-3.5" />
              </Button>
            </>
          )}
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
              {members.map((c) => (
                <React.Fragment key={c.id}>
                  {/* Where the dragged card would land. Rendered from live
                      pointer position, so the user sees the order change
                      before releasing. */}
                  {reorder && !reorder.unchanged && reorder.beforeKey === c.id && (
                    <div className="h-0.5 rounded-full bg-indigo-400/80 shadow-[0_0_8px] shadow-indigo-400/50" />
                  )}
                  <MemberCard
                    c={c}
                    stats={statsMap?.[c.id]}
                    busy={!!busyMap?.[c.id]}
                    ownGroupId={g.id}
                    onAction={(action) => onContainerAction(c, action)}
                    onOpenDetails={(tab, debug) => onOpenDetails(c.id, tab, debug)}
                    onEditContainer={onEditContainer ? () => onEditContainer(c.id) : undefined}
                  />
                </React.Fragment>
              ))}
              {reorder && !reorder.unchanged && reorder.beforeKey === null && (
                <div className="h-0.5 rounded-full bg-indigo-400/80 shadow-[0_0_8px] shadow-indigo-400/50" />
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
};

interface IsolatedCardProps {
  isolated: ContainerItem[];
  isOpen: boolean;
  onToggle: () => void;
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
  onToggle,
  children,
}) => {
  const drop = useDropTarget(null);
  const hasMembers = isolated.length > 0;
  const running = isolated.filter((c) => c.state === "running").length;

  return (
    <div
      ref={drop.ref}
      data-group-card="__isolated__"
      className={`rounded-2xl border bg-zinc-900/40 backdrop-blur-xl shadow-xl transition-colors duration-150 ${
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
