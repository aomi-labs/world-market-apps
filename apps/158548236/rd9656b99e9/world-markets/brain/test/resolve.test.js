import assert from "node:assert/strict";
import test from "node:test";
import { looksLikeExecution, resolvePredicate } from "../src/resolve.js";

test("exact percent phrase stores a closed predicate", () => {
  const got = resolvePredicate({
    phrase: "up 5% in a day",
    symbol: "ETH",
    token_id: 2,
  });
  assert.equal(got.ok, true);
  assert.equal(got.predicate.kind, "pct_move");
  assert.equal(got.predicate.pct, "5");
  assert.match(got.predicate.resolved, /ETH/);
});

test("vague rally asks one clarifying question and stores nothing", () => {
  const got = resolvePredicate({ phrase: "watch the ETH rally", symbol: "ETH" });
  assert.equal(got.ok, false);
  assert.equal(got.needs_clarification, true);
  assert.ok(got.options.length >= 2);
});

test("folding a buy into a watch is the execution wall", () => {
  assert.equal(looksLikeExecution("watch ETH and buy $500 if it does"), true);
  const got = resolvePredicate({
    phrase: "watch ETH and buy 500 if it does",
    symbol: "ETH",
  });
  assert.equal(got.execution_folded, true);
  assert.equal(got.ok, false);
});

test("funding and risk phrases are not stolen by the price parser", () => {
  const funding = resolvePredicate({
    phrase: "funding above 0.01%",
    symbol: "ETH",
  });
  assert.equal(funding.predicate.kind, "funding");
  assert.equal(funding.predicate.rate_pct, "0.01");
  const risk = resolvePredicate({
    phrase: "risk above 8",
    symbol: "ETH",
  });
  assert.equal(risk.predicate.kind, "risk");
  assert.equal(risk.predicate.level, "8");
});
