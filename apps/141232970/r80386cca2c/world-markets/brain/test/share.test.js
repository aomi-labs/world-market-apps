import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { createRequire } from "node:module";

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "aomi-share-"));
process.env.WORLD_BRAIN_DIR = dir;
process.env.WORLD_TELEGRAM_BOT = "WorldMarketsBot";

const {
  SHARE,
  fillTemplate,
  proseBlocks,
  renderM10,
  templateSlots,
} = await import("../src/copy.js");
const {
  REGEN_PER_DAY,
  attribute,
  ensureCode,
  handleShare,
  regenerate,
} = await import("../src/share.js");

const require = createRequire(import.meta.url);
const shareSrc = fs.readFileSync(
  path.join(path.dirname(require.resolve("../src/share.js")), "share.js"),
  "utf8",
);
const copySrc = fs.readFileSync(
  path.join(path.dirname(require.resolve("../src/copy.js")), "copy.js"),
  "utf8",
);
const outboundSrc = fs.readFileSync(
  path.join(path.dirname(require.resolve("../src/outbound.js")), "outbound.js"),
  "utf8",
);

test("M10 templates interpolate only first_name and ref_link", () => {
  assert.deepEqual(templateSlots(SHARE.m10_with_name), ["first_name", "ref_link"]);
  assert.deepEqual(templateSlots(SHARE.m10_anon), ["ref_link"]);
  for (const key of ["pnl", "balance", "position", "equity", "nav"]) {
    assert.equal(SHARE.m10_with_name.includes(`{${key}}`), false);
    assert.equal(SHARE.m10_anon.includes(`{${key}}`), false);
  }
  const named = renderM10({
    includeName: true,
    firstName: "Lucas",
    refLink: "https://t.me/WorldMarketsBot?start=ref_abc",
  });
  assert.match(named, /Lucas thought you should meet me/);
  assert.match(named, /https:\/\/t\.me\/WorldMarketsBot\?start=ref_abc/);
  assert.doesNotMatch(named, /\$/);
  const anon = renderM10({
    includeName: false,
    firstName: "Lucas",
    refLink: "https://t.me/WorldMarketsBot?start=ref_abc",
  });
  assert.match(anon, /A friend thought you should meet me/);
  assert.doesNotMatch(anon, /Lucas/);
  assert.throws(() => fillTemplate(SHARE.m10_with_name, { ref_link: "x" }));
});

test("copy register: no bang, prose blocks fit 160", () => {
  const values = Object.values(SHARE);
  for (const value of values) {
    assert.equal(value.includes("!"), false, value);
    for (const block of proseBlocks(value)) {
      const filled = block
        .replace("{first_name}", "A")
        .replace("{ref_link}", "https://t.me/WorldMarketsBot?start=ref_abcdefghij");
      assert.ok(filled.length <= 160, `${filled.length} ${filled}`);
    }
  }
});

test("ensureCode is one active code per user", () => {
  const first = ensureCode("17", 1_700_000_000);
  const again = ensureCode("17", 1_700_000_010);
  assert.equal(first.code.code, again.code.code);
  assert.equal(first.created, true);
  assert.equal(again.created, false);
  const other = ensureCode("18", 1_700_000_020);
  assert.notEqual(other.code.code, first.code.code);
});

test("regenerate revokes the prior code and is rate-limited", () => {
  const now = 1_800_000_000;
  ensureCode("19", now);
  const a = regenerate("19", now + 1);
  assert.equal(a.ok, true);
  const prior = a.data.codes.find((row) => row.user_id === "19" && row.revoked_at);
  assert.ok(prior);
  assert.notEqual(a.code.code, prior.code);
  assert.equal(attribute({ code: prior.code, guest_id: "g_x" }, now + 2).attributed, false);
  assert.equal(attribute({ code: prior.code, guest_id: "g_x" }, now + 2).reason, "revoked");
  for (let i = 1; i < REGEN_PER_DAY; i += 1) {
    assert.equal(regenerate("19", now + 10 + i).ok, true);
  }
  const limited = regenerate("19", now + 20);
  assert.equal(limited.ok, false);
  assert.equal(limited.error, "rate_limited");
  const nextDay = regenerate("19", now + 20 + 86_400 + 1);
  assert.equal(nextDay.ok, true);
});

test("valid ref writes exactly one attribution row and unknown is silent", () => {
  const { code } = ensureCode("21", 1_900_000_000);
  const first = attribute(
    { code: code.code, guest_id: "tg-9" },
    1_900_000_100,
  );
  const again = attribute(
    { code: code.code, guest_id: "tg-9" },
    1_900_000_200,
  );
  assert.equal(first.attributed, true);
  assert.equal(first.duplicate, false);
  assert.equal(again.attributed, true);
  assert.equal(again.duplicate, true);
  assert.equal(first.message, undefined);
  assert.equal(attribute({ code: "nope", guest_id: "tg-9" }).attributed, false);
  const data = JSON.parse(
    fs.readFileSync(path.join(dir, "share.json"), "utf8"),
  );
  const rows = data.attribution.filter(
    (row) => row.code === code.code && row.guest_id === "tg-9",
  );
  assert.equal(rows.length, 1);
});

test("introduce asks name once, shows hint once, then only M10", () => {
  const first = handleShare({
    action: "introduce",
    user_id: "22",
    first_name: "Ada",
  });
  assert.equal(first.name_ask, SHARE.name_ask);
  assert.equal(first.hint, SHARE.hint);
  assert.match(first.message, /Ada thought you should meet me/);
  assert.match(first.message, /\?start=ref_/);
  assert.equal(first.messages.length, 3);
  const second = handleShare({
    action: "introduce",
    user_id: "22",
    first_name: "Ada",
  });
  assert.equal(second.name_ask, null);
  assert.equal(second.hint, null);
  assert.equal(second.messages.length, 1);
  assert.equal(second.messages[0].kind, "m10");
  const anon = handleShare({
    action: "without_name",
    user_id: "22",
    first_name: "Ada",
  });
  assert.match(anon.message, /A friend thought you should meet me/);
  assert.doesNotMatch(anon.message, /Ada/);
});

test("who and revoke copy; attribute never notifies", () => {
  const who = handleShare({ action: "who", user_id: "23" });
  assert.equal(who.message, SHARE.who_asked);
  ensureCode("23", 2_000_000_000);
  const revoked = handleShare({ action: "revoke", user_id: "23" });
  assert.equal(revoked.ack, SHARE.revoke_ack);
  assert.match(revoked.message, /ref_/);
  const attr = handleShare({
    action: "attribute",
    code: "x",
    guest_id: "g_1",
  });
  assert.equal(attr.attributed, false);
  assert.equal(attr.message, undefined);
});

test("grep-proof: share never messages a sharer from guest activity", () => {
  assert.doesNotMatch(shareSrc, /from ["'].*outbound/);
  assert.doesNotMatch(shareSrc, /enqueue\(/);
  assert.doesNotMatch(shareSrc, /your friend joined/i);
  assert.doesNotMatch(shareSrc, /opened your/i);
  assert.doesNotMatch(copySrc, /your friend joined/i);
  assert.doesNotMatch(shareSrc, /notify_sharer["']:\s*true/);
  assert.match(outboundSrc, /export function enqueue/);
});
