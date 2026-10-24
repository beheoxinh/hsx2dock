/**
 * Unit tests for the published-port exposure check.
 *
 * Pure logic only — no DOM, no engine, no Tauri IPC. Run with:
 *   node --test src/lib/portExposure.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  isPublicBinding,
  portChips,
} from "./portExposure.ts";
import type { PublishedPort } from "../types/index.ts";

const port = (host_ip: string, host_port: number, container_port = host_port): PublishedPort => ({
  host_ip,
  host_port,
  container_port,
  protocol: "tcp",
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

test("portChips renders one row per published port with a browser URL", () => {
  const chips = portChips({
    published_ports: [
      port("127.0.0.1", 8080, 80),
      port("0.0.0.0", 5432, 5432),
    ],
  });

  assert.equal(chips.length, 2);
  assert.deepEqual(chips[0], {
    hostPort: 8080,
    containerPort: 80,
    protocol: "tcp",
    public: false,
    label: "8080 → 80",
    url: "http://localhost:8080",
  });
  assert.equal(chips[1].public, true, "a wildcard binding is flagged public");
  assert.equal(chips[1].url, "http://localhost:5432");
});

test("portChips gives udp ports no browser URL", () => {
  const [chip] = portChips({
    published_ports: [
      { host_ip: "127.0.0.1", host_port: 5353, container_port: 53, protocol: "udp" },
    ],
  });
  assert.equal(chip.url, null, "a udp port cannot be opened in a browser");
  assert.equal(chip.public, false);
});

test("portChips falls back to the legacy ports string", () => {
  const chips = portChips({ ports: "0.0.0.0:8080->80/tcp, 127.0.0.1:5432->5432/tcp, 443/tcp" });

  assert.equal(chips.length, 3);
  assert.equal(chips[0].hostPort, 8080);
  assert.equal(chips[0].containerPort, 80);
  assert.equal(chips[0].public, true);
  assert.equal(chips[1].public, false);
  // Exposed-only: no host port, no URL.
  assert.equal(chips[2].hostPort, null);
  assert.equal(chips[2].url, null);
  assert.equal(chips[2].label, "443");
});

test("portChips handles an empty or absent ports value", () => {
  assert.deepEqual(portChips({ ports: "" }), []);
  assert.deepEqual(portChips({ ports: "—" }), []);
});
