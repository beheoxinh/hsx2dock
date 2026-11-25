import { useMemo } from "react";
import { Terminal as TerminalIcon, X } from "lucide-react";
import { Button } from "../ui/Button";
import { InteractiveTerminal } from "../terminal/InteractiveTerminal";

/** Single-quotes a value for the shell, escaping embedded single quotes. */
const shq = (s: string): string => `'${s.replace(/'/g, `'\\''`)}'`;

/**
 * Interactive bash inside the control-plane node with kubectl ready:
 * KUBECONFIG exported, `k` alias, kubectl completion. The settings go in an
 * rcfile written to a temp file, because aliases and completion
 * set by a `bash -lc` wrapper do not survive an `exec bash`.
 */
function buildShellCommand(cluster: string): string[] {
  // Only a name that matches this pattern reaches PS1 or the echo line.
  const safe = /^[a-z0-9][a-z0-9.-]*$/.test(cluster) ? cluster : "cluster";
  const rc = [
    "export KUBECONFIG=/etc/kubernetes/admin.conf",
    "alias k=kubectl",
    "source <(kubectl completion bash) 2>/dev/null",
    "complete -o default -F __start_kubectl k 2>/dev/null",
    `PS1='[\\u@${safe} \\W]\\$ '`,
    `echo "kubectl shell · cluster ${safe} (k = kubectl)"`,
  ];
  // A temp file, not `<(…)`: the process-substitution fd is gone once
  // `exec` replaces the shell, and bash silently skips a missing rcfile.
  // The rcfile deletes itself as its first line.
  const script =
    `f=$(mktemp /tmp/kubeshell.XXXXXX) && ` +
    `printf '%s\\n' "rm -f $f" ${rc.map(shq).join(" ")} > "$f" && ` +
    `exec bash --rcfile "$f" -i`;
  return ["bash", "-c", script];
}

export function KubeShellModal({ cluster, onClose }: { cluster: string; onClose: () => void }) {
  const cmd = useMemo(() => buildShellCommand(cluster), [cluster]);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
      <div
        role="dialog"
        aria-modal="true"
        aria-label="kubectl shell"
        className="flex h-[90vh] w-[min(1500px,96vw)] flex-col rounded-2xl border border-zinc-800 bg-zinc-950 shadow-2xl"
      >
        <div className="flex items-center gap-2 border-b border-zinc-800 px-4 py-3">
          <TerminalIcon className="h-4 w-4 shrink-0 text-zinc-400" />
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-white">kubectl shell</p>
            <p className="truncate text-[11px] text-zinc-500">
              <span className="font-mono">{cluster}</span> control-plane node · KUBECONFIG and{" "}
              <span className="font-mono">k</span> alias preset
            </p>
          </div>
          <Button
            variant="ghost"
            size="icon"
            title="Close shell"
            aria-label="Close kubectl shell"
            onClick={onClose}
            className="ml-auto h-7 w-7 text-zinc-400 hover:text-white"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
        <div className="min-h-0 flex-1">
          <InteractiveTerminal containerId={`${cluster}-control-plane`} cmd={cmd} />
        </div>
      </div>
    </div>
  );
}
