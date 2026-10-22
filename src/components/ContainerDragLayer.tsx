import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";
import { dropIndexAt } from "../lib/containerGroups";

/**
 * Pointer-driven drag layer for container cards.
 *
 * The WebKitGTK webview Tauri uses on Linux does not deliver a reliable HTML5
 * drag-and-drop payload: `dataTransfer.setData` often arrives empty at the drop
 * handler and `dragover` is not fired continuously, so a drop silently does
 * nothing. Chromium (and therefore Playwright) implements it correctly, which
 * is exactly why the browser tests passed while the real app did not.
 *
 * Dragging is therefore built on plain pointer events — both ends live in this
 * module, and the dragged id travels in React state instead of through the
 * platform DnD channel:
 *
 *   - a card arms a drag on pointerdown, which becomes active after 5px
 *   - a floating ghost follows the cursor
 *   - drop targets register themselves and are hit-tested via elementFromPoint
 *   - releasing over a target calls the handler registered by the group bar
 */

/** What is currently being dragged. */
export interface DragPayload {
  id: string;
  /** Rendered in the ghost so the user sees what they grabbed. */
  label: string;
  /**
   * The group this card is currently in, when it is rendered inside one. The
   * layer only computes a reorder preview for that group — anywhere else the
   * gesture is a move, not a reorder. Whether a move is *allowed* is the group
   * bar's call (a compose member may only be reordered inside its project).
   */
  ownGroupId?: string;
  /** True for a compose-managed container, which may not be moved at all. */
  composeOwned?: boolean;
}

/** Live reorder preview inside a group while a card is dragged over it. */
export interface ReorderPreview {
  groupId: string;
  /** Stable key of the card the placeholder sits before. */
  beforeKey: string | null;
  /** Placement index, so a card in its original slot draws nothing. */
  index: number;
  /** True when the placement would leave the order unchanged. */
  unchanged: boolean;
}

interface DropTarget {
  /** `null` means "not in any group" (the Isolated bucket). */
  groupId: string | null;
  element: HTMLElement;
}

type DropHandler = (
  groupId: string | null,
  payload: DragPayload,
  preview?: ReorderPreview | null
) => void;

interface DragContextValue {
  /** Non-null while a drag is in progress. */
  payload: DragPayload | null;
  /** Group currently hovered, or null when over nothing / the isolated bucket. */
  overGroupId: string | null;
  /** True when the pointer is over the isolated bucket specifically. */
  overIsolated: boolean;
  /** Where the dragged card would land, while it hovers over a group it owns. */
  reorder: ReorderPreview | null;
  startDrag: (payload: DragPayload, event: React.PointerEvent) => void;
  registerTarget: (target: DropTarget) => () => void;
  /**
   * Registers the card stack of a group so the layer can work out the reorder
   * index from the pointer position. Keyed by group id.
   */
  registerMemberList: (
    groupId: string,
    getElements: () => HTMLElement[],
    getOrderedIds: () => string[]
  ) => () => void;
}

const DragContext = createContext<DragContextValue | null>(null);
const DropHandlerContext = createContext<
  ((fn: DropHandler | null) => void) | null
>(null);

/** Movement in px before a press becomes a drag (keeps plain clicks working). */
const DRAG_THRESHOLD_PX = 5;

/** Elements that keep their own pointer behaviour (buttons, inputs, links). */
const INTERACTIVE =
  "button, a, input, textarea, select, [role='button'], [data-no-drag]";

export const ContainerDragProvider: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const [payload, setPayload] = useState<DragPayload | null>(null);
  const [overGroupId, setOverGroupId] = useState<string | null>(null);
  const [overIsolated, setOverIsolated] = useState(false);
  const [reorder, setReorder] = useState<ReorderPreview | null>(null);

  // Refs, not state: these are read on every pointermove and must not re-create
  // the window listeners or re-render the tree per frame.
  const targets = useRef<DropTarget[]>([]);
  const memberLists = useRef<
    Map<
      string,
      { getElements: () => HTMLElement[]; getOrderedIds: () => string[] }
    >
  >(new Map());  const pending = useRef<{ payload: DragPayload; x: number; y: number } | null>(
    null
  );
  const active = useRef<DragPayload | null>(null);
  const dropHandler = useRef<DropHandler | null>(null);
  // Last preview computed, mirrored into a ref so pointerup can read it without
  // depending on a state update having flushed.
  const reorderRef = useRef<ReorderPreview | null>(null);
  // Set while a drag is live so the click that follows the drop is discarded
  // instead of opening the container drawer underneath the group card. It is a
  // timestamp rather than a flag: if no click follows the drag (browsers do not
  // always emit one), a sticky flag would swallow the user's *next* real click.
  const suppressClickUntil = useRef(0);
  /** Window in which the post-drag click is treated as part of the drag. */
  const CLICK_SUPPRESS_MS = 500;
  // The ghost is positioned by mutating its style directly. Keeping the cursor
  // position out of React state means a drag does not re-render every card on
  // the page once per frame.
  const ghost = useRef<HTMLDivElement | null>(null);
  const pointer = useRef({ x: 0, y: 0 });

  const registerTarget = useCallback((target: DropTarget) => {
    targets.current = [...targets.current, target];
    return () => {
      targets.current = targets.current.filter((t) => t !== target);
    };
  }, []);

  const registerMemberList = useCallback(
    (
      groupId: string,
      getElements: () => HTMLElement[],
      getOrderedIds: () => string[]
    ) => {
      const entry = { getElements, getOrderedIds };
      memberLists.current.set(groupId, entry);
      return () => {
        // Only drop the entry if this registration is still the live one: a
        // re-render installs the new one before the old cleanup runs.
        if (memberLists.current.get(groupId) === entry) {
          memberLists.current.delete(groupId);
        }
      };
    },
    []
  );

  /** Innermost registered target under a viewport point. */
  const targetAt = (x: number, y: number): DropTarget | null => {
    const el = document.elementFromPoint(x, y);
    if (!el) return null;
    for (let i = targets.current.length - 1; i >= 0; i--) {
      const t = targets.current[i];
      if (t.element.contains(el)) return t;
    }
    return null;
  };

  /**
   * Where the dragged card would land inside `groupId`, or null when that
   * group has no registered stack (empty group — the drop is an append).
   */
  const previewAt = (groupId: string, y: number): ReorderPreview | null => {
    const list = memberLists.current.get(groupId);
    if (!list) return null;
    const elements = list.getElements();
    const ordered = list.getOrderedIds();
    if (elements.length === 0) return null;
    const dragging = active.current;
    const index = dropIndexAt(elements, y, dragging?.id ?? null);
    const without = ordered.filter((id) => id !== dragging?.id);
    // `index` counts the cards above the pointer *excluding* the dragged one,
    // so the placement is a no-op exactly when it equals the dragged card's
    // original position — dropping a card onto itself must not dirty the order.
    const originalIndex = ordered.indexOf(dragging?.id ?? "");
    return {
      groupId,
      beforeKey: without[index] ?? null,
      index,
      unchanged: originalIndex === -1 || index === originalIndex,
    };
  };

  const startDrag = useCallback(
    (next: DragPayload, event: React.PointerEvent) => {
      // Primary button only — right-click keeps its context menu.
      if (event.button !== 0) return;
      // Buttons and inputs inside a card keep their own behaviour.
      if ((event.target as HTMLElement).closest(INTERACTIVE)) return;
      pending.current = { payload: next, x: event.clientX, y: event.clientY };
    },
    []
  );

  useEffect(() => {
    const moveGhost = () => {
      if (ghost.current) {
        ghost.current.style.left = `${pointer.current.x}px`;
        ghost.current.style.top = `${pointer.current.y}px`;
      }
    };

    const finish = () => {
      pending.current = null;
      active.current = null;
      reorderRef.current = null;
      setPayload(null);
      setOverGroupId(null);
      setOverIsolated(false);
      setReorder(null);
    };

    const onMove = (e: PointerEvent) => {
      pointer.current = { x: e.clientX, y: e.clientY };
      const p = pending.current;
      if (!active.current) {
        if (!p) return;
        const moved =
          Math.abs(e.clientX - p.x) > DRAG_THRESHOLD_PX ||
          Math.abs(e.clientY - p.y) > DRAG_THRESHOLD_PX;
        if (!moved) return;
        // The press became a drag: from here the gesture belongs to the layer.
        active.current = p.payload;
        suppressClickUntil.current = performance.now() + CLICK_SUPPRESS_MS;
        setPayload(p.payload);
        return;
      }
      moveGhost();
      const t = targetAt(e.clientX, e.clientY);
      setOverGroupId(t ? t.groupId : null);
      setOverIsolated(!!t && t.groupId === null);
      // Only the group that already holds the card gets a reorder preview;
      // anywhere else the layer is showing a move, not a reorder.
      const owning = active.current?.ownGroupId;
      const next =
        t && t.groupId && t.groupId === owning
          ? previewAt(t.groupId, e.clientY)
          : null;
      reorderRef.current = next;
      setReorder(next);
    };

    const onUp = (e: PointerEvent) => {
      const dragging = active.current;
      if (!dragging) {
        pending.current = null;
        return;
      }
      const t = targetAt(e.clientX, e.clientY);
      const preview = reorderRef.current;
      finish();
      if (!t) return; // released over nothing — cancel, no click follows
      // The group bar decides whether a drop is allowed — a compose member, for
      // instance, may only be reordered inside its project — and reports the
      // refusal, so the layer always forwards the drop.
      // Arm the window at drop time so the click that immediately follows the
      // drop is swallowed rather than opening the card underneath.
      suppressClickUntil.current = performance.now() + CLICK_SUPPRESS_MS;
      dropHandler.current?.(t.groupId, dragging, preview);
    };

    // A cancelled pointer (OS gesture, window blur) must clear the ghost too,
    // otherwise it would hang on screen with nothing driving it.
    const onCancel = () => finish();

    // Swallow the click that follows a drag so dropping a card onto a group
    // does not also open whatever sits underneath. The window is short, so a
    // click the browser never delivered cannot block a later real one.
    const onClickCapture = (e: MouseEvent) => {
      if (performance.now() > suppressClickUntil.current) return;
      suppressClickUntil.current = 0;
      e.stopPropagation();
      e.preventDefault();
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("click", onClickCapture, true);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("click", onClickCapture, true);
    };
  }, []);

  // Position the ghost on the frame it mounts, then pointermove takes over.
  useEffect(() => {
    if (!payload || !ghost.current) return;
    ghost.current.style.left = `${pointer.current.x}px`;
    ghost.current.style.top = `${pointer.current.y}px`;
  }, [payload]);

  const setDropHandler = useCallback((fn: DropHandler | null) => {
    dropHandler.current = fn;
  }, []);

  return (
    <DragContext.Provider
      value={{
        payload,
        overGroupId,
        overIsolated,
        reorder,
        startDrag,
        registerTarget,
        registerMemberList,
      }}
    >
      <DropHandlerContext.Provider value={setDropHandler}>
        {children}
        {payload && (
          <div
            ref={ghost}
            className="pointer-events-none fixed z-[9999] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-indigo-500/60 bg-zinc-900/95 px-2.5 py-1 text-xs font-medium text-white shadow-2xl"
          >
            {payload.label}
          </div>
        )}
      </DropHandlerContext.Provider>
    </DragContext.Provider>
  );
};

/**
 * Lets the group bar receive drops from the shared layer. The bar owns the
 * move logic; the layer only reports where the drop landed.
 */
export const useDropHandler = (fn: DropHandler) => {
  const set = useContext(DropHandlerContext);
  const stable = useRef(fn);
  stable.current = fn;
  useEffect(() => {
    if (!set) return;
    set((groupId, payload, preview) =>
      stable.current(groupId, payload, preview)
    );
    return () => set(null);
  }, [set]);
};

/** Props that make an element draggable. */
export const useDraggable = (payload: DragPayload) => {
  const ctx = useContext(DragContext);
  if (!ctx) throw new Error("useDraggable requires <ContainerDragProvider>");
  return {
    onPointerDown: (e: React.PointerEvent) => ctx.startDrag(payload, e),
  };
};

/**
 * Registers a group's card stack for reorder hit-testing and reports where the
 * dragged card would land. Returns `null` until the pointer is over the stack.
 */
export const useReorderTarget = (
  groupId: string,
  getElements: () => HTMLElement[],
  getOrderedIds: () => string[]
) => {
  const ctx = useContext(DragContext);
  if (!ctx) throw new Error("useReorderTarget requires <ContainerDragProvider>");
  const register = ctx.registerMemberList;
  useEffect(() => register(groupId, getElements, getOrderedIds), [
    register,
    groupId,
    getElements,
    getOrderedIds,
  ]);
  return ctx.reorder?.groupId === groupId ? ctx.reorder : null;
};

/** Registers an element as a drop target and reports hover state. */
export const useDropTarget = <T extends HTMLElement = HTMLDivElement>(
  groupId: string | null
) => {
  const ctx = useContext(DragContext);
  if (!ctx) throw new Error("useDropTarget requires <ContainerDragProvider>");
  const ref = useRef<T | null>(null);
  // Only `registerTarget` is stable; depending on the whole context object
  // would tear down and re-add this target on every provider render.
  const register = ctx.registerTarget;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    return register({ groupId, element: el });
  }, [register, groupId]);

  // `overGroupId === null` is ambiguous on its own: it means both "over nothing"
  // and "over the isolated bucket", so the isolated flag disambiguates.
  const isOver =
    ctx.payload !== null &&
    (groupId === null ? ctx.overIsolated : ctx.overGroupId === groupId);

  return { ref, isOver, isDragging: ctx.payload !== null, payload: ctx.payload };
};
