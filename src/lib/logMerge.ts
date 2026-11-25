/**
 * Incremental merge of `docker logs --timestamps` output for live views.
 *
 * The first read takes the last N lines; every refresh after it asks only for
 * lines since the newest second already shown (`since` has one-second
 * granularity and is inclusive), so the lines of that boundary second come
 * back again and are dropped here. Pure: no DOM, no Tauri IPC.
 */

export interface LogBuffer {
  /** Raw lines, oldest first, each `<RFC3339Nano timestamp> <message>`. */
  lines: string[];
  /** Unix seconds of the newest timestamped line; 0 when none was seen. */
  lastSec: number;
}

/**
 * Milliseconds of a Docker RFC3339Nano timestamp. The fraction is cut to
 * milliseconds first: engines differ on how many fraction digits `Date.parse`
 * accepts, and Docker writes up to nine.
 */
export function parseDockerTime(stamp: string): number {
  return Date.parse(stamp.replace(/(\.\d{3})\d+/, "$1"));
}

/** Unix seconds of a line's leading Docker timestamp, or null without one. */
export function lineSeconds(line: string): number | null {
  const space = line.indexOf(" ");
  if (space < 20 || !/^\d{4}-/.test(line)) return null;
  const ms = parseDockerTime(line.slice(0, space));
  return Number.isNaN(ms) ? null : Math.floor(ms / 1000);
}

function splitLines(chunk: string): string[] {
  return chunk.split("\n").filter((l) => l.trim() !== "");
}

function newestSeconds(lines: string[], fallback: number): number {
  for (let i = lines.length - 1; i >= 0; i--) {
    const sec = lineSeconds(lines[i]);
    if (sec !== null) return sec;
  }
  return fallback;
}

/**
 * The `since` value for the next read: the newest second shown, or
 * undefined when the buffer is empty (take a fresh tail instead).
 */
export function nextSince(buffer: LogBuffer | undefined): number | undefined {
  return buffer && buffer.lastSec > 0 ? buffer.lastSec : undefined;
}

/**
 * Appends a fetched chunk to `prev`, keeping at most `cap` newest lines.
 * Without `prev` the chunk is a fresh tail. Returns `prev` itself when the
 * chunk held nothing new, so a React state update can bail out.
 */
export function mergeLogChunk(
  prev: LogBuffer | undefined,
  chunk: string,
  cap: number
): LogBuffer {
  const incoming = splitLines(chunk);
  if (!prev) {
    const lines = incoming.length > cap ? incoming.slice(-cap) : incoming;
    return { lines, lastSec: newestSeconds(lines, 0) };
  }

  // Lines already shown for the boundary second, counted so a line that
  // legitimately repeats within that second is still matched one-for-one.
  const seen = new Map<string, number>();
  for (let i = prev.lines.length - 1; i >= 0; i--) {
    const sec = lineSeconds(prev.lines[i]);
    if (sec !== null && sec < prev.lastSec) break;
    seen.set(prev.lines[i], (seen.get(prev.lines[i]) ?? 0) + 1);
  }

  const fresh: string[] = [];
  for (const line of incoming) {
    const sec = lineSeconds(line);
    if (sec !== null && sec < prev.lastSec) continue;
    const count = seen.get(line) ?? 0;
    if (count > 0) {
      seen.set(line, count - 1);
      continue;
    }
    fresh.push(line);
  }
  if (fresh.length === 0) return prev;

  const merged = prev.lines.concat(fresh);
  const lines = merged.length > cap ? merged.slice(-cap) : merged;
  return { lines, lastSec: newestSeconds(fresh, prev.lastSec) };
}
