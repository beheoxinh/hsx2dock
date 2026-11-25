/**
 * Unit tests for the kind cluster wizard helpers. Run with:
 *   node --test src/lib/clusterSpec.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  applyPreset,
  cloneSpec,
  defaultClusterSpec,
  localRegistryRef,
  presetFromSpec,
  sameSpec,
  specFromDefaults,
  totalNodes,
  validateCidr,
  validateClusterName,
  validateFeatureGate,
  validateHostPort,
  validateSpec,
} from "./clusterSpec.ts";

test("the default spec is one control plane with no extras", () => {
  const spec = defaultClusterSpec();
  assert.equal(spec.controlPlanes, 1);
  assert.equal(spec.workers, 0);
  assert.equal(totalNodes(spec), 1);
  assert.deepEqual(validateSpec(spec), []);
});

test("settings defaults become a total node count and a null image when blank", () => {
  const spec = specFromDefaults(3, "  ");
  assert.equal(spec.workers, 2);
  assert.equal(spec.image, null);
  assert.equal(specFromDefaults(3, "kindest/node:v1.31.0").image, "kindest/node:v1.31.0");
});

test("settings defaults are clamped to the node limit", () => {
  assert.equal(totalNodes(specFromDefaults(0, "")), 1);
  assert.equal(totalNodes(specFromDefaults(99, "")), 10);
});

test("cluster names allow letters, digits, dash and dot only", () => {
  assert.equal(validateClusterName("dev-cluster.1"), null);
  assert.equal(validateClusterName("   "), "Name is required.");
  assert.match(validateClusterName("my_cluster") ?? "", /letters, digits/);
  // Same rules as the backend's check_cluster_name.
  assert.notEqual(validateClusterName("Dev"), null);
  assert.notEqual(validateClusterName("-dev"), null);
  assert.notEqual(validateClusterName("a..b"), null);
  assert.equal(validateClusterName("a".repeat(50)), null);
  assert.equal(validateClusterName("a".repeat(51)), "At most 50 characters.");
});

test("host ports in the reserved and unforwarded ranges are rejected", () => {
  assert.equal(validateHostPort(18080), null);
  assert.match(validateHostPort(16443) ?? "", /reserved/);
  assert.match(validateHostPort(16999) ?? "", /reserved/);
  assert.match(validateHostPort(40000) ?? "", /not forwarded/);
  assert.match(validateHostPort(0) ?? "", /1–65535/);
  assert.match(validateHostPort(70000) ?? "", /1–65535/);
});

test("subnets must be IPv4 CIDR, and an empty subnet is allowed", () => {
  assert.equal(validateCidr(null), null);
  assert.equal(validateCidr("  "), null);
  assert.equal(validateCidr("10.244.0.0/16"), null);
  assert.match(validateCidr("10.244.0.0") ?? "", /CIDR/);
  assert.match(validateCidr("10.300.0.0/16") ?? "", /valid IPv4/);
  assert.match(validateCidr("10.0.0.0/33") ?? "", /valid IPv4/);
});

test("feature gates use Name=true or Name=false", () => {
  assert.equal(validateFeatureGate("EphemeralContainers=true"), null);
  assert.equal(validateFeatureGate("Gate=false"), null);
  assert.notEqual(validateFeatureGate("Gate"), null);
  assert.notEqual(validateFeatureGate("Gate=yes"), null);
});

test("validateSpec flags control plane counts, duplicate host ports and bad mounts", () => {
  const spec = { ...defaultClusterSpec(), controlPlanes: 2 };
  assert.ok(validateSpec(spec).some((m) => /1, 3 or 5/.test(m)));

  const dup = {
    ...defaultClusterSpec(),
    ingress: true,
    ingressHttpPort: 18080,
    ingressHttpsPort: 18443,
    portMappings: [{ containerPort: 80, hostPort: 18080, protocol: "TCP" as const }],
  };
  assert.ok(validateSpec(dup).some((m) => /Host port 18080 is used more than once/.test(m)));

  const mounts = {
    ...defaultClusterSpec(),
    mounts: [{ hostPath: "/data", containerPath: "relative", readOnly: false }],
  };
  assert.ok(validateSpec(mounts).some((m) => /must be absolute/.test(m)));
});

test("ten nodes is the limit, counting control planes", () => {
  const spec = { ...defaultClusterSpec(), controlPlanes: 3, workers: 8 };
  assert.ok(validateSpec(spec).some((m) => /At most 10/.test(m)));
  assert.deepEqual(validateSpec({ ...spec, workers: 7 }), []);
});

test("applying a preset copies it, so later edits leave the preset alone", () => {
  const preset = {
    name: "ha",
    description: "",
    builtin: false,
    spec: { ...defaultClusterSpec(), controlPlanes: 3, featureGates: ["A=true"] },
  };
  const applied = applyPreset(preset);
  applied.featureGates.push("B=true");
  applied.controlPlanes = 1;
  assert.deepEqual(preset.spec.featureGates, ["A=true"]);
  assert.equal(preset.spec.controlPlanes, 3);
});

test("a preset saved from a spec is trimmed and detached from it", () => {
  const spec = { ...defaultClusterSpec(), mounts: [{ hostPath: "/a", containerPath: "/b", readOnly: true }] };
  const preset = presetFromSpec("  lab ", " notes ", spec);
  assert.equal(preset.name, "lab");
  assert.equal(preset.description, "notes");
  assert.equal(preset.builtin, false);
  spec.mounts[0].readOnly = false;
  assert.equal(preset.spec.mounts[0].readOnly, true);
});

test("sameSpec detects when the form matches a preset", () => {
  const a = defaultClusterSpec();
  assert.equal(sameSpec(a, cloneSpec(a)), true);
  assert.equal(sameSpec(a, { ...a, ingress: true }), false);
});

test("local registry refs mirror the backend rewrite", () => {
  assert.equal(localRegistryRef("nginx:1.27"), "localhost:5001/nginx:1.27");
  assert.equal(localRegistryRef("nginx"), "localhost:5001/nginx:latest");
  assert.equal(localRegistryRef("ghcr.io/a/b:t"), "localhost:5001/a/b:t");
  assert.equal(localRegistryRef("docker.io/library/redis"), "localhost:5001/redis:latest");
  assert.equal(localRegistryRef("me/app"), "localhost:5001/me/app:latest");
  assert.equal(localRegistryRef("  "), null);
  assert.equal(localRegistryRef("bad image"), null);
});
