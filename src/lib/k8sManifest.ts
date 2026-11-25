/**
 * Pure helpers for the Kubernetes manifest tools: the quick
 * Deployment + Service generator, unified-diff classification for the
 * colored diff view, and parsing of selector-log lines. No DOM, no IPC.
 */

export type QuickServiceType = "none" | "ClusterIP" | "NodePort";

export interface QuickDeployInput {
  name: string;
  image: string;
  namespace: string;
  replicas: number;
  /** Port the container listens on (0 = none, then no Service either). */
  containerPort: number;
  serviceType: QuickServiceType;
  /** Fixed NodePort (30000–32767); 0 = let Kubernetes pick. */
  nodePort: number;
  /** `KEY=value` per line. */
  env: string;
}

export const QUICK_DEPLOY_DEFAULTS: QuickDeployInput = {
  name: "",
  image: "",
  namespace: "default",
  replicas: 1,
  containerPort: 80,
  serviceType: "ClusterIP",
  nodePort: 0,
  env: "",
};

const DNS_LABEL = /^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/;
const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_.-]*$/;

/** Parsed `KEY=value` lines; blank lines and `#` comments are skipped. */
export function parseEnvLines(text: string): { pairs: [string, string][]; errors: string[] } {
  const pairs: [string, string][] = [];
  const errors: string[] = [];
  text.split("\n").forEach((raw, i) => {
    const line = raw.trim();
    if (!line || line.startsWith("#")) return;
    const eq = line.indexOf("=");
    const key = eq < 0 ? line : line.slice(0, eq).trim();
    if (eq < 0 || !ENV_NAME.test(key)) {
      errors.push(`Env line ${i + 1}: expected KEY=value`);
      return;
    }
    pairs.push([key, line.slice(eq + 1)]);
  });
  return { pairs, errors };
}

/** Problems with the form, empty when it can be generated. */
export function validateQuickDeploy(input: QuickDeployInput): string[] {
  const errors: string[] = [];
  if (!DNS_LABEL.test(input.name) || input.name.length > 63)
    errors.push("Name must be a DNS label: lowercase letters, digits and '-' (max 63)");
  if (!input.image.trim() || /\s/.test(input.image.trim()))
    errors.push("Image is required and cannot contain spaces");
  if (!DNS_LABEL.test(input.namespace)) errors.push("Namespace must be a DNS label");
  if (!Number.isInteger(input.replicas) || input.replicas < 0 || input.replicas > 50)
    errors.push("Replicas must be 0–50");
  if (!Number.isInteger(input.containerPort) || input.containerPort < 0 || input.containerPort > 65535)
    errors.push("Container port must be 0–65535");
  if (input.serviceType === "NodePort" && input.nodePort !== 0 &&
      (input.nodePort < 30000 || input.nodePort > 32767))
    errors.push("NodePort must be 30000–32767 (or 0 for automatic)");
  if (input.serviceType !== "none" && input.containerPort === 0)
    errors.push("A Service needs a container port");
  errors.push(...parseEnvLines(input.env).errors);
  return errors;
}

/** YAML double-quoted scalar: safe for any value. */
function q(s: string): string {
  return JSON.stringify(s);
}

/** Deployment (+ Service) manifest for the quick-deploy form. */
export function buildQuickDeploy(input: QuickDeployInput): string {
  const { name, namespace } = input;
  const image = input.image.trim();
  const env = parseEnvLines(input.env).pairs;
  const lines: string[] = [
    "apiVersion: apps/v1",
    "kind: Deployment",
    "metadata:",
    `  name: ${name}`,
    `  namespace: ${namespace}`,
    "  labels:",
    `    app: ${name}`,
    "spec:",
    `  replicas: ${input.replicas}`,
    "  selector:",
    "    matchLabels:",
    `      app: ${name}`,
    "  template:",
    "    metadata:",
    "      labels:",
    `        app: ${name}`,
    "    spec:",
    "      containers:",
    `        - name: ${name}`,
    `          image: ${q(image)}`,
  ];
  if (input.containerPort > 0) {
    lines.push("          ports:", `            - containerPort: ${input.containerPort}`);
  }
  if (env.length) {
    lines.push("          env:");
    for (const [k, v] of env) lines.push(`            - name: ${k}`, `              value: ${q(v)}`);
  }
  if (input.serviceType !== "none" && input.containerPort > 0) {
    lines.push(
      "---",
      "apiVersion: v1",
      "kind: Service",
      "metadata:",
      `  name: ${name}`,
      `  namespace: ${namespace}`,
      "spec:",
      `  type: ${input.serviceType}`,
      "  selector:",
      `    app: ${name}`,
      "  ports:",
      `    - port: ${input.containerPort}`,
      `      targetPort: ${input.containerPort}`
    );
    if (input.serviceType === "NodePort" && input.nodePort > 0) {
      lines.push(`      nodePort: ${input.nodePort}`);
    }
  }
  return lines.join("\n") + "\n";
}

export type DiffLineKind = "header" | "hunk" | "add" | "del" | "context" | "meta";

/** Classifies each line of `kubectl diff` (unified) output for coloring. */
export function classifyDiff(diff: string): { kind: DiffLineKind; text: string }[] {
  return diff.split("\n").map((text) => {
    let kind: DiffLineKind = "context";
    if (text.startsWith("+++") || text.startsWith("---")) kind = "header";
    else if (text.startsWith("@@")) kind = "hunk";
    else if (text.startsWith("+")) kind = "add";
    else if (text.startsWith("-")) kind = "del";
    else if (text.startsWith("diff ")) kind = "meta";
    return { kind, text };
  });
}

/** `+N -M` totals of a unified diff (headers excluded). */
export function diffStats(diff: string): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const { kind } of classifyDiff(diff)) {
    if (kind === "add") added++;
    else if (kind === "del") removed++;
  }
  return { added, removed };
}

export interface SelectorLogLine {
  /** RFC 3339 timestamp ("" when the line had none). */
  ts: string;
  pod: string;
  container: string;
  message: string;
}

/** Splits `<ts> [pod/<pod>/<container>] <msg>` (the backend's order). */
export function parseSelectorLogLine(line: string): SelectorLogLine {
  const m = /^(\S+) \[pod\/([^/\]]+)\/([^\]]+)\] ?(.*)$/.exec(line);
  if (m) return { ts: m[1], pod: m[2], container: m[3], message: m[4] };
  return { ts: "", pod: "", container: "", message: line };
}

/** Stable palette index for a pod name, so each pod keeps its color. */
export function podColorIndex(pod: string, paletteSize: number): number {
  let h = 0;
  for (let i = 0; i < pod.length; i++) h = (h * 31 + pod.charCodeAt(i)) >>> 0;
  return paletteSize > 0 ? h % paletteSize : 0;
}

/** RFC 3339 `--since-time` for kubectl from unix seconds. */
export function sinceTimeFromSeconds(sec: number): string {
  return new Date(sec * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");
}
