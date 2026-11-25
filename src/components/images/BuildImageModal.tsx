import React, { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import {
  CheckCircle,
  FolderOpen,
  Hammer,
  Play,
  Plus,
  RotateCcw,
  Terminal,
  Trash2,
  Upload,
  X,
  XCircle,
  Copy,
} from "lucide-react";
import { Button } from "../ui/Button";
import { TerminalViewer } from "../ui/TerminalViewer";
import { useToast } from "../ui/Toast";
import { api, disposeListener, events } from "../../lib/api";

/** Build log lines kept in memory (and in the terminal view). */
const MAX_BUILD_LINES = 2000;
const LINE_TRIM_CHUNK = 500;
import type {
  BuildArg,
  BuildContextInfo,
  BuildProgressPayload,
  BuildRequest,
} from "../../types";

interface BuildImageModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** Called once an image was built, so the list can refresh. */
  onSuccess: (tag: string) => void;
  /** Ask the parent to open the run wizard pre-filled with this image. */
  onRunImage?: (image: string) => void;
}

const RECENT_KEY = "buildRecentContexts";
const MAX_RECENT = 5;

interface LogLine {
  text: string;
  error: boolean;
}

function loadRecent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((v) => typeof v === "string") : [];
  } catch {
    return [];
  }
}

function rememberContext(path: string): string[] {
  const next = [path, ...loadRecent().filter((p) => p !== path)].slice(0, MAX_RECENT);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(next));
  } catch {
    /* storage disabled — the recent list is a convenience, not a requirement */
  }
  return next;
}

function formatElapsed(ms: number): string {
  const total = Math.floor(ms / 1000);
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

type Phase = "form" | "running" | "succeeded" | "failed" | "cancelled";

const EMPTY_INFO: BuildContextInfo = {
  exists: false,
  is_dir: false,
  entries: [],
  dockerfiles: [],
  suggested_tag: "",
};

export const BuildImageModal: React.FC<BuildImageModalProps> = ({
  isOpen,
  onClose,
  onSuccess,
  onRunImage,
}) => {
  const toast = useToast();

  const [contextDir, setContextDir] = useState("");
  const [dockerfile, setDockerfile] = useState("Dockerfile");
  const [tag, setTag] = useState("");
  const [target, setTarget] = useState("");
  const [noCache, setNoCache] = useState(false);
  const [pull, setPull] = useState(false);
  const [buildArgs, setBuildArgs] = useState<BuildArg[]>([]);
  const [recent, setRecent] = useState<string[]>([]);

  const [info, setInfo] = useState<BuildContextInfo>(EMPTY_INFO);
  const [checking, setChecking] = useState(false);
  const [preview, setPreview] = useState("docker build .");

  const [phase, setPhase] = useState<Phase>("form");
  const [lines, setLines] = useState<LogLine[]>([]);
  const [status, setStatus] = useState("");
  const [progress, setProgress] = useState<{ current: number | null; total: number | null }>({
    current: null,
    total: null,
  });
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [imageId, setImageId] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  const logRef = useRef<HTMLDivElement>(null);
  const pinnedRef = useRef(true);
  const running = phase === "running";

  // Seed the form when the modal opens; never mid-build.
  useEffect(() => {
    if (!isOpen) return;
    setRecent(loadRecent());
    setPhase("form");
    setLines([]);
    setStatus("");
    setFailure(null);
    setImageId(null);
    setProgress({ current: null, total: null });
    setStartedAt(null);
    setElapsed(0);
  }, [isOpen]);

  // Build-arg-free requests still need a preview; keep it in sync with the form.
  useEffect(() => {
    if (!isOpen) return;
    const req: BuildRequest = {
      context_dir: contextDir,
      dockerfile,
      tag,
      target: target || null,
      no_cache: noCache,
      pull,
      build_args: buildArgs,
    };
    let alive = true;
    api
      .previewBuildCommand(req)
      .then((p) => {
        if (alive) setPreview(p);
      })
      .catch(() => {
        if (alive) setPreview("docker build .");
      });
    return () => {
      alive = false;
    };
  }, [isOpen, contextDir, dockerfile, tag, target, noCache, pull, buildArgs]);

  // Debounced context probe.
  useEffect(() => {
    if (!isOpen) return;
    const path = contextDir.trim();
    if (!path) {
      setInfo(EMPTY_INFO);
      return;
    }
    let alive = true;
    setChecking(true);
    const timer = window.setTimeout(() => {
      api
        .validateBuildContext(path)
        .then((res) => {
          if (!alive) return;
          setInfo(res);
          if (res.exists && res.is_dir && !tag.trim() && res.suggested_tag) {
            setTag(res.suggested_tag);
          }
        })
        .catch(() => {
          if (alive) setInfo(EMPTY_INFO);
        })
        .finally(() => {
          if (alive) setChecking(false);
        });
    }, 400);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [isOpen, contextDir, tag]);

  // Live build stream — subscribed only while the modal is open.
  useEffect(() => {
    if (!isOpen) return;
    return disposeListener(
      events.onBuildProgress((payload: BuildProgressPayload) => {
        if (payload.log) {
          const text = payload.log;
          setLines((prev) => {
            const next = [...prev, { text, error: /error|failed/i.test(text) && !!payload.error }];
            // A long build streams tens of thousands of lines. Keep the tail,
            // trimmed in chunks so the terminal view is rebuilt once per
            // chunk rather than on every line.
            return next.length > MAX_BUILD_LINES + LINE_TRIM_CHUNK
              ? next.slice(-MAX_BUILD_LINES)
              : next;
          });
        }
        setStatus(payload.status);
        setProgress({ current: payload.current, total: payload.total });

        if (payload.done) {
          setProgress({ current: null, total: null });
          if (payload.cancelled) {
            setPhase("cancelled");
          } else if (payload.error) {
            setFailure(payload.error);
            setPhase("failed");
          } else {
            setImageId(payload.image_id);
            setPhase("succeeded");
            onSuccess(payload.tag);
            toast.success("Image built", payload.tag);
          }
        }
      })
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen]);

  // Elapsed timer while a build runs.
  useEffect(() => {
    if (!running || startedAt === null) return;
    const id = window.setInterval(() => setElapsed(Date.now() - startedAt), 1000);
    return () => window.clearInterval(id);
  }, [running, startedAt]);

  // Keep the log pinned to the bottom unless the user scrolled up.
  useEffect(() => {
    const el = logRef.current;
    if (!el || !pinnedRef.current) return;
    el.scrollTop = el.scrollHeight;
  }, [lines]);

  if (!isOpen) return null;

  const contextValid = info.exists && info.is_dir;
  const canBuild = contextValid && !!tag.trim() && !checking;

  const handleLogScroll = () => {
    const el = logRef.current;
    if (!el) return;
    pinnedRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  };

  const handleSubmit = async () => {
    if (!canBuild) return;
    const req: BuildRequest = {
      context_dir: contextDir.trim(),
      dockerfile: dockerfile.trim() || "Dockerfile",
      tag: tag.trim(),
      target: target.trim() || null,
      no_cache: noCache,
      pull,
      build_args: buildArgs.filter((a) => a.key.trim() !== ""),
    };
    try {
      setLines([]);
      setFailure(null);
      setImageId(null);
      setStatus("Starting");
      pinnedRef.current = true;
      setElapsed(0);
      setStartedAt(Date.now());
      setPhase("running");
      setRecent(rememberContext(req.context_dir));
      await api.buildImage(req);
    } catch (e) {
      setPhase("form");
      toast.error("Build failed to start", String(e));
    }
  };

  const handleCancel = async () => {
    try {
      await api.cancelImageBuild();
    } catch (e) {
      toast.error("Could not cancel", String(e));
    }
  };

  const handleClose = async () => {
    if (running) {
      const ok = window.confirm("Stop the running build and close?");
      if (!ok) return;
      try {
        await api.cancelImageBuild();
      } catch {
        /* the build may have finished between the prompt and here */
      }
    }
    onClose();
  };

  const statusPill = () => {
    if (running) {
      return (
        <span className="flex items-center gap-1.5 text-xs font-medium text-cyan-400">
          <Hammer className="h-3.5 w-3.5 animate-pulse" />
          Building
        </span>
      );
    }
    if (phase === "succeeded") {
      return (
        <span className="flex items-center gap-1.5 text-xs font-semibold text-emerald-400">
          <CheckCircle className="h-3.5 w-3.5" />
          Finished
        </span>
      );
    }
    if (phase === "failed") {
      return (
        <span className="flex items-center gap-1.5 text-xs font-semibold text-rose-400">
          <XCircle className="h-3.5 w-3.5" />
          Failed
        </span>
      );
    }
    if (phase === "cancelled") {
      return <span className="text-xs font-semibold text-amber-400">Cancelled</span>;
    }
    return null;
  };

  const pct =
    progress.current !== null && progress.total
      ? Math.min(100, Math.round((progress.current / progress.total) * 100))
      : null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      <div className="fixed inset-0 bg-black/70 backdrop-blur-md" onClick={handleClose} />
      <motion.div
        initial={{ opacity: 0, scale: 0.96 }}
        animate={{ opacity: 1, scale: 1 }}
        transition={{ duration: 0.15 }}
        className="relative flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border border-zinc-800 bg-zinc-950 shadow-2xl"
      >
        <div className="flex shrink-0 items-center justify-between border-b border-zinc-800 px-5 py-3.5">
          <div className="flex items-center gap-3">
            <h3 className="text-lg font-semibold text-white">Build Image</h3>
            {statusPill()}
          </div>
          <Button variant="ghost" size="icon" onClick={handleClose} title="Close">
            <X className="h-5 w-5" />
          </Button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {phase === "form" ? (
            <div className="space-y-4">
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-zinc-400">Build context</label>
                <div className="flex items-center gap-2 rounded-lg border border-zinc-800 bg-zinc-900/60 px-3">
                  <FolderOpen className="h-4 w-4 shrink-0 text-zinc-500" />
                  <input
                    type="text"
                    autoFocus
                    placeholder="/path/to/project"
                    value={contextDir}
                    onChange={(e) => setContextDir(e.target.value)}
                    className="h-10 w-full bg-transparent font-mono text-sm text-white outline-none placeholder:text-zinc-600"
                  />
                  {checking && <span className="shrink-0 text-[11px] text-zinc-500">checking…</span>}
                </div>

                {contextDir.trim() !== "" && !checking && !contextValid && (
                  <p className="text-xs text-amber-400/90">
                    {info.exists
                      ? "That path is not a directory."
                      : "That directory does not exist."}
                  </p>
                )}

                {contextValid && info.entries.length > 0 && (
                  <div className="flex flex-wrap gap-1.5 pt-0.5">
                    {info.entries.slice(0, 12).map((name) => (
                      <button
                        key={name}
                        type="button"
                        onClick={() => setDockerfile(name)}
                        className="rounded-md border border-zinc-800 bg-zinc-900/70 px-2 py-0.5 font-mono text-[11px] text-zinc-400 hover:border-zinc-700 hover:text-zinc-200"
                        title="Use as the Dockerfile"
                      >
                        {name}
                      </button>
                    ))}
                  </div>
                )}

                {recent.length > 0 && (
                  <div className="flex flex-wrap items-center gap-1.5 pt-1">
                    <span className="text-[11px] uppercase tracking-wider text-zinc-600">
                      Recent
                    </span>
                    {recent.map((path) => (
                      <button
                        key={path}
                        type="button"
                        onClick={() => setContextDir(path)}
                        className="max-w-[280px] truncate rounded-md border border-zinc-800/70 bg-zinc-900/40 px-2 py-0.5 font-mono text-[11px] text-zinc-500 hover:text-zinc-300"
                      >
                        {path}
                      </button>
                    ))}
                  </div>
                )}
              </div>

              <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <label className="text-xs font-medium text-zinc-400">Dockerfile</label>
                  <input
                    type="text"
                    value={dockerfile}
                    onChange={(e) => setDockerfile(e.target.value)}
                    placeholder="Dockerfile"
                    className="h-10 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-3 font-mono text-sm text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
                  />
                </div>
                <div className="space-y-1.5">
                  <label className="text-xs font-medium text-zinc-400">
                    Tag <span className="text-rose-400/80">*</span>
                  </label>
                  <input
                    type="text"
                    value={tag}
                    onChange={(e) => setTag(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && canBuild) handleSubmit();
                    }}
                    placeholder="my-app:latest"
                    className="h-10 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-3 font-mono text-sm text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
                  />
                </div>
              </div>

              <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <label className="text-xs font-medium text-zinc-400">
                    Target stage <span className="text-zinc-600">(optional)</span>
                  </label>
                  <input
                    type="text"
                    value={target}
                    onChange={(e) => setTarget(e.target.value)}
                    placeholder="builder"
                    className="h-10 w-full rounded-lg border border-zinc-800 bg-zinc-900/60 px-3 font-mono text-sm text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
                  />
                </div>
                <div className="flex items-end gap-4 pb-2">
                  <label className="flex cursor-pointer items-center gap-2 text-xs text-zinc-300">
                    <input
                      type="checkbox"
                      checked={noCache}
                      onChange={(e) => setNoCache(e.target.checked)}
                      className="h-4 w-4 rounded border-zinc-700 bg-zinc-900 accent-cyan-500"
                    />
                    No cache
                  </label>
                  <label className="flex cursor-pointer items-center gap-2 text-xs text-zinc-300">
                    <input
                      type="checkbox"
                      checked={pull}
                      onChange={(e) => setPull(e.target.checked)}
                      className="h-4 w-4 rounded border-zinc-700 bg-zinc-900 accent-cyan-500"
                    />
                    Pull base image
                  </label>
                </div>
              </div>

              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <label className="text-xs font-medium text-zinc-400">Build arguments</label>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setBuildArgs((prev) => [...prev, { key: "", value: "" }])}
                    className="text-[11px] text-zinc-400 hover:text-zinc-100"
                  >
                    <Plus className="h-3.5 w-3.5" />
                    Add
                  </Button>
                </div>
                {buildArgs.length === 0 && (
                  <p className="text-[11px] text-zinc-600">
                    None — add a key/value pair to pass <code className="font-mono">--build-arg</code>.
                  </p>
                )}
                {buildArgs.map((arg, i) => (
                  <div key={i} className="flex gap-2">
                    <input
                      type="text"
                      value={arg.key}
                      onChange={(e) =>
                        setBuildArgs((prev) =>
                          prev.map((a, j) => (j === i ? { ...a, key: e.target.value } : a))
                        )
                      }
                      placeholder="KEY"
                      className="h-9 w-2/5 rounded-lg border border-zinc-800 bg-zinc-900/60 px-2.5 font-mono text-xs text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
                    />
                    <input
                      type="text"
                      value={arg.value}
                      onChange={(e) =>
                        setBuildArgs((prev) =>
                          prev.map((a, j) => (j === i ? { ...a, value: e.target.value } : a))
                        )
                      }
                      placeholder="value"
                      className="h-9 flex-1 rounded-lg border border-zinc-800 bg-zinc-900/60 px-2.5 font-mono text-xs text-white outline-none focus:border-zinc-700 placeholder:text-zinc-600"
                    />
                    <Button
                      variant="ghost"
                      size="icon"
                      onClick={() => setBuildArgs((prev) => prev.filter((_, j) => j !== i))}
                      className="h-9 w-9 text-zinc-500 hover:text-rose-400"
                      title="Remove"
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  </div>
                ))}
              </div>

              <div className="rounded-lg border border-zinc-800/70 bg-zinc-900/40 px-3 py-2">
                <div className="mb-1 flex items-center gap-1.5 text-[10px] uppercase tracking-wider text-zinc-500">
                  <Terminal className="h-3 w-3" />
                  Equivalent command
                </div>
                <code className="block break-all font-mono text-[11px] text-zinc-400">
                  {preview}
                </code>
              </div>
            </div>
          ) : (
            <div className="flex h-[46vh] min-h-[240px] flex-col gap-2">
              <div className="flex shrink-0 items-center justify-between gap-3 text-xs">
                <span className="truncate font-mono text-zinc-400">{tag}</span>
                <div className="flex shrink-0 items-center gap-3">
                  {startedAt !== null && (
                    <span className="tabular-nums text-zinc-500">{formatElapsed(elapsed)}</span>
                  )}
                  {pct !== null && <span className="tabular-nums text-zinc-400">{pct}%</span>}
                  <button
                    type="button"
                    title="Copy build logs"
                    onClick={() => {
                      const all = lines.map((l) => l.text).join("\n");
                      if (!all) return;
                      navigator.clipboard.writeText(all);
                      toast.success("Logs copied", "Build logs copied to clipboard.");
                    }}
                    className="flex items-center gap-1 text-[11px] text-zinc-400 hover:text-white transition-colors"
                  >
                    <Copy className="h-3.5 w-3.5" />
                    <span>Copy</span>
                  </button>
                </div>
              </div>
              {pct !== null && (
                <div className="h-1 shrink-0 overflow-hidden rounded-none bg-zinc-800">
                  <div
                    className="h-full rounded-none bg-cyan-500 transition-[width] duration-200"
                    style={{ width: `${pct}%` }}
                  />
                </div>
              )}
              <div
                ref={logRef}
                onScroll={handleLogScroll}
                className="min-h-0 flex-1 overflow-hidden rounded-lg border border-zinc-800/70 bg-black/40 p-1"
              >
                <TerminalViewer logs={lines.map((l) => l.text).join("\n")} />
              </div>
              {status && (
                <p className="shrink-0 truncate text-[11px] text-zinc-500">{status}</p>
              )}
              {failure && (
                <p className="shrink-0 break-all text-[11px] text-rose-400">{failure}</p>
              )}
            </div>
          )}
        </div>

        <div className="flex shrink-0 items-center justify-between gap-2 border-t border-zinc-800 px-5 py-3.5">
          <div className="min-w-0 text-[11px] text-zinc-500">
            {phase === "succeeded" && imageId && (
              <span className="font-mono text-zinc-400">
                {imageId.startsWith("sha256:") ? imageId.slice(7, 19) : imageId.slice(0, 12)}
              </span>
            )}
          </div>
          <div className="flex shrink-0 gap-2">
            {running ? (
              <>
                <Button variant="secondary" onClick={handleCancel}>
                  Cancel build
                </Button>
              </>
            ) : phase === "form" ? (
              <>
                <Button variant="ghost" onClick={handleClose}>
                  Cancel
                </Button>
                <Button variant="primary" onClick={handleSubmit} disabled={!canBuild}>
                  <Hammer className="h-4 w-4" />
                  Build
                </Button>
              </>
            ) : (
              <>
                {phase === "succeeded" && onRunImage && (
                  <Button
                    variant="primary"
                    onClick={() => {
                      onRunImage(tag);
                      onClose();
                    }}
                  >
                    <Play className="h-4 w-4" />
                    Run container
                  </Button>
                )}
                {phase === "succeeded" && (
                  <Button
                    variant="secondary"
                    onClick={() => {
                      toast.info("Push", "Pushing images is not implemented yet.");
                    }}
                  >
                    <Upload className="h-4 w-4" />
                    Push
                  </Button>
                )}
                <Button
                  variant="ghost"
                  onClick={() => {
                    setPhase("form");
                    setLines([]);
                    setFailure(null);
                    setImageId(null);
                    setStatus("");
                    setStartedAt(null);
                    setElapsed(0);
                  }}
                >
                  <RotateCcw className="h-4 w-4" />
                  Build again
                </Button>
                <Button variant="secondary" onClick={handleClose}>
                  Close
                </Button>
              </>
            )}
          </div>
        </div>
      </motion.div>
    </div>
  );
};
