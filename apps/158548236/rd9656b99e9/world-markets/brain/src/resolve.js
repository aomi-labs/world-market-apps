/**
 * Map a user phrase onto the closed watch-predicate vocabulary.
 * Never invent a threshold: vague phrases return needs_clarification.
 */

const PCT =
  /(?:up|down|moves?|[+\-−])\s*(\d+(?:\.\d+)?)\s*%(?:\s*(?:in\s+a\s+day|\/\s*1d|over\s+(?:a\s+)?day|\/day))?/i;
const PCT_ALT = /(\d+(?:\.\d+)?)\s*%\s*(?:up|down|move|day)/i;
const PRICE = /(?:above|below|crosses?|hits?|reaches?|≥|>=|≤|<=)\s*\$?\s*(\d+(?:\.\d+)?)/i;
const PRICE_MARK = /mark\s*(?:≥|>=|≤|<=|>|<)\s*\$?\s*(\d+(?:\.\d+)?)/i;
const FUNDING = /funding\s*(?:rate\s*)?(?:≥|>=|>|above)\s*(\d+(?:\.\d+)?)\s*%?/i;
const RISK = /(?:portfolio\s+)?risk\s*(?:≥|>=|>|above)\s*(\d+(?:\.\d+)?)/i;
const RAPV = /(?:rapv|floor)\s*(?:≥|>=|>|≤|<=|<)\s*(\d+(?:\.\d+)?)/i;
const IDLE = /idle\s*(?:>|≥|>=)?\s*(\d+)\s*d(?:ays?)?/i;
const LOAN = /loan\s+(renewed|renewal|rolls?|extended)/i;
const VAGUE =
  /\b(rally|dump|moon|crash|pumps?|dumps?|moves?|when it (?:does|happens|moves)|something happens|vol(?:atility)?)\b/i;
const EXEC =
  /\b(buy|sell|long|short|order|market order|limit order|if it does,? buy|and buy|and sell)\b/i;

function opFrom(phrase, fallback = "gte") {
  const lower = phrase.toLowerCase();
  if (lower.includes("below") || lower.includes("≤") || lower.includes("<=")) {
    return "lte";
  }
  if (lower.includes("down") || lower.includes("−") || /-\s*\d/.test(lower)) {
    return "lte";
  }
  return fallback;
}

export function looksLikeExecution(phrase) {
  return EXEC.test(phrase || "");
}

export function resolvePredicate({ phrase, symbol, token_id }) {
  const text = String(phrase || "").trim();
  const sym = String(symbol || "").toUpperCase();
  if (!text || !sym) {
    return { ok: false, error: "symbol_and_phrase_required" };
  }
  if (looksLikeExecution(text)) {
    return { ok: false, execution_folded: true };
  }

  let match = text.match(PCT) || text.match(PCT_ALT);
  if (match) {
    const pct = match[1];
    return {
      ok: true,
      predicate: {
        kind: "pct_move",
        symbol: sym,
        token_id: token_id ?? null,
        op: opFrom(text, "gte"),
        pct,
        window: "1d",
        resolved: `${sym} ${opFrom(text, "gte") === "lte" ? "≤" : "≥"} ${opFrom(text, "gte") === "lte" ? "−" : "+"}${pct}% / 1d`,
      },
    };
  }

  match = text.match(FUNDING);
  if (match) {
    return {
      ok: true,
      predicate: {
        kind: "funding",
        symbol: sym,
        token_id: token_id ?? null,
        op: "gt",
        rate_pct: match[1],
        resolved: `${sym} funding > ${match[1]}%`,
      },
    };
  }

  match = text.match(RISK) || text.match(RAPV);
  if (match) {
    const kind = /rapv|floor/i.test(text) ? "rapv" : "risk";
    return {
      ok: true,
      predicate: {
        kind,
        symbol: sym,
        account_scoped: true,
        op: "gte",
        level: match[1],
        resolved:
          kind === "rapv"
            ? `portfolio RAPV ≥ ${match[1]}`
            : `portfolio risk ≥ ${match[1]}`,
      },
    };
  }

  match = text.match(PRICE_MARK) || text.match(PRICE);
  if (match) {
    const level = match[1];
    const op = opFrom(text, "gte");
    return {
      ok: true,
      predicate: {
        kind: "price_level",
        symbol: sym,
        token_id: token_id ?? null,
        op,
        level,
        resolved: `${sym} mark ${op === "lte" ? "≤" : "≥"} ${level}`,
      },
    };
  }

  match = text.match(IDLE);
  if (match) {
    return {
      ok: true,
      predicate: {
        kind: "idle",
        symbol: sym,
        op: "gt",
        days: match[1],
        resolved: `${sym} idle > ${match[1]}d`,
      },
    };
  }

  if (LOAN.test(text)) {
    return {
      ok: true,
      predicate: {
        kind: "loan_renewal",
        symbol: sym,
        resolved: `${sym} loan renewed`,
      },
    };
  }

  if (VAGUE.test(text) || text.split(/\s+/).length <= 3) {
    return {
      ok: false,
      needs_clarification: true,
      options: [
        { id: "pct_5_1d", label: "Up 5% in a day", phrase: "up 5% in a day" },
        { id: "pick_price", label: "Pick a price", phrase: null },
      ],
    };
  }

  return {
    ok: false,
    needs_clarification: true,
    options: [
      { id: "pct_5_1d", label: "Up 5% in a day", phrase: "up 5% in a day" },
      { id: "pick_price", label: "Pick a price", phrase: null },
    ],
  };
}
