/**
 * Unit tests for the published-port exposure check.
 *
 * Pure logic only — no DOM, no engine, no Tauri IPC. Run with:
 *   node --test src/lib/portExposure.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  describeBinding,
  exposedContainers,
  exposureSummary,
  isPublicBinding,
  publicBindings,
} from "./portExposure.ts";
import type { ContainerItem, PublishedPort } from "../types/index.ts";

const port = (host_ip: string, host_port: number, container_port = host_port): PublishedPort => ({
  host_ip,
  host_port,
  container_port,
  protocol: "tcp",
});

const container = (
  name: string,
  published_ports?: PublishedPort[]
): ContainerItem => ({
  id: name,
  short_id: name,
  name: `/${name}`,
  image: "nginx",
  state: "running",
  status: "Up",
  created: 0,
  ports: "",
  published_ports,
});

test("wildcard and empty bind addresses count as public", () => {
  for (const ip of ["", "0.0.0.0", "::", "[::]", "*", " 0.0.0.0 "]) {
    assert.equal(isPublicBinding(ip), true, `${JSON.stringify(ip)} is public`);
  }
  assert.equal(isPublicBinding(undefined), true, "an absent address is 0.0.0.0");
});

test("loopback and a specific interface address are not flagged", () => {
  for (const ip of ["127.0.0.1", "::1", "[::1]", "192.168.1.10"]) {
    assert.equal(isPublicBinding(ip), false, `${ip} is not a wildcard`);
  }
});

test("publicBindings keeps only exposed ports", () => {
  const bindings = publicBindings(
    container("web", [port("0.0.0.0", 8080), port("127.0.0.1", 5432), port("", 9000)])
  );
  assert.deepEqual(
    bindings.map((p) => p.host_port),
    [8080, 9000]
  );
});

test("a container without structured ports is never exposed", () => {
  assert.deepEqual(publicBindings(container("legacy")), []);
});

test("describeBinding spells the wildcard out for the tooltip", () => {
  assert.equal(describeBinding(port("", 8080, 80)), "0.0.0.0:8080→80/tcp");
  assert.equal(
    describeBinding(port("127.0.0.1", 5432)),
    "127.0.0.1:5432→5432/tcp"
  );
});

test("exposureSummary names containers and caps the list", () => {
  const containers = [
    container("web", [port("0.0.0.0", 8080)]),
    container("api", [port("", 8000), port("", 8001), port("127.0.0.1", 9000)]),
    container("db", [port("127.0.0.1", 5432)]),
    container("cache", [port("::", 6379)]),
    container("worker", [port("[::]", 7000)]),
  ];

  assert.equal(exposedContainers(containers).length, 4);
  assert.equal(
    exposureSummary(containers),
    "web (8080), api (8000, 8001), cache (6379) +1 more"
  );
  assert.equal(exposureSummary(containers, 10), "web (8080), api (8000, 8001), cache (6379), worker (7000)");
});
