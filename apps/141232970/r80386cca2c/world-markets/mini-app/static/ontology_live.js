/* global SPEECH_ONTOLOGY */
/** Client-side ontology alias rewrite for live hold-to-talk words. */

const PROTECTED_KINDS = new Set(["act", "opener", "size", "unit", "size_frame", "product", "order_type"]);
const PROTECTED_TOKENS = new Set(["of", "a", "an", "the", "and", "then", "to", "for", "me", "my"]);
/** Phonetic STT misses shown as the instrument while holding; not pronouns or real words like beef. */
const LIVE_HINTS = new Set(["east", "eath", "eeth", "eeths", "ease", "soul", "sawl"]);
const DEFAULT_OPENERS = new Set([
  "buy",
  "sell",
  "unwind",
  "leverage",
  "long",
  "short",
  "lend",
  "borrow",
  "close",
  "cancel",
  "watch",
  "pause",
  "resume",
  "open",
  "what",
  "what's",
  "whats",
  "how",
  "why",
  "who",
  "when",
  "where",
  "can",
  "could",
  "should",
  "show",
  "list",
  "tell",
]);
const SIZE_FRAMES = new Set(["dollars", "bucks", "worth", "notional"]);
const TICKERS = new Set([
  "eth",
  "weth",
  "ether",
  "ethereum",
  "btc",
  "wbtc",
  "bitcoin",
  "sol",
  "solana",
  "usdc",
  "usdt",
]);

const FALLBACK_ENTRIES = [
  { surface_form: "ETH", normalized_target: "WETH", kind: "instrument" },
  { surface_form: "ether", normalized_target: "WETH", kind: "instrument" },
  { surface_form: "ethereum", normalized_target: "WETH", kind: "instrument" },
  { surface_form: "wrapped ether", normalized_target: "WETH", kind: "instrument" },
  { surface_form: "wrapped eth", normalized_target: "WETH", kind: "instrument" },
  { surface_form: "wrapped ethereum", normalized_target: "WETH", kind: "instrument" },
  { surface_form: "bitcoin", normalized_target: "WBTC", kind: "instrument" },
  { surface_form: "BTC", normalized_target: "WBTC", kind: "instrument" },
  { surface_form: "wrapped bitcoin", normalized_target: "WBTC", kind: "instrument" },
  { surface_form: "solana", normalized_target: "SOL", kind: "instrument" },
  { surface_form: "USD coin", normalized_target: "USDC", kind: "instrument" },
  { surface_form: "tether", normalized_target: "USDT", kind: "instrument" },
];

let aliasBySurface = new Map();
let phraseAliases = [];
let protectedSurfaces = new Set(PROTECTED_TOKENS);
let openerSurfaces = new Set(DEFAULT_OPENERS);
let instrumentSurfaces = new Set(TICKERS);

function normalizeKey(surface) {
  return String(surface || "")
    .trim()
    .toLowerCase()
    .replace(/\s+/g, " ");
}

function loadEntries(entries) {
  const rows = Array.isArray(entries) && entries.length ? entries : FALLBACK_ENTRIES;
  aliasBySurface = new Map();
  phraseAliases = [];
  protectedSurfaces = new Set(PROTECTED_TOKENS);
  openerSurfaces = new Set(DEFAULT_OPENERS);
  instrumentSurfaces = new Set(TICKERS);
  for (const row of rows) {
    const key = normalizeKey(row.surface_form);
    const target = String(row.normalized_target || "").trim();
    const kind = row.kind || "";
    if (!key) continue;
    if (kind === "act" || kind === "opener") {
      openerSurfaces.add(key);
      openerSurfaces.add(key.split(" ")[0]);
    }
    if (kind === "instrument") {
      instrumentSurfaces.add(key);
      if (target) instrumentSurfaces.add(normalizeKey(target));
    }
    if (PROTECTED_KINDS.has(kind)) {
      protectedSurfaces.add(key);
      continue;
    }
    if (kind === "confusable") {
      if (LIVE_HINTS.has(key) && target) {
        aliasBySurface.set(key, target);
      }
      continue;
    }
    if (kind !== "instrument") continue;
    if (!target || key === normalizeKey(target)) continue;
    aliasBySurface.set(key, target);
    if (key.includes(" ")) phraseAliases.push([key.split(" "), target]);
  }
  phraseAliases.sort((a, b) => b[0].length - a[0].length);
}

loadEntries(typeof SPEECH_ONTOLOGY !== "undefined" ? SPEECH_ONTOLOGY.entries : null);

function setOntologyEntries(entries) {
  loadEntries(entries);
}

function tokenize(raw) {
  return String(raw || "")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
}

function stripQty(token) {
  return String(token || "")
    .replace(/^\$/, "")
    .replace(/[?!.,;:]+$/g, "");
}

function isQtyToken(token) {
  return /^\d+(?:\.\d+)?$/.test(stripQty(token));
}

function unfuseQty(token) {
  const t = stripQty(token);
  const m = t.match(/^(\d+)\.(\d+)$/);
  if (!m) return t;
  const whole = m[1];
  const frac = m[2];
  if ([...frac].every((ch) => ch === "0")) return whole;
  if (frac.startsWith("0") && frac.replace(/^0+/, "") === whole) return whole;
  return t;
}

function isOpenerToken(token) {
  const key = normalizeKey(stripQty(token));
  return openerSurfaces.has(key);
}

function looksLikeSizedInstrument(tokens) {
  return tokens.some((token) => {
    const key = normalizeKey(stripQty(token));
    return instrumentSurfaces.has(key) || SIZE_FRAMES.has(key) || aliasBySurface.has(key);
  });
}

function isTradeAct(token) {
  return /^(buy|sell|long|short|lend|borrow|close|unwind)$/i.test(stripQty(token));
}

function isQuestionOpener(token) {
  return /^(what|what's|whats|why|how|who|when|where|can|could|should)$/i.test(stripQty(token));
}

function isFiveToken(token) {
  const t = stripQty(token).toLowerCase();
  return t === "5" || t === "five";
}

function isEightToken(token) {
  const t = stripQty(token).toLowerCase();
  return t === "8" || t === "eight" || t === "ate";
}

function hasNamedInstrument(tokens) {
  return tokens.some((token) => {
    const key = normalizeKey(stripQty(token));
    return instrumentSurfaces.has(key) || aliasBySurface.has(key);
  });
}

function hasMoneyFrame(tokens) {
  return tokens.some((token) => SIZE_FRAMES.has(normalizeKey(stripQty(token))));
}

function ethCollapsedAsEight(tokens) {
  if (!tokens.length || isQuestionOpener(tokens[0]) || hasMoneyFrame(tokens) || hasNamedInstrument(tokens)) {
    return false;
  }
  const rest = isTradeAct(tokens[0]) ? tokens.slice(1) : tokens;
  if (rest.length === 1 && stripQty(rest[0]) === "58") return true;
  if (rest.length === 2 && isFiveToken(rest[0]) && isEightToken(rest[1])) return true;
  if (rest.length === 3 && isFiveToken(rest[0]) && isFiveToken(rest[1]) && isEightToken(rest[2])) {
    return true;
  }
  return false;
}

function repairEthHeardAsEight(raw) {
  const original = String(raw || "").trim();
  if (!original) return original;
  const tokens = tokenize(original);
  if (!ethCollapsedAsEight(tokens)) return original;
  const verb = isTradeAct(tokens[0]) ? stripQty(tokens[0]).toLowerCase() : "buy";
  return `${verb} 5 ETH`;
}

function restoreLeadingCommand(raw) {
  const original = repairEthHeardAsEight(raw);
  if (!original) return original;
  const tokens = tokenize(original);
  if (!tokens.length) return original;
  if (tokens.some(isOpenerToken)) return original;
  if (!isQtyToken(tokens[0]) || !looksLikeSizedInstrument(tokens)) return original;
  tokens[0] = unfuseQty(tokens[0]);
  tokens.unshift("buy");
  return tokens.join(" ");
}

function annotateLiveTranscript(raw) {
  const original = restoreLeadingCommand(raw);
  const tokens = tokenize(original);
  if (!tokens.length) return [];
  const out = [];
  let i = 0;
  while (i < tokens.length) {
    let hit = null;
    let consumed = 1;
    for (const [parts, target] of phraseAliases) {
      if (i + parts.length > tokens.length) continue;
      let ok = true;
      for (let k = 0; k < parts.length; k++) {
        if (normalizeKey(tokens[i + k]) !== parts[k]) {
          ok = false;
          break;
        }
      }
      if (ok) {
        hit = target;
        consumed = parts.length;
        break;
      }
    }
    if (!hit) {
      const key = normalizeKey(tokens[i]);
      if (!protectedSurfaces.has(key) && aliasBySurface.has(key)) {
        hit = aliasBySurface.get(key);
      }
    }
    const surface = tokens.slice(i, i + consumed).join(" ");
    out.push({
      surface,
      display: hit || surface,
      rewritten: Boolean(hit),
    });
    i += consumed;
  }
  return out;
}

function correctLiveTranscript(raw) {
  const spans = annotateLiveTranscript(raw);
  if (!spans.length) return String(raw || "").trim();
  return spans.map((span) => span.display).join(" ");
}

if (typeof window !== "undefined") {
  window.correctLiveTranscript = correctLiveTranscript;
  window.annotateLiveTranscript = annotateLiveTranscript;
  window.setOntologyEntries = setOntologyEntries;
}
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    correctLiveTranscript,
    annotateLiveTranscript,
    setOntologyEntries,
    loadEntries,
    restoreLeadingCommand,
  };
}
