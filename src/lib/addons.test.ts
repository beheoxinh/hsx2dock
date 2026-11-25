/**
 * Unit tests for the addon helpers. Run with:
 *   node --test src/lib/addons.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  DASHBOARD_ADDON_ID,
  INGRESS_ADDON_ID,
  addonPollIntervalMs,
  addonStateTone,
  isAddonBusy,
  toHttpsUrl,
} from "./addons.ts";
import type { AddonStatus } from "../types/index.ts";

function addon(state: string): AddonStatus {
  return {
    id: "x",
    name: "X",
    description: "",
    version: "v1",
    state,
    message: null,
    detail: null,
  };
}

test("addon ids match the backend catalog", () => {
  assert.equal(DASHBOARD_ADDON_ID, "dashboard");
  assert.equal(INGRESS_ADDON_ID, "ingress-nginx");
});

test("isAddonBusy is true only for installing and uninstalling", () => {
  assert.equal(isAddonBusy("installing"), true);
  assert.equal(isAddonBusy("uninstalling"), true);
  for (const s of ["ready", "progressing", "error", "not-installed", "unknown", ""]) {
    assert.equal(isAddonBusy(s), false, s);
  }
});

test("addonStateTone maps every known state", () => {
  assert.equal(addonStateTone("ready"), "success");
  assert.equal(addonStateTone("progressing"), "info");
  assert.equal(addonStateTone("installing"), "info");
  assert.equal(addonStateTone("uninstalling"), "info");
  assert.equal(addonStateTone("error"), "danger");
  assert.equal(addonStateTone("not-installed"), "neutral");
  assert.equal(addonStateTone("unknown"), "neutral");
});

test("addonStateTone falls back to neutral for unexpected states", () => {
  assert.equal(addonStateTone("something-new"), "neutral");
  assert.equal(addonStateTone(""), "neutral");
});

test("addonPollIntervalMs polls fast only while an addon is busy", () => {
  assert.equal(addonPollIntervalMs([]), 15000);
  assert.equal(addonPollIntervalMs([addon("ready"), addon("not-installed")]), 15000);
  assert.equal(addonPollIntervalMs([addon("ready"), addon("installing")]), 3000);
  assert.equal(addonPollIntervalMs([addon("uninstalling")]), 3000);
});

test("toHttpsUrl rewrites the scheme and keeps host and port", () => {
  assert.equal(toHttpsUrl("http://127.0.0.1:12345"), "https://127.0.0.1:12345");
  assert.equal(toHttpsUrl("http://localhost:8443/path?q=1"), "https://localhost:8443/path?q=1");
});

test("toHttpsUrl leaves other input unchanged", () => {
  assert.equal(toHttpsUrl("https://127.0.0.1:12345"), "https://127.0.0.1:12345");
  assert.equal(toHttpsUrl("ftp://host"), "ftp://host");
  assert.equal(toHttpsUrl("host:1234"), "host:1234");
  assert.equal(toHttpsUrl(""), "");
});
