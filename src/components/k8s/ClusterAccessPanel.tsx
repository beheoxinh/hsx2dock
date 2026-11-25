import React from "react";
import { AlertTriangle } from "lucide-react";
import type { ClusterAccess } from "../../types";
import { accessWarning } from "../../lib/k8sClusterList";

interface ClusterAccessPanelProps {
  running: boolean;
  /** undefined = first fetch in flight, null = fetch failed. */
  access: ClusterAccess | null | undefined;
}

/**
 * Host-side reachability of one cluster's API server and kubeconfig state, as
 * one compact inline line. A problem shows as a small amber icon whose tooltip
 * carries the full text.
 */
export const ClusterAccessPanel: React.FC<ClusterAccessPanelProps> = ({ running, access }) => {
  if (!running) {
    return <span className="text-[11px] text-zinc-600">Host access is available while the cluster runs.</span>;
  }
  if (access === undefined) {
    return <span className="text-[11px] text-zinc-500">Checking host access…</span>;
  }
  if (access === null) {
    return <span className="text-[11px] text-zinc-500">Host access unavailable right now.</span>;
  }

  const warning = accessWarning(access);
  const dotClass = access.apiReachable ? "bg-emerald-400" : "bg-zinc-500";
  const apiLabel =
    access.apiPort != null ? `API 127.0.0.1:${access.apiPort}` : "API port not published";

  return (
    <div className="flex flex-wrap items-center gap-1.5 text-[11px] min-w-0">
      <span
        className="inline-flex items-center gap-1.5 font-mono text-zinc-300"
        title={
          access.apiReachable
            ? `${access.context} answers on the host`
            : "No answer from the host on this port yet"
        }
      >
        <span
          aria-hidden="true"
          className={`inline-block w-1.5 h-1.5 rounded-full ${dotClass}`}
        />
        <span>{apiLabel}</span>
        <span className="sr-only">
          {access.apiReachable ? ", reachable from host" : ", not reachable from host"}
        </span>
      </span>
      {access.inHostKubeconfig && (
        <span
          title={`Context ${access.context} is in ${access.hostKubeconfig}`}
          className="text-[10px] px-1.5 py-0.5 rounded bg-zinc-800 text-zinc-300 border border-zinc-700/60 font-mono"
        >
          in ~/.kube/config
        </span>
      )}
      {access.hostCurrent && (
        <span className="text-[10px] px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-300 border border-blue-500/30 font-mono">
          current
        </span>
      )}
      {!access.hostKubectl && (
        <span
          title="kubectl is not on the host PATH; the host terminal will say so"
          className="text-[10px] px-1.5 py-0.5 rounded bg-zinc-800/60 text-zinc-500 border border-zinc-700/40"
        >
          no host kubectl
        </span>
      )}
      {warning && (
        <span
          role="img"
          aria-label={warning}
          title={warning}
          className="inline-flex text-amber-300 cursor-help"
        >
          <AlertTriangle className="w-3.5 h-3.5" aria-hidden="true" />
        </span>
      )}
    </div>
  );
};
