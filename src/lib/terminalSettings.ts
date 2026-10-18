import { useEffect, useState } from "react";
import { api } from "./api";

export interface TerminalSettings {
  /** Shell opened by the terminal / Debug flow (used by the backend default). */
  shell: string;
  /** Font size (px) for terminal panes. */
  fontSize: number;
}

const FALLBACK: TerminalSettings = { shell: "sh", fontSize: 12 };

let cached: TerminalSettings | null = null;

/** Drops the module-level cache; call after the settings are saved. */
export function invalidateTerminalSettings(): void {
  cached = null;
}

async function load(): Promise<TerminalSettings> {
  if (cached) return cached;
  try {
    const cfg = await api.getAppConfig();
    const shell = cfg.terminal_shell?.trim() || FALLBACK.shell;
    const fontSize =
      typeof cfg.terminal_font_size === "number" && cfg.terminal_font_size > 0
        ? Math.min(Math.max(cfg.terminal_font_size, 8), 32)
        : FALLBACK.fontSize;
    cached = { shell, fontSize };
  } catch {
    cached = FALLBACK;
  }
  return cached;
}

/**
 * Terminal font size from the app config, cached across panes. Returns the
 * fallback (12) while loading.
 */
export function useTerminalFontSize(): number {
  const [fontSize, setFontSize] = useState<number>(cached?.fontSize ?? FALLBACK.fontSize);

  useEffect(() => {
    let alive = true;
    load().then((s) => {
      if (alive) setFontSize(s.fontSize);
    });
    return () => {
      alive = false;
    };
  }, []);

  return fontSize;
}
