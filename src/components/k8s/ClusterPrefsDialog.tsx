import React, { useState } from "react";
import { Loader2, X } from "lucide-react";
import { Button } from "../ui/Button";
import {
  CLUSTER_COLOR_PALETTE,
  MAX_TAGS,
  MAX_TAG_LENGTH,
  normalizeColor,
  normalizeTags,
} from "../../lib/k8sClusterList";

/** Editable copy of one cluster's prefs while the dialog is open. */
export interface ClusterPrefsDraft {
  name: string;
  autoStart: boolean;
  nodeCpuM: number;
  nodeMemoryMb: number;
  /** `#rrggbb`, or "" for the default accent. */
  color: string;
  tags: string[];
}

interface ClusterPrefsDialogProps {
  draft: ClusterPrefsDraft;
  /** Every tag already used by any cluster, offered as suggestions. */
  knownTags: string[];
  saving: boolean;
  onChange: (next: ClusterPrefsDraft) => void;
  onSave: () => void;
  onClose: () => void;
}

/** Cluster settings: colour, tags, auto-start and per-node caps. */
export const ClusterPrefsDialog: React.FC<ClusterPrefsDialogProps> = ({
  draft,
  knownTags,
  saving,
  onChange,
  onSave,
  onClose,
}) => {
  const [tagText, setTagText] = useState("");
  const tagsFull = draft.tags.length >= MAX_TAGS;
  const suggestions = knownTags.filter((t) => !draft.tags.includes(t));
  const color = normalizeColor(draft.color);

  const addTag = (raw: string) => {
    onChange({ ...draft, tags: normalizeTags([...draft.tags, raw]) });
    setTagText("");
  };

  const onTagInput = (value: string) => {
    if (!value.includes(",")) {
      setTagText(value);
      return;
    }
    // Pasted or typed "a,b": every comma-separated part before the last one
    // becomes a tag; the remainder stays in the input.
    const parts = value.split(",");
    const rest = parts.pop() ?? "";
    onChange({ ...draft, tags: normalizeTags([...draft.tags, ...parts]) });
    setTagText(rest);
  };

  const onTagKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      if (tagText.trim() && !tagsFull) addTag(tagText);
    } else if (e.key === "Backspace" && tagText === "" && draft.tags.length > 0) {
      onChange({ ...draft, tags: draft.tags.slice(0, -1) });
    }
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape" && !saving) {
      e.preventDefault();
      onClose();
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={() => !saving && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="cluster-prefs-title"
        onKeyDown={onKeyDown}
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-md max-h-[90vh] overflow-y-auto rounded-2xl border border-zinc-800 bg-zinc-900 shadow-2xl"
      >
        <div className="flex items-center justify-between px-5 py-4 border-b border-zinc-800">
          <div>
            <span id="cluster-prefs-title" className="text-sm font-semibold text-white">
              Cluster settings: {draft.name}
            </span>
            <p className="text-[11px] text-zinc-500 mt-0.5">
              Appearance, auto-start and per-node caps keep clusters from starving the host.
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={saving}
            aria-label="Close settings"
            title="Close"
            className="text-zinc-400 hover:text-white outline-none focus-visible:ring-2 focus-visible:ring-blue-500 rounded"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="px-5 py-4 space-y-5">
          <fieldset>
            <legend className="block text-xs font-medium text-zinc-400 mb-2">Colour</legend>
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                aria-label="No colour"
                aria-pressed={color === ""}
                title="Default accent"
                onClick={() => onChange({ ...draft, color: "" })}
                className={`w-6 h-6 rounded-none border bg-zinc-950 flex items-center justify-center text-zinc-500 outline-none focus-visible:ring-2 focus-visible:ring-blue-500 ${
                  color === "" ? "border-white ring-2 ring-white/30" : "border-zinc-700"
                }`}
              >
                <span className="block w-3.5 h-px bg-zinc-500 rotate-45" aria-hidden="true" />
              </button>
              {CLUSTER_COLOR_PALETTE.map((hex) => (
                <button
                  key={hex}
                  type="button"
                  aria-label={`Colour ${hex}`}
                  aria-pressed={color === hex}
                  title={hex}
                  onClick={() => onChange({ ...draft, color: hex })}
                  style={{ backgroundColor: hex }}
                  className={`w-6 h-6 rounded-none outline-none focus-visible:ring-2 focus-visible:ring-blue-500 ${
                    color === hex ? "ring-2 ring-white ring-offset-2 ring-offset-zinc-900" : ""
                  }`}
                />
              ))}
            </div>
          </fieldset>

          <div>
            <label htmlFor="cluster-tag-input" className="block text-xs font-medium text-zinc-400 mb-2">
              Tags{" "}
              <span className="text-zinc-600 font-normal">
                (Enter or comma to add, Backspace removes the last)
              </span>
            </label>
            <div className="flex flex-wrap items-center gap-1.5 min-h-9 px-2 py-1.5 rounded-xl bg-zinc-950 border border-zinc-800 focus-within:border-blue-500">
              {draft.tags.map((t) => (
                <span
                  key={t}
                  className="inline-flex items-center gap-1 pl-2 pr-1 py-0.5 rounded-md bg-zinc-800 border border-zinc-700/60 text-[11px] text-zinc-200"
                >
                  {t}
                  <button
                    type="button"
                    aria-label={`Remove tag ${t}`}
                    title={`Remove ${t}`}
                    disabled={saving}
                    onClick={() =>
                      onChange({ ...draft, tags: draft.tags.filter((x) => x !== t) })
                    }
                    className="p-0.5 rounded text-zinc-500 hover:text-white outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
                  >
                    <X className="w-3 h-3" />
                  </button>
                </span>
              ))}
              <input
                id="cluster-tag-input"
                type="text"
                list="cluster-tag-suggestions"
                value={tagText}
                maxLength={MAX_TAG_LENGTH + 1}
                onChange={(e) => onTagInput(e.target.value)}
                onKeyDown={onTagKeyDown}
                onBlur={() => {
                  if (tagText.trim() && !tagsFull) addTag(tagText);
                }}
                disabled={saving || tagsFull}
                placeholder={tagsFull ? `Up to ${MAX_TAGS} tags` : "add tag"}
                className="flex-1 min-w-[6rem] h-7 bg-transparent text-sm text-white placeholder-zinc-600 focus:outline-none disabled:opacity-60"
              />
              <datalist id="cluster-tag-suggestions">
                {suggestions.map((t) => (
                  <option key={t} value={t} />
                ))}
              </datalist>
            </div>
            <p className="text-[11px] text-zinc-600 mt-1">
              Up to {MAX_TAGS} tags, {MAX_TAG_LENGTH} characters each.
            </p>
          </div>

          <label className="flex items-center justify-between gap-3 text-sm text-zinc-200">
            <span>
              Auto-start with engine
              <span className="block text-[11px] text-zinc-500">
                Honoured when the master Auto-start switch on the page is on.
              </span>
            </span>
            <input
              type="checkbox"
              checked={draft.autoStart}
              onChange={(e) => onChange({ ...draft, autoStart: e.target.checked })}
              className="rounded border-zinc-700 w-4 h-4"
            />
          </label>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label htmlFor="cluster-cpu-cap" className="block text-xs text-zinc-400 mb-1">
                CPU per node (cores, 0 = unlimited)
              </label>
              <input
                id="cluster-cpu-cap"
                type="number"
                min={0}
                step={0.5}
                value={draft.nodeCpuM / 1000}
                onChange={(e) =>
                  onChange({
                    ...draft,
                    nodeCpuM: Math.max(0, Math.round((Number(e.target.value) || 0) * 1000)),
                  })
                }
                className="h-9 w-full px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-indigo-500"
              />
            </div>
            <div>
              <label htmlFor="cluster-mem-cap" className="block text-xs text-zinc-400 mb-1">
                Memory per node (MB, 0 = unlimited)
              </label>
              <input
                id="cluster-mem-cap"
                type="number"
                min={0}
                value={draft.nodeMemoryMb}
                onChange={(e) =>
                  onChange({
                    ...draft,
                    nodeMemoryMb: Math.max(0, Number(e.target.value) || 0),
                  })
                }
                className="h-9 w-full px-3 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-indigo-500"
              />
            </div>
          </div>
          <p className="text-[11px] text-zinc-600">
            Caps apply immediately (running nodes update via docker update) and re-apply on every
            start. The auto-start master switch and stagger delay live in Settings → Kubernetes.
          </p>
        </div>

        <div className="flex justify-end gap-2 px-5 py-4 border-t border-zinc-800">
          <Button variant="ghost" size="sm" onClick={onClose} disabled={saving}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" onClick={onSave} disabled={saving} className="gap-2">
            {saving && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            Save
          </Button>
        </div>
      </div>
    </div>
  );
};
