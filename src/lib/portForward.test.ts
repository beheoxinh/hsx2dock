/**
 * Unit tests for the port-forward helpers.
 *
 * Pure logic only. Run with:
 *   node --test src/lib/portForward.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  MAX_LOCAL_PORT,
  forwardStatusTone,
  parseLocalPort,
  parsePodContainerPorts,
  parseRemotePort,
} from "./portForward.ts";

test("MAX_LOCAL_PORT is 32767", () => {
  assert.equal(MAX_LOCAL_PORT, 32767);
});

test("parseLocalPort: empty or whitespace means auto (0)", () => {
  assert.deepEqual(parseLocalPort(""), { ok: true, port: 0 });
  assert.deepEqual(parseLocalPort("   "), { ok: true, port: 0 });
});

test("parseLocalPort: 0 is auto", () => {
  assert.deepEqual(parseLocalPort("0"), { ok: true, port: 0 });
});

test("parseLocalPort: accepts 1 and 32767, trims whitespace", () => {
  assert.deepEqual(parseLocalPort("1"), { ok: true, port: 1 });
  assert.deepEqual(parseLocalPort(" 32767 "), { ok: true, port: 32767 });
});

test("parseLocalPort: rejects 32768 and up with the reachability message", () => {
  for (const input of ["32768", "40000", "65535", "99999"]) {
    const r = parseLocalPort(input);
    assert.equal(r.ok, false, input);
    if (!r.ok) assert.match(r.error, /32768 and up/);
  }
});

test("parseLocalPort: rejects non-numeric, negative and fractional input", () => {
  for (const input of ["abc", "-1", "80.5", "8 0", "1e3"]) {
    const r = parseLocalPort(input);
    assert.equal(r.ok, false, input);
    if (!r.ok) assert.ok(r.error.length > 0);
  }
});

test("parseRemotePort: accepts 1 and 65535", () => {
  assert.deepEqual(parseRemotePort("1"), { ok: true, port: 1 });
  assert.deepEqual(parseRemotePort("65535"), { ok: true, port: 65535 });
  assert.deepEqual(parseRemotePort(" 8080 "), { ok: true, port: 8080 });
});

test("parseRemotePort: rejects empty, 0, above 65535 and non-numeric", () => {
  for (const input of ["", "   ", "0", "65536", "http", "-5", "3.5"]) {
    const r = parseRemotePort(input);
    assert.equal(r.ok, false, JSON.stringify(input));
    if (!r.ok) assert.ok(r.error.length > 0);
  }
});

test("forwardStatusTone maps each known status", () => {
  assert.equal(forwardStatusTone("active"), "success");
  assert.equal(forwardStatusTone("reconnecting"), "warning");
  assert.equal(forwardStatusTone("starting"), "info");
  assert.equal(forwardStatusTone("error"), "danger");
  assert.equal(forwardStatusTone("stopped"), "neutral");
});

test("forwardStatusTone: unknown status is neutral", () => {
  assert.equal(forwardStatusTone("weird"), "neutral");
  assert.equal(forwardStatusTone(""), "neutral");
});

const KUBECTL_POD = `apiVersion: v1
kind: Pod
metadata:
  name: web-0
  namespace: default
spec:
  containers:
  - image: nginx:1.27
    imagePullPolicy: IfNotPresent
    name: web
    ports:
    - containerPort: 80
      name: http
      protocol: TCP
    - containerPort: 9113
      protocol: UDP
    resources: {}
  - image: busybox
    name: sidecar
    ports:
    - containerPort: 8081
  - image: alpine
    name: worker
  restartPolicy: Always
status:
  phase: Running
  containerStatuses:
  - name: web
    ports: []
`;

test("parsePodContainerPorts reads kubectl-style flush lists", () => {
  assert.deepEqual(parsePodContainerPorts(KUBECTL_POD), [
    { container: "web", name: "http", port: 80, protocol: "TCP" },
    { container: "web", name: null, port: 9113, protocol: "UDP" },
    { container: "sidecar", name: null, port: 8081, protocol: "TCP" },
  ]);
});

test("parsePodContainerPorts ignores ports listed under status", () => {
  const ports = parsePodContainerPorts(KUBECTL_POD);
  assert.equal(ports.length, 3);
});

test("parsePodContainerPorts handles deeper, hand-indented YAML", () => {
  const yaml = `spec:
    containers:
        -   name: api
            ports:
                - containerPort: "8443"
                  name: "https"
                  protocol: "TCP"
`;
  assert.deepEqual(parsePodContainerPorts(yaml), [
    { container: "api", name: "https", port: 8443, protocol: "TCP" },
  ]);
});

test("parsePodContainerPorts skips ports without a valid containerPort", () => {
  const yaml = `spec:
  containers:
  - name: c
    ports:
    - name: no-number
      protocol: TCP
    - containerPort: 0
    - containerPort: 70000
    - containerPort: abc
    - containerPort: 443
`;
  assert.deepEqual(parsePodContainerPorts(yaml), [
    { container: "c", name: null, port: 443, protocol: "TCP" },
  ]);
});

test("parsePodContainerPorts defaults protocol to TCP and empty name to null", () => {
  const yaml = `spec:
  containers:
  - name: c
    ports:
    - containerPort: 53
      name: ""
`;
  assert.deepEqual(parsePodContainerPorts(yaml), [
    { container: "c", name: null, port: 53, protocol: "TCP" },
  ]);
});

test("parsePodContainerPorts returns [] when there are no containers or ports", () => {
  assert.deepEqual(parsePodContainerPorts(""), []);
  assert.deepEqual(parsePodContainerPorts("kind: Pod\nmetadata:\n  name: x\n"), []);
  assert.deepEqual(
    parsePodContainerPorts("spec:\n  containers:\n  - name: c\n    image: x\n"),
    [],
  );
  assert.deepEqual(parsePodContainerPorts("spec:\n  containers:\n  - name: c\n    ports: []\n"), []);
});

test("parsePodContainerPorts tolerates CRLF line endings", () => {
  const yaml = "spec:\r\n  containers:\r\n  - name: c\r\n    ports:\r\n    - containerPort: 80\r\n";
  assert.deepEqual(parsePodContainerPorts(yaml), [
    { container: "c", name: null, port: 80, protocol: "TCP" },
  ]);
});
