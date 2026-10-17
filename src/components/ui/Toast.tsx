import React, { createContext, useContext, useState, ReactNode } from "react";
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
}

const ToastContext = createContext<ToastContextType | undefined>(undefined);

export const ToastProvider: React.FC<{ children: ReactNode }> = ({ children }) => {
  const [toasts, setToasts] = useState<ToastMessage[]>([]);

  const addToast = (title: string, description?: string, type: ToastType = "info") => {
    const id = Math.random().toString(36).substring(2, 9);
    setToasts((prev) => [...prev, { id, title, description, type }]);
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id));
    }, 4000);
  };

  const removeToast = (id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  };

  return (
    <ToastContext.Provider
      value={{
        toast: addToast,
        success: (title, desc) => addToast(title, desc, "success"),
        error: (title, desc) => addToast(title, desc, "error"),
        info: (title, desc) => addToast(title, desc, "info"),
      }}
    >
      {children}
      {/* Toast container */}
      <div className="fixed bottom-6 right-6 z-50 flex flex-col gap-2 max-w-sm pointer-events-none select-none">
        <AnimatePresence>
          {toasts.map((t) => {
            const icons = {
              success: <CheckCircle2 className="w-5 h-5 text-emerald-400 shrink-0" />,
              error: <AlertCircle className="w-5 h-5 text-rose-400 shrink-0" />,
              info: <Info className="w-5 h-5 text-blue-400 shrink-0" />,
            };

            const borders = {
              success: "border-emerald-500/40 bg-zinc-900/95 shadow-emerald-500/10",
              error: "border-rose-500/40 bg-zinc-900/95 shadow-rose-500/10",
              info: "border-blue-500/40 bg-zinc-900/95 shadow-blue-500/10",
            };

            return (
              <motion.div
                key={t.id}
                initial={{ opacity: 0, y: 20, scale: 0.95 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, scale: 0.9, transition: { duration: 0.15 } }}
                className={`toast-item pointer-events-auto flex items-start gap-3 p-4 rounded-2xl border shadow-2xl backdrop-blur-xl ${borders[t.type]}`}
              >
                {icons[t.type]}
                <div className="flex-1 min-w-0">
                  <h4 className="text-sm font-semibold text-white tracking-tight">{t.title}</h4>
                  {t.description && (
                    <p className="text-xs text-zinc-400 mt-0.5 leading-relaxed">{t.description}</p>
                  )}
                </div>
                <button
                  onClick={() => removeToast(t.id)}
                  className="text-zinc-500 hover:text-white transition-colors"
                >
                  <X className="w-4 h-4" />
                </button>
              </motion.div>
            );
          })}
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
