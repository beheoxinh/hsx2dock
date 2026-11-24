import React, { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

interface TerminalViewerProps {
  logs: string;
}

export const TerminalViewer: React.FC<TerminalViewerProps> = ({ logs }) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  // What the terminal currently shows. Streaming logs only ever grow, so the
  // new tail is appended; clearing and re-parsing the whole buffer on every
  // change made long logs slower with each line.
  const writtenRef = useRef("");

  useEffect(() => {
    if (!containerRef.current) return;

    const term = new Terminal({
      cursorBlink: false,
      disableStdin: true,
      fontSize: 12,
      fontFamily: "'JetBrains Mono', 'Fira Code', Menlo, Consolas, monospace",
      theme: {
        background: "#09090b",
        foreground: "#f4f4f5",
        cursor: "#3b82f6",
        black: "#18181b",
        red: "#f43f5e",
        green: "#10b981",
        yellow: "#f59e0b",
        blue: "#3b82f6",
        magenta: "#d946ef",
        cyan: "#06b6d4",
        white: "#f4f4f5",
        brightBlack: "#71717a",
        brightRed: "#fb7185",
        brightGreen: "#34d399",
        brightYellow: "#fbbf24",
        brightBlue: "#60a5fa",
        brightMagenta: "#e879f9",
        brightCyan: "#22d3ee",
        brightWhite: "#ffffff",
      },
      convertEol: true,
      scrollback: 5000,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(containerRef.current);
    fitAddon.fit();

    // Support keyboard copy (Ctrl+C / Ctrl+Shift+C / Cmd+C) when text is selected
    term.attachCustomKeyEventHandler((event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c" && term.hasSelection()) {
        const text = term.getSelection();
        if (text) {
          navigator.clipboard.writeText(text).catch(() => {});
        }
        return false;
      }
      return true;
    });

    termRef.current = term;
    fitAddonRef.current = fitAddon;
    // A fresh terminal is empty, whatever an earlier instance had written.
    writtenRef.current = "";

    const handleResize = () => {
      try {
        fitAddon.fit();
      } catch (e) {
        // ignore resize error on unmounted
      }
    };
    window.addEventListener("resize", handleResize);

    return () => {
      window.removeEventListener("resize", handleResize);
      term.dispose();
    };
  }, []);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    const prev = writtenRef.current;
    if (prev && logs.startsWith(prev)) {
      const tail = logs.slice(prev.length);
      if (tail) term.write(tail);
    } else {
      // Replaced or trimmed content: start over.
      term.reset();
      term.write(logs);
      fitAddonRef.current?.fit();
    }
    writtenRef.current = logs;
  }, [logs]);

  return <div ref={containerRef} className="h-full w-full overflow-hidden" />;
};
