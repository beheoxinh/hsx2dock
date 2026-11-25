/**
 * Line-preserving `.env` model for the Compose env editor. Variables become
 * editable rows; comments, blank lines and anything else are kept verbatim so
 * a save never reformats the file beyond the rows that changed.
 */

export type EnvRow =
  | { kind: "var"; key: string; value: string; exported: boolean }
  | { kind: "raw"; text: string };

const VAR_LINE = /^(\s*)(export\s+)?([A-Za-z_][A-Za-z0-9_.-]*)\s*=(.*)$/;

export function parseEnv(text: string): EnvRow[] {
  if (text === "") return [];
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  // A trailing newline is not an extra blank row.
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines.map((line): EnvRow => {
    if (line.trimStart().startsWith("#")) return { kind: "raw", text: line };
    const m = line.match(VAR_LINE);
    if (!m) return { kind: "raw", text: line };
    return { kind: "var", key: m[3], value: m[4], exported: Boolean(m[2]) };
  });
}

export function serializeEnv(rows: EnvRow[]): string {
  if (rows.length === 0) return "";
  return (
    rows
      .map((r) => (r.kind === "raw" ? r.text : `${r.exported ? "export " : ""}${r.key}=${r.value}`))
      .join("\n") + "\n"
  );
}

export function isValidEnvKey(key: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_.-]*$/.test(key);
}

const SECRET_HINTS = ["PASSWORD", "PASSWD", "SECRET", "TOKEN", "KEY", "PRIVATE", "CREDENTIAL", "AUTH"];

/** Keys whose values are hidden until revealed. */
export function isSecretKey(key: string): boolean {
  const k = key.toUpperCase();
  return SECRET_HINTS.some((h) => k.includes(h));
}

/** Keys defined more than once (the last one wins in compose). */
export function duplicateKeys(rows: EnvRow[]): Set<string> {
  const seen = new Set<string>();
  const dup = new Set<string>();
  for (const r of rows) {
    if (r.kind !== "var") continue;
    if (seen.has(r.key)) dup.add(r.key);
    seen.add(r.key);
  }
  return dup;
}
