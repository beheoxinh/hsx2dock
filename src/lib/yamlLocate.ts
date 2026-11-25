/**
 * Maps a `docker compose config` error back to a line of the YAML text, so
 * the editor can mark and jump to it. Compose reports either a parser line
 * (`yaml: line 4: …`) or a key path (`services.web.portz …`,
 * `service "web" depends on …`).
 */

function indentOf(line: string): number {
  return line.length - line.trimStart().length;
}

function isContent(line: string): boolean {
  const t = line.trim();
  return t !== "" && !t.startsWith("#");
}

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * 1-based line of the deepest key of `path` found in `text`
 * (`["services","web","ports"]`), or null when even the first key is missing.
 */
export function findYamlPath(text: string, path: string[]): number | null {
  const lines = text.split("\n");
  let from = 0;
  let parentIndent = -1;
  let found: number | null = null;
  for (const seg of path) {
    const keyRe = new RegExp(`^(\\s*)(?:-\\s+)?["']?${escapeRe(seg)}["']?\\s*:`);
    let hit = -1;
    for (let i = from; i < lines.length; i++) {
      const line = lines[i];
      if (!isContent(line)) continue;
      const ind = indentOf(line);
      // Left the parent's block without finding the key.
      if (i > from && ind <= parentIndent) break;
      if (ind > parentIndent && keyRe.test(line)) {
        hit = i;
        break;
      }
    }
    if (hit < 0) break;
    found = hit + 1;
    parentIndent = indentOf(lines[hit]);
    from = hit + 1;
  }
  return found;
}

export interface ErrorLocation {
  line: number;
}

/** Best-effort line for a compose validation error. */
export function locateComposeError(error: string, text: string): ErrorLocation | null {
  const yamlLine = error.match(/\byaml: (?:line|.*?line) (\d+)/i) ?? error.match(/\bline (\d+)\b/i);
  if (yamlLine) {
    const n = Number(yamlLine[1]);
    const total = text.split("\n").length;
    if (n >= 1) return { line: Math.min(n, total) };
  }
  const svc = error.match(/service "([^"]+)"/);
  const keyPath = error.match(/\b((?:services|networks|volumes|configs|secrets)(?:\.[\w-]+)+)/);
  const segs = keyPath ? keyPath[1].split(".") : svc ? ["services", svc[1]] : null;
  if (!segs) return null;
  const line = findYamlPath(text, segs);
  return line ? { line } : null;
}
