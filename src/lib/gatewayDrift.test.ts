import test from "node:test";
import assert from "node:assert/strict";

import { mappedTargetDrift, parseTarget } from "./gatewayDrift.ts";
import type { ContainerItem, GatewayRoute } from "../types/index.ts";

const route = (target: string): GatewayRoute => ({
  id: "r1",
  domain: "app.local",
  target,
  path: null,
  strip_prefix: false,
  container_name: "web",
  group_name: null,
  enabled: true,
  auto_detected: false,
});

const container = (hostPorts: number[], state = "running"): ContainerItem => ({
  id: "abc",
  short_id: "abc",
  name: "/web",
  image: "nginx",
  state,
  status: "",
  created: 0,
  ports: "",
  published_ports: hostPorts.map((p) => ({
    host_ip: "127.0.0.1",
    host_port: p,
    container_port: 80,
    protocol: "tcp",
  })),
});

test("parses host:port targets", () => {
  assert.deepEqual(parseTarget("http://127.0.0.1:8080/"), { host: "127.0.0.1", port: 8080 });
  assert.equal(parseTarget("localhost"), null);
  assert.equal(parseTarget("x:99999"), null);
});

test("is quiet while the target port is still published", () => {
  assert.equal(mappedTargetDrift(route("127.0.0.1:8080"), container([8080])).drifted, false);
});

test("suggests the single new port after a recreate", () => {
  assert.deepEqual(mappedTargetDrift(route("127.0.0.1:8080"), container([32768])), {
    drifted: true,
    suggestion: "127.0.0.1:32768",
  });
});

test("flags but does not guess between several ports", () => {
  assert.deepEqual(mappedTargetDrift(route("127.0.0.1:8080"), container([1, 2])), {
    drifted: true,
    suggestion: null,
  });
});

test("makes no claim for stopped or unpublished containers", () => {
  assert.equal(mappedTargetDrift(route("127.0.0.1:8080"), container([9], "exited")).drifted, false);
  assert.equal(mappedTargetDrift(route("127.0.0.1:8080"), container([])).drifted, false);
  assert.equal(mappedTargetDrift(route("127.0.0.1:8080"), undefined).drifted, false);
});
