/**
 * Pure helpers for the port-forward manager: port input validation, status
 * tone mapping and container port extraction from pod YAML.
 *
 * No DOM, no IPC. Run tests with:
 *   node --test src/lib/portForward.test.ts
 */

/** Ports 32768 and up are rejected because they are not reachable on this host. */
export const MAX_LOCAL_PORT = 32767;

export type PortParse = { ok: true; port: number } | { ok: false; error: string };

/**
 * Parses the local port field. Empty or whitespace means 0 (auto-pick).
 * 0 is also auto. 1..32767 is a fixed port.
 */
export function parseLocalPort(input: string): PortParse {
  const text = input.trim();
  if (text === "") return { ok: true, port: 0 };
  if (!/^\d+$/.test(text)) {
    return { ok: false, error: "Local port must be a whole number, or empty for auto." };
  }
  const port = Number(text);
  if (port === 0) return { ok: true, port: 0 };
  if (port > MAX_LOCAL_PORT) {
    return { ok: false, error: "Ports 32768 and up are not reachable on this host." };
  }
  return { ok: true, port };
}

/** Parses the remote (container or service) port. Must be 1..65535. */
export function parseRemotePort(input: string): PortParse {
  const text = input.trim();
  if (text === "") return { ok: false, error: "Remote port is required." };
  if (!/^\d+$/.test(text)) {
    return { ok: false, error: "Remote port must be a whole number." };
  }
  const port = Number(text);
  if (port < 1 || port > 65535) {
    return { ok: false, error: "Remote port must be between 1 and 65535." };
  }
  return { ok: true, port };
}

export type ForwardTone = "success" | "warning" | "info" | "danger" | "neutral";

/** Maps a backend forward status to a badge tone. Unknown states are neutral. */
export function forwardStatusTone(status: string): ForwardTone {
  switch (status) {
    case "active":
      return "success";
    case "reconnecting":
      return "warning";
    case "starting":
      return "info";
    case "error":
      return "danger";
    default:
      return "neutral";
  }
}

export interface ContainerPort {
  container: string;
  name: string | null;
  port: number;
  protocol: string;
}

const indentOf = (line: string): number => line.length - line.trimStart().length;

/** Matches `key: value` lines, optionally prefixed by `-<spaces>` (sequence item). */
const KEY_LINE = /^(\s*)(-\s+)?([A-Za-z][\w-]*):(?:\s+(.*))?$/;

const DASH_LINE = /^\s*-\s/;

/**
 * Lines that belong under a parent whose key sits at `parentIndent`: deeper
 * lines, plus `- ` items at the parent's own indent (kubectl prints lists
 * flush with their key). Blank lines are kept and ignored by callers.
 */
function childBlock(lines: string[], start: number, parentIndent: number): string[] {
  const out: string[] = [];
  for (let i = start; i < lines.length; i++) {
    const line = lines[i];
    if (line.trim() === "") continue;
    const ind = indentOf(line);
    if (ind > parentIndent || (ind === parentIndent && DASH_LINE.test(line))) {
      out.push(line);
    } else {
      break;
    }
  }
  return out;
}

/** Strips one pair of matching YAML quotes and surrounding whitespace. */
function scalar(value: string | undefined): string {
  const v = (value ?? "").trim();
  if (v.length >= 2 && ((v[0] === '"' && v.endsWith('"')) || (v[0] === "'" && v.endsWith("'")))) {
    return v.slice(1, -1);
  }
  return v;
}

/** Column of a `key:` line; a `-<spaces>` prefix counts toward the column. */
function keyColumn(line: string): { col: number; key: string; value: string } | null {
  const m = KEY_LINE.exec(line);
  if (!m) return null;
  return { col: m[1].length + (m[2]?.length ?? 0), key: m[3], value: m[4] ?? "" };
}

/** Splits a sequence block into items; each item starts at a dash at `dashIndent`. */
function splitItems(block: string[], dashIndent: number): string[][] {
  const items: string[][] = [];
  for (const line of block) {
    if (indentOf(line) === dashIndent && DASH_LINE.test(line)) {
      items.push([line]);
    } else if (items.length > 0) {
      items[items.length - 1].push(line);
    }
  }
  return items;
}

/**
 * Direct keys of one sequence item: the keys whose column matches the first
 * key of the item (the column after its dash).
 */
function directKeys(item: string[]): Map<string, { index: number; value: string }> {
  const keys = new Map<string, { index: number; value: string }>();
  const itemCol = keyColumn(item[0] ?? "")?.col ?? -1;
  item.forEach((line, index) => {
    const k = keyColumn(line);
    if (k && k.col === itemCol && !keys.has(k.key)) {
      keys.set(k.key, { index, value: k.value });
    }
  });
  return keys;
}

/** Indent of the first dash line in a block, or -1 when there is none. */
function dashIndentOf(block: string[]): number {
  const first = block.find((l) => DASH_LINE.test(l));
  return first ? indentOf(first) : -1;
}

/**
 * Lists the container ports of a pod manifest as printed by
 * `kubectl get pod -o yaml`. Reads `spec.containers[].ports[]` only (init and
 * ephemeral containers are not included). Ports without a valid containerPort
 * (1..65535) are skipped. Protocol defaults to TCP, as the API server does.
 *
 * This is an indentation-based reader, not a YAML parser. It handles the
 * layout kubectl prints and any consistent indentation of it.
 */
export function parsePodContainerPorts(yaml: string): ContainerPort[] {
  const lines = yaml.replace(/\r\n?/g, "\n").split("\n");
  const specIdx = lines.findIndex((l) => /^spec:\s*$/.test(l));
  if (specIdx < 0) return [];

  const specBlock = childBlock(lines, specIdx + 1, 0);
  const containersLine = specBlock.find((l) => /^\s*containers:\s*$/.test(l));
  if (!containersLine) return [];
  const containersIndent = indentOf(containersLine);
  const containersStart = specBlock.indexOf(containersLine) + 1;
  const containersBlock = childBlock(specBlock, containersStart, containersIndent);

  const containerDash = dashIndentOf(containersBlock);
  if (containerDash < 0) return [];

  const result: ContainerPort[] = [];
  for (const item of splitItems(containersBlock, containerDash)) {
    const keys = directKeys(item);
    const containerName = scalar(keys.get("name")?.value);

    const portsKey = keys.get("ports");
    if (!portsKey) continue;
    const portsIndent = keyColumn(item[portsKey.index])?.col ?? -1;
    const portsBlock = childBlock(item, portsKey.index + 1, portsIndent);
    const portDash = dashIndentOf(portsBlock);
    if (portDash < 0) continue;

    for (const portItem of splitItems(portsBlock, portDash)) {
      const pk = directKeys(portItem);
      const raw = scalar(pk.get("containerPort")?.value);
      if (!/^\d+$/.test(raw)) continue;
      const port = Number(raw);
      if (port < 1 || port > 65535) continue;

      const name = scalar(pk.get("name")?.value);
      const protocol = scalar(pk.get("protocol")?.value) || "TCP";
      result.push({
        container: containerName,
        name: name === "" ? null : name,
        port,
        protocol,
      });
    }
  }
  return result;
}
