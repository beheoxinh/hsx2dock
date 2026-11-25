import React, { useEffect, useId, useRef, useState } from "react";
import { cn } from "../../lib/utils";

export interface OverflowItem {
  key: string;
  label: string;
  icon: React.ReactNode;
  onSelect: () => void;
  disabled?: boolean;
  /** Tooltip; also explains why the item is disabled. */
  title?: string;
  danger?: boolean;
  /** Draws a separator above this item. */
  dividerBefore?: boolean;
}

interface ClusterOverflowMenuProps {
  items: OverflowItem[];
  /** Accessible name of the trigger, e.g. "More actions for dev". */
  label: string;
  disabled?: boolean;
  onOpenChange?: (open: boolean) => void;
  icon: React.ReactNode;
}

/**
 * Small keyboard-usable menu: Enter/Space/ArrowDown opens it, arrows move
 * between enabled items, Escape or Tab closes it and returns focus to the
 * trigger, and an outside click closes it.
 */
export const ClusterOverflowMenu: React.FC<ClusterOverflowMenuProps> = ({
  items,
  label,
  disabled,
  onOpenChange,
  icon,
}) => {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuId = useId();

  const setOpenState = (next: boolean) => {
    setOpen(next);
    onOpenChange?.(next);
  };

  const enabledItems = (): HTMLButtonElement[] =>
    Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? []);

  // Focus the first item on open.
  useEffect(() => {
    if (open) enabledItems()[0]?.focus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Outside click closes the menu.
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpenState(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const close = (returnFocus: boolean) => {
    setOpenState(false);
    if (returnFocus) triggerRef.current?.focus();
  };

  const onMenuKeyDown = (e: React.KeyboardEvent) => {
    const list = enabledItems();
    const idx = list.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "Escape") {
      e.preventDefault();
      close(true);
    } else if (e.key === "Tab") {
      close(false);
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      list[(idx + 1 + list.length) % list.length]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      list[(idx - 1 + list.length) % list.length]?.focus();
    } else if (e.key === "Home") {
      e.preventDefault();
      list[0]?.focus();
    } else if (e.key === "End") {
      e.preventDefault();
      list[list.length - 1]?.focus();
    }
  };

  const onTriggerKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setOpenState(true);
    }
  };

  return (
    <div ref={rootRef} className="relative">
      <button
        ref={triggerRef}
        type="button"
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        disabled={disabled}
        onClick={() => setOpenState(!open)}
        onKeyDown={onTriggerKeyDown}
        className="inline-flex items-center justify-center h-8 w-8 rounded-lg text-zinc-400 hover:text-white hover:bg-zinc-800/60 outline-none focus-visible:ring-2 focus-visible:ring-blue-500 disabled:opacity-50 disabled:pointer-events-none"
      >
        {icon}
      </button>
      {open && (
        <div
          ref={menuRef}
          id={menuId}
          role="menu"
          aria-label={label}
          onKeyDown={onMenuKeyDown}
          className="absolute right-0 top-full mt-1 z-40 w-60 rounded-xl border border-zinc-800 bg-zinc-900 shadow-2xl py-1"
        >
          {items.map((item) => (
            <React.Fragment key={item.key}>
              {item.dividerBefore && <div role="separator" className="my-1 h-px bg-zinc-800" />}
              <button
                type="button"
                role="menuitem"
                disabled={item.disabled}
                title={item.title ?? item.label}
                onClick={() => {
                  close(true);
                  item.onSelect();
                }}
                className={cn(
                  "w-full flex items-center gap-2.5 px-3 py-2 text-left text-xs outline-none focus:bg-zinc-800 hover:bg-zinc-800 disabled:opacity-40 disabled:pointer-events-none",
                  item.danger ? "text-rose-300" : "text-zinc-200"
                )}
              >
                <span className="shrink-0 inline-flex">{item.icon}</span>
                <span className="truncate">{item.label}</span>
              </button>
            </React.Fragment>
          ))}
        </div>
      )}
    </div>
  );
};
