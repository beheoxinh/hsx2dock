import { useCallback, useEffect, useMemo, useState } from "react";
import { disposeListener, events } from "../../lib/api";
import { newOpId } from "../../lib/composeProject";
import { useToast } from "../ui/Toast";
import { useOpLineBatcher } from "../../hooks/useOpLineBatcher";
import type { ComposeOp } from "./ComposeProgressPanel";

/** Result of a drawer operation: the value on success, otherwise just a failure flag. */
export type DrawerRunResult<T> = { ok: true; value: T } | { ok: false };

/** Keeps the drawer's own output list short. */
const MAX_OPS = 10;

/**
 * Operations started from inside the drawer (pull, build, volume backup and
 * restore). They use the same op id and `compose-progress` stream as the
 * page's operations, but the page's runOp is not reachable from here, so the
 * drawer tracks its own list and shows the output itself.
 */
export function useDrawerOps() {
  const toast = useToast();
  const [ops, setOps] = useState<ComposeOp[]>([]);
  // Output lines are applied once per frame, not one re-render per line.
  const pushLine = useOpLineBatcher(setOps);

  useEffect(() => {
    return disposeListener(events.onComposeProgress(({ op_id, line }) => pushLine(op_id, line)));
  }, [pushLine]);

  const run = useCallback(
    async <T,>(project: string, title: string, fn: (opId: string) => Promise<T>): Promise<DrawerRunResult<T>> => {
      const id = newOpId();
      setOps((prev) => [...prev.slice(-(MAX_OPS - 1)), { id, project, title, status: "running", lines: [], started: Date.now() }]);
      try {
        const value = await fn(id);
        setOps((prev) => prev.map((o) => (o.id === id ? { ...o, status: "ok", finished: Date.now() } : o)));
        return { ok: true, value };
      } catch (e) {
        const error = String(e);
        setOps((prev) => prev.map((o) => (o.id === id ? { ...o, status: "error", error, finished: Date.now() } : o)));
        toast.error(`${title} failed`, error.length > 300 ? `${error.slice(0, 300)}…` : error);
        return { ok: false };
      }
    },
    [toast]
  );

  const busy = useMemo(() => ops.some((o) => o.status === "running"), [ops]);
  const latest = ops.length ? ops[ops.length - 1] : null;
  return { ops, latest, busy, run };
}
