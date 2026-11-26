import { useCallback, useEffect, useRef, type Dispatch, type SetStateAction } from "react";
import type { ComposeOp } from "../components/compose/ComposeProgressPanel";
import { appendOpLines, type OpLine } from "../lib/composeOpLines";

/**
 * Returns a push function for compose progress lines. A long pull emits many
 * lines a second; each used to be a separate state update (and a re-render of
 * the page). Lines are queued and applied once per animation frame instead.
 * The pending frame is cancelled on unmount so no update lands afterwards.
 */
export function useOpLineBatcher(
  setOps: Dispatch<SetStateAction<ComposeOp[]>>
): (opId: string, line: string) => void {
  const queue = useRef<OpLine[]>([]);
  const frame = useRef(0);

  useEffect(
    () => () => {
      if (frame.current) cancelAnimationFrame(frame.current);
      frame.current = 0;
      queue.current = [];
    },
    []
  );

  return useCallback(
    (opId: string, line: string) => {
      queue.current.push({ opId, line });
      if (frame.current) return;
      frame.current = requestAnimationFrame(() => {
        frame.current = 0;
        const batch = queue.current;
        queue.current = [];
        if (batch.length > 0) setOps((prev) => appendOpLines(prev, batch));
      });
    },
    [setOps]
  );
}
