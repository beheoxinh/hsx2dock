import React, { createContext, useContext, useState, ReactNode } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { AlertTriangle, Info, HelpCircle, CheckCircle2, X } from "lucide-react";
import { Button } from "./Button";

export type ModalType =
  | "info"
  | "warning"
  | "danger"
  | "success"
  | "confirm"
  | "prompt"
  | "choice";

/** One button in a multi-choice dialog. `value` is what the promise resolves to. */
export interface ModalChoice {
  value: string;
  label: string;
  variant?: "primary" | "danger" | "secondary" | "ghost";
  /** Shown under the label in smaller type. */
  hint?: string;
}

export interface ModalOptions {
  title?: string;
  message: string;
  type?: ModalType;
  confirmText?: string;
  cancelText?: string;
  placeholder?: string;
  defaultValue?: string;
  /** Present only for `choice` dialogs; replaces the confirm/cancel pair. */
  choices?: ModalChoice[];
}

interface ModalContextType {
  showAlert: (message: string, title?: string, type?: ModalType) => Promise<void>;
  showConfirm: (message: string, title?: string, type?: ModalType) => Promise<boolean>;
  showPrompt: (message: string, defaultValue?: string, title?: string) => Promise<string | null>;
  /**
   * A dialog with N labelled outcomes, for decisions a yes/no cannot express
   * (delete the folder *and* its contents, or only the folder). Resolves to the
   * chosen `value`, or null when dismissed.
   */
  showChoice: (
    message: string,
    choices: ModalChoice[],
    title?: string,
    type?: ModalType
  ) => Promise<string | null>;
}

const ModalContext = createContext<ModalContextType | undefined>(undefined);

export const ModalProvider: React.FC<{ children: ReactNode }> = ({ children }) => {
  const [modalState, setModalState] = useState<{
    isOpen: boolean;
    options: ModalOptions;
    resolve: (value: any) => void;
  } | null>(null);

  const [promptInput, setPromptInput] = useState("");

  const showAlert = (message: string, title: string = "Notice", type: ModalType = "info"): Promise<void> => {
    return new Promise((resolve) => {
      setModalState({
        isOpen: true,
        options: { message, title, type, confirmText: "OK" },
        resolve,
      });
    });
  };

  const showConfirm = (message: string, title: string = "Confirmation", type: ModalType = "warning"): Promise<boolean> => {
    return new Promise((resolve) => {
      setModalState({
        isOpen: true,
        options: { message, title, type, confirmText: "Confirm", cancelText: "Cancel" },
        resolve,
      });
    });
  };

  const showPrompt = (message: string, defaultValue: string = "", title: string = "Input Required"): Promise<string | null> => {
    setPromptInput(defaultValue);
    return new Promise((resolve) => {
      setModalState({
        isOpen: true,
        options: { message, title, type: "prompt", confirmText: "Submit", cancelText: "Cancel", defaultValue },
        resolve,
      });
    });
  };

  const showChoice = (
    message: string,
    choices: ModalChoice[],
    title: string = "Choose an action",
    type: ModalType = "warning"
  ): Promise<string | null> => {
    return new Promise((resolve) => {
      setModalState({ isOpen: true, options: { message, title, type, choices }, resolve });
    });
  };

  const handleClose = (value: any) => {
    if (modalState) {
      modalState.resolve(value);
      setModalState(null);
    }
  };

  const currentOptions = modalState?.options;

  const renderIcon = () => {
    if (!currentOptions) return null;
    switch (currentOptions.type) {
      case "danger":
        return <AlertTriangle className="w-6 h-6 text-rose-500 shrink-0" />;
      case "warning":
        return <AlertTriangle className="w-6 h-6 text-amber-500 shrink-0" />;
      case "success":
        return <CheckCircle2 className="w-6 h-6 text-emerald-500 shrink-0" />;
      case "prompt":
        return <HelpCircle className="w-6 h-6 text-indigo-500 shrink-0" />;
      case "choice":
        return <HelpCircle className="w-6 h-6 text-amber-500 shrink-0" />;
      case "info":
      case "confirm":
      default:
        return <Info className="w-6 h-6 text-blue-500 shrink-0" />;
    }
  };

  return (
    <ModalContext.Provider value={{ showAlert, showConfirm, showPrompt, showChoice }}>
      {children}
      <AnimatePresence>
        {modalState?.isOpen && (
          <div className="fixed inset-0 z-[100] flex items-center justify-center p-4 select-none">
            {/* Backdrop */}
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onClick={() => handleClose(currentOptions?.type === "prompt" ? null : false)}
              className="absolute inset-0 bg-black/60 backdrop-blur-sm"
            />

            {/* Dialog Content */}
            <motion.div
              initial={{ opacity: 0, scale: 0.95, y: 10 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.95, y: 10 }}
              transition={{ duration: 0.15, ease: "easeOut" }}
              className="relative w-full max-w-md bg-zinc-900 border border-zinc-800 rounded-2xl shadow-2xl overflow-hidden z-10 flex flex-col"
            >
              {/* Header */}
              <div className="flex items-center justify-between px-6 pt-5 pb-3">
                <div className="flex items-center gap-3">
                  <div className="p-2 rounded-xl bg-zinc-800/80 border border-zinc-700/50 flex items-center justify-center">
                    {renderIcon()}
                  </div>
                  <h3 className="text-base font-semibold text-white tracking-tight">
                    {currentOptions?.title}
                  </h3>
                </div>
                <button
                  type="button"
                  onClick={() => handleClose(currentOptions?.type === "prompt" ? null : false)}
                  className="text-zinc-400 hover:text-white p-1 rounded-lg transition-colors cursor-pointer"
                >
                  <X className="w-4 h-4" />
                </button>
              </div>

              {/* Body */}
              <div className="px-6 py-3 space-y-4 select-text">
                <p className="text-sm text-zinc-300 leading-relaxed whitespace-pre-wrap select-text">
                  {currentOptions?.message}
                </p>

                {currentOptions?.type === "prompt" && (
                  <form
                    onSubmit={(e) => {
                      e.preventDefault();
                      handleClose(promptInput);
                    }}
                  >
                    <input
                      type="text"
                      autoFocus
                      value={promptInput}
                      onChange={(e) => setPromptInput(e.target.value)}
                      placeholder={currentOptions?.placeholder}
                      className="w-full h-10 px-3.5 rounded-xl bg-zinc-950 border border-zinc-800 text-sm text-white focus:outline-none focus:border-blue-500 transition-colors"
                    />
                  </form>
                )}

                {currentOptions?.choices && (
                  <div className="space-y-2.5 pt-1">
                    {currentOptions.choices.map((choice) => (
                      <button
                        key={choice.value}
                        type="button"
                        onClick={() => handleClose(choice.value)}
                        className={`w-full group text-left px-4 py-3 rounded-xl border transition-all cursor-pointer flex items-center justify-between gap-3 ${
                          choice.variant === "danger"
                            ? "border-rose-500/30 bg-rose-500/10 hover:bg-rose-500/20 hover:border-rose-500/50"
                            : "border-zinc-800 bg-zinc-950/60 hover:bg-zinc-800/60 hover:border-zinc-700"
                        }`}
                      >
                        <div className="min-w-0">
                          <p
                            className={`text-sm font-semibold tracking-tight transition-colors ${
                              choice.variant === "danger"
                                ? "text-rose-300 group-hover:text-rose-200"
                                : "text-zinc-100 group-hover:text-white"
                            }`}
                          >
                            {choice.label}
                          </p>
                          {choice.hint && (
                            <p className="text-xs text-zinc-400 mt-0.5 leading-normal">
                              {choice.hint}
                            </p>
                          )}
                        </div>
                        <div
                          className={`w-6 h-6 rounded-lg flex items-center justify-center shrink-0 transition-transform group-hover:translate-x-0.5 ${
                            choice.variant === "danger"
                              ? "bg-rose-500/20 text-rose-300"
                              : "bg-zinc-800 text-zinc-400 group-hover:text-white"
                          }`}
                        >
                          <span className="text-sm font-bold">→</span>
                        </div>
                      </button>
                    ))}
                  </div>
                )}
              </div>

              {/* Footer Actions */}
              <div className="flex items-center justify-end gap-3 px-6 py-4 bg-zinc-950/40 border-t border-zinc-800/80">
                {currentOptions?.choices ? (
                  <Button type="button" variant="ghost" onClick={() => handleClose(null)}>
                    Cancel
                  </Button>
                ) : (
                  <>
                    {(currentOptions?.cancelText || currentOptions?.type === "confirm" || currentOptions?.type === "prompt") && (
                      <Button
                        type="button"
                        variant="ghost"
                        onClick={() => handleClose(currentOptions?.type === "prompt" ? null : false)}
                      >
                        {currentOptions?.cancelText || "Cancel"}
                      </Button>
                    )}
                    <Button
                      type="button"
                      variant={currentOptions?.type === "danger" ? "danger" : "primary"}
                      onClick={() => handleClose(currentOptions?.type === "prompt" ? promptInput : true)}
                      autoFocus
                    >
                      {currentOptions?.confirmText || "OK"}
                    </Button>
                  </>
                )}
              </div>
            </motion.div>
          </div>
        )}
      </AnimatePresence>
    </ModalContext.Provider>
  );
};

export const useModal = () => {
  const context = useContext(ModalContext);
  if (!context) {
    throw new Error("useModal must be used within a ModalProvider");
  }
  return context;
};
