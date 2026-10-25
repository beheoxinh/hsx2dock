import React, { useCallback } from "react";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import {
  Activity,
  Bug,
  Cpu,
  ExternalLink,
  FileText,
  Layers,
  Pause,
  Play,
  RotateCw,
  Square,
  Terminal,
  Trash2,
  Settings2,
} from "lucide-react";
import { Card } from "./ui/Card";
import { Button } from "./ui/Button";
import { StatusBadge } from "./ui/StatusBadge";
import { useToast } from "./ui/Toast";
import { formatBytes, truncateId } from "../lib/utils";
import { portChips, type PortChip } from "../lib/portExposure";
import type { ContainerItem, ContainerStats } from "../types";

export type ContainerAction =
  | "start"
  | "stop"
  | "restart"
  | "pause"
  | "remove";

interface ContainerCardProps {
  container: ContainerItem;
  stats?: ContainerStats;
  busy?: boolean;
  /** Pointer-down handle from `useDraggable`; omitted for non-draggable cards. */
  dragHandle?: { onPointerDown: (e: React.PointerEvent) => void };
  /** Highlight the card while it is the drag origin. */
  isDragSource?: boolean;
  compact?: boolean;
  onAction: (action: ContainerAction) => void;
  onOpenDetails: (
    tab?: "logs" | "inspect" | "stats" | "terminal",
    debug?: boolean
  ) => void;
  onEditContainer?: () => void;
}

/**
 * One published port. A published port is a link that opens it in the browser;
 * an exposed-only port (never published) is inert text. A wildcard binding gets
 * a small amber dot rather than a whole warning banner — it is worth knowing,
 * not worth shouting about.
 */
const PortChipBadge: React.FC<{ chip: PortChip; containerName: string }> = ({
  chip,
  containerName,
}) => {
  const toast = useToast();
  const base =
    "inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 font-mono text-[11px] leading-none";
  const tone = chip.public
    ? "border-amber-500/30 bg-amber-500/10 text-amber-200"
    : "border-zinc-700/60 bg-zinc-800/60 text-zinc-200";
  const title = chip.public
    ? `${chip.label}/${chip.protocol} — published on every interface, reachable from your network`
    : `${chip.label}/${chip.protocol}${chip.hostPort === null ? " — exposed, not published" : " — loopback only"}`;

  const inner = (
    <>
      {chip.public && (
        <span
          aria-hidden="true"
          className="w-1.5 h-1.5 rounded-full bg-amber-400 shrink-0"
        />
      )}
      <span>{chip.label}</span>
      {chip.protocol !== "tcp" && (
        <span className="text-zinc-400">/{chip.protocol}</span>
      )}
      {chip.url && <ExternalLink className="w-2.5 h-2.5 opacity-60" />}
    </>
  );

  /**
   * A `target="_blank"` anchor does not leave the Tauri webview — it either
   * does nothing or opens a second app window. Hand the URL to the shell
   * plugin instead, which runs the desktop's own opener (`xdg-open`).
   * The href stays so the URL is still visible, copyable and middle-clickable.
   */
  const openInBrowser = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (!chip.url) return;
      openExternal(chip.url).catch((err) => {
        // A failed open used to be invisible; say what happened and hand the
        // user the URL so the click is never a dead end.
        toast.error("Could not open the browser", `${chip.url} — ${err}`);
      });
    },
    [chip.url, toast]
  );

  if (!chip.url) {
    return (
      <span className={`${base} ${tone}`} title={title}>
        {inner}
      </span>
    );
  }

  return (
    <a
      href={chip.url}
      target="_blank"
      rel="noreferrer"
      title={`${title} — click to open in the browser`}
      aria-label={`Open ${containerName} port ${chip.hostPort} in the browser`}
      onClick={openInBrowser}
      className={`${base} ${tone} hover:border-blue-500/60 hover:text-white transition-colors`}
    >
      {inner}
    </a>
  );
};

/**
 * One container, rendered identically whether it sits in the flat list or
 * inside a group card. `compact` trims the padding and tightens the stats grid
 * for the nested case; the figures themselves stay, because the group header's
 * aggregate pills are a summary, not a replacement for per-container data.
 */
export const ContainerCard: React.FC<ContainerCardProps> = ({
  container: c,
  stats,
  busy = false,
  dragHandle,
  isDragSource = false,
  compact = false,
  onAction,
  onOpenDetails,
  onEditContainer,
}) => {
  const isRunning = c.state === "running";
  const name = c.name.replace(/^\//, "");
  // One chip per published port: the host→container mapping is readable at a
  // glance, and clicking a published port opens it in the browser.
  const chips = portChips(c);

  return (
    <Card
      data-container-card={c.id}
      className={`flex flex-col lg:flex-row lg:items-center justify-between transition-all ${
        compact ? "gap-3 p-3" : "gap-5 p-4"
      } ${
        dragHandle ? "cursor-grab active:cursor-grabbing" : ""
      } ${
        isDragSource
          ? "opacity-50 border-indigo-500/60"
          : "hover:border-zinc-700/80"
      }`}
      {...(dragHandle ?? {})}
    >
      {/* Left: identity */}
      <div className={`flex items-start min-w-0 ${compact ? "gap-3" : "gap-4"}`}>
        <div
          className={`shrink-0 rounded-xl bg-zinc-800/80 border border-zinc-700/60 flex items-center justify-center text-zinc-300 ${
            compact ? "w-8 h-8" : "w-10 h-10"
          }`}
        >
          <Layers className={compact ? "w-4 h-4" : "w-5 h-5"} />
        </div>
        <div className="min-w-0">
          <div className="flex items-center gap-2.5">
            <span
              className={`font-semibold text-white tracking-tight truncate ${
                compact ? "text-sm" : ""
              }`}
            >
              {name}
            </span>
            <StatusBadge status={c.state} />
          </div>
          <div className="text-xs text-zinc-400 mt-1 flex items-center gap-2">
            <span className="font-mono text-zinc-400">{truncateId(c.id)}</span>
            <span>•</span>
            <span className="truncate max-w-[200px]" title={c.image}>
              {c.image}
            </span>
          </div>
        </div>
      </div>

      {/* Middle: live figures. Group members (compact) keep the same ports and
          load readouts as isolated cards — only the spacing tightens; the
          group header's aggregate pills are a summary, not a replacement. */}
      <div
        className={`flex-1 min-w-0 grid grid-cols-2 sm:grid-cols-3 text-xs bg-zinc-950/50 border border-zinc-800/50 rounded-xl ${
          compact ? "gap-2 p-2" : "gap-4 p-3"
        }`}
      >
        <div className="min-w-0 sm:col-span-1">
          <span className="text-zinc-500 block text-[10px] uppercase font-semibold">
            Ports
          </span>
          {chips.length === 0 ? (
            <span className="text-zinc-600 font-mono mt-0.5 block">—</span>
          ) : (
            <div className="flex flex-wrap items-center gap-1 mt-1 min-w-0">
              {chips.map((chip, i) => (
                <PortChipBadge
                  key={`${chip.hostPort}-${chip.containerPort}-${chip.protocol}-${i}`}
                  chip={chip}
                  containerName={name}
                />
              ))}
            </div>
          )}
        </div>

        <div>
          <span className="text-zinc-500 block text-[10px] uppercase font-semibold">
            CPU
          </span>
          <span className="text-zinc-200 flex items-center gap-1 font-mono mt-0.5">
            <Cpu className="w-3.5 h-3.5 text-blue-400" />
            {isRunning && stats ? `${stats.cpu_percent.toFixed(1)}%` : "—"}
          </span>
        </div>

        <div>
          <span className="text-zinc-500 block text-[10px] uppercase font-semibold">
            Memory
          </span>
          <span className="text-zinc-200 flex items-center gap-1 font-mono mt-0.5">
            <Activity className="w-3.5 h-3.5 text-emerald-400" />
            {isRunning && stats ? formatBytes(stats.mem_usage) : "—"}
          </span>
        </div>
      </div>

      {/* Right: actions */}
      <div
        className={`flex items-center gap-1.5 ${
          compact ? "shrink-0" : "self-end lg:self-center shrink-0"
        }`}
      >
        {isRunning ? (
          <>
            <Button
              variant="ghost"
              size="icon"
              onClick={() => onOpenDetails("terminal")}
              title="Open container shell"
              aria-label={`Open a shell in ${name}`}
              disabled={busy}
              className="text-emerald-400 hover:text-emerald-300 hover:bg-emerald-500/10"
            >
              <Terminal className="w-4 h-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              onClick={() => onAction("pause")}
              disabled={busy}
              title="Pause container"
              aria-label={`Pause ${name}`}
              className="text-amber-400 hover:text-amber-300 hover:bg-amber-500/10"
            >
              <Pause className="w-4 h-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              onClick={() => onAction("stop")}
              disabled={busy}
              title="Stop container"
              aria-label={`Stop ${name}`}
              className="text-rose-400 hover:text-rose-300 hover:bg-rose-500/10"
            >
              <Square className="w-4 h-4 fill-current" />
            </Button>
          </>
        ) : (
          <>
            <Button
              variant="ghost"
              size="icon"
              onClick={() => onAction("start")}
              disabled={busy}
              title="Start container"
              aria-label={`Start ${name}`}
              className="text-emerald-400 hover:text-emerald-300 hover:bg-emerald-500/10"
            >
              <Play className="w-4 h-4 fill-current" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              onClick={() => onOpenDetails("terminal", true)}
              disabled={busy}
              title="Debug — start the container and attach a shell"
              aria-label={`Debug ${name}`}
              className="text-amber-400 hover:text-amber-300 hover:bg-amber-500/10"
            >
              <Bug className="w-4 h-4" />
            </Button>
          </>
        )}

        <Button
          variant="ghost"
          size="icon"
          onClick={() => onAction("restart")}
          disabled={busy}
          title="Restart container"
          aria-label={`Restart ${name}`}
          className="text-blue-400 hover:text-blue-300 hover:bg-blue-500/10"
        >
          <RotateCw className="w-4 h-4" />
        </Button>

        <Button
          variant="ghost"
          size="icon"
          onClick={() => onOpenDetails("logs")}
          title="View logs"
          aria-label={`View logs for ${name}`}
          className="text-zinc-400 hover:text-white"
        >
          <FileText className="w-4 h-4" />
        </Button>

        {onEditContainer && (
          <Button
            variant="ghost"
            size="icon"
            onClick={onEditContainer}
            title="Edit & Recreate (ports, env, mounts...)"
            aria-label={`Edit and recreate ${name}`}
            className="text-amber-400 hover:text-amber-300 hover:bg-amber-500/10"
          >
            <Settings2 className="w-4 h-4" />
          </Button>
        )}

        <Button
          variant="ghost"
          size="icon"
          onClick={() => onAction("remove")}
          disabled={busy}
          title="Delete container"
          aria-label={`Delete ${name}`}
          className="text-zinc-400 hover:text-rose-400 hover:bg-rose-500/10"
        >
          <Trash2 className="w-4 h-4" />
        </Button>
      </div>
    </Card>
  );
};
