import React, { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { api, events } from "../../lib/api";
import type { ExecOutputPayload } from "../../types";

interface InteractiveTerminalProps {
  containerId: string;
  /** true = start the container first (the "Debug" flow on stopped containers). */
  debug?: boolean;
  /** Command override; `null` opens the configured shell. */
  cmd?: string[] | null;
  /** Matches the app's terminal font-size setting. */
  fontSize?: number;
  /** Called once the session is live (the Debug flow may have started it). */
  onSessionLive?: () => void;
  /** Called when the remote process has exited (eof). */
  onSessionEnded?: () => void;
}

type SessionStatus = "connecting" | "live" | "ended" | "error";

const FONT_FAMILY =
  "'JetBrains Mono', 'Fira Code', Menlo, Consolas, monospace";

function decodeBase64Chunk(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/**
 * A live, interactive terminal bound to one exec session. Keystrokes flow to
 * the container through Tauri commands; output streams back as `exec-output`
 * events. Supports the Debug flow (starts a stopped container first).
 */
export const InteractiveTerminal: React.FC<InteractiveTerminalProps> = ({
  containerId,
  debug = false,
  cmd = null,
  fontSize = 12,
  onSessionLive,
  onSessionEnded,
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const sessionRef = useRef<string | null>(null);
  const pendingRef = useRef<ExecOutputPayload[]>([]);
  const endedRef = useRef(false);
  const [status, setStatus] = useState<SessionStatus>("connecting");
  const [error, setError] = useState<string | null>(null);
  // Keeps the latest callback without re-running the session effect.
  const liveCallbackRef = useRef(onSessionLive);
  liveCallbackRef.current = onSessionLive;
  const endedCallbackRef = useRef(onSessionEnded);
  endedCallbackRef.current = onSessionEnded;

  const finishSession = useCallback(() => {
    if (endedRef.current) return;
    endedRef.current = true;
    setStatus("ended");
    const term = termRef.current;
    if (term) {
      term.write("\r\n\x1b[90m[process exited]\x1b[0m\r\n");
      // stop accepting input once the remote side is gone
      term.options.disableStdin = true;
    }
    endedCallbackRef.current?.();
  }, []);

  // Set up the xterm instance and wire stdin/output to the backend session.
  useEffect(() => {
    const host = containerRef.current;
    if (!host) return;

    const term = new Terminal({
      cursorBlink: true,
      disableStdin: true, // enabled once the session is live
      fontSize,
      fontFamily: FONT_FAMILY,
      theme: {
        background: "#09090b",
        foreground: "#f4f4f5",
        cursor: "#22c55e",
      },
      convertEol: false,
    });
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(host);
    termRef.current = term;
    fitAddonRef.current = fitAddon;

    try {
      fitAddon.fit();
    } catch {
      // container may not be laid out yet; ResizeObserver will refit
    }

    let disposed = false;
    let resizeTimer: number | undefined;

    const handleOutput = (payload: ExecOutputPayload) => {
      if (payload.eof) {
        finishSession();
        return;
      }
      if (!payload.data) return;
      try {
        term.write(decodeBase64Chunk(payload.data));
      } catch {
        // atob can throw on malformed base64; ignore the bad chunk
      }
    };

    // Subscribe BEFORE starting the session so early output isn't dropped.
    let unlisten: (() => void) | null = null;
    let startPromise: Promise<string> | null = null;

    events
      .onExecOutput((payload) => {
        if (disposed) return;
        if (sessionRef.current === null) {
          // session id not known yet — hold the chunk until it is
          pendingRef.current.push(payload);
          return;
        }
        if (payload.session_id !== sessionRef.current) return;
        handleOutput(payload);
      })
      .then((fn) => {
        if (disposed) {
          fn();
          return;
        }
        unlisten = fn;
        startPromise = debug
          ? api.debugContainer(containerId, cmd, term.cols, term.rows)
          : api.startInteractiveExec(containerId, cmd, term.cols, term.rows);
        return startPromise
          .then((sessionId) => {
            if (disposed) {
              void api.stopExecSession(sessionId).catch(() => {});
              return;
            }
            sessionRef.current = sessionId;
            // flush anything that arrived before we knew our session id
            const pending = pendingRef.current;
            pendingRef.current = [];
            for (const p of pending) {
              if (p.session_id === sessionId) handleOutput(p);
            }
            term.options.disableStdin = false;
            setStatus("live");
            term.focus();
            // Re-assert the size now that the pty exists on the engine.
            void api
              .resizeExecSession(sessionId, term.cols, term.rows)
              .catch(() => {});
            liveCallbackRef.current?.();
          })
          .catch((e: unknown) => {
            if (disposed) return;
            const message = typeof e === "string" ? e : String(e);
            setError(message);
            setStatus("error");
            term.write(
              `\r\n\x1b[31m${message.replace(/[\r\n]+/g, " ")}\x1b[0m\r\n`
            );
          });
      })
      .catch(() => {
        if (!disposed) {
          setError("Failed to subscribe to terminal output");
          setStatus("error");
        }
      });

    const dataDisposable = term.onData((data) => {
      const sessionId = sessionRef.current;
      if (!sessionId || endedRef.current) return;
      void api.writeExecStdin(sessionId, data).catch(() => {
        // session may have just closed; eof handling covers the UI state
      });
    });

    const observer = new ResizeObserver(() => {
      window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(() => {
        try {
          fitAddon.fit();
        } catch {
          return;
        }
        const sessionId = sessionRef.current;
        if (!sessionId || endedRef.current) return;
        void api
          .resizeExecSession(sessionId, term.cols, term.rows)
          .catch(() => {});
      }, 120);
    });
    observer.observe(host);

    return () => {
      disposed = true;
      window.clearTimeout(resizeTimer);
      observer.disconnect();
      dataDisposable.dispose();
      unlisten?.();
      const sessionId = sessionRef.current;
      sessionRef.current = null;
      pendingRef.current = [];
      if (sessionId) {
        void api.stopExecSession(sessionId).catch(() => {});
      }
      // If start is still in flight, close the session once it resolves.
      if (startPromise) {
        startPromise
          .then((sid) => api.stopExecSession(sid))
          .catch(() => {});
      }
      term.dispose();
      termRef.current = null;
      fitAddonRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [containerId, debug, cmd?.join("\u0000"), fontSize]);

  if (status === "error" && error) {
    return (
      <div className="h-full w-full flex flex-col items-center justify-center gap-3 bg-zinc-950 rounded-lg border border-zinc-800 p-6 text-center">
        <p className="text-sm text-red-400 max-w-md break-words">{error}</p>
        <p className="text-xs text-zinc-500">
          {debug
            ? "The container could not be started for debugging."
            : "The terminal session could not be opened."}
        </p>
      </div>
    );
  }

  return (
    <div className="relative h-full w-full">
      {status === "connecting" && (
        <div className="absolute inset-0 z-10 flex items-center justify-center bg-zinc-950/70">
          <span className="text-xs text-zinc-400 animate-pulse">
            {debug ? "Starting container & attaching…" : "Connecting…"}
          </span>
        </div>
      )}
      <div ref={containerRef} className="h-full w-full overflow-hidden" />
    </div>
  );
};
