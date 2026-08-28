/**
 * Invite codes and silent attribution. JSON only. Never messages a sharer
 * because a guest arrived, opened a link, or upgraded.
 */

import { randomBytes } from "node:crypto";
import { SHARE, renderM10 } from "./copy.js";
import { filePath, readJson, writeJson } from "./store.js";

export const REGEN_PER_DAY = 3;
const DAY_SECS = 24 * 60 * 60;
const CODE_LEN = 10;

function nowSecs() {
  return Math.floor(Date.now() / 1000);
}

function sharePath() {
  return filePath("share.json");
}

function empty() {
  return { codes: [], attribution: [], prefs: {} };
}

function load() {
  const data = readJson(sharePath(), empty());
  data.codes = data.codes || [];
  data.attribution = data.attribution || [];
  data.prefs = data.prefs || {};
  return data;
}

function save(data) {
  writeJson(sharePath(), data);
}

function botName() {
  return process.env.WORLD_TELEGRAM_BOT || "WorldMarketsBot";
}

export function refLink(code, bot = botName()) {
  return `https://t.me/${bot}?start=ref_${code}`;
}

export function startappLink(code, bot = botName()) {
  return `https://t.me/${bot}?startapp=ref_${code}`;
}

function mintCode(existing) {
  const taken = new Set(existing.map((row) => row.code));
  for (let i = 0; i < 8; i += 1) {
    const code = randomBytes(8)
      .toString("hex")
      .replace(/[^a-z0-9]/gi, "")
      .slice(0, CODE_LEN);
    if (code.length === CODE_LEN && !taken.has(code)) return code;
  }
  return randomBytes(8).toString("hex").slice(0, CODE_LEN);
}

function prefsOf(data, userId) {
  if (!data.prefs[userId]) {
    data.prefs[userId] = {
      include_name: true,
      hint_shown: false,
      name_asked: false,
      regen_at: [],
    };
  }
  const row = data.prefs[userId];
  row.regen_at = row.regen_at || [];
  return row;
}

export function activeCode(userId, data = load()) {
  return (
    data.codes.find(
      (row) => String(row.user_id) === String(userId) && !row.revoked_at,
    ) || null
  );
}

export function ensureCode(userId, now = nowSecs()) {
  const data = load();
  const existing = activeCode(userId, data);
  if (existing) return { ok: true, created: false, code: existing, data };
  const code = {
    user_id: String(userId),
    code: mintCode(data.codes),
    created_at: now,
  };
  data.codes.push(code);
  save(data);
  return { ok: true, created: true, code, data: load() };
}

function pruneRegen(prefs, now) {
  const cutoff = now - DAY_SECS;
  prefs.regen_at = (prefs.regen_at || []).filter((ts) => ts >= cutoff);
}

export function regenerate(userId, now = nowSecs()) {
  const data = load();
  const prefs = prefsOf(data, String(userId));
  pruneRegen(prefs, now);
  if (prefs.regen_at.length >= REGEN_PER_DAY) {
    const current = activeCode(userId, data);
    return {
      ok: false,
      error: "rate_limited",
      code: current,
      data,
    };
  }
  for (const row of data.codes) {
    if (String(row.user_id) === String(userId) && !row.revoked_at) {
      row.revoked_at = now;
    }
  }
  const code = {
    user_id: String(userId),
    code: mintCode(data.codes),
    created_at: now,
  };
  data.codes.push(code);
  prefs.regen_at.push(now);
  save(data);
  return { ok: true, code, data: load() };
}

/**
 * Write one attribution row. Never builds a sharer-facing message.
 * Unknown / revoked codes: no row, no error for the guest.
 */
export function attribute(body, now = nowSecs()) {
  const code = String(body.code || "").trim();
  const guestId = String(body.guest_id || "").trim();
  if (!code || !guestId) {
    return { ok: true, attributed: false, reason: "missing" };
  }
  const data = load();
  const row = data.codes.find((item) => item.code === code);
  if (!row) {
    return { ok: true, attributed: false, reason: "unknown" };
  }
  if (row.revoked_at) {
    return { ok: true, attributed: false, reason: "revoked" };
  }
  const already = data.attribution.find(
    (item) => item.code === code && String(item.guest_id) === guestId,
  );
  if (already) {
    return { ok: true, attributed: true, duplicate: true };
  }
  data.attribution.push({ code, guest_id: guestId, ts: now });
  save(data);
  return { ok: true, attributed: true, duplicate: false };
}

function buttons(link) {
  return [
    { label: SHARE.paper, action: "paper", url: link },
    { label: SHARE.cant, action: "cant_do", url: link },
  ];
}

function packIntroduce({
  userId,
  firstName,
  includeName,
  hint,
  nameAsk,
  ack,
  code,
  bot,
}) {
  const link = refLink(code.code, bot);
  const message = renderM10({
    includeName,
    firstName,
    refLink: link,
  });
  const messages = [];
  if (nameAsk) {
    messages.push({
      kind: "name_ask",
      message: SHARE.name_ask,
      controls: [{ label: SHARE.without_name, action: "without_name" }],
    });
  }
  if (ack) {
    messages.push({ kind: "ack", message: ack });
  }
  if (hint) {
    messages.push({ kind: "hint", message: SHARE.hint });
  }
  messages.push({
    kind: "m10",
    message,
    controls: buttons(link),
    link,
  });
  return {
    ok: true,
    executable: false,
    surface: "introduction",
    message,
    hint: hint ? SHARE.hint : null,
    name_ask: nameAsk ? SHARE.name_ask : null,
    ack: ack || null,
    link,
    startapp: startappLink(code.code, bot),
    controls: buttons(link),
    name_controls: nameAsk
      ? [{ label: SHARE.without_name, action: "without_name" }]
      : [],
    messages,
  };
}

export function handleShare(body, now = nowSecs()) {
  const action = String(body.action || "introduce");
  if (action === "attribute") {
    return attribute(body, now);
  }
  const userId = String(body.user_id || body.account_id || "").trim();
  if (!userId) {
    return { ok: false, error: "user_id is required" };
  }
  const bot = body.telegram_bot || botName();
  const firstName = String(body.first_name || "").trim();

  if (action === "who") {
    return {
      ok: true,
      executable: false,
      surface: "who",
      message: SHARE.who_asked,
      hint: null,
      name_ask: null,
      messages: [{ kind: "who", message: SHARE.who_asked }],
    };
  }

  if (action === "already_user") {
    return {
      ok: true,
      executable: false,
      surface: "already_user",
      message: SHARE.already_user,
      hint: null,
      name_ask: null,
      messages: [{ kind: "already_user", message: SHARE.already_user }],
    };
  }

  if (action === "revoke") {
    const result = regenerate(userId, now);
    if (!result.ok) {
      const current = result.code || ensureCode(userId, now).code;
      const packed = packIntroduce({
        userId,
        firstName,
        includeName: prefsOf(result.data, userId).include_name,
        hint: false,
        nameAsk: false,
        ack: SHARE.rate_limited,
        code: current,
        bot,
      });
      packed.rate_limited = true;
      packed.ack = SHARE.rate_limited;
      packed.message = SHARE.rate_limited;
      packed.messages = [
        { kind: "ack", message: SHARE.rate_limited },
        packed.messages[packed.messages.length - 1],
      ];
      return packed;
    }
    const data = result.data;
    const prefs = prefsOf(data, userId);
    const packed = packIntroduce({
      userId,
      firstName,
      includeName: prefs.include_name && Boolean(firstName),
      hint: false,
      nameAsk: false,
      ack: SHARE.revoke_ack,
      code: result.code,
      bot,
    });
    packed.revoked = true;
    return packed;
  }

  const ensured = ensureCode(userId, now);
  const data = ensured.data;
  const prefs = prefsOf(data, userId);

  if (action === "without_name") {
    prefs.include_name = false;
    prefs.name_asked = true;
    save(data);
  } else if (action === "with_name") {
    prefs.include_name = true;
    prefs.name_asked = true;
    save(data);
  }

  const firstTime = action === "introduce" && !prefs.name_asked;
  if (firstTime) {
    prefs.name_asked = true;
  }
  const showHint = !prefs.hint_shown;
  if (showHint) {
    prefs.hint_shown = true;
  }
  save(data);

  const includeName = prefs.include_name && Boolean(firstName);
  return packIntroduce({
    userId,
    firstName,
    includeName,
    hint: showHint,
    nameAsk: firstTime,
    ack: null,
    code: activeCode(userId, load()),
    bot,
  });
}
