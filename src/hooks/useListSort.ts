import { useCallback, useState } from "react";
import { browserStorage } from "../lib/k8sClusterList";
import { readSort, writeSort, type SortState } from "../lib/listSort";

/** A list's sort choice, remembered per browser under `hsx2dock.sort.<list>`. */
export function useListSort<K extends string>(
  list: string,
  keys: readonly K[],
  fallback: SortState<K>
): [SortState<K>, (next: SortState<K>) => void] {
  const storageKey = `hsx2dock.sort.${list}`;
  const [state, setState] = useState<SortState<K>>(() =>
    readSort(browserStorage(), storageKey, keys, fallback)
  );
  const update = useCallback(
    (next: SortState<K>) => {
      setState(next);
      writeSort(browserStorage(), storageKey, next);
    },
    [storageKey]
  );
  return [state, update];
}
