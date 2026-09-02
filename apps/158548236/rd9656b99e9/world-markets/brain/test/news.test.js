import assert from "node:assert/strict";
import test from "node:test";
import { causeFromHeadline, newsTicker } from "../src/news/source.js";
import { SOURCES, enabledSourceIds } from "../src/news/index.js";

test("newsTicker maps World wrappers to vendor tickers", () => {
  assert.equal(newsTicker("WETH"), "ETH");
  assert.equal(newsTicker("WBTC"), "BTC");
  assert.equal(newsTicker("ETH"), "ETH");
});

test("causeFromHeadline requires the ticker and a causal cue", () => {
  assert.equal(
    causeFromHeadline("ETH slides after ETF outflows", "ETH"),
    "ETH slides after ETF outflows",
  );
  assert.equal(causeFromHeadline("ETH trades sideways on Tuesday", "ETH"), null);
  assert.equal(causeFromHeadline("SOL rallies after unlock", "ETH"), null);
});

test("cryptocompare is registered and is the default source", () => {
  const previous = process.env.WORLD_NEWS_SOURCES;
  delete process.env.WORLD_NEWS_SOURCES;
  assert.ok(SOURCES.cryptocompare);
  assert.equal(SOURCES.cryptocompare.id, "cryptocompare");
  assert.deepEqual(enabledSourceIds(), ["cryptocompare"]);
  if (previous == null) delete process.env.WORLD_NEWS_SOURCES;
  else process.env.WORLD_NEWS_SOURCES = previous;
});
