/**
 * Shares one fetch between callers that poll the same answer.
 *
 * Concurrent callers join the request in flight; callers within `ttlMs` of a
 * finished request get its value without a new round-trip. `invalidate` drops
 * the cached value and detaches any request still in flight, so a read that
 * starts after a mutation never returns the pre-mutation answer (a result that
 * lands after invalidation is not cached either).
 */
export interface CoalescedFetch<T> {
  get(): Promise<T>;
  invalidate(): void;
}

export function createCoalescedFetch<T>(
  load: () => Promise<T>,
  ttlMs: number,
  now: () => number = Date.now
): CoalescedFetch<T> {
  let value: { at: number; data: T } | null = null;
  let inflight: Promise<T> | null = null;
  let generation = 0;

  return {
    get() {
      if (inflight) return inflight;
      if (value && now() - value.at < ttlMs) return Promise.resolve(value.data);
      const gen = generation;
      const request: Promise<T> = load()
        .then((data) => {
          if (gen === generation) value = { at: now(), data };
          return data;
        })
        .finally(() => {
          if (inflight === request) inflight = null;
        });
      inflight = request;
      return request;
    },
    invalidate() {
      generation += 1;
      value = null;
      inflight = null;
    },
  };
}
