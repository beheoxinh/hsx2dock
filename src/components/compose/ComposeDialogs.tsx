import React, { useEffect, useMemo, useState } from "react";
import { FilePlus2, FolderSearch, Globe, Loader2, Play, Power, Scaling, Search } from "lucide-react";
import { Button } from "../ui/Button";
import { Switch } from "../ui/Switch";
import { useToast } from "../ui/Toast";
import { DialogShell, inputClass, openUrl } from "./composeUi";
import { api } from "../../lib/api";
import { setGatewayRoutes } from "../../lib/containerStore";
import { defaultDomain, publishedPorts, type ComposeServiceRow } from "../../lib/composeProject";
import { cn } from "../../lib/utils";
import type { ComposeDownOptions, ComposeProjectInfo, ComposeUpOptions, GatewayRoute } from "../../types";

// ---------------------------------------------------------------------------
// Add / import project
// ---------------------------------------------------------------------------

export const AddProjectDialog: React.FC<{
  known: string[];
  onClose: () => void;
  onAdded: (path: string) => void;
}> = ({ known, onClose, onAdded }) => {
  const toast = useToast();
  const [path, setPath] = useState("");
  const [scanDir, setScanDir] = useState("~");
  const [depth, setDepth] = useState(3);
  const [found, setFound] = useState<string[] | null>(null);
  const [scanning, setScanning] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);

  const add = async (target: string) => {
    if (!target.trim()) return;
    setAdding(target);
    try {
      const resolved = await api.composePinProject(target.trim());
      toast.success("Project added", resolved);
      onAdded(resolved);
    } catch (e) {
      toast.error("Cannot add project", String(e));
    } finally {
      setAdding(null);
    }
  };

  const scan = async () => {
    setScanning(true);
    try {
      setFound(await api.composeScanDir(scanDir, depth));
    } catch (e) {
      toast.error("Scan failed", String(e));
      setFound(null);
    } finally {
      setScanning(false);
    }
  };

  return (
    <DialogShell
      title="Add compose project"
      subtitle="Save a compose file to the list, or find the ones in a folder"
      icon={<FilePlus2 className="h-4 w-4" />}
      onClose={onClose}
      className="max-w-2xl"
    >
      <section className="space-y-2">
        <label className="text-[11px] uppercase tracking-wider text-zinc-500" htmlFor="compose-add-path">
          Compose file or project folder
        </label>
        <div className="flex gap-2">
          <input
            id="compose-add-path"
            autoFocus
            value={path}
            onChange={(e) => setPath(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void add(path)}
            placeholder="~/projects/shop  or  /srv/app/docker-compose.yml"
            className={cn(inputClass, "font-mono")}
          />
          <Button size="sm" variant="primary" onClick={() => void add(path)} disabled={!path.trim() || adding !== null}>
            {adding === path ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FilePlus2 className="h-3.5 w-3.5" />}
            Add
          </Button>
        </div>
        <p className="text-[11px] text-zinc-500">
          A folder is resolved to its compose.yaml / docker-compose.yml. The file must pass{" "}
          <code className="text-zinc-400">docker compose config</code>.
        </p>
      </section>

      <section className="mt-5 space-y-2 border-t border-zinc-800 pt-4">
        <label className="text-[11px] uppercase tracking-wider text-zinc-500" htmlFor="compose-scan-dir">
          Find compose files in a folder
        </label>
        <div className="flex gap-2">
          <input
            id="compose-scan-dir"
            value={scanDir}
            onChange={(e) => setScanDir(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void scan()}
            className={cn(inputClass, "font-mono")}
          />
          <select
            value={depth}
            onChange={(e) => setDepth(Number(e.target.value))}
            title="How many folder levels deep to search"
            className="h-8 border border-zinc-700 bg-zinc-900 px-2 text-xs text-zinc-200"
          >
            {[1, 2, 3, 4, 5, 6].map((d) => (
              <option key={d} value={d}>
                depth {d}
              </option>
            ))}
          </select>
          <Button size="sm" onClick={() => void scan()} disabled={scanning || !scanDir.trim()}>
            {scanning ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <FolderSearch className="h-3.5 w-3.5" />}
            Scan
          </Button>
        </div>
        {found !== null && (
          <div className="max-h-64 overflow-y-auto border border-zinc-800">
            {found.length === 0 ? (
              <p className="px-3 py-4 text-center text-xs text-zinc-500">No compose files found.</p>
            ) : (
              found.map((f) => {
                const already = known.includes(f);
                return (
                  <div key={f} className="flex items-center gap-2 border-b border-zinc-800/70 px-3 py-1.5 last:border-b-0">
                    <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-zinc-300" title={f}>
                      {f}
                    </span>
                    {already ? (
                      <span className="text-[11px] text-zinc-500">listed</span>
                    ) : (
                      <Button size="sm" variant="outline" className="h-7" onClick={() => void add(f)} disabled={adding !== null}>
                        {adding === f ? <Loader2 className="h-3 w-3 animate-spin" /> : <FilePlus2 className="h-3 w-3" />}
                        Add
                      </Button>
                    )}
                  </div>
                );
              })
            )}
          </div>
        )}
      </section>
    </DialogShell>
  );
};

// ---------------------------------------------------------------------------
// Up with options
// ---------------------------------------------------------------------------

export const UpDialog: React.FC<{
  project: ComposeProjectInfo;
  onClose: () => void;
  onSubmit: (options: ComposeUpOptions) => void;
}> = ({ project, onClose, onSubmit }) => {
  const [profiles, setProfiles] = useState<string[]>([]);
  const [envFile, setEnvFile] = useState("");
  const [forceRecreate, setForceRecreate] = useState(false);
  const [removeOrphans, setRemoveOrphans] = useState(false);
  const available = project.profiles ?? [];

  const toggleProfile = (p: string) =>
    setProfiles((prev) => (prev.includes(p) ? prev.filter((x) => x !== p) : [...prev, p]));

  return (
    <DialogShell
      title={`Up ${project.name}`}
      subtitle="docker compose up -d with options"
      icon={<Play className="h-4 w-4" />}
      onClose={onClose}
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            size="sm"
            variant="primary"
            onClick={() =>
              onSubmit({
                profiles,
                env_file: envFile.trim() || null,
                force_recreate: forceRecreate,
                remove_orphans: removeOrphans,
              })
            }
          >
            <Play className="h-3.5 w-3.5" />
            Up
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <div>
          <p className="mb-1.5 text-[11px] uppercase tracking-wider text-zinc-500">Profiles</p>
          {available.length === 0 ? (
            <p className="text-xs text-zinc-500">The file defines no profiles.</p>
          ) : (
            <div className="flex flex-wrap gap-1.5">
              {available.map((p) => {
                const on = profiles.includes(p);
                return (
                  <button
                    key={p}
                    type="button"
                    role="switch"
                    aria-checked={on}
                    onClick={() => toggleProfile(p)}
                    className={cn(
                      "border px-2 py-1 font-mono text-xs transition-colors",
                      on
                        ? "border-indigo-500/60 bg-indigo-500/15 text-indigo-200"
                        : "border-zinc-700 bg-zinc-900 text-zinc-400 hover:text-zinc-200"
                    )}
                  >
                    {p}
                  </button>
                );
              })}
            </div>
          )}
          <p className="mt-1 text-[11px] text-zinc-500">Services behind an enabled profile start too.</p>
        </div>
        <div>
          <label className="mb-1.5 block text-[11px] uppercase tracking-wider text-zinc-500" htmlFor="compose-up-env">
            Env file
          </label>
          <input
            id="compose-up-env"
            value={envFile}
            onChange={(e) => setEnvFile(e.target.value)}
            placeholder=".env (default)"
            className={cn(inputClass, "font-mono")}
          />
          <p className="mt-1 text-[11px] text-zinc-500">Relative to the project folder, e.g. <code>.env.staging</code>.</p>
        </div>
        <div className="space-y-2">
          <Switch
            checked={forceRecreate}
            onChange={setForceRecreate}
            label="Recreate containers even if nothing changed"
          />
          <Switch
            checked={removeOrphans}
            onChange={setRemoveOrphans}
            label="Remove containers of services no longer in the file"
          />
        </div>
      </div>
    </DialogShell>
  );
};

// ---------------------------------------------------------------------------
// Down
// ---------------------------------------------------------------------------

export const DownDialog: React.FC<{
  project: ComposeProjectInfo;
  onClose: () => void;
  onSubmit: (options: ComposeDownOptions) => void;
}> = ({ project, onClose, onSubmit }) => {
  const [removeVolumes, setRemoveVolumes] = useState(false);
  const [removeOrphans, setRemoveOrphans] = useState(true);
  return (
    <DialogShell
      title={`Down ${project.name}?`}
      subtitle="Stops and removes the project's containers and networks"
      icon={<Power className="h-4 w-4 text-rose-400" />}
      onClose={onClose}
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            size="sm"
            variant="danger"
            onClick={() => onSubmit({ remove_volumes: removeVolumes, remove_orphans: removeOrphans })}
          >
            <Power className="h-3.5 w-3.5" />
            {removeVolumes ? "Down and delete volumes" : "Down"}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <p className="text-xs text-zinc-400">
          {project.services.length} container{project.services.length === 1 ? "" : "s"} will be removed. Images and the
          compose file stay.
        </p>
        <Switch checked={removeOrphans} onChange={setRemoveOrphans} label="Also remove orphaned containers" />
        <Switch checked={removeVolumes} onChange={setRemoveVolumes} label="Also delete named volumes (data is lost)" />
        {removeVolumes && (
          <p className="border border-rose-500/40 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">
            The project's named volumes and everything stored in them will be deleted. This cannot be undone.
          </p>
        )}
      </div>
    </DialogShell>
  );
};

// ---------------------------------------------------------------------------
// Scale
// ---------------------------------------------------------------------------

export const ScaleDialog: React.FC<{
  service: string;
  current: number;
  /** Replicas the compose file asks for (`deploy.replicas` / `scale`, else 1). */
  declared: number;
  onClose: () => void;
  onSubmit: (replicas: number) => void;
}> = ({ service, current, declared, onClose, onSubmit }) => {
  const [value, setValue] = useState(String(current));
  const n = Number(value);
  const valid = Number.isInteger(n) && n >= 0 && n <= 100;
  return (
    <DialogShell
      title={`Scale ${service}`}
      subtitle={`${current} container${current === 1 ? "" : "s"} now`}
      icon={<Scaling className="h-4 w-4" />}
      onClose={onClose}
      className="max-w-sm"
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" variant="primary" disabled={!valid || n === current} onClick={() => onSubmit(n)}>
            Scale to {valid ? n : "?"}
          </Button>
        </>
      }
    >
      <div className="flex items-center gap-2">
        <Button size="sm" variant="outline" onClick={() => setValue(String(Math.max(0, (valid ? n : current) - 1)))}>
          −
        </Button>
        <input
          autoFocus
          value={value}
          onChange={(e) => setValue(e.target.value.replace(/[^0-9]/g, ""))}
          onKeyDown={(e) => e.key === "Enter" && valid && n !== current && onSubmit(n)}
          className={cn(inputClass, "w-20 text-center font-mono")}
          aria-label="Replica count"
        />
        <Button size="sm" variant="outline" onClick={() => setValue(String(Math.min(100, (valid ? n : current) + 1)))}>
          +
        </Button>
      </div>
      {valid && n !== declared && (
        <p className="mt-3 border border-amber-500/40 bg-amber-500/10 px-2.5 py-1.5 text-[11px] text-amber-200">
          Temporary: the compose file asks for {declared}. The next plain Up (or Save &amp; apply) brings {service} back
          to {declared}. To keep {n}, set <code>deploy.replicas: {n}</code> in the Compose file tab.
        </p>
      )}
      <p className="mt-2 text-[11px] text-zinc-500">
        A service with a fixed <code>container_name</code> or a fixed host port cannot run more than one replica.
      </p>
    </DialogShell>
  );
};

// ---------------------------------------------------------------------------
// Expose through the gateway
// ---------------------------------------------------------------------------

export const ExposeDialog: React.FC<{
  project: ComposeProjectInfo;
  row: ComposeServiceRow;
  onClose: () => void;
}> = ({ project, row, onClose }) => {
  const toast = useToast();
  const ports = useMemo(() => {
    const live = row.containers.flatMap((c) => publishedPorts(c.ports)).filter((p) => p.protocol === "tcp");
    const declared = (row.spec?.ports ?? [])
      .filter((p) => p.protocol === "tcp" && /^\d+$/.test(p.published))
      .map((p) => ({ published: Number(p.published), target: p.target, protocol: "tcp" }));
    const all = [...live, ...declared];
    return all.filter((p, i) => all.findIndex((q) => q.published === p.published) === i);
  }, [row]);
  const [port, setPort] = useState<number | null>(ports[0]?.published ?? null);
  const [domain, setDomain] = useState(defaultDomain(project.name, row.name));
  const [path, setPath] = useState("");
  const [saving, setSaving] = useState(false);
  const [existing, setExisting] = useState<GatewayRoute[]>([]);
  const containerName = row.containers[0]?.name ?? "";

  useEffect(() => {
    api
      .listGatewayRoutes()
      .then(setExisting)
      .catch(() => setExisting([]));
  }, []);

  const clean = domain.trim().toLowerCase().replace(/^https?:\/\//, "");
  const validDomain = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$/.test(clean);
  const taken = existing.find((r) => r.domain === clean && (r.path ?? "") === (path.trim() || ""));

  const save = async () => {
    if (port === null || !validDomain) return;
    setSaving(true);
    try {
      const routes = await api.listGatewayRoutes();
      const cleanPath = path.trim() ? (path.trim().startsWith("/") ? path.trim() : `/${path.trim()}`) : null;
      const updated = routes.filter((r) => !(r.domain === clean && (r.path ?? null) === cleanPath));
      updated.push({
        id: `route-${Date.now()}`,
        domain: clean,
        target: `127.0.0.1:${port}`,
        path: cleanPath,
        strip_prefix: false,
        container_name: containerName || null,
        group_name: null,
        enabled: true,
        auto_detected: false,
      });
      await api.saveGatewayRoutes(updated);
      setGatewayRoutes(updated);
      const status = await api.getGatewayStatus().catch(() => null);
      const portSuffix = status && status.http_port !== 80 ? `:${status.http_port}` : "";
      const url = `http://${clean}${portSuffix}${cleanPath ?? ""}`;
      if (status && !status.running) {
        toast.info("Route saved", `The gateway is off — start it on the Gateway page to serve ${url}`);
      } else {
        toast.success("Exposed through the gateway", url);
        openUrl(url);
      }
      onClose();
    } catch (e) {
      toast.error("Cannot save the route", String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <DialogShell
      title={`Expose ${row.name}`}
      subtitle="Serve this service on a domain through the gateway (Caddy)"
      icon={<Globe className="h-4 w-4" />}
      onClose={onClose}
      footer={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button size="sm" variant="primary" disabled={port === null || !validDomain || saving} onClick={() => void save()}>
            {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Globe className="h-3.5 w-3.5" />}
            {taken ? "Replace route" : "Expose"}
          </Button>
        </>
      }
    >
      {ports.length === 0 ? (
        <p className="text-xs text-amber-300">
          This service publishes no TCP port on the host. Add a <code>ports:</code> entry to the compose file first.
        </p>
      ) : (
        <div className="space-y-4">
          <div>
            <p className="mb-1.5 text-[11px] uppercase tracking-wider text-zinc-500">Host port</p>
            <div className="flex flex-wrap gap-1.5">
              {ports.map((p) => (
                <button
                  key={p.published}
                  type="button"
                  onClick={() => setPort(p.published)}
                  className={cn(
                    "border px-2 py-1 font-mono text-xs",
                    port === p.published
                      ? "border-sky-500/60 bg-sky-500/15 text-sky-200"
                      : "border-zinc-700 bg-zinc-900 text-zinc-400 hover:text-zinc-200"
                  )}
                >
                  {p.published} → {p.target}
                </button>
              ))}
            </div>
          </div>
          <div>
            <label className="mb-1.5 block text-[11px] uppercase tracking-wider text-zinc-500" htmlFor="compose-expose-domain">
              Domain
            </label>
            <input
              id="compose-expose-domain"
              value={domain}
              onChange={(e) => setDomain(e.target.value)}
              className={cn(inputClass, "font-mono", !validDomain && "border-rose-500/60")}
            />
            <p className="mt-1 text-[11px] text-zinc-500">
              <code>*.localhost</code> resolves without editing /etc/hosts. Other suffixes need Sync Hosts on the
              Gateway page.
            </p>
          </div>
          <div>
            <label className="mb-1.5 block text-[11px] uppercase tracking-wider text-zinc-500" htmlFor="compose-expose-path">
              Path (optional)
            </label>
            <input
              id="compose-expose-path"
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder="/api"
              className={cn(inputClass, "font-mono")}
            />
          </div>
          {taken && (
            <p className="border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-200">
              {clean} already routes to {taken.target}; saving replaces that route.
            </p>
          )}
        </div>
      )}
    </DialogShell>
  );
};

export const SearchBox: React.FC<{ value: string; onChange: (v: string) => void; placeholder: string }> = ({
  value,
  onChange,
  placeholder,
}) => (
  <div className="relative min-w-[12rem] flex-1">
    <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-zinc-500" />
    <input
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className={cn(inputClass, "pl-8")}
      aria-label={placeholder}
    />
  </div>
);
