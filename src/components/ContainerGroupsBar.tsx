import React, { useCallback, useEffect, useState } from "react";
import {
  FolderTree,
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
} from "lucide-react";
import { Button } from "./ui/Button";
import { useToast } from "./ui/Toast";
import { useModal } from "./ui/ModalContext";
import { api } from "../lib/api";
import { formatBytes } from "../lib/utils";
import type { ContainerGroup, ContainerItem, ContainerStats } from "../types";

interface ContainerGroupsBarProps {
  /** Current container list — drives the member picker and drag-drop resolution. */
  containers: ContainerItem[];
  /** Live per-container stats from the page's stats tick — powers the RAM/CPU pills. */
  statsMap?: Record<string, ContainerStats>;
  /** Called after a bulk start/stop so the page re-fetches row states. */
  onChanged?: () => void;
}

const newId = () =>
  `g-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

/**
 * Docker Desktop-style container groups as collapsible cards:
 * expand/collapse per group, drag container rows onto a card to add them,
 * and live aggregated RAM / CPU / disk pills for the members.
 * Groups persist in the app config (not on the engine).
 */
export const ContainerGroupsBar: React.FC<ContainerGroupsBarProps> = ({
  containers,
  statsMap,
  onChanged,
}) => {
  const toast = useToast();
  const modal = useModal();
  const [groups, setGroups] = useState<ContainerGroup[]>([]);
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  const [sizes, setSizes] = useState<Record<string, number>>({});
  // Editor state: null = collapsed. `id` of a new group is generated on save.
  const [draft, setDraft] = useState<{
    id: string;
    name: string;
    containerIds: string[];
    autoStart: boolean;
    isNew: boolean;
  } | null>(null);
  const [saving, setSaving] = useState(false);

  const loadGroups = async () => {
    try {
      setGroups(await api.listContainerGroups());
    } catch (e) {
      console.error(e);
    }
  };

  const loadSizes = useCallback(async () => {
    try {
      setSizes(await api.getContainerSizes());
    } catch {
      /* engine offline — pills fall back to 0 */
    }
  }, []);

  useEffect(() => {
    loadGroups();
    loadSizes();
    const t = setInterval(loadSizes, 30_000);
    return () => clearInterval(t);
  }, [loadSizes]);

  const saveAll = async (next: ContainerGroup[]) => {
    await api.saveContainerGroups(next);
    setGroups(next);
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

  const openEditor = (g?: ContainerGroup) =>
    setDraft(
      g
        ? {
            id: g.id,
            name: g.name,
            containerIds: [...g.container_ids],
            autoStart: g.auto_start,
            isNew: false,
          }
        : { id: newId(), name: "", containerIds: [], autoStart: false, isNew: true }
    );

  const saveDraft = async () => {
    if (!draft) return;
    const name = draft.name.trim();
    if (!name) {
      toast.error("Name required", "Give the group a name.");
      return;
    }
    if (draft.containerIds.length === 0) {
      toast.error("No containers", "Pick at least one member container.");
      return;
    }
    setSaving(true);
    try {
      const next: ContainerGroup = {
        id: draft.id,
        name,
        container_ids: draft.containerIds,
        auto_start: draft.autoStart,
      };
      const merged = draft.isNew
        ? [...groups, next]
        : groups.map((x) => (x.id === next.id ? next : x));
      await saveAll(merged);
      setDraft(null);
      setExpanded((e) => ({ ...e, [next.id]: true }));
      toast.success(draft.isNew ? "Group created" : "Group updated", name);
    } catch (e) {
      toast.error("Could not save group", String(e));
    } finally {
      setSaving(false);
    }
  };

  /** Drop target: add the dragged container id to the group and persist. */
  const dropInto = async (g: ContainerGroup, e: React.DragEvent) => {
    e.preventDefault();
    setDragOverId(null);
    const id = e.dataTransfer.getData("text/dm-container-id");
    if (!id) return;
    const c = containers.find((x) => x.id === id || x.short_id === id);
    const resolvedId = c?.id ?? id;
    if (g.container_ids.includes(resolvedId)) {
      toast.info("Already a member", `${c?.name ?? resolvedId} is already in ${g.name}.`);
      return;
    }
    try {
      await saveAll(
        groups.map((x) =>
          x.id === g.id ? { ...x, container_ids: [...x.container_ids, resolvedId] } : x
        )
      );
      setExpanded((e2) => ({ ...e2, [g.id]: true }));
      toast.success("Added to group", `${c?.name ?? resolvedId} → ${g.name}`);
    } catch (err) {
      toast.error("Could not update group", String(err));
    }
  };

  const removeMember = async (g: ContainerGroup, memberId: string) => {
    try {
      await saveAll(
        groups.map((x) =>
          x.id === g.id
            ? { ...x, container_ids: x.container_ids.filter((id) => id !== memberId) }
            : x
        )
      );
    } catch (e) {
      toast.error("Could not update group", String(e));
    }
  };

  const resolveMember = (id: string) =>
    containers.find((x) => x.id === id || x.short_id === id || x.name === id);

  const runningOf = (g: ContainerGroup) => {
    let running = 0;
    for (const id of g.container_ids) {
      if (resolveMember(id)?.state === "running") running += 1;
    }
    return { running, total: g.container_ids.length };
  };

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

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-zinc-400">
          <FolderTree className="w-4 h-4" />
          <span className="text-xs font-semibold uppercase tracking-wider">Groups</span>
          <span className="text-[11px] text-zinc-600">
            drag a container onto a card to group it · start &amp; stop together
          </span>
        </div>
        <Button
          variant="ghost"
          size="sm"
          className="gap-1.5 text-xs"
          onClick={() => openEditor()}
        >
          <Plus className="w-3.5 h-3.5" />
          New group
        </Button>
      </div>

      {groups.length === 0 && !draft && (
        <div className="rounded-2xl border border-dashed border-zinc-800 bg-zinc-900/30 px-4 py-6 text-center">
          <FolderTree className="w-5 h-5 mx-auto mb-1.5 text-zinc-600" />
          <p className="text-xs text-zinc-500">
            No groups yet — create one, then drag containers onto its card.
          </p>
        </div>
      )}

      {groups.length > 0 && (
        <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-3">
          {groups.map((g) => {
            const isBusy = !!busy[g.id];
            const isOpen = !!expanded[g.id];
            const { running, total } = runningOf(g);
            const agg = aggregate(g);
            const isDropTarget = dragOverId === g.id;
            return (
              <div
                key={g.id}
                className={`rounded-2xl border bg-zinc-900/60 backdrop-blur-xl shadow-xl transition-all duration-200 ${
                  isDropTarget
                    ? "border-emerald-500/70 bg-emerald-500/5 ring-1 ring-emerald-500/40"
                    : "border-zinc-800/80 hover:border-zinc-700/60"
                }`}
                title={g.auto_start ? "Auto-starts with the engine" : "Manual start only"}
                onDragOver={(e) => {
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "copy";
                  if (dragOverId !== g.id) setDragOverId(g.id);
                }}
                onDragLeave={(e) => {
                  if (!e.currentTarget.contains(e.relatedTarget as Node)) {
                    setDragOverId((d) => (d === g.id ? null : d));
                  }
                }}
                onDrop={(e) => dropInto(g, e)}
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
                    <FolderTree className="w-4 h-4 shrink-0 text-indigo-400" />
                    <span className="truncate text-sm font-semibold text-white">{g.name}</span>
                    <span className="shrink-0 text-[11px] text-zinc-500">
                      {running}/{total} running
                    </span>
                  </button>
                  <div className="flex items-center gap-0.5 shrink-0">
                    <Button
                      variant="ghost"
                      size="icon"
                      title="Start group"
                      aria-label="Start group"
                      disabled={isBusy}
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
                      disabled={isBusy}
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
                      title="Edit group"
                      aria-label="Edit group"
                      onClick={() => openEditor(g)}
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

                {/* Expanded body: member list + drop hint */}
                {isOpen && (
                  <div className="mx-4 mb-3 rounded-xl border border-zinc-800/80 divide-y divide-zinc-800/60 max-h-48 overflow-y-auto">
                    {g.container_ids.length === 0 && (
                      <p className="px-3 py-2 text-xs text-zinc-500">
                        Empty — drag a container row here to add it.
                      </p>
                    )}
                    {g.container_ids.map((memberId) => {
                      const c = resolveMember(memberId);
                      const s = statsMap?.[c?.id ?? memberId];
                      return (
                        <div
                          key={memberId}
                          className="flex items-center gap-2 px-3 py-1.5 text-xs"
                        >
                          <span
                            className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                              c?.state === "running" ? "bg-emerald-400" : "bg-zinc-600"
                            }`}
                          />
                          <span className="text-zinc-200 truncate">
                            {c?.name?.replace(/^\//, "") ?? memberId.slice(0, 12)}
                          </span>
                          {s && c?.state === "running" && (
                            <span className="shrink-0 font-mono text-[10px] text-zinc-500">
                              {s.cpu_percent.toFixed(1)}% · {formatBytes(s.mem_usage)}
                            </span>
                          )}
                          {!c && (
                            <span className="shrink-0 text-[10px] text-zinc-600">
                              (not found)
                            </span>
                          )}
                          <button
                            type="button"
                            title="Remove from group"
                            aria-label="Remove from group"
                            onClick={() => removeMember(g, memberId)}
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
            );
          })}
        </div>
      )}

      {draft && (
        <div className="rounded-2xl border border-zinc-800 bg-zinc-900/70 p-4 space-y-3">
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-semibold text-white">
              {draft.isNew ? "New group" : `Edit group: ${draft.name}`}
            </span>
            <Button
              variant="ghost"
              size="icon"
              title="Close"
              aria-label="Close editor"
              onClick={() => setDraft(null)}
              disabled={saving}
            >
              <X className="w-4 h-4" />
            </Button>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <input
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              placeholder="Group name"
              className="flex-1 min-w-[160px] rounded-lg border border-zinc-700 bg-zinc-800/60 px-3 py-1.5 text-sm text-white placeholder:text-zinc-500"
            />
            <label className="flex items-center gap-1.5 text-xs text-zinc-400 cursor-pointer">
              <input
                type="checkbox"
                checked={draft.autoStart}
                onChange={(e) => setDraft({ ...draft, autoStart: e.target.checked })}
                className="rounded border-zinc-700"
              />
              Auto-start with engine
            </label>
            <Button
              variant="primary"
              size="sm"
              onClick={saveDraft}
              disabled={saving}
              isLoading={saving}
            >
              Save
            </Button>
          </div>

          <div className="max-h-48 overflow-y-auto rounded-lg border border-zinc-800/80 divide-y divide-zinc-800/60">
            {visibleContainers.length === 0 && (
              <p className="px-3 py-2 text-xs text-zinc-500">No containers yet.</p>
            )}
            {visibleContainers.map((c) => {
              const checked = draft.containerIds.includes(c.id);
              return (
                <label
                  key={c.id}
                  className="flex cursor-pointer items-center gap-2 px-3 py-1.5 text-xs hover:bg-zinc-800/40"
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        containerIds: e.target.checked
                          ? [...draft.containerIds, c.id]
                          : draft.containerIds.filter((id) => id !== c.id),
                      })
                    }
                    className="rounded border-zinc-700"
                  />
                  <span className="text-zinc-200">{c.name.replace(/^\//, "")}</span>
                  <span className="text-zinc-600 truncate">{c.image}</span>
                  <span
                    className={`ml-auto shrink-0 ${
                      c.state === "running" ? "text-emerald-400" : "text-zinc-600"
                    }`}
                  >
                    {c.state}
                  </span>
                </label>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
};
