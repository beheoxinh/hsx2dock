/**
 * Applies buffered compose progress lines to the op list in one pass.
 * The page queues lines per animation frame (see useOpLineBatcher), so a long
 * pull is one state update per frame instead of one per output line.
 */
import type { ComposeOp } from "../components/compose/ComposeProgressPanel";

/** Lines kept per operation; the oldest fall off first. */
export const MAX_OP_LINES = 400;

export interface OpLine {
  opId: string;
  line: string;
}

export function appendOpLines(ops: ComposeOp[], batch: readonly OpLine[]): ComposeOp[] {
  let next: ComposeOp[] | null = null;
  for (const { opId, line } of batch) {
    const i = (next ?? ops).findIndex((o) => o.id === opId);
    if (i < 0) continue;
    if (!next) next = [...ops];
    const op = next[i];
    const lines =
      op.lines.length >= MAX_OP_LINES ? [...op.lines.slice(-MAX_OP_LINES + 1), line] : [...op.lines, line];
    next[i] = { ...op, lines };
  }
  return next ?? ops;
}
