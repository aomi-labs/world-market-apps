/* global SPEECH_ONTOLOGY */
/** Client-side ontology alias rewrite for live hold-to-talk words. */

const PROTECTED_KINDS = new Set(["act", "opener", "size", "unit", "size_frame", "product", "order_type"]);
const PROTECTED_TOKENS = new Set(["of", "a", "an", "the", "and", "then", "to", "for", "me", "my"]);
/** Phonetic STT misses shown as the instrument while holding; not pronouns or real words like beef. */
const LIVE_HINTS = new Set(["east", "eath", "eeth", "eeths", "ease", "soul", "sawl", "salt"]);
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
const SIZE_FRAMES = new Set(["dollar", "dollars", "bucks", "worth", "notional"]);
const USD_QTY_WORDS = new Set([
  "ten",
  "fifteen",
  "twenty",
  "thirty",
  "forty",
  "fifty",
  "sixty",
  "seventy",
  "eighty",
  "ninety",
  "hundred",
  "thousand",
]);
const SMALL_QTY_WORDS = new Set(["one", "two", "three", "four", "five", "six", "seven", "eight", "nine"]);
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
  let start = 0;
  if (/^(a|an)$/i.test(stripQty(tokens[0])) && tokens.length > 1) start = 1;
  if (!isQtyToken(tokens[start]) || !looksLikeSizedInstrument(tokens)) return original;
  tokens[start] = unfuseQty(tokens[start]);
  if (start === 1) tokens.splice(0, 1);
  tokens.unshift("buy");
  return tokens.join(" ");
}

function qtyTokenLooksLikeUsd(token) {
  const t = stripQty(token).toLowerCase();
  if (!t || t.includes(".")) return false;
  if (USD_QTY_WORDS.has(t)) return true;
  if (t.endsWith("k") && /^\d+$/.test(t.slice(0, -1))) return true;
  if (!/^\d+$/.test(t)) return false;
  return Number(t) >= 10;
}

function isInstrumentishToken(token) {
  const key = normalizeKey(stripQty(token));
  return instrumentSurfaces.has(key) || aliasBySurface.has(key);
}

function isSizeCommandAct(token) {
  return /^(buy|sell|long|short|lend|borrow|close|unwind|open|put|spend|invest|deploy|purchase|dump|twap|dca)$/i.test(
    stripQty(token),
  );
}

function isQtyLinker(token) {
  return /^(of|in|into|on|for|with|a|an|the|my|me)$/i.test(stripQty(token));
}

function instrumentLenAt(tokens, i) {
  if (i >= tokens.length || isSizeCommandAct(tokens[i]) || isQtyLinker(tokens[i])) return 0;
  for (const [parts] of phraseAliases) {
    if (i + parts.length > tokens.length) continue;
    let ok = true;
    for (let k = 0; k < parts.length; k++) {
      if (normalizeKey(stripQty(tokens[i + k])) !== parts[k]) {
        ok = false;
        break;
      }
    }
    if (ok) return parts.length;
  }
  if (isInstrumentishToken(tokens[i])) return 1;
  return 0;
}

function skipQtyLinkers(tokens, i) {
  while (i < tokens.length && isQtyLinker(tokens[i])) i += 1;
  return i;
}

function insertDollarsWorthFrame(tokens, qtyEnd) {
  if (qtyEnd >= tokens.length) return;
  if (normalizeKey(stripQty(tokens[qtyEnd])) === "of") {
    tokens.splice(qtyEnd, 0, "dollars", "worth");
    return;
  }
  if (isQtyLinker(tokens[qtyEnd])) {
    tokens.splice(qtyEnd, 1, "dollars", "worth", "of");
    return;
  }
  tokens.splice(qtyEnd, 0, "dollars", "worth", "of");
}

function rewriteInstrumentFirst(tokens, instIdx, instLen, qtyIdx, qtyEnd) {
  const prefix = tokens.slice(0, instIdx);
  while (prefix.length && isQtyLinker(prefix[prefix.length - 1]) && !/^(a|an)$/i.test(prefix[prefix.length - 1])) {
    prefix.pop();
  }
  const inst = tokens.slice(instIdx, instIdx + instLen);
  const qty = tokens.slice(qtyIdx, qtyEnd);
  const mid = tokens.slice(instIdx + instLen, qtyIdx);
  while (mid.length && isQtyLinker(mid[mid.length - 1])) mid.pop();
  const tail = tokens.slice(qtyEnd);
  return prefix.concat(qty, ["dollars", "worth", "of"], inst, mid, tail);
}

function rewriteYardsWorth(tokens) {
  for (let i = 0; i < tokens.length; i++) {
    const key = normalizeKey(stripQty(tokens[i]));
    if ((key === "yards" || key === "yard") && tokens[i + 1] && normalizeKey(stripQty(tokens[i + 1])) === "worth") {
      tokens[i] = "dollars";
    }
  }
}

function rewriteBuyMishear(tokens) {
  const prefix = iHavePrefixLen(tokens);
  if (prefix > 0) {
    if (!looksLikeTradeActMishearRest(tokens.slice(prefix))) return;
    tokens.splice(0, prefix, "buy");
    return;
  }
  const first = normalizeKey(stripQty(tokens[0]));
  if (!/^(about|by|bye|wait)$/.test(first)) return;
  if (!looksLikeTradeActMishearRest(tokens)) return;
  tokens[0] = "buy";
}

function rewriteSellMishear(tokens) {
  const prefix = wellPrefixLen(tokens);
  if (prefix > 0) {
    if (!looksLikeTradeActMishearRest(tokens.slice(prefix))) return;
    tokens.splice(0, prefix, "sell");
    return;
  }
  const first = normalizeKey(stripQty(tokens[0])).replace(/['’]/g, "");
  if (!/^(well|cell|sale|shell|so)$/.test(first)) return;
  if (!looksLikeTradeActMishearRest(tokens)) return;
  tokens[0] = "sell";
}

function wellPrefixLen(tokens) {
  if (!tokens.length) return 0;
  const first = normalizeKey(stripQty(tokens[0])).replace(/['’]/g, "");
  if (first === "we" && tokens[1] && /^(ll|l)$/.test(normalizeKey(stripQty(tokens[1])))) return 2;
  return 0;
}

function iHavePrefixLen(tokens) {
  if (!tokens.length) return 0;
  const first = normalizeKey(stripQty(tokens[0])).replace(/['’]/g, "");
  if (first === "ive") return 1;
  if (first === "i" && tokens[1] && /^(have)$/.test(normalizeKey(stripQty(tokens[1])))) return 2;
  return 0;
}

function looksLikeTradeActMishearRest(tokens) {
  const hasQty = tokens.some((token) => {
    const key = normalizeKey(stripQty(token));
    return isQtyToken(token) || USD_QTY_WORDS.has(key) || SMALL_QTY_WORDS.has(key);
  });
  if (!hasQty) return false;
  return (
    hasMoneyFrame(tokens) ||
    hasNamedInstrument(tokens) ||
    tokens.some((token) => {
      const key = normalizeKey(stripQty(token));
      return key === "salt" || key === "yards" || key === "yard" || isInstrumentishToken(token);
    })
  );
}

function rewriteSaltInMoneyFrame(tokens) {
  if (!hasMoneyFrame(tokens) && !tokens.some(isSizeCommandAct)) return;
  for (let i = 0; i < tokens.length; i++) {
    if (normalizeKey(stripQty(tokens[i])) === "salt") tokens[i] = "SOL";
  }
}

function ensureDollarsBeforeWorth(tokens) {
  for (let i = 0; i < tokens.length; i++) {
    if (normalizeKey(stripQty(tokens[i])) !== "worth") continue;
    if (i === 0) continue;
    const prev = normalizeKey(stripQty(tokens[i - 1]));
    if (/^(dollar|dollars|bucks|buck)$/.test(prev)) continue;
    if (isQtyToken(tokens[i - 1]) || USD_QTY_WORDS.has(prev)) {
      tokens.splice(i, 0, "dollars");
      i += 1;
    }
  }
}

function repairFiveFifty(tokens) {
  if (!tokens.some((token) => normalizeKey(stripQty(token)) === "worth")) return;
  for (let i = 0; i < tokens.length; i++) {
    if (stripQty(tokens[i]) !== "550") continue;
    tokens[i] = "fifty";
    const after = tokens[i + 1] ? normalizeKey(stripQty(tokens[i + 1])) : "";
    if (after === "worth") tokens.splice(i + 1, 0, "dollars");
    break;
  }
}

function repairSpeechDollarFrame(raw) {
  const original = String(raw || "").trim();
  if (!original) return original;
  const tokens = tokenize(original);
  if (!tokens.length) return original;
  rewriteYardsWorth(tokens);
  rewriteBuyMishear(tokens);
  rewriteSellMishear(tokens);
  rewriteSaltInMoneyFrame(tokens);
  ensureDollarsBeforeWorth(tokens);
  return tokens.join(" ");
}

function applyFiveFifty(raw) {
  const original = String(raw || "").trim();
  if (!original) return original;
  const tokens = tokenize(original);
  repairFiveFifty(tokens);
  return tokens.join(" ");
}

function restoreDollarsWorthOf(raw) {
  const original = String(raw || "").trim();
  if (!original) return original;
  const tokens = tokenize(original);
  if (!tokens.length || hasMoneyFrame(tokens)) return original;
  if (isQuestionOpener(tokens[0]) && !tokens.some(isSizeCommandAct)) return original;
  const actIdx = tokens.findIndex(isSizeCommandAct);
  if (actIdx < 0) return original;
  let qtyIdx = -1;
  for (let i = actIdx + 1; i < tokens.length; i++) {
    const key = normalizeKey(stripQty(tokens[i]));
    if (isQtyToken(tokens[i]) || USD_QTY_WORDS.has(key)) {
      qtyIdx = i;
      break;
    }
  }
  if (qtyIdx < 0) return original;
  let qtyEnd = qtyIdx + 1;
  while (qtyEnd < tokens.length) {
    const key = normalizeKey(stripQty(tokens[qtyEnd]));
    if (isQtyToken(tokens[qtyEnd]) || USD_QTY_WORDS.has(key) || key === "a" || key === "the") {
      qtyEnd += 1;
      continue;
    }
    break;
  }
  const span = tokens.slice(qtyIdx, qtyEnd);
  if (!span.some(qtyTokenLooksLikeUsd)) return original;
  const after = skipQtyLinkers(tokens, qtyEnd);
  if (instrumentLenAt(tokens, after)) {
    insertDollarsWorthFrame(tokens, qtyEnd);
    return tokens.join(" ");
  }
  for (let i = actIdx + 1; i < qtyIdx; i++) {
    const ilen = instrumentLenAt(tokens, i);
    if (ilen) {
      return rewriteInstrumentFirst(tokens, i, ilen, qtyIdx, qtyEnd).join(" ");
    }
  }
  return original;
}

function annotateLiveTranscript(raw) {
  const original = applyFiveFifty(restoreDollarsWorthOf(restoreLeadingCommand(repairSpeechDollarFrame(raw))));
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
    restoreDollarsWorthOf,
  };
}
