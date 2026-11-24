import { useCallback, useState } from "react";

export function useSeries<T>(maxPoints = 90, initial: T[] = []) {
  // Copied: callers seed from a module-level cache they keep appending to,
  // and sharing the array made the first push after a remount land twice.
  const [samples, setSamples] = useState<T[]>(() => initial.slice(-maxPoints));
  const push = useCallback(
    (s: T) => {
      setSamples((p) => {
        const next = [...p, s];
        if (next.length > maxPoints) {
          return next.slice(next.length - maxPoints);
        }
        return next;
      });
    },
    [maxPoints]
  );
  const reset = useCallback(() => setSamples([]), []);
  return { samples, push, reset };
}
