/**
 * Unit tests for the Kubernetes manifest helpers. Run with:
 *   node --test src/lib/k8sManifest.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  QUICK_DEPLOY_DEFAULTS,
  buildQuickDeploy,
  classifyDiff,
  diffStats,
  parseEnvLines,
  parseSelectorLogLine,
  podColorIndex,
  sinceTimeFromSeconds,
  validateQuickDeploy,
} from "./k8sManifest.ts";

const base = { ...QUICK_DEPLOY_DEFAULTS, name: "web", image: "nginx:1.27" };

test("a valid form has no errors", () => {
  assert.deepEqual(validateQuickDeploy(base), []);
});

test("bad names, images, ports and env lines are reported", () => {
  const errs = validateQuickDeploy({
    ...base,
    name: "Web_App",
    image: "nginx latest",
    serviceType: "NodePort",
    nodePort: 8080,
    env: "OK=1\nnot an env",
  });
  assert.equal(errs.length, 4);
  assert.ok(validateQuickDeploy({ ...base, containerPort: 0 }).some((e) => e.includes("Service")));
  assert.deepEqual(validateQuickDeploy({ ...base, containerPort: 0, serviceType: "none" }), []);
});

test("the deployment and service share the app label", () => {
  const yaml = buildQuickDeploy({ ...base, replicas: 2, serviceType: "NodePort", nodePort: 30080 });
  assert.match(yaml, /kind: Deployment/);
  assert.match(yaml, /replicas: 2/);
  assert.match(yaml, /image: "nginx:1\.27"/);
  assert.match(yaml, /---\napiVersion: v1\nkind: Service/);
  assert.match(yaml, /type: NodePort/);
  assert.match(yaml, /nodePort: 30080/);
  // metadata, matchLabels, pod template, Service selector.
  assert.equal(yaml.match(/app: web/g)?.length, 4);
});

test("no service without a port, env values are quoted", () => {
  const yaml = buildQuickDeploy({
    ...base,
    containerPort: 0,
    serviceType: "none",
    env: "# comment\nGREETING=hello: world\n\nEMPTY=",
  });
  assert.doesNotMatch(yaml, /kind: Service/);
  assert.doesNotMatch(yaml, /ports:/);
  assert.match(yaml, /- name: GREETING\n {14}value: "hello: world"/);
  assert.match(yaml, /- name: EMPTY\n {14}value: ""/);
});

test("env parsing keeps '=' in values", () => {
  assert.deepEqual(parseEnvLines("A=b=c").pairs, [["A", "b=c"]]);
  assert.equal(parseEnvLines("=x").errors.length, 1);
});

test("diff lines are classified and counted", () => {
  const diff = [
    "diff -u -N /tmp/LIVE/apps.v1.Deployment.default.web /tmp/MERGED/apps.v1.Deployment.default.web",
    "--- /tmp/LIVE/x",
    "+++ /tmp/MERGED/x",
    "@@ -1,3 +1,3 @@",
    " spec:",
    "-  replicas: 1",
    "+  replicas: 2",
  ].join("\n");
  assert.deepEqual(
    classifyDiff(diff).map((l) => l.kind),
    ["meta", "header", "header", "hunk", "context", "del", "add"]
  );
  assert.deepEqual(diffStats(diff), { added: 1, removed: 1 });
  assert.deepEqual(diffStats(""), { added: 0, removed: 0 });
});

test("selector log lines split into pod, container and message", () => {
  assert.deepEqual(
    parseSelectorLogLine("2026-11-25T05:07:56.5Z [pod/web-7d9/nginx] GET / 200"),
    { ts: "2026-11-25T05:07:56.5Z", pod: "web-7d9", container: "nginx", message: "GET / 200" }
  );
  assert.equal(parseSelectorLogLine("plain").message, "plain");
  const i = podColorIndex("web-7d9", 6);
  assert.ok(i >= 0 && i < 6);
  assert.equal(podColorIndex("web-7d9", 6), i);
});

test("since-time is RFC 3339 without milliseconds", () => {
  assert.equal(sinceTimeFromSeconds(1_795_000_000), "2026-11-18T11:06:40Z");
});
