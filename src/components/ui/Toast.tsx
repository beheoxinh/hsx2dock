import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  ReactNode,
} from "react";
import { motion, AnimatePresence } from "framer-motion";
import { CheckCircle2, AlertCircle, Info, X } from "lucide-react";

type ToastType = "success" | "error" | "info";

interface ToastMessage {
  id: string;
  type: ToastType;
  title: string;
  description?: string;
}

interface ToastContextType {
  toast: (title: string, description?: string, type?: ToastType) => void;
  success: (title: string, description?: string) => void;
  error: (title: string, description?: string) => void;
  info: (title: string, description?: string) => void;
  /** Dismiss everything (used when the engine goes away). */
  dismissAll: () => void;
}

const ToastContext = createContext<ToastContextType | undefined>(undefined);

/** Errors need reading time; successes are glanceable and leave quickly. */
const DURATION: Record<ToastType, number> = {
  success: 2600,
  info: 3200,
  error: 6500,
};

/** Never stack more than this — older ones fall off instead of growing. */
const MAX_VISIBLE = 3;

/** Identical messages inside this window are duplicates, not new events. */
const DEDUPE_MS = 1200;

const TONE: Record<
  ToastType,
  { icon: React.ReactNode; accent: string; ring: string }
> = {
  success: {
    icon: <CheckCircle2 className="w-4 h-4 text-emerald-400" />,
    accent: "bg-emerald-400/80",
    ring: "ring-emerald-500/20",
  },
  error: {
    icon: <AlertCircle className="w-4 h-4 text-rose-400" />,
    accent: "bg-rose-400/80",
    ring: "ring-rose-500/25",
  },
  info: {
    icon: <Info className="w-4 h-4 text-blue-400" />,
    accent: "bg-blue-400/80",
    ring: "ring-blue-500/20",
  },
};

export const ToastProvider: React.FC<{ children: ReactNode }> = ({ children }) => {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const lastMessage = useRef<{ key: string; at: number }>({ key: "", at: 0 });

  const removeToast = useCallback((id: string) => {
    const timer = timers.current.get(id);
    if (timer) clearTimeout(timer);
    timers.current.delete(id);
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const dismissAll = useCallback(() => {
    for (const timer of timers.current.values()) clearTimeout(timer);
    timers.current.clear();
    setToasts([]);
  }, []);

  const addToast = useCallback(
    (title: string, description?: string, type: ToastType = "info") => {
      const cleanTitle = title.trim();
      if (!cleanTitle) return;
      const cleanDescription = description?.trim() || undefined;

      // A double click or two listeners reacting to the same event must not
      // produce two identical bubbles.
      const key = `${type}|${cleanTitle}|${cleanDescription ?? ""}`;
      const now = Date.now();
      if (lastMessage.current.key === key && now - lastMessage.current.at < DEDUPE_MS) {
        return;
      }
      lastMessage.current = { key, at: now };

      const id = `${now.toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
      setToasts((prev) => [...prev, { id, title: cleanTitle, description: cleanDescription, type }].slice(-MAX_VISIBLE));

      timers.current.set(
        id,
        setTimeout(() => removeToast(id), DURATION[type])
      );
    },
    [removeToast]
  );

  useEffect(() => {
    const pending = timers.current;
    return () => {
      for (const timer of pending.values()) clearTimeout(timer);
      pending.clear();
    };
  }, []);

  const value = useMemo<ToastContextType>(
    () => ({
      toast: addToast,
      success: (title, description) => addToast(title, description, "success"),
      error: (title, description) => addToast(title, description, "error"),
      info: (title, description) => addToast(title, description, "info"),
      dismissAll,
    }),
    [addToast, dismissAll]
  );

  return (
    <ToastContext.Provider value={value}>
      {children}
      {/* Toasts sit above the shell but never capture clicks except on the
          bubble itself, so the app stays usable while they are on screen. */}
      <div
        className="fixed bottom-5 right-5 z-[120] flex w-[min(22rem,calc(100vw-2rem))] flex-col gap-2 pointer-events-none"
        role="region"
        aria-label="Notifications"
        aria-live="polite"
      >
        <AnimatePresence initial={false}>
          {toasts.map((t) => (
            <motion.div
              key={t.id}
              layout="position"
              initial={{ opacity: 0, y: 14, scale: 0.97 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, x: 24, scale: 0.97, transition: { duration: 0.16, ease: "easeIn" } }}
              transition={{ type: "spring", stiffness: 420, damping: 34, mass: 0.8 }}
              role={t.type === "error" ? "alert" : "status"}
              className={`pointer-events-auto relative flex items-start gap-2.5 overflow-hidden rounded-xl border border-white/5 bg-zinc-900/95 px-3.5 py-2.5 shadow-lg shadow-black/30 ring-1 ring-inset backdrop-blur-xl ${TONE[t.type].ring}`}
              onMouseEnter={() => {
                const timer = timers.current.get(t.id);
                if (timer) clearTimeout(timer);
                timers.current.delete(t.id);
              }}
              onMouseLeave={() => {
                if (timers.current.has(t.id)) return;
                timers.current.set(
                  t.id,
                  setTimeout(() => removeToast(t.id), DURATION[t.type])
                );
              }}
            >
              <span
                aria-hidden="true"
                className={`absolute inset-y-0 left-0 w-[2px] ${TONE[t.type].accent}`}
              />
              <span className="mt-[1px] shrink-0">{TONE[t.type].icon}</span>
              <div className="min-w-0 flex-1">
                <p className="truncate text-[13px] font-medium leading-snug text-zinc-100">
                  {t.title}
                </p>
                {t.description && (
                  <p className="mt-0.5 line-clamp-2 text-[11.5px] leading-snug text-zinc-400">
                    {t.description}
                  </p>
                )}
              </div>
              <button
                type="button"
                onClick={() => removeToast(t.id)}
                aria-label="Dismiss notification"
                className="-mr-1 shrink-0 rounded-md p-0.5 text-zinc-500 transition-colors hover:text-white focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-zinc-500"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
    </ToastContext.Provider>
  );
};

export const useToast = () => {
  const context = useContext(ToastContext);
  if (!context) {
    throw new Error("useToast must be used within a ToastProvider");
  }
  return context;
};
