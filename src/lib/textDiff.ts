/** Line diff for showing what a save will change. */

export type DiffKind = "same" | "add" | "del";

export interface DiffLine {
  kind: DiffKind;
  text: string;
  /** 1-based line in the old text (same/del). */
  oldNo?: number;
  /** 1-based line in the new text (same/add). */
  newNo?: number;
}

/** Above this many cells the LCS table is skipped for a plain replace. */
const MAX_CELLS = 4_000_000;

function splitLines(text: string): string[] {
  if (text === "") return [];
  const lines = text.replace(/\r\n/g, "\n").split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  return lines;
}

export function diffLines(oldText: string, newText: string): DiffLine[] {
  const a = splitLines(oldText);
  const b = splitLines(newText);
  // Common prefix and suffix need no table.
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const out: DiffLine[] = [];
  for (let i = 0; i < start; i++) out.push({ kind: "same", text: a[i], oldNo: i + 1, newNo: i + 1 });

  const midA = a.slice(start, endA);
  const midB = b.slice(start, endB);
  const n = midA.length;
  const m = midB.length;
  if (n * m > MAX_CELLS) {
    midA.forEach((t, i) => out.push({ kind: "del", text: t, oldNo: start + i + 1 }));
    midB.forEach((t, j) => out.push({ kind: "add", text: t, newNo: start + j + 1 }));
  } else {
    // lcs[i][j] = LCS length of midA[i..] and midB[j..]
    const lcs: Uint32Array[] = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
    for (let i = n - 1; i >= 0; i--) {
      for (let j = m - 1; j >= 0; j--) {
        lcs[i][j] = midA[i] === midB[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
      }
    }
    let i = 0;
    let j = 0;
    while (i < n || j < m) {
      if (i < n && j < m && midA[i] === midB[j]) {
        out.push({ kind: "same", text: midA[i], oldNo: start + i + 1, newNo: start + j + 1 });
        i++;
        j++;
      } else if (i < n && (j === m || lcs[i + 1][j] >= lcs[i][j + 1])) {
        // Removals before additions, like unified diffs.
        out.push({ kind: "del", text: midA[i], oldNo: start + i + 1 });
        i++;
      } else {
        out.push({ kind: "add", text: midB[j], newNo: start + j + 1 });
        j++;
      }
    }
  }

  for (let k = 0; k < a.length - endA; k++) {
    out.push({ kind: "same", text: a[endA + k], oldNo: endA + k + 1, newNo: endB + k + 1 });
  }
  return out;
}

export function diffCounts(lines: DiffLine[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const l of lines) {
    if (l.kind === "add") added++;
    else if (l.kind === "del") removed++;
  }
  return { added, removed };
}

/**
 * Only the changed lines with `context` unchanged lines around them; gaps
 * become `null` separators.
 */
export function withContext(lines: DiffLine[], context = 3): (DiffLine | null)[] {
  const keep = new Array<boolean>(lines.length).fill(false);
  lines.forEach((l, i) => {
    if (l.kind === "same") return;
    for (let k = Math.max(0, i - context); k <= Math.min(lines.length - 1, i + context); k++) keep[k] = true;
  });
  const out: (DiffLine | null)[] = [];
  let gap = false;
  lines.forEach((l, i) => {
    if (keep[i]) {
      if (gap && out.length > 0) out.push(null);
      gap = false;
      out.push(l);
    } else {
      gap = true;
    }
  });
  return out;
}
