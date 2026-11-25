/**
 * Unit tests for the Compose page helpers (project model, .env, line diff).
 *
 * Pure logic only. Run with:
 *   node --test src/lib/compose.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  defaultDomain,
  dependencyLayers,
  filterProjects,
  projectCounts,
  projectHealth,
  projectPorts,
  projectStatus,
  publishedPorts,
  serviceRows,
  sortProjects,
} from "./composeProject.ts";
import { duplicateKeys, isSecretKey, isValidEnvKey, parseEnv, serializeEnv } from "./dotenv.ts";
import { diffCounts, diffLines, withContext } from "./textDiff.ts";
import type { ComposeProjectInfo, ComposeServiceSpec } from "../types/index.ts";

const spec = (name: string, extra: Partial<ComposeServiceSpec> = {}): ComposeServiceSpec => ({
  name,
  image: `${name}:latest`,
  build: false,
  depends_on: [],
  ports: [],
  profiles: [],
  healthcheck: false,
  replicas: null,
  container_name: "",
  ...extra,
});

const ctr = (service: string, state = "running", ports = "", health = "") => ({
  id: `id-${service}-${state}`,
  name: `p-${service}-1`,
  service,
  image: "",
  state,
  health,
  ports,
});

const project = (over: Partial<ComposeProjectInfo> = {}): ComposeProjectInfo => ({
  name: "shop",
  path: "/srv/shop/compose.yaml",
  services: [],
  declared: [spec("api", { depends_on: ["db"] }), spec("db"), spec("debug", { profiles: ["tools"] })],
  ...over,
});

test("service rows join declared services with their containers", () => {
  const p = project({ services: [ctr("api"), ctr("api", "exited"), ctr("legacy")] });
  const rows = serviceRows(p);
  assert.deepEqual(
    rows.map((r) => [r.name, r.containers.length, r.running, r.optional, r.spec !== null]),
    [
      ["api", 2, 1, false, true],
      ["db", 0, 0, false, true],
      ["debug", 0, 0, true, true],
      ["legacy", 1, 1, false, false],
    ]
  );
});

test("status ignores unused profile services", () => {
  assert.equal(projectStatus(project()), "stopped");
  assert.equal(projectStatus(project({ services: [ctr("api")] })), "partial");
  const all = project({ services: [ctr("api"), ctr("db")] });
  assert.deepEqual(projectCounts(all), { running: 2, total: 2 });
  assert.equal(projectStatus(all), "running");
  // A profile service with a stopped container counts against the project.
  assert.equal(projectStatus(project({ services: [ctr("api"), ctr("db"), ctr("debug", "exited")] })), "partial");
  assert.equal(projectStatus(project({ declared: [], config_error: "bad yaml" })), "error");
});

test("published ports are parsed and deduplicated", () => {
  assert.deepEqual(publishedPorts("0.0.0.0:8080->80/tcp, :::8080->80/tcp, 53/udp, 127.0.0.1:5432->5432/tcp"), [
    { published: 8080, target: 80, protocol: "tcp" },
    { published: 5432, target: 5432, protocol: "tcp" },
  ]);
  const p = project({ services: [ctr("api", "running", "0.0.0.0:9000->9000/tcp"), ctr("db", "running", "8080->80/tcp")] });
  assert.deepEqual(projectPorts(p).map((x) => x.published), [8080, 9000]);
});

test("health summary prefers the worst state", () => {
  assert.equal(projectHealth(project({ services: [ctr("api", "running", "", "healthy"), ctr("db", "running", "", "unhealthy")] })), "unhealthy");
  assert.equal(projectHealth(project({ services: [ctr("api", "running", "", "healthy")] })), "healthy");
  assert.equal(projectHealth(project()), "none");
});

test("filter and sort", () => {
  const a = project({ name: "alpha", services: [ctr("api"), ctr("db")] });
  const b = project({ name: "beta", declared: [spec("web", { image: "nginx:alpine" })] });
  assert.deepEqual(filterProjects([a, b], "nginx", "all").map((p) => p.name), ["beta"]);
  assert.deepEqual(filterProjects([a, b], "", "running").map((p) => p.name), ["alpha"]);
  assert.deepEqual(filterProjects([a, b], "srv/shop", "all").length, 2);
  assert.deepEqual(sortProjects([b, a], "name").map((p) => p.name), ["alpha", "beta"]);
  assert.deepEqual(sortProjects([b, a], "status").map((p) => p.name), ["alpha", "beta"]);
  assert.deepEqual(sortProjects([b, a], "services").map((p) => p.name), ["alpha", "beta"]);
});

test("dependency layers put dependencies left and survive cycles", () => {
  const layers = dependencyLayers([
    spec("web", { depends_on: ["api"] }),
    spec("api", { depends_on: ["db", "cache", "missing"] }),
    spec("db"),
    spec("cache", { depends_on: ["db"] }),
  ]);
  assert.deepEqual(Object.fromEntries(layers), { db: 0, cache: 1, api: 2, web: 3 });
  const cyclic = dependencyLayers([spec("a", { depends_on: ["b"] }), spec("b", { depends_on: ["a"] })]);
  assert.equal(cyclic.size, 2);
});

test("default gateway domain", () => {
  assert.equal(defaultDomain("My Shop", "web_api"), "web-api.my-shop.localhost");
});

test(".env round-trips untouched lines and edits variables", () => {
  const text = "# db settings\nDB_NAME=shop\n\nexport TOKEN=\"a b\"\nnot a var\n";
  const rows = parseEnv(text);
  assert.equal(serializeEnv(rows), text);
  assert.deepEqual(rows[3], { kind: "var", key: "TOKEN", value: "\"a b\"", exported: true });
  const edited = rows.map((r) => (r.kind === "var" && r.key === "DB_NAME" ? { ...r, value: "prod" } : r));
  assert.equal(serializeEnv(edited), text.replace("DB_NAME=shop", "DB_NAME=prod"));
  assert.equal(serializeEnv([]), "");
  assert.deepEqual(parseEnv(""), []);
  assert.deepEqual([...duplicateKeys(parseEnv("A=1\nB=2\nA=3\n"))], ["A"]);
  assert.ok(isSecretKey("DB_PASSWORD") && isSecretKey("api_key") && !isSecretKey("DB_NAME"));
  assert.ok(isValidEnvKey("A_B.c") && !isValidEnvKey("1A") && !isValidEnvKey("A B"));
});

test("line diff marks changes and keeps context", () => {
  const before = "a\nb\nc\nd\ne\nf\ng\nh\n";
  const after = "a\nb\nC\nd\ne\nf\ng\nh\ni\n";
  const lines = diffLines(before, after);
  assert.deepEqual(diffCounts(lines), { added: 2, removed: 1 });
  assert.deepEqual(
    lines.filter((l) => l.kind !== "same").map((l) => `${l.kind}:${l.text}`),
    ["del:c", "add:C", "add:i"]
  );
  const ctx = withContext(lines, 1);
  assert.deepEqual(
    ctx.map((l) => (l ? l.text : "…")),
    ["b", "c", "C", "d", "…", "h", "i"]
  );
  assert.deepEqual(diffCounts(diffLines("x\n", "x\n")), { added: 0, removed: 0 });
  assert.deepEqual(diffCounts(diffLines("", "x\ny")), { added: 2, removed: 0 });
});

import { findYamlPath, locateComposeError } from "./yamlLocate.ts";

const YAML = [
  "services:",
  "  api:",
  "    image: busybox",
  "    ports: [1]",
  "  web:",
  "    image: nginx",
  "    portz: [1]",
  "volumes:",
  "  data: {}",
].join("\n");

test("findYamlPath walks nested keys and stays inside the parent block", () => {
  assert.equal(findYamlPath(YAML, ["services", "web", "portz"]), 7);
  assert.equal(findYamlPath(YAML, ["services", "api", "ports"]), 4);
  // web has no ports: the deepest found key is web itself, not api's ports.
  assert.equal(findYamlPath(YAML, ["services", "web", "ports"]), 5);
  assert.equal(findYamlPath(YAML, ["volumes", "data"]), 9);
  assert.equal(findYamlPath(YAML, ["nope"]), null);
});

test("locateComposeError reads parser lines, key paths and service names", () => {
  assert.deepEqual(locateComposeError("yaml: line 4: did not find expected key", YAML), { line: 4 });
  assert.deepEqual(locateComposeError("validating -: services.web.portz false schema", YAML), { line: 7 });
  assert.deepEqual(
    locateComposeError('service "web" depends on undefined service "nope": invalid compose project', YAML),
    { line: 5 }
  );
  assert.equal(locateComposeError("something else", YAML), null);
});
