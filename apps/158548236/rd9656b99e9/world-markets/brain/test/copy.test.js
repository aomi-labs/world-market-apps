import assert from "node:assert/strict";
import test from "node:test";
import {
  SHARE,
  CANT,
  alreadyTrueMessage,
  attachSetCopy,
  bundleMessage,
  expiredMessage,
  firedMessage,
  proseBlocks,
  setMessage,
  templateSlots,
} from "../src/copy.js";

const watch = {
  mark_at_set: "2180",
  expires_at: 1_700_000_000 + 10 * 86400,
  predicate: {
    kind: "pct_move",
    symbol: "ETH",
    op: "gte",
    pct: "5",
    resolved: "ETH ≥ +5% / 1d",
  },
};

test("set copy is tool-filled and never a trade", () => {
  const text = setMessage(watch, 1_700_000_000);
  assert.match(text, /`ETH`/);
  assert.match(text, /`2180`/);
  assert.match(text, /I won't buy or sell anything/);
  assert.equal(text.includes("so that's"), false);
});

test("already-true copy names the mark and the two controls", () => {
  const text = alreadyTrueMessage({
    symbol: "WETH",
    predicate: { symbol: "WETH", level: "3000" },
    now: "2465.71",
  });
  assert.match(text, /already true/);
  assert.match(text, /`WETH`/);
  assert.match(text, /`2465.71`/);
  assert.match(text, /`3000`/);
  const attached = attachSetCopy({
    ok: true,
    stored: false,
    already_true: true,
    now: "2465.71",
    symbol: "WETH",
    predicate: { symbol: "WETH", level: "3000" },
  });
  assert.deepEqual(attached.controls, [
    "Watch the next crossing",
    "Change the level",
  ]);
  assert.equal(attached.already_true, true);
  assert.equal(attached.now, "2465.71");
});

test("clarify and fold return paste-ready messages", () => {
  const clarify = attachSetCopy({
    ok: true,
    stored: false,
    needs_clarification: true,
    symbol: "ETH",
    options: [{ label: "Up 5% in a day" }, { label: "Pick a price" }],
  });
  assert.match(clarify.message, /`ETH`/);
  assert.deepEqual(clarify.controls, ["Up 5% in a day", "Pick a price"]);
  const folded = attachSetCopy({
    ok: true,
    stored: false,
    execution_folded: true,
    symbol: "ETH",
  });
  assert.match(folded.message, /signed on World/);
});

test("share copy register has no bangs and 160-char blocks", () => {
  assert.deepEqual(templateSlots(SHARE.m10_with_name), ["first_name", "ref_link"]);
  assert.deepEqual(templateSlots(SHARE.m10_anon), ["ref_link"]);
  for (const value of Object.values(SHARE)) {
    assert.equal(String(value).includes("!"), false, value);
    for (const block of proseBlocks(value)) {
      assert.ok(block.length <= 160, block);
    }
  }
});

test("cant copy register has no bangs and 160-char blocks", () => {
  for (const value of Object.values(CANT)) {
    assert.equal(String(value).includes("!"), false, value);
    for (const block of proseBlocks(value)) {
      assert.ok(block.length <= 160, block);
    }
  }
});

test("unclear is the non-trade register", () => {
  assert.match(CANT.unclear, /I trade crypto spot, perps, and lending/);
  assert.match(CANT.unclear, /\/p/);
  assert.equal(/say buy/i.test(CANT.unclear), false);
});

test("fire, expire, and bundle copy use record fields only", () => {
  const fire = {
    created_at: 1_700_000_000,
    live: "2290",
    spent: true,
    predicate: { symbol: "ETH", resolved: "ETH ≥ +5% / 1d" },
    original_phrase: "up 5% in a day",
  };
  assert.match(firedMessage(fire), /`ETH`/);
  assert.match(firedMessage(fire), /`2290`/);
  assert.match(expiredMessage(fire), /expired without firing/);
  assert.match(bundleMessage([fire]), /`1` more/);
});
