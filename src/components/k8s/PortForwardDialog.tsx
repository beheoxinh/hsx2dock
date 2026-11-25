import React, { useEffect, useId, useState } from "react";
import { Loader2, X } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api } from "../../lib/api";
import type { PortForwardInfo } from "../../types";
import { parseLocalPort, parseRemotePort } from "../../lib/portForward";

interface PortForwardDialogProps {
  cluster: string;
  namespace: string;
  kind: "svc" | "pod" | "deployment";
  name: string;
  remotePort: number;
  onClose: () => void;
  onStarted?: (info: PortForwardInfo) => void;
}

const FIELD_CLASS =
  "h-9 w-full px-3 rounded-none bg-zinc-950 border border-zinc-800 text-sm text-white placeholder-zinc-600 focus:outline-none focus:border-blue-500 disabled:opacity-60";

const errorText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** Starts a port-forward to a service, pod or deployment and closes on success. */
export const PortForwardDialog: React.FC<PortForwardDialogProps> = ({
  cluster,
  namespace,
  kind,
  name,
  remotePort,
  onClose,
  onStarted,
}) => {
  const toast = useToast();
  const remoteId = useId();
  const localId = useId();
  const autoId = useId();

  const [remote, setRemote] = useState(String(remotePort));
  const [local, setLocal] = useState("");
  const [autoStart, setAutoStart] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const remoteParse = parseRemotePort(remote);
  const localParse = parseLocalPort(local);
  const valid = remoteParse.ok && localParse.ok;

  // Escape closes the dialog unless a start is in flight.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const start = async () => {
    if (!remoteParse.ok || !localParse.ok || busy) return;
    setBusy(true);
    setError(null);
    try {
      const info = await api.startPortForward({
        cluster,
        namespace,
        kind,
        name,
        remotePort: remoteParse.port,
        localPort: localParse.port,
        autoStart,
      });
      toast.success("Port-forward active", info.url);
      onStarted?.(info);
      onClose();
    } catch (e) {
      // Keep the dialog open so the user can fix the ports and retry.
      setError(errorText(e));
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={() => !busy && onClose()}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={`${remoteId}-title`}
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-md max-h-[90vh] overflow-y-auto rounded-none border border-zinc-800 bg-zinc-900 shadow-2xl"
      >
        <div className="flex items-center justify-between px-5 py-4 border-b border-zinc-800">
          <div className="min-w-0">
            <span id={`${remoteId}-title`} className="block text-sm font-semibold text-white">
              Forward port
            </span>
            <p className="text-[11px] text-zinc-500 mt-0.5 truncate">
              {kind}/{namespace}/{name}
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            aria-label="Close port-forward dialog"
            title="Close"
            className="rounded-none text-zinc-400 hover:text-white outline-none focus-visible:ring-2 focus-visible:ring-blue-500 disabled:opacity-50"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="px-5 py-4 space-y-4">
          <div className="grid grid-cols-2 divide-x divide-zinc-800 border border-zinc-800 rounded-none">
            <div className="px-3 py-2 space-y-1.5">
              <label htmlFor={remoteId} className="block text-xs font-medium text-zinc-400">
                Remote port
              </label>
              <input
                id={remoteId}
                type="text"
                inputMode="numeric"
                value={remote}
                disabled={busy}
                onChange={(e) => setRemote(e.target.value)}
                aria-invalid={!remoteParse.ok}
                className={FIELD_CLASS}
              />
              {!remoteParse.ok && (
                <p className="text-[11px] text-rose-400">{remoteParse.error}</p>
              )}
            </div>
            <div className="px-3 py-2 space-y-1.5">
              <label htmlFor={localId} className="block text-xs font-medium text-zinc-400">
                Local port <span className="text-zinc-600 font-normal">(empty = auto)</span>
              </label>
              <input
                id={localId}
                type="text"
                inputMode="numeric"
                placeholder="auto"
                value={local}
                disabled={busy}
                onChange={(e) => setLocal(e.target.value)}
                aria-invalid={!localParse.ok}
                className={FIELD_CLASS}
              />
              {!localParse.ok && <p className="text-[11px] text-rose-400">{localParse.error}</p>}
            </div>
          </div>

          <label
            htmlFor={autoId}
            className="flex items-start gap-3 border border-zinc-800 rounded-none px-3 py-2.5 cursor-pointer"
          >
            <input
              id={autoId}
              type="checkbox"
              checked={autoStart}
              disabled={busy}
              onChange={(e) => setAutoStart(e.target.checked)}
              className="mt-0.5 rounded-none border-zinc-700 w-4 h-4"
            />
            <span className="text-sm text-zinc-200">
              Auto-start
              <span className="block text-[11px] text-zinc-500">
                saved, so it starts again with the cluster
              </span>
            </span>
          </label>

          {busy && (
            <p className="text-[11px] text-sky-400">
              The first start may take a while: kubectl is downloaded on first use.
            </p>
          )}

          {error && (
            <div
              role="alert"
              className="border border-rose-500/30 bg-rose-500/10 rounded-none px-3 py-2 text-xs text-rose-300 whitespace-pre-wrap break-words"
            >
              {error}
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 px-5 py-4 border-t border-zinc-800">
          <Button
            variant="ghost"
            size="sm"
            className="rounded-none"
            onClick={onClose}
            disabled={busy}
          >
            Cancel
          </Button>
          <Button
            variant="primary"
            size="sm"
            className="rounded-none gap-2"
            onClick={start}
            disabled={!valid || busy}
          >
            {busy && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            Start
          </Button>
        </div>
      </div>
    </div>
  );
};
