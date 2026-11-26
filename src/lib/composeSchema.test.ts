import { test } from "node:test";
import assert from "node:assert/strict";
import { composeSuggestions, yamlKeyPath } from "./composeSchema.ts";

/** Splits `doc` at the "|" marker and returns the text and the cursor offset. */
function at(marked: string): { doc: string; offset: number } {
  const offset = marked.indexOf("|");
  return { doc: marked.slice(0, offset) + marked.slice(offset + 1), offset };
}

const keysOf = (marked: string) => {
  const { doc, offset } = at(marked);
  return composeSuggestions(doc, offset)?.options.map((o) => o.label) ?? [];
};

test("yamlKeyPath follows indentation to the enclosing keys", () => {
  const { doc, offset } = at("services:\n  web:\n    healthcheck:\n      |");
  assert.deepEqual(yamlKeyPath(doc, offset), ["services", "web", "healthcheck"]);
});

test("yamlKeyPath is empty at the top level and skips comments", () => {
  assert.deepEqual(yamlKeyPath(...unpack(at("|"))), []);
  const { doc, offset } = at("services:\n  web:\n    # note: not a key\n    |");
  assert.deepEqual(yamlKeyPath(doc, offset), ["services", "web"]);
});

test("yamlKeyPath marks sequence items", () => {
  const { doc, offset } = at("services:\n  web:\n    volumes:\n      - type: bind\n        |");
  assert.deepEqual(yamlKeyPath(doc, offset), ["services", "web", "volumes", "[]"]);
});

test("top-level keys are suggested with a colon, x- without one", () => {
  const { doc, offset } = at("ser|");
  const result = composeSuggestions(doc, offset);
  assert.ok(result);
  assert.equal(result.from, 0);
  assert.deepEqual(result.options.map((o) => o.label), ["services"]);
  assert.equal(result.options[0].insert, "services: ");
  assert.equal(result.options[0].kind, "key");

  const all = composeSuggestions(...unpack(at("|")));
  assert.ok(all?.options.some((o) => o.label === "x-" && o.insert === "x-"));
});

test("service keys are offered inside a service", () => {
  const { doc, offset } = at("services:\n  web:\n    im|");
  const result = composeSuggestions(doc, offset);
  assert.ok(result);
  assert.equal(result.from, offset - 2);
  assert.deepEqual(result.options.map((o) => o.label), ["image"]);
  assert.ok(result.options[0].description.length > 0);
});

test("nested healthcheck keys are offered", () => {
  const labels = keysOf("services:\n  web:\n    healthcheck:\n      |");
  for (const k of ["test", "interval", "timeout", "retries", "start_period", "start_interval", "disable"]) {
    assert.ok(labels.includes(k), `missing ${k}`);
  }
});

test("any service name resolves to the service schema", () => {
  const labels = keysOf("services:\n  my-api:\n    deploy:\n      resources:\n        |");
  assert.deepEqual(labels, ["limits", "reservations"]);
});

test("long-form depends_on keys are offered under a service name", () => {
  const labels = keysOf("services:\n  web:\n    depends_on:\n      db:\n        |");
  assert.deepEqual(labels, ["condition", "restart", "required"]);
});

test("restart values are offered after 'restart: '", () => {
  const { doc, offset } = at("services:\n  web:\n    restart: on-|");
  const result = composeSuggestions(doc, offset);
  assert.ok(result);
  assert.equal(result.from, offset - 3);
  assert.deepEqual(result.options.map((o) => o.label), ["on-failure"]);
  assert.equal(result.options[0].kind, "value");
  assert.equal(result.options[0].insert, "on-failure");
});

test("depends_on condition values are offered for a named dependency", () => {
  const labels = keysOf("services:\n  web:\n    depends_on:\n      db:\n        condition: service_h|");
  assert.deepEqual(labels, ["service_healthy"]);
});

test("a list item's keys are offered under its sequence", () => {
  const labels = keysOf("services:\n  web:\n    volumes:\n      - type: bind\n        so|");
  assert.deepEqual(labels, ["source"]);
});

test("unknown paths and plain values yield no suggestions", () => {
  assert.equal(composeSuggestions(...unpack(at("foo:\n  |"))), null);
  assert.equal(composeSuggestions(...unpack(at("services:\n  web:\n    image: nginx|"))), null);
});

test("CRLF documents keep the same key path", () => {
  const { doc, offset } = at("services:\r\n  web:\r\n    im|");
  assert.deepEqual(yamlKeyPath(doc, offset), ["services", "web"]);
  assert.deepEqual(keysOf("services:\r\n  web:\r\n    im|"), ["image"]);
});

function unpack(p: { doc: string; offset: number }): [string, number] {
  return [p.doc, p.offset];
}
