import React, { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { X, Tag } from "lucide-react";
import { Button } from "../ui/Button";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";
import type { ImageItem } from "../../types";

interface TagImageModalProps {
  /** Image to tag; null closes the modal. */
  image: ImageItem | null;
  onClose: () => void;
  onSuccess?: () => void;
}

/** Splits "registry.example.com/ns/name:tag" into repo + tag. */
const splitRef = (ref: string): { repo: string; tag: string } => {
  if (ref.startsWith("sha256:")) return { repo: "", tag: "latest" };
  // Docker's rule: the last ":" opens a tag only when it appears after the
  // last "/" — before it, the colon is part of a registry port ("host:5000").
  const lastColon = ref.lastIndexOf(":");
  const lastSlash = ref.lastIndexOf("/");
  if (lastColon > lastSlash && lastColon !== -1) {
    return { repo: ref.slice(0, lastColon), tag: ref.slice(lastColon + 1) };
  }
  return { repo: ref, tag: "latest" };
};

/**
 * Adds a `repo:tag` to an existing local image (`docker tag`) — the same
 * workflow Docker Desktop exposes as "Add tag" in the image row menu.
 */
export const TagImageModal: React.FC<TagImageModalProps> = ({
  image,
  onClose,
  onSuccess,
}) => {
  const toast = useToast();
  const [repo, setRepo] = useState("");
  const [tag, setTag] = useState("latest");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!image) return;
    const primary = image.tags.find((t) => t && !t.startsWith("sha256:"));
    if (primary) {
      const s = splitRef(primary);
      setRepo(s.repo);
      setTag(s.tag);
    } else {
      setRepo(image.short_id?.slice(0, 12) ?? "");
      setTag("latest");
    }
  }, [image]);

  const trimmedRepo = repo.trim().replace(/:+$/, "");
  const trimmedTag = tag.trim();
  const valid =
    !!image &&
    trimmedRepo.length > 0 &&
    trimmedTag.length > 0 &&
    !/\s/.test(trimmedRepo) &&
    !/\s/.test(trimmedTag);

  const handleSave = async () => {
    if (!image || !valid || saving) return;
    try {
      setSaving(true);
      await api.tagImage(image.id, trimmedRepo, trimmedTag);
      toast.success("Tag added", `${trimmedRepo}:${trimmedTag}`);
      onSuccess?.();
      onClose();
    } catch (e) {
      toast.error("Could not tag image", String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <AnimatePresence>
      {image && (
        <>
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="fixed inset-0 bg-black/70 backdrop-blur-md z-50"
            onClick={() => !saving && onClose()}
          />
          <motion.div
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.95 }}
            className="fixed inset-0 z-50 flex items-center justify-center p-6 pointer-events-none"
          >
            <div className="pointer-events-auto w-full max-w-md bg-zinc-950 border border-zinc-800 rounded-2xl shadow-2xl p-6 space-y-5">
              <div className="flex items-center justify-between">
                <h3 className="font-semibold text-lg text-white flex items-center gap-2">
                  <Tag className="w-4 h-4 text-amber-400" />
                  Add tag
                </h3>
                <Button variant="ghost" size="icon" onClick={onClose} disabled={saving} title="Close">
                  <X className="w-5 h-5" />
                </Button>
              </div>

              <p className="text-xs text-zinc-500">
                Points another name at{" "}
                <span className="font-mono text-zinc-300">
                  {(image.tags.find((t) => t && !t.startsWith("sha256:")) ??
                    image.id.slice(0, 12)) as string}
                </span>
                . No new layers are created.
              </p>

              <div className="flex items-end gap-2">
                <div className="flex-1 min-w-0">
                  <label className="block text-xs text-zinc-400 mb-1.5">Repository</label>
                  <input
                    value={repo}
                    onChange={(e) => setRepo(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && handleSave()}
                    placeholder="my-app"
                    autoFocus
                    className="h-9 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-3 font-mono text-sm text-white outline-none focus:border-amber-500/60"
                  />
                </div>
                <span className="text-zinc-500 pb-2.5 font-mono">:</span>
                <div className="w-32">
                  <label className="block text-xs text-zinc-400 mb-1.5">Tag</label>
                  <input
                    value={tag}
                    onChange={(e) => setTag(e.target.value)}
                    onKeyDown={(e) => e.key === "Enter" && handleSave()}
                    placeholder="latest"
                    className="h-9 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-3 font-mono text-sm text-white outline-none focus:border-amber-500/60"
                  />
                </div>
              </div>

              <div className="flex justify-end gap-2">
                <Button variant="ghost" size="sm" onClick={onClose} disabled={saving}>
                  Cancel
                </Button>
                <Button
                  variant="primary"
                  size="sm"
                  onClick={handleSave}
                  disabled={!valid || saving}
                  isLoading={saving}
                >
                  Add tag
                </Button>
              </div>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
};
