import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "aomi-cant-"));
process.env.WORLD_BRAIN_DIR = dir;

const { handleHeard } = await import("../src/cant.js");
const { summary, listInstructions, getInstruction } = await import("../src/instructions.js");
const { lexiconOf } = await import("../src/voice.js");

const UNIVERSE = [
  { symbol: "ETH", name: "Ether" },
  { symbol: "WETH", name: "Wrapped Ether" },
  { symbol: "BIFI", name: "Beefy Finance" },
];

function heard(account, text, extra = {}) {
  return handleHeard(account, { text, universe: UNIVERSE, ...extra });
}

test("source cannot reach execute helpers", () => {
  const src = fs.readFileSync(path.join(path.dirname(new URL(import.meta.url).pathname), "../src/cant.js"), "utf8");
  assert.equal(src.includes("stageTrade"), false);
  assert.equal(src.includes("beginExecute"), false);
  assert.equal(src.includes("completeExecute"), false);
  assert.equal(src.includes("execute_"), false);
});

test("buy beef walls immediately as food, never a near-match", () => {
  const account = "cant-1";
  const first = heard(account, "Buy me $50 of beef");
  assert.equal(first.kind, "cant");
  assert.equal(first.skip_llm, true);
  assert.equal(first.executable, false);
  assert.match(first.message, /heard: "buy me \$50 of beef"/i);
  assert.match(first.message, /World doesn't trade beef/);
  assert.match(first.message, /Kept for the record/);
  assert.equal(first.message.includes("!"), false);
  const rows = listInstructions(account).filter((row) => row.status === "cant");
  assert.equal(rows.length, 1);
  assert.equal(rows[0].display_status, "can't");
  assert.equal(summary(account).holding, 0);
});

test("buy $50 with no instrument noun is unclear", () => {
  const account = "cant-1b";
  const out = heard(account, "buy $50");
  assert.equal(out.kind, "unclear");
  assert.equal(out.skip_llm, true);
});

test("repeat ask same day appends trail and does not add a row", () => {
  const account = "cant-2";
  heard(account, "Buy me $50 of beef");
  const again = heard(account, "Buy me $50 of beef");
  assert.equal(again.kind, "cant");
  assert.match(again.message, /Still can't/);
  const rows = listInstructions(account).filter((row) => row.status === "cant");
  assert.equal(rows.length, 1);
  assert.equal(rows[0].repeat_count, 2);
  const detail = getInstruction(account, rows[0].instruction_id);
  const trail = (detail.trail || []).filter((row) => row.event_type === "heard");
  assert.equal(trail.length, 2);
});

test("book me a flight is out of scope", () => {
  const account = "cant-3";
  const out = heard(account, "Book me a flight");
  assert.equal(out.kind, "cant");
  assert.equal(out.cant_kind, "out_of_scope");
  assert.match(out.message, /outside what I do/);
  const rows = listInstructions(account).filter((row) => row.status === "cant");
  assert.equal(rows.length, 1);
});

test("mixed note walls beef and leaves the close", () => {
  const account = "cant-4";
  const wall = heard(account, "buy fifty of beef and close half the perp");
  assert.equal(wall.kind, "cant");
  assert.match(wall.remaining_text, /close half the perp/i);
  assert.equal(wall.skip_llm, false);
  const rows = listInstructions(account).filter((row) => row.status === "cant");
  assert.equal(rows.length, 1);
  assert.equal(rows[0].asked_entity, "beef");
});

test("in-book asset does not create a cant row", () => {
  const account = "cant-5";
  const out = heard(account, "Buy $50 of WETH");
  assert.equal(out.kind, "unmatched");
  assert.equal(out.skip_llm, false);
  assert.equal(listInstructions(account).filter((row) => row.status === "cant").length, 0);
});

test("eth ether ethereum resolve on a World book that only lists WETH", () => {
  const world = [
    { symbol: "WETH", name: "Wrapped Ether" },
    { symbol: "WBTC", name: "Wrapped Bitcoin" },
  ];
  const account = "cant-5-world";
  for (const text of [
    "Buy $50 of ETH",
    "Buy $50 of eth",
    "Buy $50 of ether",
    "Buy $50 of ethereum",
    "Buy $50 of BTC",
    "Buy $50 of bitcoin",
  ]) {
    const out = handleHeard(account, { text, universe: world });
    assert.equal(out.kind, "unmatched", text);
    assert.equal(out.skip_llm, false, text);
  }
  assert.equal(listInstructions(account).filter((row) => row.status === "cant").length, 0);
});

test("phonetic near-match still offers book names", () => {
  const account = "cant-6";
  const first = heard(account, "Buy $50 of etherium");
  assert.equal(first.kind, "near_match");
  const eth = first.controls.find((label) => /ETH/.test(label));
  assert.ok(eth);
  const confirmed = heard(account, eth);
  assert.equal(confirmed.kind, "resolved");
  assert.equal(confirmed.skip_llm, false);
});

test("unclear transcripts skip llm and write no row", () => {
  const account = "cant-8";
  for (const text of ["buy fifty", "buy fifty dollars worth of hmm", "asdfgh"]) {
    const out = heard(account, text);
    assert.equal(out.kind, "unclear", text);
    assert.equal(out.skip_llm, true, text);
    assert.match(out.message, /didn't catch/i);
  }
  assert.equal(listInstructions(account).filter((row) => row.status === "cant").length, 0);
  const these = heard(account, "buy fifty dollars worth of these");
  assert.equal(these.kind, "near_match");
});
