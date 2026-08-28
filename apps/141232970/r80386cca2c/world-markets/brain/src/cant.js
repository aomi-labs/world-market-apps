/**
 * Unfulfillable / near-match / unclear heard path.
 * Unsigned. Never stages, begins, or completes a trade.
 */

import { filePath, readJson, writeJson } from "./store.js";
import {
  CANT,
  nearMatchEscape,
  nearMatchMessage,
  wallMessage,
} from "./copy.js";
import {
  actSet,
  confusableTarget,
  fillerSet,
  instrumentAlias,
  kindOf,
} from "./ontology.js";
import { appendEvent, upsertCant } from "./instructions.js";
import { lexiconOf, stampUtterance, upsertLexicon, voiceContext } from "./voice.js";
import { recordCandidateOutcome } from "./ontology_stats.js";

const MAX_CANDIDATES = 3;
const PHONETIC_FLOOR = 0.65;
const MIN_ENTITY_LEN = 3;
const UNCLEAR_FILLERS = new Set([
  "hmm",
  "uh",
  "um",
  "er",
  "ah",
  "huh",
  "yeah",
  "yes",
  "ok",
  "okay",
  "please",
  "half",
  "all",
  "some",
  "quarter",
]);
const SCOPE_CUES = new Set([
  "flight",
  "flights",
  "hotel",
  "hotels",
  "pizza",
  "taxi",
  "uber",
  "reservation",
  "table",
  "appointment",
  "plane",
  "ticket",
]);
const CATEGORY_LEXICON = {
  food: [
    "beef",
    "pork",
    "chicken",
    "steak",
    "pizza",
    "burger",
    "coffee",
    "milk",
    "eggs",
    "bread",
    "rice",
  ],
  commodities: [
    "gold",
    "silver",
    "oil",
    "crude",
    "gas",
    "wheat",
    "corn",
    "copper",
    "platinum",
  ],
  equities: [
    "tsla",
    "aapl",
    "nvda",
    "msft",
    "amzn",
    "goog",
    "meta",
    "spy",
    "qqq",
    "stock",
    "stocks",
    "share",
    "shares",
    "equity",
    "equities",
  ],
  fx: ["euro", "euros", "yen", "gbp", "pound", "pounds", "franc", "cad", "aud", "fx", "forex"],
};

function instrumentCategory(noun) {
  const key = String(noun || "")
    .trim()
    .toLowerCase()
    .replace(/s$/, "");
  const raw = String(noun || "").trim().toLowerCase();
  for (const [category, words] of Object.entries(CATEGORY_LEXICON)) {
    if (words.includes(raw) || words.includes(key)) return category;
  }
  return null;
}

const LOOKUP_WORDS = new Set([
  "b",
  "p",
  "r",
  "a",
  "d",
  "balance",
  "positions",
  "risk",
  "available",
  "dollarpower",
  "commands",
  "shortcuts",
]);
const NUMBER_WORDS = {
  zero: 0,
  oh: 0,
  one: 1,
  two: 2,
  three: 3,
  four: 4,
  five: 5,
  six: 6,
  seven: 7,
  eight: 8,
  nine: 9,
  ten: 10,
  eleven: 11,
  twelve: 12,
  thirteen: 13,
  fourteen: 14,
  fifteen: 15,
  sixteen: 16,
  seventeen: 17,
  eighteen: 18,
  nineteen: 19,
  twenty: 20,
  thirty: 30,
  forty: 40,
  fifty: 50,
  sixty: 60,
  seventy: 70,
  eighty: 80,
  ninety: 90,
};

const ACTS = actSet();
const FILLERS = fillerSet();

function nowSecs(now) {
  return Number.isFinite(now) ? now : Math.floor(Date.now() / 1000);
}

function statePath(accountId) {
  return filePath("cant", `${accountId}.json`);
}

function loadState(accountId) {
  return readJson(statePath(accountId), { pending: null, declined: {} });
}

function saveState(accountId, data) {
  writeJson(statePath(accountId), data);
}

function telemetryPath(accountId) {
  return filePath("unfulfillable", `${accountId}.json`);
}

function emitTelemetry(accountId, event) {
  const data = readJson(telemetryPath(accountId), { items: [] });
  data.items = data.items || [];
  data.items.push(event);
  writeJson(telemetryPath(accountId), data);
}

function tokenize(text) {
  return String(text || "")
    .toLowerCase()
    .replace(/\$/g, " ")
    .replace(/[^a-z0-9.]+/g, " ")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
}

function isNumberToken(token) {
  return /^\d+(?:\.\d+)?$/.test(token);
}

function isFiller(token) {
  return FILLERS.has(token) || UNCLEAR_FILLERS.has(token) || isNumberToken(token);
}

function isStableEntity(token) {
  if (!token) return false;
  if (token.length < MIN_ENTITY_LEN && !confusableTarget(token) && !instrumentAlias(token)) {
    return false;
  }
  if (UNCLEAR_FILLERS.has(token)) return false;
  if (FILLERS.has(token) && !confusableTarget(token)) return false;
  if (kindOf(token) === "product") return false;
  return true;
}

function metaphone(raw) {
  let word = String(raw || "")
    .toUpperCase()
    .replace(/[^A-Z]/g, "");
  if (!word) return "";
  const first = word[0];
  word = word
    .replace(/^KN|^GN|^PN|^AE|^WR/, "N")
    .replace(/^X/, "S")
    .replace(/^WH/, "W");
  word = first + word.slice(1);
  word = word
    .replace(/MB$/g, "M")
    .replace(/SCH/g, "SK")
    .replace(/CIA|CH/g, "X")
    .replace(/DGE|DGI|GY|GI|GE/g, "J")
    .replace(/C([IEY])/g, "S$1")
    .replace(/C/g, "K")
    .replace(/Q/g, "K")
    .replace(/Z/g, "S")
    .replace(/PH/g, "F")
    .replace(/TH/g, "0")
    .replace(/V/g, "F")
    .replace(/DG/g, "J")
    .replace(/TCH/g, "CH")
    .replace(/[AEIOUY]/g, "");
  word = word.replace(/(.)\1+/g, "$1");
  return (first + word.slice(1)).slice(0, 4);
}

function jaroWinkler(a, b) {
  const s1 = String(a || "").toLowerCase();
  const s2 = String(b || "").toLowerCase();
  if (!s1 || !s2) return 0;
  if (s1 === s2) return 1;
  const matchDist = Math.floor(Math.max(s1.length, s2.length) / 2) - 1;
  const s1Match = new Array(s1.length).fill(false);
  const s2Match = new Array(s2.length).fill(false);
  let matches = 0;
  for (let i = 0; i < s1.length; i++) {
    const lo = Math.max(0, i - matchDist);
    const hi = Math.min(i + matchDist + 1, s2.length);
    for (let j = lo; j < hi; j++) {
      if (s2Match[j] || s1[i] !== s2[j]) continue;
      s1Match[i] = true;
      s2Match[j] = true;
      matches++;
      break;
    }
  }
  if (!matches) return 0;
  let k = 0;
  let trans = 0;
  for (let i = 0; i < s1.length; i++) {
    if (!s1Match[i]) continue;
    while (!s2Match[k]) k++;
    if (s1[i] !== s2[k]) trans++;
    k++;
  }
  const m = matches;
  const jaro = (m / s1.length + m / s2.length + (m - trans / 2) / m) / 3;
  let prefix = 0;
  for (let i = 0; i < Math.min(4, s1.length, s2.length); i++) {
    if (s1[i] === s2[i]) prefix++;
    else break;
  }
  return jaro + prefix * 0.1 * (1 - jaro);
}

function phoneticScore(query, candidate) {
  const q = String(query || "").toLowerCase();
  const c = String(candidate || "").toLowerCase();
  if (!q || !c) return 0;
  let score = jaroWinkler(q, c) * 0.9;
  const qMeta = metaphone(q);
  const cMeta = metaphone(c);
  if (qMeta && qMeta === cMeta) score = Math.max(score, 0.72);
  return score;
}

function universeIndex(universe) {
  const rows = [];
  const seen = new Set();
  for (const row of universe || []) {
    const symbol = String(row.symbol || "").trim();
    if (!symbol) continue;
    const key = symbol.toUpperCase();
    if (seen.has(key)) continue;
    seen.add(key);
    rows.push({
      symbol,
      name: String(row.name || "").trim(),
    });
  }
  return rows;
}

function inUniverse(universe, symbol) {
  if (!symbol) return false;
  const key = String(symbol).toLowerCase();
  return universe.some(
    (row) =>
      row.symbol.toLowerCase() === key ||
      row.name.toLowerCase() === key,
  );
}

function exactResolve(entity, universe, lexicon) {
  if (!entity) return null;
  const key = entity.toLowerCase();
  for (const row of lexicon || []) {
    if (String(row.surface_form || "").toLowerCase() === key) {
      const target = row.normalized_target;
      if (inUniverse(universe, target)) return target;
    }
  }
  const alias = instrumentAlias(entity);
  if (alias && inUniverse(universe, alias)) return alias;
  for (const row of universe) {
    if (row.symbol.toLowerCase() === key || row.name.toLowerCase() === key) {
      return row.symbol;
    }
  }
  return null;
}

function nearMatches(entity, universe, lexicon, declined) {
  if (!entity || declined[entity.toLowerCase()]) return [];
  const ranked = [];
  const seen = new Set();
  const push = (symbol, name, score) => {
    if (!symbol || !inUniverse(universe, symbol)) return;
    const key = symbol.toUpperCase();
    if (seen.has(key)) return;
    seen.add(key);
    ranked.push({ symbol, name: name || symbol, score });
  };

  const conf = confusableTarget(entity);
  if (conf) {
    const row = universe.find((u) => u.symbol.toUpperCase() === conf.toUpperCase());
    push(conf, row ? row.name : conf, 0.95);
  }

  for (const row of universe) {
    const score = Math.max(
      phoneticScore(entity, row.symbol),
      phoneticScore(entity, row.name),
    );
    if (score >= PHONETIC_FLOOR) push(row.symbol, row.name, score);
  }

  for (const row of lexicon || []) {
    const surface = String(row.surface_form || "");
    const target = String(row.normalized_target || "");
    const score = phoneticScore(entity, surface);
    if (score >= PHONETIC_FLOOR) {
      const uni = universe.find((u) => u.symbol.toLowerCase() === target.toLowerCase());
      push(target, uni ? uni.name : target, score);
    }
  }

  ranked.sort((a, b) => b.score - a.score);
  return ranked.slice(0, MAX_CANDIDATES).map((row) => ({
    symbol: row.symbol,
    name: row.name,
    label: row.name && row.name.toLowerCase() !== row.symbol.toLowerCase()
      ? `${row.symbol} — ${row.name}`
      : row.symbol,
  }));
}

function splitClauses(text) {
  const raw = String(text || "").trim();
  if (!raw) return [];
  return raw
    .split(/\s+(?:and|then)\s+|;\s+|\.\s+/i)
    .map((part) => part.trim())
    .filter(Boolean);
}

function extractEntity(clause, slots) {
  const tokens = tokenize(clause);
  if (!tokens.length) return null;
  const control = tokens.some((t) => t === "cancel" || t === "pause" || t === "resume");
  const watches = tokens.some((t) =>
    t === "watches" || t === "tasks" || t === "watch" || t === "task",
  );
  if (control && watches) return null;
  if (Array.isArray(slots) && slots.length) {
    const lower = String(clause || "").toLowerCase();
    const inst = slots.find((row) => {
      if (row?.kind !== "instrument") return false;
      const surface = String(row.surface || "").toLowerCase();
      const target = String(row.target || "").toLowerCase();
      return (surface && lower.includes(surface)) || (target && lower.includes(target));
    });
    if (inst) return String(inst.target || inst.surface).toLowerCase();
    return null;
  }
  const actAt = tokens.findIndex((t) => ACTS.has(t));
  if (actAt >= 0) {
    let j = actAt + 1;
    while (j < tokens.length && isFiller(tokens[j])) j++;
    if (j < tokens.length && tokens[j] !== "worth" && tokens[j] !== "of") {
      return tokens[j];
    }
  }
  return null;
}

function hasAct(clause) {
  return tokenize(clause).some((t) => ACTS.has(t));
}

function hasSizeFrame(clause) {
  return tokenize(clause).some((t) => kindOf(t) === "size_frame");
}

function isLookupText(text) {
  const t = String(text || "")
    .trim()
    .replace(/^\//, "")
    .toLowerCase();
  if (LOOKUP_WORDS.has(t) || t === "?") return true;
  return false;
}

function isQuestion(text) {
  const t = String(text || "").trim().toLowerCase();
  return /^(what|why|how|who|when|where|walk me|tell me)\b/.test(t) || t.endsWith("?");
}

function isOutOfScope(clause) {
  const tokens = tokenize(clause);
  if (tokens.some((t) => SCOPE_CUES.has(t))) return true;
  if (tokens[0] === "book" && !ACTS.has("book")) return true;
  return false;
}

function isValidRemainder(clause) {
  const tokens = tokenize(clause);
  if (tokens.includes("close") || tokens.includes("cancel") || tokens.includes("pause") || tokens.includes("resume")) {
    return true;
  }
  const entity = extractEntity(clause);
  if (entity && kindOf(entity) === "product") return true;
  return false;
}

function heardForm(text) {
  return String(text || "")
    .trim()
    .replace(/^["']|["']$/g, "")
    .toLowerCase();
}

function normalizeSentence(clause) {
  let text = String(clause || "").trim();
  if (!text) return "";
  text = text.replace(/\bfifty\s+dollars\b/gi, "$50");
  text = text.replace(/\b(\d+)\s+dollars\b/gi, "$$$1");
  const words = Object.keys(NUMBER_WORDS);
  for (const word of words) {
    const n = NUMBER_WORDS[word];
    if (n >= 20) {
      text = text.replace(new RegExp(`\\b${word}\\s+dollars\\b`, "gi"), `$${n}`);
    }
  }
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function sublineFor(repeatCount) {
  if (!repeatCount || repeatCount <= 1) {
    return "World doesn't trade this · kept for the record";
  }
  if (repeatCount === 2) return "asked twice · kept for the record";
  if (repeatCount === 3) return "asked three times · kept for the record";
  return `asked ${repeatCount} times · kept for the record`;
}

function trainingUseOf(accountId) {
  return voiceContext(accountId).consents?.training_use === "granted";
}

function stampHeard(accountId, body, result) {
  const ref = body.utterance_ref || body.utterance_id;
  if (!ref || body.peek) return;
  const kind = result.kind || result.cant_kind || result.voice_kind;
  stampUtterance(accountId, ref, { cant_kind: kind || null });
}

function recordProposalOutcome(accountId, body, outcome) {
  const ref = body.utterance_ref || body.utterance_id;
  if (!ref || body.peek) return;
  const utt = stampUtterance(accountId, ref, {});
  const proposals = utt?.proposals || body.proposals || body.proposed_confusables || [];
  const training = trainingUseOf(accountId);
  for (const row of proposals) {
    recordCandidateOutcome(accountId, {
      surface: row.surface || body.asked_entity,
      target: row.target,
      slotKind: row.kind || "confusable",
      channel: utt?.channel || body.channel,
      outcome,
      trainingUse: training,
    });
  }
  if (!proposals.length && body.asked_entity) {
    recordCandidateOutcome(accountId, {
      surface: body.asked_entity,
      target: body.resolved || body.asked_entity,
      slotKind: "confusable",
      channel: utt?.channel || body.channel,
      outcome,
      trainingUse: training,
    });
  }
}

function payloadBase(kind) {
  return {
    ok: true,
    source: "world-markets-cant",
    executable: false,
    matched: kind !== "unmatched",
    skip_llm: kind !== "unmatched" && kind !== "resolved",
    reply_verbatim: kind !== "unmatched" && kind !== "resolved",
    kind,
    controls: [],
    remaining_text: "",
    instruction: null,
  };
}

function classifyClause(clause, universe, lexicon, declined, slots) {
  const tokens = tokenize(clause);
  if (!tokens.length) return { kind: "empty" };
  if (isLookupText(clause) || isQuestion(clause)) return { kind: "pass" };
  if (isOutOfScope(clause) && !hasAct(clause)) {
    const entity = tokens.find((t) => SCOPE_CUES.has(t)) || tokens[tokens.length - 1];
    return { kind: "out_of_scope", entity, clause };
  }
  const tradeShaped = hasAct(clause) || hasSizeFrame(clause);
  if (isValidRemainder(clause)) {
    return { kind: "pass", clause };
  }

  if (!tradeShaped) {
    if (isOutOfScope(clause)) {
      const entity = tokens.find((t) => SCOPE_CUES.has(t)) || "this";
      return { kind: "out_of_scope", entity, clause };
    }
    return { kind: "unclear" };
  }

  const entity = extractEntity(clause, slots);
  if (!entity || !isStableEntity(entity)) {
    return { kind: "unclear" };
  }
  const resolved = exactResolve(entity, universe, lexicon);
  if (resolved) return { kind: "pass", clause, resolved };
  const category = instrumentCategory(entity);
  if (category) {
    return { kind: "cant_category", entity, clause, category, candidates: [] };
  }
  const candidates = nearMatches(entity, universe, lexicon, declined);
  return {
    kind: "unresolved",
    entity,
    clause,
    candidates,
  };
}

function wallFor(clause, entity, kind, repeat, index, total) {
  return wallMessage({
    heard: heardForm(clause),
    entity,
    kind,
    repeat,
    index,
    total,
  });
}

function applyPending(accountId, text, state, universe, body, now) {
  const pending = state.pending;
  if (!pending) return null;
  const trimmed = String(text || "").trim();
  const escape = nearMatchEscape(pending.entity);
  const lower = trimmed.toLowerCase();
  if (
    lower === escape.toLowerCase() ||
    lower === `no — i meant ${pending.entity}` ||
    lower === `no - i meant ${pending.entity}` ||
    lower === `no i meant ${pending.entity}`
  ) {
    state.declined = state.declined || {};
    state.declined[pending.entity] = now;
    state.pending = null;
    saveState(accountId, state);
    recordProposalOutcome(accountId, {
      ...body,
      utterance_ref: pending.utterance_ref || body.utterance_ref,
      asked_entity: pending.entity,
    }, "rejected");
    return wallOutcome(accountId, {
      clause: pending.original_text,
      entity: pending.entity,
      kind: "no_market",
      origin: body.origin,
      utteranceRef: pending.utterance_ref || body.utterance_ref,
      remaining: pending.remaining_text || "",
      nearOffered: (pending.candidates || []).length,
      now,
      peek: body.peek,
    });
  }
  const hit = (pending.candidates || []).find((row) => {
    const label = String(row.label || "").toLowerCase();
    const symbol = String(row.symbol || "").toLowerCase();
    return lower === label || lower === symbol || lower.startsWith(`${symbol} `);
  });
  if (hit) {
    state.pending = null;
    saveState(accountId, state);
    if (!body.peek) {
      upsertLexicon(accountId, {
        surface_form: pending.entity,
        normalized_target: hit.symbol,
        kind: "instrument",
        source: "confirmed",
      }, now);
    }
    const rewritten = String(pending.original_text || "").replace(
      new RegExp(pending.entity, "ig"),
      hit.symbol,
    );
    const remaining = pending.remaining_text
      ? `${rewritten} and ${pending.remaining_text}`
      : rewritten;
    const resolved = {
      ...payloadBase("resolved"),
      skip_llm: false,
      reply_verbatim: false,
      rewritten_text: remaining,
      remaining_text: remaining,
      message: "",
      asked_entity: pending.entity,
    };
    recordProposalOutcome(
      accountId,
      {
        ...body,
        utterance_ref: pending.utterance_ref || body.utterance_ref,
        asked_entity: pending.entity,
        resolved: hit.symbol,
      },
      "accepted",
    );
    stampHeard(accountId, { ...body, utterance_ref: pending.utterance_ref || body.utterance_ref }, resolved);
    return resolved;
  }
  state.pending = null;
  saveState(accountId, state);
  return null;
}

function wallOutcome(accountId, opts) {
  const {
    clause,
    entity,
    kind,
    origin,
    utteranceRef,
    remaining,
    nearOffered,
    now,
    peek,
    index,
    total,
  } = opts;
  const sentence = normalizeSentence(clause);
  let recorded = { ok: true, repeat: false, instruction: null };
  if (!peek) {
    recorded = upsertCant(
      accountId,
      {
        asked_entity: entity,
        cant_kind: kind,
        heard: heardForm(clause),
        sentence,
        wall: "",
        origin,
        utterance_ref: utteranceRef,
        sub_line: sublineFor(1),
      },
      now,
    );
  }
  const repeatNow = Boolean(recorded.repeat);
  const wall = wallFor(clause, entity, kind, repeatNow, index || 1, total || 1);
  if (!peek && recorded.instruction) {
    recorded.instruction.sub_line = sublineFor(recorded.instruction.repeat_count);
    appendEvent(accountId, recorded.instruction.instruction_id, "sent_to_thread", wall, now, {
      ref: utteranceRef,
    });
    emitTelemetry(accountId, {
      user: String(accountId),
      asked_entity_normalized: entity,
      kind,
      near_matches_offered: nearOffered || 0,
      repeat_count: recorded.instruction.repeat_count || 1,
      utterance_ref: utteranceRef || null,
      at: now,
    });
  }
  return {
    ...payloadBase("cant"),
    skip_llm: !remaining,
    reply_verbatim: true,
    message: wall,
    remaining_text: remaining || "",
    instruction: recorded.instruction,
    asked_entity: entity,
    cant_kind: kind,
    voice_kind: "cant",
  };
}

export function handleHeard(accountId, body = {}, now = nowSecs()) {
  const text = String(body.text || body.message || "").trim();
  const universe = universeIndex(body.universe);
  const lexicon = lexiconOf(accountId);
  const origin = body.origin || (body.source === "mini_app" || body.utterance_ref ? "voice" : null);
  const peek = Boolean(body.peek);
  const slots = Array.isArray(body.slots) ? body.slots : null;
  if (!text) {
    const unclear = { ...payloadBase("unclear"), skip_llm: true, message: CANT.unclear, voice_kind: "unclear" };
    stampHeard(accountId, body, unclear);
    return unclear;
  }
  if (isLookupText(text)) {
    return { ...payloadBase("unmatched"), skip_llm: false };
  }

  const state = loadState(accountId);
  const pendingHit = applyPending(accountId, text, state, universe, { ...body, origin, peek }, now);
  if (pendingHit) {
    stampHeard(accountId, body, pendingHit);
    return pendingHit;
  }

  const clauses = splitClauses(text);
  const classified = clauses.map((clause) =>
    classifyClause(clause, universe, lexicon, state.declined || {}, slots),
  );

  const unresolved = classified
    .map((row, i) => ({ ...row, index: i }))
    .filter(
      (row) =>
        row.kind === "unresolved" ||
        row.kind === "out_of_scope" ||
        row.kind === "cant_category",
    );
  const unclear = classified.filter((row) => row.kind === "unclear");
  const remainder = classified
    .filter((row) => row.kind === "pass")
    .map((row) => row.clause)
    .filter(Boolean);

  if (!unresolved.length && unclear.length && !remainder.length) {
    const out = {
      ...payloadBase("unclear"),
      skip_llm: true,
      message: CANT.unclear,
      voice_kind: "unclear",
    };
    stampHeard(accountId, body, out);
    return out;
  }

  if (!unresolved.length) {
    const out = { ...payloadBase("unmatched"), skip_llm: false, remaining_text: "" };
    stampHeard(accountId, body, out);
    return out;
  }

  const first = unresolved[0];
  if (first.kind === "unresolved") {
    const declined = Boolean(state.declined?.[first.entity]);
    const candidates = declined ? [] : first.candidates || [];
    if (candidates.length && !declined) {
      if (!peek) {
        state.pending = {
          entity: first.entity,
          original_text: first.clause,
          candidates,
          remaining_text: remainder.join(" and "),
          utterance_ref: body.utterance_ref || null,
        };
        saveState(accountId, state);
      }
      const controls = [
        ...candidates.map((row) => row.label),
        nearMatchEscape(first.entity),
      ];
      const out = {
        ...payloadBase("near_match"),
        skip_llm: true,
        message: nearMatchMessage(first.entity),
        controls,
        remaining_text: remainder.join(" and "),
        asked_entity: first.entity,
        candidates,
        voice_kind: "near_match",
      };
      stampHeard(accountId, body, out);
      return out;
    }
  }

  const handled = [];
  const total = unresolved.length + (remainder.length ? 1 : 0);
  for (let i = 0; i < unresolved.length; i++) {
    const row = unresolved[i];
    const entity = row.entity || "this";
    const kind = row.kind === "out_of_scope" ? "out_of_scope" : "no_market";
    handled.push(
      wallOutcome(accountId, {
        clause: row.clause,
        entity,
        kind,
        origin,
        utteranceRef: body.utterance_ref,
        remaining: "",
        nearOffered: (row.candidates || []).length,
        now,
        peek,
        index: i + 1,
        total: Math.max(total, unresolved.length),
      }),
    );
  }
  const last = handled[handled.length - 1];
  last.remaining_text = remainder.join(" and ");
  last.skip_llm = !last.remaining_text;
  if (handled.length > 1) {
    last.message = handled.map((row) => row.message).join("\n\n");
  }
  last.voice_kind = "cant";
  stampHeard(accountId, body, last);
  return last;
}

export function __testables() {
  return {
    tokenize,
    extractEntity,
    phoneticScore,
    nearMatches,
    splitClauses,
    classifyClause,
    normalizeSentence,
    metaphone,
    jaroWinkler,
  };
}
