import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const dir = mkdtempSync(join(tmpdir(), "aomi-answers-"));
process.env.WORLD_BRAIN_DIR = dir;

const { getAnswer, upsertAnswer, latestWorking } = await import("../src/answers.js");

test("answer projection is keyed by correlation id and is not a ledger row", () => {
  const opened = upsertAnswer("17", {
    correlation_id: "c1",
    question: "what's funding at",
    heard_echo: "what's funding at",
    status: "working",
  });
  assert.equal(opened.ok, true);
  assert.equal(opened.status, "working");
  assert.equal(opened.answer, null);
  const got = getAnswer("17", "c1");
  assert.equal(got.found, true);
  assert.equal(got.question, "what's funding at");
  const missing = getAnswer("17", "nope");
  assert.equal(missing.found, false);
});

test("projecting an answer does not duplicate the row", () => {
  upsertAnswer("17", {
    correlation_id: "c2",
    question: "what's my liq",
    status: "working",
  });
  const projected = upsertAnswer("17", {
    correlation_id: "c2",
    answer: "liq is 1800",
    status: "answered",
  });
  assert.equal(projected.question, "what's my liq");
  assert.equal(projected.answer, "liq is 1800");
  assert.equal(projected.status, "answered");
  assert.equal(getAnswer("17", "c2").answer, "liq is 1800");
});

test("latest working is the most recent unfinished projection", () => {
  upsertAnswer("17", { correlation_id: "old", status: "working" });
  upsertAnswer("17", { correlation_id: "done", status: "answered", answer: "x" });
  upsertAnswer("17", { correlation_id: "new", status: "working" });
  assert.equal(latestWorking("17").correlation_id, "new");
});
