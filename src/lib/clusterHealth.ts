import type { KindCluster } from "../types";

/** Colour vocabulary shared by `StatusBadge` and the header health summary. */
export type HealthTone = "neutral" | "success" | "warning" | "danger" | "info";

export interface HealthBadge {
  label: string;
  tone: HealthTone;
}

export interface ClusterHealth {
  /** Cluster-count badge, e.g. `2 Running`, `0/1 Running`, `None`. */
  clusters: HealthBadge & { pulse: boolean };
  /** Node-container health; null while no cluster exists. */
  nodes: HealthBadge | null;
  /** Pod health; null until a running control plane reports counts. */
  pods: HealthBadge | null;
  /** API readiness; null when no running cluster has answered yet. */
  api: HealthBadge | null;
  /** Per-cluster summary for the badge tooltip, one line per cluster. */
  detail: string;
}

/**
 * Rolls the per-cluster snapshot the backend already sends into the four
 * compact badges the header shows.
 *
 * The important rule: a label carrying a count must not inherit the
 * `StatusBadge` "contains the word running" heuristic — `0/1 Running` is a
 * stopped cluster, not a healthy one. Tone is therefore decided here, from
 * the counts, and passed down explicitly.
 */
export const summarizeClusters = (
  clusters: KindCluster[],
  engineRunning: boolean
): ClusterHealth => {
  const total = clusters.length;
  const running = clusters.filter((c) => c.running).length;

  const clusterLabel =
    total === 0
      ? engineRunning
        ? "None"
        : "—"
      : running === total
        ? `${running} Running`
        : `${running}/${total} Running`;

  const clusterTone: HealthTone =
    running === 0 ? "neutral" : running === total ? "success" : "warning";

  const totalNodes = clusters.reduce((n, c) => n + c.node_count, 0);
  const runningNodes = clusters.reduce((n, c) => n + c.running_nodes, 0);
  const nodes: HealthBadge | null =
    totalNodes === 0
      ? null
      : {
          label: `${runningNodes}/${totalNodes} Nodes`,
          tone:
            runningNodes === 0
              ? "neutral"
              : runningNodes === totalNodes
                ? "success"
                : "warning",
        };

  // A stopped or unreachable cluster reports null counts; `?? 0` keeps those
  // out of the totals so the badge reflects only what the control plane
  // actually told us.
  const totalPods = clusters.reduce((n, c) => n + (c.pod_count ?? 0), 0);
  const runningPods = clusters.reduce((n, c) => n + (c.running_pod_count ?? 0), 0);
  const podsKnown = clusters.some((c) => c.running && c.pod_count != null);
  // Not every non-running pod is a problem (completed Jobs, CrashLoopBackOff),
  // so a partial count is informational — amber here would cry wolf forever.
  const pods: HealthBadge | null =
    !podsKnown || totalPods === 0
      ? null
      : {
          label: `${runningPods}/${totalPods} Pods`,
          tone: runningPods === totalPods ? "success" : "info",
        };

  const apiReported = clusters.filter((c) => c.running && c.api_ready !== null);
  const apiDown = apiReported.filter((c) => c.api_ready === false).length;
  const api: HealthBadge | null =
    apiReported.length === 0
      ? null
      : {
          label: apiDown > 0 ? `API ${apiDown} down` : "API Ready",
          tone: apiDown > 0 ? "danger" : "success",
        };

  const detail = total
    ? clusters
        .map((c) => {
          const bits = [
            `${c.name}: ${c.running ? "running" : "stopped"}`,
            `${c.running_nodes}/${c.node_count} nodes`,
          ];
          if (c.running && c.pod_count != null) {
            bits.push(`${c.running_pod_count ?? 0}/${c.pod_count} pods`);
          }
          if (c.running && c.api_ready != null) {
            bits.push(c.api_ready ? "API ready" : "API unreachable");
          }
          return bits.join(" · ");
        })
        .join("\n")
    : "No kind clusters";

  return {
    clusters: {
      label: clusterLabel,
      tone: clusterTone,
      // Pulse only when everything is healthy; a warning badge that blinks
      // reads as "activity" rather than "attention".
      pulse: running > 0 && running === total,
    },
    nodes,
    pods,
    api,
    detail,
  };
};
