import React, { useEffect, useMemo, useState } from "react";
import {
  Bot,
  Check,
  Copy,
  KeyRound,
  RefreshCw,
  Plug,
  BookOpen,
  MessageSquareText,
  ShieldAlert,
  Terminal,
  Globe,
} from "lucide-react";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import { Switch } from "../components/ui/Switch";
import { useToast } from "../components/ui/Toast";
import { api } from "../lib/api";
import type { McpPromptInfo, McpResourceInfo, McpStatus, McpToolInfo } from "../types";

/** One copy-to-clipboard block: a command line or a config file body. */
const CopyBlock: React.FC<{
  label: string;
  body: string;
  language?: string;
  onCopied?: () => void;
}> = ({ label, body, onCopied }) => {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(body);
      setCopied(true);
      onCopied?.();
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // Clipboard access can be refused; the text is selectable either way.
    }
  };

  return (
    <div className="rounded-xl border border-zinc-800/80 bg-zinc-950/70 overflow-hidden">
      <div className="flex items-center justify-between gap-2 px-3 py-2 border-b border-zinc-800/70">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
          {label}
        </span>
        <Button
          variant="ghost"
          size="sm"
          onClick={copy}
          aria-label={`Copy ${label}`}
          className="text-xs"
        >
          {copied ? <Check className="w-3.5 h-3.5" /> : <Copy className="w-3.5 h-3.5" />}
          {copied ? "Copied" : "Copy"}
        </Button>
      </div>
      <pre className="p-3 text-[12px] leading-relaxed text-zinc-300 overflow-x-auto whitespace-pre-wrap break-words font-mono max-h-96">
        {body}
      </pre>
    </div>
  );
};

const AccessBadge: React.FC<{ access: string }> = ({ access }) => {
  const styles: Record<string, string> = {
    read: "bg-emerald-500/15 text-emerald-400 border-emerald-500/30",
    write: "bg-amber-500/15 text-amber-400 border-amber-500/30",
    destructive: "bg-rose-500/15 text-rose-400 border-rose-500/30",
  };
  return (
    <span
      className={`px-1.5 py-0.5 rounded-md border text-[10px] font-semibold uppercase tracking-wide ${
        styles[access] ?? styles.write
      }`}
    >
      {access}
    </span>
  );
};

/** Section heading used between the page's blocks. */
const Section: React.FC<{
  icon: React.ElementType;
  title: string;
  subtitle?: string;
  children: React.ReactNode;
}> = ({ icon: Icon, title, subtitle, children }) => (
  <Card className="space-y-4">
    <div className="flex items-start gap-3">
      <div className="w-9 h-9 shrink-0 rounded-xl bg-blue-500/10 border border-blue-500/20 flex items-center justify-center text-blue-400">
        <Icon className="w-4 h-4" />
      </div>
      <div className="min-w-0">
        <h3 className="text-base font-semibold text-white">{title}</h3>
        {subtitle && <p className="text-sm text-zinc-400 mt-0.5">{subtitle}</p>}
      </div>
    </div>
    {children}
  </Card>
);

export const McpPage: React.FC = () => {
  const { success: toastSuccess, error: toastError } = useToast();
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [tools, setTools] = useState<McpToolInfo[]>([]);
  const [resources, setResources] = useState<McpResourceInfo[]>([]);
  const [prompts, setPrompts] = useState<McpPromptInfo[]>([]);
  const [token, setToken] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState("");
  const [showAllTools, setShowAllTools] = useState(false);

  const load = async () => {
    try {
      const [s, t, r, p] = await Promise.all([
        api.mcpStatus(),
        api.mcpCatalogue(),
        api.mcpResources(),
        api.mcpPrompts(),
      ]);
      setStatus(s);
      setTools(t);
      setResources(r);
      setPrompts(p);
    } catch (e) {
      toastError("Could not read MCP status", String(e));
    }
  };

  // Load once: `load` closes over the toast helpers, which are stable per
  // provider render but not referentially equal, so depending on it would
  // re-fetch on every render.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    load();
  }, []);

  const toggleEnabled = async (next: boolean) => {
    try {
      setBusy(true);
      const s = await api.mcpSetEnabled(next);
      setStatus(s);
      toastSuccess(
        next ? "MCP HTTP server enabled" : "MCP HTTP server disabled",
        next ? s.url : "The stdio binary keeps working."
      );
    } catch (e) {
      toastError("Could not change the MCP server", String(e));
    } finally {
      setBusy(false);
    }
  };

  const rotate = async () => {
    try {
      setBusy(true);
      const s = await api.mcpRotateToken();
      setStatus(s);
      setToken(null);
      toastSuccess("Token rotated", "Update every agent that used the old token.");
    } catch (e) {
      toastError("Could not rotate the token", String(e));
    } finally {
      setBusy(false);
    }
  };

  const revealToken = async () => {
    try {
      setToken(await api.mcpToken());
    } catch (e) {
      toastError("Could not read the token", String(e));
    }
  };

  // Grouped by the first word of the tool name: engine_*, container_*, …
  const groups = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const matched = tools.filter(
      (t) =>
        !q ||
        t.name.toLowerCase().includes(q) ||
        t.title.toLowerCase().includes(q) ||
        t.description.toLowerCase().includes(q)
    );
    const byPrefix = new Map<string, McpToolInfo[]>();
    for (const t of matched) {
      const key = t.name.split("_")[0];
      const bucket = byPrefix.get(key) ?? [];
      bucket.push(t);
      byPrefix.set(key, bucket);
    }
    return [...byPrefix.entries()].sort(
      (a, b) => b[1].length - a[1].length || a[0].localeCompare(b[0])
    );
  }, [tools, filter]);

  const stdioPath = status?.stdioPath || "hsx2dock-mcp";

  const claudeCodeSnippet = `# Claude Code — register the app's MCP server (stdio)
claude mcp add hsx2dock -- ${stdioPath}`;

  const desktopSnippet = `{
  "mcpServers": {
    "hsx2dock": {
      "command": "${stdioPath}"
    }
  }
}`;

  const httpSnippet = `{
  "mcpServers": {
    "hsx2dock": {
      "type": "http",
      "url": "${status?.url || "http://127.0.0.1:45123/mcp"}",
      "headers": {
        "Authorization": "Bearer ${token ?? "<token>"}"
      }
    }
  }
}`;

  const skillPrompt = `You have the Hsx2Dock MCP server connected. It is the control
plane for the rootless Docker engine on this machine: containers, images,
volumes, networks, compose projects, kind/Kubernetes clusters, the engine's own
lifecycle and self-repair, remote endpoints, registry logins, storage and app
config.

Operating rules:
1. Start with engine_get_status. If the engine is stopped, engine_start first;
   if something is wrong, run engine_diagnose before guessing.
2. Read before you write. Every list/get/inspect tool is read-only and cheap.
   Never guess an id, name, or port — fetch it.
3. Tools marked engine_only are refused while a remote endpoint is active. That
   is deliberate: they touch the bundled engine's own files. Call endpoint_list
   and endpoint_set_active to switch back before retrying.
4. Destructive tools (remove_*, prune_*, *_delete_*, engine_repair) cannot be
   undone. Confirm the exact target with a read first, and state what you are
   about to remove.
5. Long operations (image_pull, image_build, compose_up, cluster_create) return
   as soon as they are accepted. Poll the matching *_list tool for progress
   instead of assuming success.
6. An error message is the engine's own wording. Report it as-is rather than
   paraphrasing it into something more or less severe.
7. Prefer the narrowest tool that answers the question. container_list with a
   filter beats inspecting every container one by one.

Read hsx2dock://guide for the full tool reference, and use the built-in prompts
(diagnose-engine, container-health, cluster-audit, plan-environment) when the
user's request matches one of those jobs.`;

  return (
    <div className="h-full overflow-y-auto">
      <div className="p-8 space-y-6 max-w-7xl mx-auto">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-white">MCP Server</h2>
          <p className="text-sm text-zinc-400 mt-1">
            Expose Hsx2Dock to AI agents — {status?.toolCount ?? 0} tools,{" "}
            {status?.resourceCount ?? 0} resources, {status?.promptCount ?? 0} prompts.
          </p>
        </div>
        <Button variant="ghost" size="icon" onClick={load} title="Reload" aria-label="Reload MCP status">
          <RefreshCw className="w-4 h-4" />
        </Button>
      </div>

      {/* ---------- Live status ---------- */}
      <Section
        icon={Plug}
        title="Transports"
        subtitle="stdio needs no setup and no network. HTTP is opt-in and stays on loopback."
      >
        <div className="grid gap-4 md:grid-cols-2">
          <div className="rounded-xl border border-zinc-800/80 bg-zinc-950/50 p-4 space-y-3">
            <div className="flex items-center gap-2">
              <Terminal className="w-4 h-4 text-emerald-400" />
              <span className="font-medium text-white text-sm">stdio binary</span>
              <span className="px-1.5 py-0.5 rounded-md border border-emerald-500/30 bg-emerald-500/15 text-emerald-400 text-[10px] font-semibold uppercase">
                recommended
              </span>
            </div>
            <p className="text-xs text-zinc-400">
              Launched by the agent itself; no port, no token, no listener. The process
              that owns the pipe is the only thing that can call a tool.
            </p>
            <CopyBlock label="Binary" body={`$ ${stdioPath}`} />
          </div>

          <div className="rounded-xl border border-zinc-800/80 bg-zinc-950/50 p-4 space-y-3">
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <Globe className="w-4 h-4 text-blue-400" />
                <span className="font-medium text-white text-sm">Loopback HTTP</span>
                <span
                  className={`px-1.5 py-0.5 rounded-md border text-[10px] font-semibold uppercase ${
                    status?.running
                      ? "border-emerald-500/30 bg-emerald-500/15 text-emerald-400"
                      : "border-zinc-700 bg-zinc-800/60 text-zinc-400"
                  }`}
                >
                  {status?.running ? "running" : "stopped"}
                </span>
              </div>
              <Switch
                checked={!!status?.enabled}
                disabled={busy}
                onChange={toggleEnabled}
                label="Enable the MCP HTTP server"
              />
            </div>
            <p className="text-xs text-zinc-400">
              Binds <span className="font-mono text-zinc-300">127.0.0.1:{status?.port ?? "—"}</span>{" "}
              only, and every request must carry the bearer token. Use it for agents that
              cannot spawn a child process.
            </p>
            {status?.running && <CopyBlock label="Endpoint" body={status.url} />}
          </div>
        </div>
      </Section>

      {/* ---------- Token ---------- */}
      <Section
        icon={KeyRound}
        title="Access token"
        subtitle="Required by the HTTP transport. Generated locally, never leaves this machine."
      >
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="secondary" size="sm" onClick={revealToken} disabled={busy}>
              <KeyRound className="w-3.5 h-3.5" />
              {token ? "Token shown below" : "Reveal token"}
            </Button>
            <Button variant="outline" size="sm" onClick={rotate} disabled={busy}>
              <RefreshCw className="w-3.5 h-3.5" />
              Rotate token
            </Button>
            <span className="text-xs text-zinc-500">
              Stored at <span className="font-mono">{status?.tokenPath || "~/.config/hsx2dock/mcp.json"}</span>{" "}
              with owner-only permissions.
            </span>
          </div>
          {token && <CopyBlock label="Bearer token" body={token} />}
          <p className="text-xs text-zinc-500">
            Rotating invalidates every agent already configured against the HTTP transport.
            Disabling the server keeps the token so re-enabling changes nothing for them.
          </p>
        </div>
      </Section>

      {/* ---------- Setup ---------- */}
      <Section
        icon={BookOpen}
        title="Connect an agent"
        subtitle="Pick the transport your agent supports and paste the snippet."
      >
        <div className="space-y-4">
          <div className="space-y-2">
            <h4 className="text-sm font-semibold text-zinc-200">1 · stdio — Claude Code</h4>
            <CopyBlock label="Shell command" body={claudeCodeSnippet} />
          </div>
          <div className="space-y-2">
            <h4 className="text-sm font-semibold text-zinc-200">
              2 · stdio — Claude Desktop / Cursor / any JSON-configured client
            </h4>
            <CopyBlock label="mcpServers config" body={desktopSnippet} />
          </div>
          <div className="space-y-2">
            <h4 className="text-sm font-semibold text-zinc-200">
              3 · HTTP — for agents that cannot spawn a process
            </h4>
            <p className="text-xs text-zinc-500">
              Enable the loopback server above first, then reveal the token and copy this.
            </p>
            <CopyBlock label="HTTP server config" body={httpSnippet} />
          </div>
        </div>
      </Section>

      {/* ---------- Skill prompt ---------- */}
      <Section
        icon={MessageSquareText}
        title="Agent instructions"
        subtitle="Paste this into the agent's system prompt, custom instructions, or project rules."
      >
        <CopyBlock
          label="Skill prompt"
          body={skillPrompt}
          onCopied={() => toastSuccess("Skill prompt copied", "Paste it into your agent's instructions.")}
        />
        <p className="text-xs text-zinc-500">
          The same rules ship inside the server itself: every client receives them on
          connect, and they are readable as the{" "}
          <span className="font-mono text-zinc-400">hsx2dock://guide</span> resource.
        </p>
      </Section>

      {/* ---------- Safety ---------- */}
      <Section
        icon={ShieldAlert}
        title="What an agent can do"
        subtitle="Every tool is annotated, and the annotations are enforced — not advisory."
      >
        <div className="grid gap-3 sm:grid-cols-3">
          {[
            {
              access: "read",
              detail: "Lists, inspections, status reads. Safe to call freely, never mutate.",
            },
            {
              access: "write",
              detail: "Starts, creates, pulls, tags. Reversible, but not free.",
            },
            {
              access: "destructive",
              detail: "Removals, prunes, cluster deletes, engine repair. Cannot be undone.",
            },
          ].map((r) => (
            <div
              key={r.access}
              className="rounded-xl border border-zinc-800/80 bg-zinc-950/50 p-3 space-y-2"
            >
              <AccessBadge access={r.access} />
              <p className="text-xs text-zinc-400">{r.detail}</p>
            </div>
          ))}
        </div>
        <p className="text-xs text-zinc-500">
          Engine lifecycle and self-repair tools are refused outright while a remote
          endpoint is active — they would otherwise touch the bundled engine's own files
          through someone else's daemon.
        </p>
        <p className="text-xs text-zinc-500">
          stdio is the safer transport: the agent owns the process, so only it can call a
          tool. The loopback HTTP listener is what makes the same surface reachable by any
          process on this machine that holds the token — leave it off unless an agent
          cannot spawn a child process.
        </p>
      </Section>

      {/* ---------- Tool catalogue ---------- */}
      <Section
        icon={Bot}
        title={`Tool catalogue (${tools.length})`}
        subtitle="Exactly what an agent sees when it lists tools."
      >
        <input
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter by name, title, or description…"
          aria-label="Filter tools"
          className="w-full rounded-xl border border-zinc-800 bg-zinc-950/70 px-3 py-2 text-sm text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-blue-500/50"
        />

        <div className="space-y-4">
          {(showAllTools ? groups : groups.slice(0, 6)).map(([prefix, items]) => (
            <div key={prefix} className="space-y-1.5">
              <h4 className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
                {prefix} · {items.length}
              </h4>
              <div className="rounded-xl border border-zinc-800/80 divide-y divide-zinc-800/60 overflow-hidden">
                {items.map((t) => (
                  <div key={t.name} className="px-3 py-2 hover:bg-zinc-900/40">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-mono text-[12px] text-zinc-100">{t.name}</span>
                      <AccessBadge access={t.access} />
                      {t.engineOnly && (
                        <span className="px-1.5 py-0.5 rounded-md border border-fuchsia-500/30 bg-fuchsia-500/15 text-fuchsia-300 text-[10px] font-semibold uppercase">
                          local engine only
                        </span>
                      )}
                    </div>
                    <p className="text-xs text-zinc-400 mt-1">{t.description}</p>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>

        {groups.length > 6 && (
          <Button variant="ghost" size="sm" onClick={() => setShowAllTools((v) => !v)}>
            {showAllTools ? "Show fewer groups" : `Show all ${groups.length} groups`}
          </Button>
        )}
      </Section>

      {/* ---------- Resources and prompts ---------- */}
      <div className="grid gap-6 lg:grid-cols-2">
        <Section
          icon={BookOpen}
          title={`Resources (${resources.length})`}
          subtitle="Read-only JSON and markdown an agent can pull instead of calling tools."
        >
          <div className="rounded-xl border border-zinc-800/80 divide-y divide-zinc-800/60 overflow-hidden">
            {resources.map((r) => (
              <div key={r.uri} className="px-3 py-2 hover:bg-zinc-900/40">
                <span className="font-mono text-[12px] text-zinc-100 break-all">{r.uri}</span>
                <p className="text-xs text-zinc-400 mt-1">{r.description}</p>
              </div>
            ))}
          </div>
        </Section>

        <Section
          icon={MessageSquareText}
          title={`Prompts (${prompts.length})`}
          subtitle="Ready-made briefs for the jobs this app exists to do."
        >
          <div className="rounded-xl border border-zinc-800/80 divide-y divide-zinc-800/60 overflow-hidden">
            {prompts.map((p) => (
              <div key={p.name} className="px-3 py-2 hover:bg-zinc-900/40">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-mono text-[12px] text-zinc-100">{p.name}</span>
                  {p.arguments.map((a) => (
                    <span
                      key={a.name}
                      title={a.description}
                      className="px-1.5 py-0.5 rounded-md border border-zinc-700 bg-zinc-800/60 text-zinc-400 text-[10px] font-mono"
                    >
                      {a.name}
                      {a.required ? "" : "?"}
                    </span>
                  ))}
                </div>
                <p className="text-xs text-zinc-400 mt-1">{p.description}</p>
              </div>
            ))}
          </div>
        </Section>
      </div>

        <p className="text-xs text-zinc-600 pb-4">
          {status ? `${status.serverName} v${status.serverVersion}` : "hsx2dock"} ·{" "}
          {tools.length} tools · protocol: Model Context Protocol (stdio + streamable HTTP)
        </p>
      </div>
    </div>
  );
};
