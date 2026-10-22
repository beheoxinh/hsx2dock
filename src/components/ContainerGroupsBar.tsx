import React, { useCallback, useEffect, useMemo, useState } from "react";
import {
  FolderTree,
  FolderOpen,
  Play,
  Square,
  Pencil,
  Trash2,
  Plus,
  Check,
  X,
  ChevronDown,
  Activity,
  Cpu,
  HardDrive,
  GripVertical,
} from "lucide-react";
import { Button } from "./ui/Button";
import { useToast } from "./ui/Toast";
import { useModal } from "./ui/ModalContext";
import { api } from "../lib/api";
import { formatBytes } from "../lib/utils";
import {
  canonicalContainerIds,
  groupOf,
  moveMember,
} from "../lib/containerGroups";
import type { ContainerGroup, ContainerItem, ContainerStats } from "../types";

interface ContainerGroupsBarProps {
  /** Current container list — drives member resolution and the Isolated group. */
  containers: ContainerItem[];
  /** Saved groups, owned by the page so the container list can filter by them. */
  groups: ContainerGroup[];
  /** Persists the whole group set (the page keeps the returned state). */
  onSaveGroups: (next: ContainerGroup[]) => Promise<void>;
  /** Live per-container stats from the page's stats tick — powers the RAM/CPU pills. */
  statsMap?: Record<string, ContainerStats>;
  /** Called after a bulk start/stop so the page re-fetches row states. */
  onChanged?: () => void;
}

const newId = () =>
  `g-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

/** The drop payload shared with the container rows on the page. */
export const CONTAINER_DRAG_TYPE = "text/dm-container-id";

/** Synthetic id for the "Isolated" folder (ungrouped containers). */
const ISOLATED_ID = "__isolated__";

/**
 * Docker Desktop-style container groups as collapsible folder cards.
 *
 * A container belongs to exactly one group — like a file in one folder — so a
 * drop *moves* it there, out of whatever group held it before. Containers in no
 * group are listed under the pinned "Isolated" group, which is where a drop
 * target is also used to take a container back out of every group.
 *
 * Groups persist in the app config (not on the engine).
 */
export const ContainerGroupsBar: React.FC<ContainerGroupsBarProps> = ({
  containers,
  groups,
  onSaveGroups,
  statsMap,
  onChanged,
}) => {
  const toast = useToast();
  const modal = useModal();
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const [sizes, setSizes] = useState<Record<string, number>>({});
  // Inline rename: which card is being edited and the text being typed.
  const [editing, setEditing] = useState<{ id: string; name: string } | null>(null);
  const [saving, setSaving] = useState(false);

  const loadSizes = useCallback(async () => {
    try {
      setSizes(await api.getContainerSizes());
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
    await onSaveGroups(next);
  };

  const toggleAutoStart = async (g: ContainerGroup) => {
    try {
      await saveAll(
        groups.map((x) => (x.id === g.id ? { ...x, auto_start: !x.auto_start } : x))
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

  const removeGroup = async (g: ContainerGroup) => {
    const ok = await modal.showConfirm(
      `Delete group "${g.name}"?`,
      "The containers themselves are not touched.",
      "danger"
    );
    if (!ok) return;
    try {
      await saveAll(groups.filter((x) => x.id !== g.id));
    } catch (e) {
      toast.error("Could not delete group", String(e));
    }
  };

  /** Creates an empty group immediately — members are added by dragging. */
  const createGroup = async () => {
    const base = "New group";
    let name = base;
    let n = 2;
    const taken = new Set(groups.map((g) => g.name.trim().toLowerCase()));
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
      setExpanded((e) => ({ ...e, [next.id]: true }));
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
      await saveAll(groups.map((x) => (x.id === current.id ? { ...x, name } : x)));
    } catch (e) {
      toast.error("Could not rename group", String(e));
    }
  };

  /**
   * Drops a container into `groupId`, or takes it out of every group when
   * `groupId` is null. A container lives in one group only, so this is a move
   * rather than an append.
   */
  const moveInto = async (groupId: string | null, e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOverId(null);
    const token = e.dataTransfer.getData(CONTAINER_DRAG_TYPE);
    if (!token) return;
    const target = containers.find(
      (x) => x.id === token || x.short_id === token || x.name === token
    );
    if (!target) return;
    const map = canonicalContainerIds(containers);
    const from = groupOf(groups, map, target.id);
    if ((from?.id ?? null) === groupId) {
      setExpanded((e2) => (groupId ? { ...e2, [groupId]: true } : e2));
      return;
    }
    try {
      await saveAll(moveMember(groups, map, target.id, groupId));
      const label = target.name.replace(/^\//, "");
      const dest = groupId
        ? groups.find((g) => g.id === groupId)?.name ?? "group"
        : "Isolated";
      toast.success("Container moved", `${label} → ${dest}`);
    } catch (err) {
      toast.error("Could not move container", String(err));
    }
  };

  const removeMember = async (memberId: string) => {
    try {
      const map = canonicalContainerIds(containers);
      const target = resolveMember(memberId);
      await saveAll(moveMember(groups, map, target?.id ?? memberId, null));
    } catch (e) {
      toast.error("Could not update group", String(e));
    }
  };

  const resolveMember = (id: string) =>
    containers.find((x) => x.id === id || x.short_id === id || x.name === id);

  /** Aggregated live stats across the group's running members. */
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

  // kind node containers belong to the Kubernetes tab; they must never be
  // offered as group members or counted in the picker.
  const visibleContainers = containers.filter((c) => !c.kind_cluster);
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
      seen.add(c.id);
      out.push(c);
    }
    return out;
  };

  // Everything not claimed by a user group is "Isolated" — the default folder.
  const isolated = visibleContainers.filter((c) => !groupOf(groups, canonicalMap, c.id));

  const groupDragProps = (id: string | null) => ({
    onDragOver: (e: React.DragEvent) => {
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      if (dragOverId !== (id ?? ISOLATED_ID)) setDragOverId(id ?? ISOLATED_ID);
    },
    onDragLeave: (e: React.DragEvent) => {
      if (!e.currentTarget.contains(e.relatedTarget as Node)) {
        setDragOverId((d) => (d === (id ?? ISOLATED_ID) ? null : d));
      }
    },
    onDrop: (e: React.DragEvent) => void moveInto(id, e),
  });

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2 text-zinc-400">
          <FolderTree className="w-4 h-4 shrink-0" />
          <span className="shrink-0 text-xs font-semibold uppercase tracking-wider">Groups</span>
          <span className="truncate text-[11px] text-zinc-600">
            drag a card into a group to move it · one group per container
          </span>
        </div>
        <Button
          variant="ghost"
          size="sm"
          className="shrink-0 gap-1.5 text-xs"
          onClick={createGroup}
          disabled={saving}
          title="Create an empty group"
        >
          <Plus className="w-3.5 h-3.5" />
          New group
        </Button>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
        {groups.map((g) => {
          const isBusy = !!busy[g.id];
          const isOpen = !!expanded[g.id];
          const members = membersOf(g);
          const running = members.filter((c) => c.state === "running").length;
          const agg = aggregate(g);
          const isDropTarget = dragOverId === g.id;
          const isRenaming = editing?.id === g.id;
          return (
            <div
              key={g.id}
              className={`rounded-2xl border bg-zinc-900/60 backdrop-blur-xl shadow-xl transition-all duration-200 ${
                isDropTarget
                  ? "border-emerald-500/70 bg-emerald-500/5 ring-1 ring-emerald-500/40"
                  : "border-zinc-800/80 hover:border-zinc-700/60"
              }`}
              title={g.auto_start ? "Auto-starts with the engine" : "Manual start only"}
              {...groupDragProps(g.id)}
            >
              {/* Card header: name + running + expand + actions */}
              <div className="flex items-center gap-2 px-4 pt-3 pb-2">
                <button
                  type="button"
                  onClick={() => setExpanded((e) => ({ ...e, [g.id]: !isOpen }))}
                  aria-expanded={isOpen}
                  aria-label={isOpen ? "Collapse group" : "Expand group"}
                  className="flex items-center gap-2 min-w-0 flex-1 text-left rounded-lg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-indigo-500"
                >
                  <ChevronDown
                    className={`w-4 h-4 shrink-0 text-zinc-500 transition-transform duration-200 ${isOpen ? "rotate-0" : "-rotate-90"}`}
                  />
                  {isOpen ? (
                    <FolderOpen className="w-4 h-4 shrink-0 text-indigo-400" />
                  ) : (
                    <FolderTree className="w-4 h-4 shrink-0 text-indigo-400" />
                  )}
                  {isRenaming ? (
                    <input
                      autoFocus
                      value={editing.name}
                      onChange={(e) => setEditing({ id: g.id, name: e.target.value })}
                      onClick={(e) => e.stopPropagation()}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") void commitRename();
                        if (e.key === "Escape") setEditing(null);
                      }}
                      onBlur={() => void commitRename()}
                      aria-label="Group name"
                      className="min-w-0 flex-1 rounded border border-zinc-700 bg-zinc-800/80 px-1.5 py-0.5 text-sm text-white focus:outline-none focus:border-indigo-500"
                    />
                  ) : (
                    <span className="truncate text-sm font-semibold text-white">{g.name}</span>
                  )}
                  <span className="shrink-0 text-[11px] text-zinc-500">
                    {running}/{members.length} running
                  </span>
                </button>
                <div className="flex items-center gap-0.5 shrink-0">
                  <Button
                    variant="ghost"
                    size="icon"
                    title="Start group"
                    aria-label="Start group"
                    disabled={isBusy || members.length === 0}
                    onClick={() => runBulk(g, "start")}
                    className="text-emerald-400 hover:text-emerald-300 hover:bg-emerald-500/10"
                  >
                    <Play className="w-3.5 h-3.5 fill-current" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    title="Stop group"
                    aria-label="Stop group"
                    disabled={isBusy || members.length === 0}
                    onClick={() => runBulk(g, "stop")}
                    className="text-rose-400 hover:text-rose-300 hover:bg-rose-500/10"
                  >
                    <Square className="w-3.5 h-3.5 fill-current" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    title={
                      g.auto_start
                        ? "Auto-start: on — starts with the engine"
                        : "Auto-start: off"
                    }
                    aria-label="Toggle auto-start"
                    disabled={members.length === 0}
                    onClick={() => toggleAutoStart(g)}
                    className={
                      g.auto_start
                        ? "text-amber-400 hover:text-amber-300"
                        : "text-zinc-500 hover:text-zinc-300"
                    }
                  >
                    <Check
                      className={`w-3.5 h-3.5 ${g.auto_start ? "opacity-100" : "opacity-30"}`}
                    />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    title="Rename group"
                    aria-label="Rename group"
                    onClick={() => setEditing({ id: g.id, name: g.name })}
                    className="text-zinc-400 hover:text-white"
                  >
                    <Pencil className="w-3.5 h-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    title="Delete group"
                    aria-label="Delete group"
                    onClick={() => removeGroup(g)}
                    className="text-zinc-400 hover:text-rose-400"
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </Button>
                </div>
              </div>

              {/* Aggregated resource pills */}
              {members.length > 0 && (
                <div className="flex flex-wrap items-center gap-1.5 px-4 pb-2">
                  <span className="inline-flex items-center gap-1 rounded-lg bg-zinc-800/60 border border-zinc-700/50 px-2 py-0.5 text-[11px] font-mono text-emerald-300">
                    <Activity className="w-3 h-3 text-emerald-400" />
                    {formatBytes(agg.mem)}
                  </span>
                  <span className="inline-flex items-center gap-1 rounded-lg bg-zinc-800/60 border border-zinc-700/50 px-2 py-0.5 text-[11px] font-mono text-blue-300">
                    <Cpu className="w-3 h-3 text-blue-400" />
                    {agg.cpu.toFixed(1)}%
                  </span>
                  <span className="inline-flex items-center gap-1 rounded-lg bg-zinc-800/60 border border-zinc-700/50 px-2 py-0.5 text-[11px] font-mono text-amber-300">
                    <HardDrive className="w-3 h-3 text-amber-400" />
                    {formatBytes(agg.disk)}
                  </span>
                </div>
              )}

              {/* Expanded body: members, or a dashed drop zone while empty */}
              {isOpen && (
                <div className="mx-4 mb-3">
                  {members.length === 0 ? (
                    <div className="flex flex-col items-center justify-center gap-1 rounded-xl border border-dashed border-zinc-700/70 bg-zinc-950/30 px-3 py-6 text-center">
                      <FolderTree className="w-5 h-5 text-zinc-600" />
                      <p className="text-xs text-zinc-500">
                        Drop container cards here to add them
                      </p>
                    </div>
                  ) : (
                    <div className="rounded-xl border border-zinc-800/80 divide-y divide-zinc-800/60 max-h-48 overflow-y-auto">
                      {members.map((c) => {
                        const s = statsMap?.[c.id];
                        return (
                          <div
                            key={c.id}
                            draggable
                            onDragStart={(e) => {
                              e.dataTransfer.setData(CONTAINER_DRAG_TYPE, c.id);
                              e.dataTransfer.effectAllowed = "move";
                            }}
                            title="Drag into another group, or onto Isolated to remove"
                            className="flex cursor-grab items-center gap-2 px-3 py-1.5 text-xs active:cursor-grabbing"
                          >
                            <GripVertical className="w-3 h-3 shrink-0 text-zinc-700" />
                            <span
                              className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                                c.state === "running" ? "bg-emerald-400" : "bg-zinc-600"
                              }`}
                            />
                            <span className="text-zinc-200 truncate">
                              {c.name.replace(/^\//, "")}
                            </span>
                            {s && c.state === "running" && (
                              <span className="shrink-0 font-mono text-[10px] text-zinc-500">
                                {s.cpu_percent.toFixed(1)}% · {formatBytes(s.mem_usage)}
                              </span>
                            )}
                            <button
                              type="button"
                              title="Move to Isolated"
                              aria-label="Move to Isolated"
                              onClick={() => removeMember(c.id)}
                              className="ml-auto shrink-0 text-zinc-600 hover:text-rose-400"
                            >
                              <X className="w-3.5 h-3.5" />
                            </button>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}

        {/* Isolated — the pinned default folder holding every ungrouped card */}
        <div
          className={`rounded-2xl border bg-zinc-900/40 backdrop-blur-xl shadow-xl transition-all duration-200 ${
            dragOverId === ISOLATED_ID
              ? "border-emerald-500/70 bg-emerald-500/5 ring-1 ring-emerald-500/40"
              : "border-dashed border-zinc-800/80 hover:border-zinc-700/60"
          }`}
          title="Containers that are not in any group"
          {...groupDragProps(null)}
        >
          <div className="flex items-center gap-2 px-4 pt-3 pb-2">
            <button
              type="button"
              onClick={() => setExpanded((e) => ({ ...e, [ISOLATED_ID]: !e[ISOLATED_ID] }))}
              aria-expanded={!!expanded[ISOLATED_ID]}
              aria-label={expanded[ISOLATED_ID] ? "Collapse Isolated" : "Expand Isolated"}
              className="flex items-center gap-2 min-w-0 flex-1 text-left rounded-lg focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-indigo-500"
            >
              <ChevronDown
                className={`w-4 h-4 shrink-0 text-zinc-500 transition-transform duration-200 ${expanded[ISOLATED_ID] ? "rotate-0" : "-rotate-90"}`}
              />
              <FolderTree className="w-4 h-4 shrink-0 text-zinc-400" />
              <span className="truncate text-sm font-semibold text-zinc-300">Isolated</span>
              <span className="shrink-0 text-[11px] text-zinc-500">
                {isolated.filter((c) => c.state === "running").length}/{isolated.length} running
              </span>
            </button>
          </div>
          <div className="px-4 pb-3">
            <div className="rounded-xl border border-dashed border-zinc-700/70 bg-zinc-950/30 px-3 py-3 text-center">
              <p className="text-xs text-zinc-500">
                Drop a card here to take it out of its group
              </p>
              {expanded[ISOLATED_ID] && (
                <p className="mt-1 text-[11px] text-zinc-600">
                  {isolated.length === 0
                    ? "Every container is grouped"
                    : `${isolated.length} ungrouped card${isolated.length === 1 ? "" : "s"} listed below`}
                </p>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
