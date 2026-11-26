import React, { useEffect, useMemo, useState } from "react";
import { AlertTriangle, FileCode2, FilePlus2, FolderInput, Layers, Loader2, RefreshCw, Wand2 } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { DialogShell, inputClass } from "./composeUi";
import { api } from "../../lib/api";
import {
  COMPOSE_TEMPLATES,
  defaultProjectDir,
  formatEnvFile,
  slugProjectName,
  templateSecrets,
  type ComposeTemplate,
} from "../../lib/composeTemplates";
import { cn } from "../../lib/utils";
import type { ContainerGroup } from "../../types";

type Mode = "template" | "group" | "import";

const MODES: { key: Mode; label: string }[] = [
  { key: "template", label: "Template" },
  { key: "group", label: "From container group" },
  { key: "import", label: "Import archive" },
];

const textareaClass =
  "min-h-[220px] w-full rounded-none border border-zinc-700 bg-zinc-900 p-2 font-mono text-[11px] leading-4 text-zinc-100 outline-none focus:border-blue-500";

const label = "mb-1.5 block text-[11px] uppercase tracking-wider text-zinc-500";

/**
 * Creates a compose project in one of three ways: from a built-in template
 * (generated secrets in `.env`), from a saved container group (a compose file
 * describing running containers), or by importing an export archive.
 * Creation only writes files and pins the project; starting it is `onCreated`'s
 * decision, taken from the "Up now" checkbox.
 */
export const NewProjectDialog: React.FC<{
  onClose: () => void;
  onCreated: (path: string, upNow: boolean) => void;
}> = ({ onClose, onCreated }) => {
  const toast = useToast();
  const [mode, setMode] = useState<Mode>("template");
  const [busy, setBusy] = useState(false);

  // Shared by the template and group modes: the project name drives the folder.
  const [name, setName] = useState("my-app");
  const [dir, setDir] = useState(defaultProjectDir("my-app"));
  const [dirTouched, setDirTouched] = useState(false);
  const [upNow, setUpNow] = useState(false);

  // Template mode.
  const [tplId, setTplId] = useState(COMPOSE_TEMPLATES[0].id);
  const [yaml, setYaml] = useState("");
  const [secrets, setSecrets] = useState<Record<string, string>>({});
  const tpl: ComposeTemplate = COMPOSE_TEMPLATES.find((t) => t.id === tplId) ?? COMPOSE_TEMPLATES[0];

  // Group mode.
  const [groups, setGroups] = useState<ContainerGroup[] | null>(null);
  const [groupId, setGroupId] = useState<string>("");

  // Import mode.
  const [archive, setArchive] = useState("");
  const [importDir, setImportDir] = useState("~/compose");

  const slug = slugProjectName(name);
  const nameChanged = slug !== name.trim();

  // Starting template: its YAML and freshly generated secrets.
  const pickTemplate = (t: ComposeTemplate) => {
    setTplId(t.id);
    setSecrets(templateSecrets(t));
    setYaml(t.yaml);
  };

  // Load the template on open, so the textarea is never empty.
  useEffect(() => {
    pickTemplate(COMPOSE_TEMPLATES[0]);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (mode !== "group" || groups !== null) return;
    api
      .listContainerGroups()
      .then((list) => {
        setGroups(list);
        if (list.length > 0) setGroupId(list[0].id);
      })
      .catch((e) => {
        setGroups([]);
        toast.error("Could not load container groups", String(e));
      });
  }, [mode, groups, toast]);

  // The folder follows the name until the user types a folder of their own.
  const onName = (v: string) => {
    setName(v);
    if (!dirTouched) setDir(defaultProjectDir(v));
  };

  const envText = useMemo(() => (tpl.secrets.length ? formatEnvFile(secrets) : ""), [tpl, secrets]);
  const group = groups?.find((g) => g.id === groupId) ?? null;

  const generateFromGroup = async () => {
    if (!group) return;
    setBusy(true);
    try {
      setYaml(await api.composeFromContainers(group.container_ids, slug));
    } catch (e) {
      toast.error("Could not describe the containers", String(e));
    } finally {
      setBusy(false);
    }
  };

  const canCreate = (() => {
    if (busy || !dir.trim()) return false;
    if (mode === "template") return yaml.trim().length > 0;
    if (mode === "group") return yaml.trim().length > 0;
    return false;
  })();

  const create = async () => {
    setBusy(true);
    try {
      // Only the template mode has generated secrets; a group's file takes no .env.
      const env = mode === "template" && envText ? envText : undefined;
      const path = await api.composeCreateProject(dir.trim(), yaml, env);
      toast.success("Project created", path);
      onCreated(path, upNow);
    } catch (e) {
      toast.error("Could not create the project", String(e));
    } finally {
      setBusy(false);
    }
  };

  const importArchive = async () => {
    setBusy(true);
    try {
      const path = await api.composeImport(archive.trim(), importDir.trim());
      toast.success("Project imported", path);
      onCreated(path, false);
    } catch (e) {
      toast.error("Import failed", String(e));
    } finally {
      setBusy(false);
    }
  };

  const footer = (
    <>
      <Button size="sm" variant="ghost" className="rounded-none" onClick={onClose}>
        Cancel
      </Button>
      {mode === "import" ? (
        <Button
          size="sm"
          variant="primary"
          className="rounded-none"
          disabled={busy || !archive.trim() || !importDir.trim()}
          onClick={() => void importArchive()}
        >
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FolderInput className="h-3.5 w-3.5" />}
          Import
        </Button>
      ) : (
        <Button size="sm" variant="primary" className="rounded-none" disabled={!canCreate} onClick={() => void create()}>
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FilePlus2 className="h-3.5 w-3.5" />}
          Create project
        </Button>
      )}
    </>
  );

  return (
    <DialogShell
      title="New compose project"
      subtitle="From a template, from running containers, or from an export archive"
      icon={<FilePlus2 className="h-4 w-4" />}
      onClose={onClose}
      className="max-w-3xl"
      footer={footer}
    >
      <div className="space-y-5">
        <div className="flex border border-zinc-700" role="tablist" aria-label="How to create the project">
          {MODES.map((m, i) => (
            <button
              key={m.key}
              type="button"
              role="tab"
              aria-selected={mode === m.key}
              onClick={() => setMode(m.key)}
              className={cn(
                "flex-1 px-3 py-1.5 text-xs",
                i > 0 && "border-l border-zinc-700",
                mode === m.key ? "bg-zinc-800 text-white" : "text-zinc-400 hover:text-zinc-200"
              )}
            >
              {m.label}
            </button>
          ))}
        </div>

        {mode !== "import" && (
          <div className="grid gap-3 sm:grid-cols-[1fr_1.6fr]">
            <div>
              <label className={label} htmlFor="new-project-name">
                Project name
              </label>
              <input
                id="new-project-name"
                value={name}
                onChange={(e) => onName(e.target.value)}
                className={cn(inputClass, "font-mono")}
              />
              {nameChanged && (
                <p className="mt-1 text-[11px] text-zinc-500">Saved as <code>{slug}</code> (compose name rules).</p>
              )}
            </div>
            <div>
              <label className={label} htmlFor="new-project-dir">
                Folder
              </label>
              <input
                id="new-project-dir"
                value={dir}
                onChange={(e) => {
                  setDirTouched(true);
                  setDir(e.target.value);
                }}
                className={cn(inputClass, "font-mono")}
              />
            </div>
          </div>
        )}

        {mode === "template" && (
          <div className="space-y-4">
            <div>
              <p className={label}>Template</p>
              <div className="grid gap-2 sm:grid-cols-2">
                {COMPOSE_TEMPLATES.map((t) => (
                  <button
                    key={t.id}
                    type="button"
                    onClick={() => pickTemplate(t)}
                    aria-pressed={tplId === t.id}
                    className={cn(
                      "flex items-start gap-2 rounded-none border px-3 py-2 text-left",
                      tplId === t.id
                        ? "border-blue-500/60 bg-blue-500/10"
                        : "border-zinc-800 bg-zinc-900/60 hover:border-zinc-700"
                    )}
                  >
                    <FileCode2 className="mt-0.5 h-3.5 w-3.5 shrink-0 text-zinc-400" aria-hidden="true" />
                    <span className="min-w-0">
                      <span className="block text-xs font-medium text-zinc-100">{t.title}</span>
                      <span className="block text-[11px] text-zinc-500">{t.description}</span>
                    </span>
                  </button>
                ))}
              </div>
              {tpl.hint && <p className="mt-2 text-[11px] text-zinc-400">{tpl.hint}</p>}
            </div>

            <div>
              <label className={label} htmlFor="new-project-yaml">
                compose.yaml (editable)
              </label>
              <textarea
                id="new-project-yaml"
                value={yaml}
                onChange={(e) => setYaml(e.target.value)}
                spellCheck={false}
                className={textareaClass}
              />
            </div>

            {tpl.secrets.length > 0 && (
              <div>
                <div className="mb-1.5 flex items-center justify-between">
                  <p className={label}>.env (generated secrets)</p>
                  <button
                    type="button"
                    onClick={() => setSecrets(templateSecrets(tpl))}
                    className="flex items-center gap-1 text-[11px] text-zinc-400 hover:text-zinc-200"
                  >
                    <RefreshCw className="h-3 w-3" aria-hidden="true" /> Regenerate
                  </button>
                </div>
                <pre className="max-h-32 overflow-auto border border-zinc-800 bg-zinc-950 p-2 font-mono text-[11px] text-zinc-300">
                  {envText}
                </pre>
                <p className="mt-1 text-[11px] text-zinc-500">
                  <Wand2 className="mr-1 inline h-3 w-3" aria-hidden="true" />
                  Random values from the browser's crypto source. They are written to <code>.env</code> in the folder.
                </p>
              </div>
            )}
          </div>
        )}

        {mode === "group" && (
          <div className="space-y-4">
            {groups === null ? (
              <p className="text-xs text-zinc-500">Loading container groups…</p>
            ) : groups.length === 0 ? (
              <p className="border border-dashed border-zinc-800 px-3 py-6 text-center text-xs text-zinc-500">
                No container groups yet. Create one on the Containers page first.
              </p>
            ) : (
              <>
                <div className="flex flex-col gap-2 sm:flex-row">
                  <label className="sr-only" htmlFor="new-project-group">
                    Container group
                  </label>
                  <select
                    id="new-project-group"
                    value={groupId}
                    onChange={(e) => {
                      setGroupId(e.target.value);
                      setYaml("");
                    }}
                    className={cn(inputClass, "sm:w-72")}
                  >
                    {groups.map((g) => (
                      <option key={g.id} value={g.id}>
                        {g.name} ({g.container_ids.length})
                      </option>
                    ))}
                  </select>
                  <Button
                    size="sm"
                    variant="outline"
                    className="rounded-none"
                    disabled={busy || !group || group.container_ids.length === 0}
                    onClick={() => void generateFromGroup()}
                  >
                    {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Layers className="h-3.5 w-3.5" />}
                    Describe containers
                  </Button>
                </div>
                <p className="flex items-start gap-2 border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
                  <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                  The original containers keep running. Stop and remove them before you run <code>Up</code>: their
                  container names and host ports would clash with the new project.
                </p>
                <div>
                  <label className={label} htmlFor="new-project-group-yaml">
                    compose.yaml (review and edit)
                  </label>
                  <textarea
                    id="new-project-group-yaml"
                    value={yaml}
                    onChange={(e) => setYaml(e.target.value)}
                    spellCheck={false}
                    placeholder="Press Describe containers to generate the file from the group."
                    className={textareaClass}
                  />
                </div>
              </>
            )}
          </div>
        )}

        {mode === "import" && (
          <div className="space-y-4">
            <div>
              <label className={label} htmlFor="new-project-archive">
                Archive path
              </label>
              <input
                id="new-project-archive"
                value={archive}
                onChange={(e) => setArchive(e.target.value)}
                placeholder="~/hsx2dock-backups/projects/my-app.tar.gz"
                className={cn(inputClass, "font-mono")}
              />
            </div>
            <div>
              <label className={label} htmlFor="new-project-import-dir">
                Destination folder
              </label>
              <input
                id="new-project-import-dir"
                value={importDir}
                onChange={(e) => setImportDir(e.target.value)}
                className={cn(inputClass, "font-mono")}
              />
              <p className="mt-1 text-[11px] text-zinc-500">
                The archive unpacks into <code>&lt;folder&gt;/&lt;project&gt;</code> and is saved to the list.
              </p>
            </div>
          </div>
        )}

        {mode !== "import" && (
          <label className="flex cursor-pointer items-center gap-2 text-xs text-zinc-300">
            <input
              type="checkbox"
              checked={upNow}
              onChange={(e) => setUpNow(e.target.checked)}
              className="h-3.5 w-3.5 rounded-none accent-emerald-500"
            />
            Start it now (docker compose up -d)
          </label>
        )}
      </div>
    </DialogShell>
  );
};
