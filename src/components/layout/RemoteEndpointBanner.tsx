import React, { useEffect, useState } from "react";
import { Cloud, HardDrive } from "lucide-react";
import { Button } from "../ui/Button";
import { useToast } from "../ui/Toast";
import { api, events } from "../../lib/api";
import type { EndpointStatus } from "../../types";

/**
 * Shown only while a remote daemon is active.
 *
 * Engine controls (start/stop, updates, data-root migration, CLI bridge) all
 * address the bundled daemon, so the app says so plainly instead of leaving
 * the user to wonder why those buttons now fail.
 */
export const RemoteEndpointBanner: React.FC = () => {
  const toast = useToast();
  const [active, setActive] = useState<EndpointStatus | null>(null);
  const [switching, setSwitching] = useState(false);

  const load = async () => {
    try {
      const ep = await api.getActiveEndpoint();
      setActive(ep);
    } catch {
      setActive(null);
    }
  };

  useEffect(() => {
    load();
    let unlisten: (() => void) | undefined;
    let disposed = false;
    events
      .onEndpointChanged(() => {
        load();
      })
      .then((u) => {
        if (disposed) u();
        else unlisten = u;
      });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);

  if (!active || active.kind === "local") return null;

  const switchToLocal = async () => {
    try {
      setSwitching(true);
      await api.setActiveEndpoint("local");
      toast.success("Switched to bundled engine");
      await load();
    } catch (e) {
      toast.error("Could not switch back", String(e));
    } finally {
      setSwitching(false);
    }
  };

  return (
    <div className="flex shrink-0 items-center justify-between gap-3 border-b border-amber-500/25 bg-amber-500/10 px-4 py-2">
      <div className="flex min-w-0 items-center gap-2 text-xs">
        <Cloud className="h-3.5 w-3.5 shrink-0 text-amber-400" />
        <span className="truncate text-amber-200">
          Remote endpoint <span className="font-medium">{active.name}</span> — engine
          management is disabled
        </span>
        {!active.reachable && (
          <span className="shrink-0 text-rose-400">· unreachable</span>
        )}
      </div>
      <Button
        variant="ghost"
        size="sm"
        onClick={switchToLocal}
        disabled={switching}
        className="shrink-0 text-[11px] text-amber-200 hover:text-white"
      >
        <HardDrive className="h-3.5 w-3.5" />
        Switch to local
      </Button>
    </div>
  );
};
