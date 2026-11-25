/**
 * Line-based helpers for YAML that came out of `kubectl get -o yaml`.
 *
 * Pure: no DOM, no IPC. It relies on the layout kubectl prints: two-space
 * indentation, top-level keys at column 0, and `metadata` fields at column 2.
 * It is not a YAML parser; it only removes the server-owned parts that make
 * a round trip through `kubectl apply` noisy or wrong.
 */

/** metadata fields the API server owns; applying them back is noise. */
const METADATA_DROP = new Set([
  "managedFields",
  "resourceVersion",
  "uid",
  "creationTimestamp",
  "generation",
]);

/** Annotation kubectl writes on every apply; copying it back is pointless. */
const LAST_APPLIED = "kubectl.kubernetes.io/last-applied-configuration";

const indentOf = (line: string): number => line.length - line.trimStart().length;

/**
 * Removes server-owned fields from a `kubectl get -o yaml` document:
 * the top-level `status:` block, `metadata.managedFields`,
 * `metadata.resourceVersion`, `metadata.uid`, `metadata.creationTimestamp`,
 * `metadata.generation`, and the `last-applied-configuration` annotation.
 * An `annotations:` key left with no entries is removed as well.
 */
export function stripServerFields(yaml: string): string {
  const out: string[] = [];
  // Set while `annotations:` children are being collected; flushed to `out`
  // only if at least one child survives.
  let annotations: string[] | null = null;
  const flushAnnotations = () => {
    if (annotations && annotations.length > 1) out.push(...annotations);
    annotations = null;
  };

  // Indent of the key whose children are being dropped; -1 when not dropping.
  // Children are deeper lines, plus `- ` list items at the key's own indent.
  let dropIndent = -1;
  let section: "status" | "metadata" | "other" = "other";

  for (const line of yaml.replace(/\r\n?/g, "\n").split("\n")) {
    const blank = line.trim() === "";

    if (!blank && !line.startsWith(" ")) {
      // A new top-level key ends whatever block was open.
      flushAnnotations();
      dropIndent = -1;
      if (line.startsWith("status:")) section = "status";
      else if (line.startsWith("metadata:")) section = "metadata";
      else section = "other";
      if (section !== "status") out.push(line);
      continue;
    }

    if (section === "status") continue;
    if (section !== "metadata") {
      out.push(line);
      continue;
    }

    if (blank) {
      if (dropIndent < 0 && annotations === null) out.push(line);
      continue;
    }

    const ind = indentOf(line);
    if (dropIndent >= 0) {
      const isChild = ind > dropIndent || (ind === dropIndent && /^\s*- /.test(line));
      if (isChild) continue;
      dropIndent = -1;
    }

    if (ind === 2) {
      const key = /^ {2}([^\s:]+):/.exec(line)?.[1];
      if (key && METADATA_DROP.has(key)) {
        flushAnnotations();
        dropIndent = 2;
        continue;
      }
      if (key === "annotations") {
        flushAnnotations();
        annotations = [line];
        continue;
      }
      flushAnnotations();
      out.push(line);
      continue;
    }

    if (annotations !== null) {
      if (line.startsWith(`    ${LAST_APPLIED}:`)) {
        // Its value may be a block scalar on the following lines; they are
        // dropped through the dropIndent check above.
        dropIndent = 4;
        continue;
      }
      annotations.push(line);
      continue;
    }

    out.push(line);
  }

  flushAnnotations();
  return out.join("\n");
}
