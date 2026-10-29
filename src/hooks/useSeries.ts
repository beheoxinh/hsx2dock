import { useCallback, useState } from "react";

export function useSeries<T>(maxPoints = 90, initial: T[] = []) {
  const [samples, setSamples] = useState<T[]>(initial);
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
