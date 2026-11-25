import React from "react";
import { TerminalSquare } from "lucide-react";
import { InteractiveTerminal } from "../terminal/InteractiveTerminal";
import { useTerminalFontSize } from "../../lib/terminalSettings";
import { isRunning, type ComposeServiceRow } from "../../lib/composeProject";
import { cn } from "../../lib/utils";

/**
 * A shell inside one running container of the project. The picker lists every
 * running container (a scaled service has several); switching remounts the
 * terminal so each choice gets its own exec session.
 */
export const TerminalTab: React.FC<{
  rows: ComposeServiceRow[];
  containerId: string | null;
  onSelect: (id: string) => void;
}> = ({ rows, containerId, onSelect }) => {
  const fontSize = useTerminalFontSize();
  const running = rows.flatMap((r) =>
    r.containers.filter(isRunning).map((c) => ({ id: c.id, label: c.name, service: r.name }))
  );
  const current = running.find((c) => c.id === containerId) ?? null;

  if (running.length === 0) {
    return (
      <div className="flex flex-col items-center gap-2 p-10 text-center text-sm text-zinc-500">
        <TerminalSquare className="h-8 w-8 text-zinc-600" />
        No running containers. Start a service to open a shell in it.
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-2">
      <div className="flex flex-wrap gap-1">
        {running.map((c) => (
          <button
            key={c.id}
            type="button"
            onClick={() => onSelect(c.id)}
            title={`${c.service} — ${c.label}`}
            className={cn(
              "flex items-center gap-1.5 border px-2 py-1 font-mono text-[11px]",
              c.id === current?.id
                ? "border-indigo-500/60 bg-indigo-500/15 text-indigo-200"
                : "border-zinc-700 text-zinc-400 hover:border-zinc-500 hover:text-zinc-200"
            )}
          >
            <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" aria-hidden="true" />
            {c.label}
          </button>
        ))}
      </div>
      {current ? (
        <div className="min-h-0 flex-1 border border-zinc-800 bg-black">
          <InteractiveTerminal key={current.id} containerId={current.id} fontSize={fontSize} />
        </div>
      ) : (
        <p className="p-6 text-center text-sm text-zinc-500">Pick a container above to open a shell.</p>
      )}
    </div>
  );
};
