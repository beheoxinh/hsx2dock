import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
} from "react";

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
}

interface DropTarget {
  /** `null` means "not in any group" (the Isolated bucket). */
  groupId: string | null;
  element: HTMLElement;
}

type DropHandler = (groupId: string | null, payload: DragPayload) => void;

interface DragContextValue {
  /** Non-null while a drag is in progress. */
  payload: DragPayload | null;
  /** Group currently hovered, or null when over nothing / the isolated bucket. */
  overGroupId: string | null;
  /** True when the pointer is over the isolated bucket specifically. */
  overIsolated: boolean;
  startDrag: (payload: DragPayload, event: React.PointerEvent) => void;
  registerTarget: (target: DropTarget) => () => void;
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

  // Refs, not state: these are read on every pointermove and must not re-create
  // the window listeners or re-render the tree per frame.
  const targets = useRef<DropTarget[]>([]);
  const pending = useRef<{ payload: DragPayload; x: number; y: number } | null>(
    null
  );
  const active = useRef<DragPayload | null>(null);
  const dropHandler = useRef<DropHandler | null>(null);
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
      setPayload(null);
      setOverGroupId(null);
      setOverIsolated(false);
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
    };

    const onUp = (e: PointerEvent) => {
      const dragging = active.current;
      if (!dragging) {
        pending.current = null;
        return;
      }
      const t = targetAt(e.clientX, e.clientY);
      finish();
      if (!t) return; // released over nothing — cancel, no click follows
      // Arm the window at drop time so the click that immediately follows the
      // drop is swallowed rather than opening the card underneath.
      suppressClickUntil.current = performance.now() + CLICK_SUPPRESS_MS;
      dropHandler.current?.(t.groupId, dragging);
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
      value={{ payload, overGroupId, overIsolated, startDrag, registerTarget }}
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
    set((groupId, payload) => stable.current(groupId, payload));
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

  return { ref, isOver, isDragging: ctx.payload !== null };
};
