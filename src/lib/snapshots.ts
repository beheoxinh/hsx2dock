/**
 * Pure helpers for the cluster snapshot dialog: the clone eligibility rule and
 * the name check for a clone target. No React, no IPC.
 */
import type { ClusterSnapshot } from "../types";
import { validateClusterName } from "./clusterSpec.ts";

/**
 * kind names control-plane nodes `<cluster>-control-plane`, then
 * `<cluster>-control-plane2`, `-control-plane3`, ... Workers never match.
 */
const CONTROL_PLANE_NODE = /-control-plane\d*$/;

/**
 * Clone is only supported for single-control-plane snapshots.
 *
 * The backend (`restore_cluster_snapshot_core` in kind_snapshots.rs) counts
 * the saved nodes whose role is `control-plane` and refuses any count other
 * than one. The frontend snapshot carries node names only, so the same count
 * is taken from those names. Restoring an HA snapshot in place still works.
 */
export function isSingleControlPlane(snap: ClusterSnapshot): boolean {
  return snap.nodes.filter((name) => CONTROL_PLANE_NODE.test(name)).length === 1;
}

/**
 * Error message for a clone target name, or null when it can be used.
 * The format rules come from the create wizard (`validateClusterName`); the
 * name must also not match a cluster that already exists.
 */
export function validateCloneName(name: string, existing: readonly string[]): string | null {
  const value = name.trim();
  const formatIssue = validateClusterName(value);
  if (formatIssue) return formatIssue;
  if (existing.includes(value)) return `A cluster named "${value}" already exists.`;
  return null;
}
