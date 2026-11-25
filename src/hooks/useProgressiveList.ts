import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Renders a long list in pages: the first `step` rows at once, the next page
 * when a sentinel after the last row scrolls into view. Hundreds of images or
 * volumes used to mount (and layout-animate) every row on each refresh, which
 * froze the WebView for seconds.
 *
 * `resetKey` (the search text, a filter) starts again from the first page.
 */
export function useProgressiveList<T>(items: T[], resetKey: unknown = null, step = 60) {
  const [limit, setLimit] = useState(step);
  useEffect(() => setLimit(step), [resetKey, step]);

  const hasMore = items.length > limit;
  const observer = useRef<IntersectionObserver | null>(null);
  const sentinelRef = useCallback(
    (node: HTMLElement | null) => {
      observer.current?.disconnect();
      observer.current = null;
      if (!node || !hasMore || typeof IntersectionObserver === "undefined") return;
      observer.current = new IntersectionObserver(
        (entries) => {
          if (entries.some((e) => e.isIntersecting)) setLimit((l) => l + step);
        },
        // Start loading a little before the end is reached.
        { rootMargin: "400px 0px" }
      );
      observer.current.observe(node);
    },
    [hasMore, step]
  );
  useEffect(() => () => observer.current?.disconnect(), []);

  return {
    visible: hasMore ? items.slice(0, limit) : items,
    hasMore,
    sentinelRef,
    /** Per-row layout animations are only affordable for short lists. */
    animate: items.length <= step,
  };
}
