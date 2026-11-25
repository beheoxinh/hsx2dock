import test from "node:test";
import assert from "node:assert/strict";

import { stripServerFields } from "./k8sYaml.ts";

const SAMPLE = `apiVersion: apps/v1
kind: Deployment
metadata:
  annotations:
    deployment.kubernetes.io/revision: "3"
    kubectl.kubernetes.io/last-applied-configuration: |
      {"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"web"}}
  creationTimestamp: "2026-01-01T00:00:00Z"
  generation: 3
  labels:
    app: web
  managedFields:
  - apiVersion: apps/v1
    fieldsType: FieldsV1
    fieldsV1:
      f:metadata:
        f:labels: {}
    manager: kubectl-client-side-apply
    operation: Update
    time: "2026-01-01T00:00:00Z"
  name: web
  namespace: default
  resourceVersion: "12345"
  uid: 7d5c1a2e-0000-0000-0000-000000000000
spec:
  replicas: 2
  template:
    metadata:
      creationTimestamp: null
      labels:
        app: web
status:
  availableReplicas: 2
  conditions:
  - type: Available
    status: "True"
`;

test("removes server-owned metadata fields and the status block", () => {
  const out = stripServerFields(SAMPLE);
  for (const gone of [
    "managedFields",
    "resourceVersion",
    "uid:",
    "generation:",
    "status:",
    "availableReplicas",
    "fieldsV1",
  ]) {
    assert.equal(out.includes(gone), false, `expected "${gone}" to be removed`);
  }
});

test("keeps user-facing metadata and spec untouched", () => {
  const out = stripServerFields(SAMPLE);
  assert.match(out, /^metadata:$/m);
  assert.match(out, /^  name: web$/m);
  assert.match(out, /^  namespace: default$/m);
  assert.match(out, /^  labels:\n    app: web$/m);
  assert.match(out, /^spec:\n  replicas: 2$/m);
  // A nested creationTimestamp (pod template) is not metadata.creationTimestamp.
  assert.match(out, /^      creationTimestamp: null$/m);
  assert.match(out, /^    deployment\.kubernetes\.io\/revision: "3"$/m);
});

test("drops last-applied-configuration with its block-scalar body", () => {
  const out = stripServerFields(SAMPLE);
  assert.equal(out.includes("last-applied-configuration"), false);
  assert.equal(out.includes('{"apiVersion"'), false);
});

test("removes the annotations key when nothing else is in it", () => {
  const yaml = `apiVersion: v1
kind: ConfigMap
metadata:
  annotations:
    kubectl.kubernetes.io/last-applied-configuration: |
      {"kind":"ConfigMap"}
  name: cfg
  namespace: default
data:
  k: v
`;
  const out = stripServerFields(yaml);
  assert.equal(out.includes("annotations"), false);
  assert.equal(
    out,
    `apiVersion: v1
kind: ConfigMap
metadata:
  name: cfg
  namespace: default
data:
  k: v
`
  );
});

test("keeps a non-empty annotations block after dropping last-applied", () => {
  const yaml = `metadata:
  annotations:
    kubectl.kubernetes.io/last-applied-configuration: '{}'
    team: platform
  name: x
`;
  const out = stripServerFields(yaml);
  assert.equal(out, `metadata:\n  annotations:\n    team: platform\n  name: x\n`);
});

test("a document without the noisy fields is returned unchanged", () => {
  const yaml = "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: dev\n";
  assert.equal(stripServerFields(yaml), yaml);
});

test("handles CRLF line endings", () => {
  const out = stripServerFields("metadata:\r\n  uid: abc\r\n  name: x\r\n");
  assert.equal(out, "metadata:\n  name: x\n");
});
