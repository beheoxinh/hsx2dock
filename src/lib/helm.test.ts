/**
 * Unit tests for the Helm helpers.
 *
 * Pure logic only. Run with:
 *   node --test src/lib/helm.test.ts
 */
import test from "node:test";
import assert from "node:assert/strict";

import {
  QUICK_REPOS,
  cleanValuesText,
  currentRevision,
  defaultChartRef,
  errorMessage,
  helmStatusTone,
  isValidReleaseName,
  shortUpdated,
} from "./helm.ts";

// ---------------------------------------------------------------------------
// defaultChartRef
// ---------------------------------------------------------------------------

test("defaultChartRef strips a numeric version suffix", () => {
  assert.equal(defaultChartRef("ingress-nginx-4.11.0"), "ingress-nginx");
});

test("defaultChartRef strips a v-prefixed version suffix", () => {
  assert.equal(defaultChartRef("redis-v17.0.5"), "redis");
});

test("defaultChartRef strips a prerelease tail after the version", () => {
  assert.equal(defaultChartRef("app-1.2.3-rc.1"), "app");
});

test("defaultChartRef keeps the repo prefix", () => {
  assert.equal(defaultChartRef("bitnami/redis-17.0.5"), "bitnami/redis");
});

test("defaultChartRef leaves names without a version suffix alone", () => {
  assert.equal(defaultChartRef("redis"), "redis");
  assert.equal(defaultChartRef("bitnami/redis"), "bitnami/redis");
});

test("defaultChartRef does not treat a bare dotless number as a version", () => {
  assert.equal(defaultChartRef("app-2048"), "app-2048");
  assert.equal(defaultChartRef("2048"), "2048");
  assert.equal(defaultChartRef("1.2.3"), "1.2.3");
});

test("defaultChartRef never returns an empty string", () => {
  assert.equal(defaultChartRef("-1.2.3"), "-1.2.3");
});

// ---------------------------------------------------------------------------
// isValidReleaseName
// ---------------------------------------------------------------------------

test("isValidReleaseName accepts lowercase names with digits and dashes", () => {
  assert.equal(isValidReleaseName("a"), true);
  assert.equal(isValidReleaseName("my-app"), true);
  assert.equal(isValidReleaseName("app2"), true);
});

test("isValidReleaseName rejects bad starts, bad ends and bad characters", () => {
  assert.equal(isValidReleaseName(""), false);
  assert.equal(isValidReleaseName("1app"), false);
  assert.equal(isValidReleaseName("-app"), false);
  assert.equal(isValidReleaseName("my-"), false);
  assert.equal(isValidReleaseName("My"), false);
  assert.equal(isValidReleaseName("a_b"), false);
  assert.equal(isValidReleaseName("a.b"), false);
});

test("isValidReleaseName caps the length at 53", () => {
  assert.equal(isValidReleaseName("a".repeat(53)), true);
  assert.equal(isValidReleaseName("a".repeat(54)), false);
});

// ---------------------------------------------------------------------------
// helmStatusTone
// ---------------------------------------------------------------------------

test("helmStatusTone maps each Helm status to its tone", () => {
  assert.equal(helmStatusTone("deployed"), "success");
  assert.equal(helmStatusTone("pending-install"), "info");
  assert.equal(helmStatusTone("pending-upgrade"), "info");
  assert.equal(helmStatusTone("failed"), "danger");
  assert.equal(helmStatusTone("uninstalling"), "warning");
  assert.equal(helmStatusTone("superseded"), "warning");
  assert.equal(helmStatusTone("uninstalled"), "neutral");
  assert.equal(helmStatusTone(""), "neutral");
});

test("helmStatusTone ignores case and surrounding space", () => {
  assert.equal(helmStatusTone("  Deployed "), "success");
});

// ---------------------------------------------------------------------------
// QUICK_REPOS
// ---------------------------------------------------------------------------

test("QUICK_REPOS lists bitnami and ingress-nginx with their URLs", () => {
  assert.deepEqual(QUICK_REPOS, [
    { name: "bitnami", url: "https://charts.bitnami.com/bitnami" },
    { name: "ingress-nginx", url: "https://kubernetes.github.io/ingress-nginx" },
  ]);
});

// ---------------------------------------------------------------------------
// shortUpdated / cleanValuesText / currentRevision / errorMessage
// ---------------------------------------------------------------------------

test("shortUpdated drops fractional seconds and the zone suffix", () => {
  assert.equal(shortUpdated("2026-11-20 10:00:00.123456789 +0000 UTC"), "2026-11-20 10:00:00");
  assert.equal(shortUpdated("2026-11-20 10:00:00 +0000 UTC"), "2026-11-20 10:00:00");
  assert.equal(shortUpdated("  "), "");
});

test("cleanValuesText returns empty for null, {} and blank values", () => {
  assert.equal(cleanValuesText("null"), "");
  assert.equal(cleanValuesText(" {} \n"), "");
  assert.equal(cleanValuesText(""), "");
  assert.equal(cleanValuesText("replicas: 2\n"), "replicas: 2\n");
});

test("currentRevision is the highest revision, or 0 for an empty history", () => {
  assert.equal(currentRevision([]), 0);
  assert.equal(currentRevision([{ revision: 1 }, { revision: 3 }, { revision: 2 }]), 3);
});

test("errorMessage surfaces string, Error and other errors as text", () => {
  assert.equal(errorMessage("helm failed"), "helm failed");
  assert.equal(errorMessage(new Error("boom")), "boom");
  assert.equal(errorMessage(42), "42");
});
