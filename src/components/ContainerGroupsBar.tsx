import React, { useEffect, useState } from "react";
import { FolderTree, Play, Square, Pencil, Trash2, Plus, Check, X } from "lucide-react";
import { Button } from "./ui/Button";
import { useToast } from "./ui/Toast";
import { useModal } from "./ui/ModalContext";
import { api } from "../lib/api";
import type { ContainerGroup, ContainerItem } from "../types";

interface ContainerGroupsBarProps {
  /** Current container list — drives the member picker. */
  containers: ContainerItem[];
  /** Called after a bulk start/stop so the page re-fetches row states. */
  onChanged?: () => void;
}

const newId = () =>
  `g-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;

/**
 * Docker Desktop-style container groups: named sets that start/stop together,
 * with an optional auto-start flag applied when the engine comes up.
 * Groups persist in the app config (not on the engine).
 */
export const ContainerGroupsBar: React.FC<ContainerGroupsBarProps> = ({
  containers,
  onChanged,
}) => {
  const toast = useToast();
  const modal = useModal();
  const [groups, setGroups] = useState<ContainerGroup[]>([]);
  const [busy, setBusy] = useState<Record<string, boolean>>({});
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

  useEffect(() => {
    loadGroups();
  }, []);

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
      toast.error("Could not save group", String(e));
      await loadGroups();
    }
  };

  const runBulk = async (g: ContainerGroup, action: "start" | "stop") => {
    try {
      setBusy((b) => ({ ...b, [g.id]: true }));
      const summary =
        action === "start"
          ? await api.startContainerGroup(g.id)
          : await api.stopContainerGroup(g.id);
      toast.success(`Group ${action}ed`, `${g.name}: ${summary}`);
      onChanged?.();
    } catch (e) {
      toast.error(`Group ${action} failed`, String(e));
    } finally {
      setBusy((b) => ({ ...b, [g.id]: false }));
    }
  };

  const removeGroup = async (g: ContainerGroup) => {
    const ok = await modal.showConfirm(
      `Delete group '${g.name}'? Containers are not affected.`,
      "Delete Group",
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
      toast.success(draft.isNew ? "Group created" : "Group updated", name);
    } catch (e) {
      toast.error("Could not save group", String(e));
    } finally {
      setSaving(false);
    }
  };

  const runningOf = (g: ContainerGroup) => {
    let running = 0;
    for (const id of g.container_ids) {
      const c = containers.find(
        (x) => x.id === id || x.short_id === id || x.name === id
      );
      if (c?.state === "running") running += 1;
    }
    return `${running}/${g.container_ids.length} running`;
  };

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-2 text-zinc-400">
          <FolderTree className="w-4 h-4" />
          <span className="text-xs font-semibold uppercase tracking-wider">Groups</span>
          <span className="text-[11px] text-zinc-600">
            start &amp; stop together · auto-start with the engine
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

      {groups.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {groups.map((g) => {
            const isBusy = !!busy[g.id];
            return (
              <div
                key={g.id}
                className="flex flex-wrap items-center gap-2 rounded-xl border border-zinc-800 bg-zinc-900/60 px-3 py-2"
                title={g.auto_start ? "Auto-starts with the engine" : "Manual start only"}
              >
                <div className="min-w-0">
                  <span className="text-sm font-medium text-white">{g.name}</span>
                  <span className="ml-2 text-[11px] text-zinc-500">{runningOf(g)}</span>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  title="Start group"
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
                  onClick={() => openEditor(g)}
                  className="text-zinc-400 hover:text-white"
                >
                  <Pencil className="w-3.5 h-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  title="Delete group"
                  onClick={() => removeGroup(g)}
                  className="text-zinc-400 hover:text-rose-400"
                >
                  <Trash2 className="w-3.5 h-3.5" />
                </Button>
              </div>
            );
          })}
        </div>
      )}

      {draft && (
        <div className="rounded-xl border border-zinc-800 bg-zinc-900/70 p-4 space-y-3">
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-semibold text-white">
              {draft.isNew ? "New group" : `Edit group: ${draft.name}`}
            </span>
            <Button
              variant="ghost"
              size="icon"
              title="Close"
              onClick={() => setDraft(null)}
              disabled={saving}
            >
              <X className="w-4 h-4" />
            </Button>
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
            <label className="space-y-1">
              <span className="text-[11px] text-zinc-500">Name</span>
              <input
                value={draft.name}
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                placeholder="My stack"
                autoFocus
                className="h-8 w-full rounded-md border border-zinc-800 bg-zinc-950/60 px-2 text-xs text-white outline-none focus:border-zinc-600 placeholder:text-zinc-700"
              />
            </label>
            <label className="flex items-end gap-2 pb-1">
              <input
                type="checkbox"
                checked={draft.autoStart}
                onChange={(e) => setDraft({ ...draft, autoStart: e.target.checked })}
                className="rounded border-zinc-700"
              />
              <span className="text-xs text-zinc-300">
                Auto-start when the engine starts
              </span>
            </label>
            <div className="flex items-end justify-end gap-2">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setDraft(null)}
                disabled={saving}
              >
                Cancel
              </Button>
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
          </div>

          <div className="max-h-48 overflow-y-auto rounded-lg border border-zinc-800/80 divide-y divide-zinc-800/60">
            {containers.length === 0 && (
              <p className="px-3 py-2 text-xs text-zinc-500">No containers yet.</p>
            )}
            {containers.map((c) => {
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
