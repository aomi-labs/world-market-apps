import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const {
  correctLiveTranscript,
  annotateLiveTranscript,
  setOntologyEntries,
} = require("../static/ontology_live.js");

test("live transcript rewrites instrument aliases", () => {
  setOntologyEntries([
    { surface_form: "ETH", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "ether", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "ethereum", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "bitcoin", normalized_target: "WBTC", kind: "instrument" },
    { surface_form: "solana", normalized_target: "SOL", kind: "instrument" },
  ]);
  assert.equal(correctLiveTranscript("buy fifty dollars worth of ether"), "buy fifty dollars worth of WETH");
  assert.equal(correctLiveTranscript("buy eth"), "buy WETH");
  assert.equal(correctLiveTranscript("watch ethereum"), "watch WETH");
  assert.equal(correctLiveTranscript("sell bitcoin"), "sell WBTC");
  assert.equal(correctLiveTranscript("watch solana"), "watch SOL");
});

test("live transcript maps phonetic ETH/SOL misses only", () => {
  setOntologyEntries([
    { surface_form: "ether", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "east", normalized_target: "WETH", kind: "confusable" },
    { surface_form: "eath", normalized_target: "WETH", kind: "confusable" },
    { surface_form: "soul", normalized_target: "SOL", kind: "confusable" },
    { surface_form: "salt", normalized_target: "SOL", kind: "confusable" },
    { surface_form: "beef", normalized_target: "WETH", kind: "confusable" },
    { surface_form: "it", normalized_target: "WETH", kind: "confusable" },
    { surface_form: "these", normalized_target: "WETH", kind: "confusable" },
  ]);
  assert.equal(correctLiveTranscript("buy one east"), "buy one WETH");
  assert.equal(correctLiveTranscript("sell soul"), "sell SOL");
  assert.equal(correctLiveTranscript("sell salt"), "sell SOL");
  assert.equal(correctLiveTranscript("buy fifty of beef"), "buy fifty of beef");
  assert.equal(correctLiveTranscript("watch it"), "watch it");
  assert.equal(correctLiveTranscript("buy these"), "buy these");
});

test("live transcript does not rewrite order_type tokens", () => {
  setOntologyEntries([
    { surface_form: "ETH", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "twap", normalized_target: "twap", kind: "order_type" },
    { surface_form: "dca", normalized_target: "dca", kind: "order_type" },
  ]);
  assert.equal(correctLiveTranscript("buy fifty ETH twap"), "buy fifty dollars worth of WETH twap");
  assert.equal(correctLiveTranscript("dca buy fifty ETH"), "dca buy fifty dollars worth of WETH");
});

test("setOntologyEntries reloads aliases", () => {
  setOntologyEntries([
    { surface_form: "ether", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "beef", normalized_target: "WETH", kind: "confusable" },
  ]);
  assert.equal(correctLiveTranscript("ether please"), "WETH please");
  assert.equal(correctLiveTranscript("beef please"), "beef please");
});

test("annotateLiveTranscript marks rewritten instrument spans", () => {
  setOntologyEntries([
    { surface_form: "ether", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "bitcoin", normalized_target: "WBTC", kind: "instrument" },
  ]);
  const spans = annotateLiveTranscript("buy fifty dollars worth of ether");
  const ether = spans.find((span) => span.surface.toLowerCase() === "ether");
  assert.equal(ether.display, "WETH");
  assert.equal(ether.rewritten, true);
  const buy = spans.find((span) => span.surface === "buy");
  assert.equal(buy.rewritten, false);
  assert.equal(buy.display, "buy");
});

test("annotateLiveTranscript leaves confusables unmarked", () => {
  setOntologyEntries([
    { surface_form: "ether", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "beef", normalized_target: "WETH", kind: "confusable" },
  ]);
  const spans = annotateLiveTranscript("buy fifty of beef");
  const beef = spans.find((span) => span.surface === "beef");
  assert.equal(beef.display, "beef");
  assert.equal(beef.rewritten, false);
});

test("live transcript restores buy when a leading 5.05 fused the command", () => {
  setOntologyEntries([
    { surface_form: "ETH", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "ether", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "buy", normalized_target: "buy", kind: "act" },
    { surface_form: "sell", normalized_target: "sell", kind: "act" },
    { surface_form: "how", normalized_target: "how", kind: "opener" },
  ]);
  assert.equal(correctLiveTranscript("5.05 ETH"), "buy 5 WETH");
  assert.equal(correctLiveTranscript("5.05 ether"), "buy 5 WETH");
  assert.equal(correctLiveTranscript("5 ETH"), "buy 5 WETH");
  assert.equal(correctLiveTranscript("buy 5.05 ETH"), "buy 5.05 WETH");
  assert.equal(correctLiveTranscript("sell 5 ETH"), "sell 5 WETH");
  assert.equal(correctLiveTranscript("how much is 5 ETH"), "how much is 5 WETH");
});

test("live transcript collapses five-five-eight into buy 5 ETH", () => {
  setOntologyEntries([
    { surface_form: "ETH", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "SOL", normalized_target: "SOL", kind: "instrument" },
    { surface_form: "buy", normalized_target: "buy", kind: "act" },
    { surface_form: "sell", normalized_target: "sell", kind: "act" },
  ]);
  assert.equal(correctLiveTranscript("five five eight"), "buy 5 WETH");
  assert.equal(correctLiveTranscript("58"), "buy 5 WETH");
  assert.equal(correctLiveTranscript("5 8"), "buy 5 WETH");
  assert.equal(correctLiveTranscript("buy 5 eight"), "buy 5 WETH");
  assert.equal(correctLiveTranscript("buy 58"), "buy 5 WETH");
  assert.equal(correctLiveTranscript("buy 5 SOL"), "buy 5 SOL");
  assert.equal(correctLiveTranscript("buy 58 SOL"), "buy 58 dollars worth of SOL");
});

test("live transcript restores dollars worth of when STT drops the frame", () => {
  setOntologyEntries([
    { surface_form: "ETH", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "ether", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "wrapped ether", normalized_target: "WETH", kind: "instrument" },
    { surface_form: "SOL", normalized_target: "SOL", kind: "instrument" },
    { surface_form: "salt", normalized_target: "SOL", kind: "confusable" },
    { surface_form: "buy", normalized_target: "buy", kind: "act" },
    { surface_form: "sell", normalized_target: "sell", kind: "act" },
  ]);
  assert.equal(correctLiveTranscript("buy fifty ether"), "buy fifty dollars worth of WETH");
  assert.equal(correctLiveTranscript("buy 50 ETH"), "buy 50 dollars worth of WETH");
  assert.equal(correctLiveTranscript("buy 50 of ETH"), "buy 50 dollars worth of WETH");
  assert.equal(correctLiveTranscript("open 50 ETH long"), "open 50 dollars worth of WETH long");
  assert.equal(correctLiveTranscript("buy ETH 50"), "buy 50 dollars worth of WETH");
  assert.equal(correctLiveTranscript("buy ETH with 50"), "buy 50 dollars worth of WETH");
  assert.equal(correctLiveTranscript("put 50 into ether"), "put 50 dollars worth of WETH");
  assert.equal(correctLiveTranscript("buy 50 wrapped ether"), "buy 50 dollars worth of WETH");
  assert.equal(correctLiveTranscript("twap 50 ETH"), "twap 50 dollars worth of WETH");
  assert.equal(correctLiveTranscript("buy 5 ETH"), "buy 5 WETH");
  assert.equal(correctLiveTranscript("buy 0.02 ether"), "buy 0.02 WETH");
  assert.equal(
    correctLiveTranscript("buy fifty dollars worth of ether"),
    "buy fifty dollars worth of WETH",
  );
  assert.equal(correctLiveTranscript("a 20 yards worth of SOL"), "buy 20 dollars worth of SOL");
  assert.equal(correctLiveTranscript("by 20 worth of salt"), "buy 20 dollars worth of SOL");
  assert.equal(correctLiveTranscript("wait 20 worth of SOL"), "buy 20 dollars worth of SOL");
  assert.equal(correctLiveTranscript("about 5 dollars worth"), "buy 5 dollars worth");
  assert.equal(
    correctLiveTranscript("about five dollars worth of ether"),
    "buy five dollars worth of WETH",
  );
  assert.equal(correctLiveTranscript("about 50 ETH"), "buy 50 dollars worth of WETH");
  assert.equal(
    correctLiveTranscript("what about 5 dollars worth of ether"),
    "what about 5 dollars worth of WETH",
  );
  assert.equal(correctLiveTranscript("buy 20 worth of SOL"), "buy 20 dollars worth of SOL");
  assert.equal(correctLiveTranscript("$550 worth of ETH"), "buy fifty dollars worth of WETH");
  assert.equal(correctLiveTranscript("buy 550 worth of ETH"), "buy fifty dollars worth of WETH");
  assert.equal(correctLiveTranscript("I have 50 ETH"), "buy 50 dollars worth of WETH");
  assert.equal(
    correctLiveTranscript("I have fifty dollars worth of ether"),
    "buy fifty dollars worth of WETH",
  );
  assert.equal(correctLiveTranscript("I've 50 ETH"), "buy 50 dollars worth of WETH");
  assert.equal(correctLiveTranscript("well 50 ETH"), "sell 50 dollars worth of WETH");
  assert.equal(
    correctLiveTranscript("cell fifty dollars worth of ether"),
    "sell fifty dollars worth of WETH",
  );
  assert.equal(correctLiveTranscript("so 50 ETH"), "sell 50 dollars worth of WETH");
  assert.equal(
    correctLiveTranscript("so fifty dollars worth of ether"),
    "sell fifty dollars worth of WETH",
  );
});
