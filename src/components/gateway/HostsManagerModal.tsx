import React, { useState, useEffect } from "react";
import {
  FileText,
  CheckCircle2,
  AlertTriangle,
  Save,
  Trash2,
  Plus,
  RefreshCw,
  Info,
  Code2,
  Table,
} from "lucide-react";
import { api } from "../../lib/api";
import { useToast } from "../ui/Toast";
import { Button } from "../ui/Button";
import type { HostsFileModel, HostsEntryItem } from "../../types";

export const HostsManagerModal: React.FC<{
  isOpen: boolean;
  onClose: () => void;
  onUpdated?: () => void;
}> = ({ isOpen, onClose, onUpdated }) => {
  const toast = useToast();
  const [model, setModel] = useState<HostsFileModel | null>(null);
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState<"visual" | "raw">("visual");
  const [rawText, setRawText] = useState("");
  const [saving, setSaving] = useState(false);
  const [cleaning, setCleaning] = useState(false);

  // New Entry Draft for Visual mode
  const [newIp, setNewIp] = useState("127.0.0.1");
  const [newHostnames, setNewHostnames] = useState("");
  const [newComment, setNewComment] = useState("");

  const loadData = async () => {
    try {
      setLoading(true);
      const res = await api.getHostsFileModel();
      setModel(res);
      setRawText(res.raw_content);
    } catch (e) {
      toast.error("Failed to load /etc/hosts", String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (isOpen) {
      loadData();
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const handleSaveRaw = async () => {
    try {
      setSaving(true);
      await api.saveRawHosts(rawText);
      toast.success("Hosts File Saved", "/etc/hosts updated successfully.");
      await loadData();
      onUpdated?.();
    } catch (e) {
      toast.error("Save Failed", String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleClearAppEntries = async () => {
    try {
      setCleaning(true);
      await api.clearAppHosts();
      toast.success("App Entries Cleaned", "Removed all Hsx2Dock managed domain entries from /etc/hosts.");
      await loadData();
      onUpdated?.();
    } catch (e) {
      toast.error("Failed to clean entries", String(e));
    } finally {
      setCleaning(false);
    }
  };

  const handleAddVisualEntry = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newIp.trim() || !newHostnames.trim()) {
      toast.error("Validation Error", "IP and at least one hostname are required.");
      return;
    }

    const hostList = newHostnames.trim().split(/\s+/).join(" ");
    const commentPart = newComment.trim() ? ` # ${newComment.trim()}` : "";
    const lineToAdd = `${newIp.trim()}\t${hostList}${commentPart}\n`;

    const updatedText = rawText.endsWith("\n") ? `${rawText}${lineToAdd}` : `${rawText}\n${lineToAdd}`;

    try {
      setSaving(true);
      await api.saveRawHosts(updatedText);
      toast.success("Entry Added", `Added ${hostList} → ${newIp.trim()}`);
      setNewHostnames("");
      setNewComment("");
      await loadData();
      onUpdated?.();
    } catch (err) {
      toast.error("Failed to add entry", String(err));
    } finally {
      setSaving(false);
    }
  };

  /**
   * The entry's own line in the file as loaded. Entries carry their 1-based
   * line number (`line-N`); matching by text hit the first identical line
   * (e.g. a duplicate entry) and edited unsaved raw-tab text along the way.
   */
  const locateLine = (index: number) => {
    if (!model) return null;
    const target = model.entries[index];
    if (!target) return null;
    if (rawText !== model.raw_content) {
      toast.info("Unsaved raw edits", "Save or reload the raw text before changing single entries.");
      return null;
    }
    const lines = model.raw_content.split("\n");
    const n = Number(target.id.replace(/^line-/, "")) - 1;
    if (!Number.isInteger(n) || lines[n] === undefined || lines[n] !== target.raw_line) {
      toast.error("/etc/hosts changed", "Reload and try again.");
      return null;
    }
    return { lines, targetIdx: n, target };
  };

  const handleToggleEntry = async (index: number) => {
    const found = locateLine(index);
    if (!found) return;
    const { lines, targetIdx } = found;

    if (lines[targetIdx].trim().startsWith("#")) {
      lines[targetIdx] = lines[targetIdx].replace(/^#\s*/, "");
    } else {
      lines[targetIdx] = `# ${lines[targetIdx]}`;
    }

    const updated = lines.join("\n");
    try {
      setSaving(true);
      await api.saveRawHosts(updated);
      toast.info("Status Toggled", `Line ${targetIdx + 1} updated.`);
      await loadData();
      onUpdated?.();
    } catch (err) {
      toast.error("Toggle Failed", String(err));
    } finally {
      setSaving(false);
    }
  };

  const handleDeleteEntry = async (index: number) => {
    if (!model) return;
    const target = model.entries[index];
    if (!target) return;

    if (!target.is_managed_by_app) {
      toast.info("Protected System Entry", "System host entries cannot be deleted directly. You can disable them by clicking the status toggle.");
      return;
    }

    const found = locateLine(index);
    if (!found) return;
    const { lines, targetIdx } = found;

    lines.splice(targetIdx, 1);
    const updated = lines.join("\n");
    try {
      setSaving(true);
      await api.saveRawHosts(updated);
      toast.success("Entry Deleted", `Removed app entry from /etc/hosts.`);
      await loadData();
      onUpdated?.();
    } catch (err) {
      toast.error("Delete Failed", String(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/75 backdrop-blur-md">
      <div className="w-full max-w-4xl max-h-[90vh] flex flex-col bg-zinc-900 border border-zinc-800 rounded-2xl shadow-2xl overflow-hidden text-zinc-100">
        {/* Modal Header */}
        <div className="p-4 border-b border-zinc-800 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-xl bg-sky-500/10 border border-sky-500/20 text-sky-400">
              <FileText className="w-5 h-5" />
            </div>
            <div>
              <h2 className="text-sm font-semibold flex items-center gap-2">
                /etc/hosts System Local DNS Manager
                {model?.tampered && (
                  <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-amber-500/15 text-amber-300 border border-amber-500/30">
                    <AlertTriangle className="w-3 h-3 text-amber-400" />
                    External Changes Detected
                  </span>
                )}
                {model?.managed_block_present && !model?.tampered && (
                  <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[11px] font-medium bg-emerald-500/15 text-emerald-300 border border-emerald-500/30">
                    <CheckCircle2 className="w-3 h-3 text-emerald-400" />
                    App Managed Block Synced
                  </span>
                )}
              </h2>
              <p className="text-xs text-zinc-400">
                Inspect, edit, validate, and manage local domain bindings securely.
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <div className="flex items-center bg-zinc-950 p-1 rounded-lg border border-zinc-800">
              <button
                type="button"
                onClick={() => setActiveTab("visual")}
                className={`px-3 py-1 text-xs rounded-md font-medium transition flex items-center gap-1.5 ${
                  activeTab === "visual"
                    ? "bg-zinc-800 text-white shadow-sm"
                    : "text-zinc-400 hover:text-zinc-200"
                }`}
              >
                <Table className="w-3.5 h-3.5" />
                Visual Cards
              </button>
              <button
                type="button"
                onClick={() => setActiveTab("raw")}
                className={`px-3 py-1 text-xs rounded-md font-medium transition flex items-center gap-1.5 ${
                  activeTab === "raw"
                    ? "bg-zinc-800 text-white shadow-sm"
                    : "text-zinc-400 hover:text-zinc-200"
                }`}
              >
                <Code2 className="w-3.5 h-3.5" />
                Raw Editor
              </button>
            </div>

            <Button
              variant="outline"
              size="sm"
              onClick={loadData}
              disabled={loading}
              className="text-xs h-8 px-2.5 border-zinc-800 bg-zinc-800/40 text-zinc-300"
            >
              <RefreshCw className={`w-3.5 h-3.5 ${loading ? "animate-spin" : ""}`} />
            </Button>

            <button
              type="button"
              onClick={onClose}
              className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 text-sm ml-1"
            >
              &times;
            </button>
          </div>
        </div>

        {/* Modal Body */}
        <div className="flex-1 overflow-y-auto p-5 space-y-4">
          {activeTab === "visual" ? (
            <div className="space-y-4">
              {/* Add New Entry Form */}
              <form
                onSubmit={handleAddVisualEntry}
                className="p-3.5 rounded-xl border border-zinc-800/80 bg-zinc-950/60 grid grid-cols-1 md:grid-cols-12 gap-3 items-end"
              >
                <div className="md:col-span-3">
                  <label className="block text-[11px] font-medium text-zinc-400 mb-1">
                    IP Address
                  </label>
                  <input
                    type="text"
                    required
                    value={newIp}
                    onChange={(e) => setNewIp(e.target.value)}
                    placeholder="127.0.0.1 or ::1"
                    className="w-full px-2.5 py-1.5 text-xs font-mono bg-zinc-900 border border-zinc-800 rounded-lg text-zinc-100 focus:outline-none focus:border-sky-500"
                  />
                </div>
                <div className="md:col-span-5">
                  <label className="block text-[11px] font-medium text-zinc-400 mb-1">
                    Domain / Hostnames (separated by space)
                  </label>
                  <input
                    type="text"
                    required
                    value={newHostnames}
                    onChange={(e) => setNewHostnames(e.target.value)}
                    placeholder="e.g. dev.local myproject.test"
                    className="w-full px-2.5 py-1.5 text-xs bg-zinc-900 border border-zinc-800 rounded-lg text-zinc-100 focus:outline-none focus:border-sky-500"
                  />
                </div>
                <div className="md:col-span-3">
                  <label className="block text-[11px] font-medium text-zinc-400 mb-1">
                    Comment (Optional)
                  </label>
                  <input
                    type="text"
                    value={newComment}
                    onChange={(e) => setNewComment(e.target.value)}
                    placeholder="Local service"
                    className="w-full px-2.5 py-1.5 text-xs bg-zinc-900 border border-zinc-800 rounded-lg text-zinc-100 focus:outline-none focus:border-sky-500"
                  />
                </div>
                <div className="md:col-span-1">
                  <Button
                    type="submit"
                    size="sm"
                    disabled={saving}
                    className="w-full text-xs h-8 bg-sky-600 hover:bg-sky-500 text-white font-medium"
                  >
                    <Plus className="w-3.5 h-3.5" />
                  </Button>
                </div>
              </form>

              {/* Entries List */}
              <div className="space-y-2">
                <div className="flex items-center justify-between text-xs text-zinc-400 px-1">
                  <span>Parsed Host Entries ({model?.entries.length || 0})</span>
                  {model?.managed_block_present && (
                    <button
                      type="button"
                      onClick={handleClearAppEntries}
                      disabled={cleaning}
                      className="text-rose-400 hover:text-rose-300 transition text-[11px] flex items-center gap-1"
                    >
                      <Trash2 className="w-3 h-3" />
                      Clear Hsx2Dock Managed Block
                    </button>
                  )}
                </div>

                <div className="rounded-xl border border-zinc-800/80 bg-zinc-950/40 divide-y divide-zinc-800/60 overflow-hidden">
                  {model?.entries.map((item: HostsEntryItem, idx: number) => (
                    <div
                      key={item.id}
                      className={`p-3 flex items-center justify-between gap-3 text-xs transition ${
                        item.is_managed_by_app
                          ? "bg-violet-950/20 border-l-2 border-l-violet-500"
                          : !item.is_syntax_valid
                          ? "bg-rose-950/20 border-l-2 border-l-rose-500"
                          : "hover:bg-zinc-900/40"
                      }`}
                    >
                      <div className="flex items-center gap-3 min-w-0">
                        <button
                          type="button"
                          onClick={() => handleToggleEntry(idx)}
                          className={`w-4 h-4 rounded-full border transition flex items-center justify-center shrink-0 ${
                            item.is_enabled
                              ? "bg-emerald-500/20 border-emerald-500 text-emerald-400"
                              : "border-zinc-700 bg-zinc-800 text-transparent"
                          }`}
                          title={item.is_enabled ? "Enabled — click to disable (comment out)" : "Disabled — click to enable"}
                        >
                          <span className={`w-1.5 h-1.5 rounded-full ${item.is_enabled ? "bg-emerald-400" : "bg-transparent"}`} />
                        </button>

                        <span className="font-mono text-zinc-300 shrink-0 w-28 truncate">
                          {item.ip}
                        </span>

                        <div className="flex items-center gap-1.5 flex-wrap min-w-0">
                          {item.hosts.map((h: string) => (
                            <span
                              key={h}
                              className={`px-2 py-0.5 rounded-md font-mono text-[11px] font-medium border ${
                                item.is_managed_by_app
                                  ? "bg-violet-500/15 border-violet-500/30 text-violet-300"
                                  : "bg-zinc-800 border-zinc-700 text-zinc-200"
                              }`}
                            >
                              {h}
                            </span>
                          ))}
                        </div>

                        {item.comment && (
                          <span className="text-zinc-500 text-[11px] italic truncate">
                            #{item.comment}
                          </span>
                        )}

                        {!item.is_syntax_valid && (
                          <span className="text-rose-400 text-[11px] flex items-center gap-1 font-medium">
                            <AlertTriangle className="w-3 h-3 shrink-0" />
                            {item.syntax_error}
                          </span>
                        )}
                      </div>

                      <div className="flex items-center gap-1.5 shrink-0">
                        {item.is_managed_by_app ? (
                          <button
                            type="button"
                            onClick={() => handleDeleteEntry(idx)}
                            className="p-1.5 text-zinc-500 hover:text-rose-400 hover:bg-rose-500/10 rounded transition"
                            title="Delete App Entry"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        ) : null}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          ) : (
            <div className="space-y-3">
              <div className="flex items-center justify-between text-xs text-zinc-400">
                <span className="flex items-center gap-1.5">
                  <Code2 className="w-4 h-4 text-sky-400" />
                  Direct /etc/hosts Text Editor (Syntax Protected)
                </span>
                <span className="text-zinc-500">Auto syntax validation on save</span>
              </div>

              <textarea
                value={rawText}
                onChange={(e) => setRawText(e.target.value)}
                rows={16}
                className="w-full p-3 font-mono text-xs bg-zinc-950 border border-zinc-800 rounded-xl text-zinc-200 focus:outline-none focus:border-sky-500 leading-relaxed resize-y"
              />

              <div className="p-3 rounded-lg border border-zinc-800 bg-zinc-950/60 flex items-center justify-between text-xs">
                <span className="text-zinc-500">
                  Total lines: {rawText.split("\n").length} · Requires root privilege confirmation (pkexec)
                </span>
                <Button
                  size="sm"
                  onClick={handleSaveRaw}
                  disabled={saving}
                  className="text-xs gap-1.5 bg-sky-600 hover:bg-sky-500 text-white font-medium"
                >
                  <Save className="w-3.5 h-3.5" />
                  {saving ? "Saving..." : "Save Changes"}
                </Button>
              </div>
            </div>
          )}
        </div>

        {/* Modal Footer */}
        <div className="p-4 border-t border-zinc-800/80 bg-zinc-950/60 flex items-center justify-between text-xs text-zinc-500">
          <div className="flex items-center gap-2">
            <Info className="w-4 h-4 text-zinc-400" />
            <span>Changes affect local hostname resolution immediately across all browsers and services.</span>
          </div>

          <Button
            variant="outline"
            size="sm"
            onClick={onClose}
            className="text-xs border-zinc-800 text-zinc-300 hover:bg-zinc-800"
          >
            Close
          </Button>
        </div>
      </div>
    </div>
  );
};
