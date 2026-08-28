/**
 * News source contract.
 *
 * To add a source:
 *   1. Create `src/news/<id>.js` that default-exports an object matching this shape.
 *   2. Register it in `src/news/index.js` (`SOURCES`).
 *   3. Enable it with `WORLD_NEWS_SOURCES` (comma-separated ids) or leave the default.
 *
 * A source MUST NOT invent a price, a cause, or a timestamp. If it cannot
 * attribute a cause, leave `cause` null — `cause_established` stays false.
 *
 * @typedef {object} NewsItem
 * @property {string} name  Outlet name (e.g. "CoinDesk")
 * @property {string} url   Canonical article URL
 * @property {string} ts    ISO-8601 or unix-seconds string from the outlet
 * @property {string} [title]
 * @property {string|null} cause  Short attributed cause, or null
 *
 * @typedef {object} NewsQuery
 * @property {string} symbol     World / crypto symbol (WETH, ETH, WBTC, …)
 * @property {number} windowSecs Lookback window
 * @property {number} now        Unix seconds
 *
 * @typedef {object} NewsResult
 * @property {"ok"|"timeout"|"unavailable"} status
 * @property {NewsItem[]} items
 *
 * @typedef {object} NewsSource
 * @property {string} id
 * @property {(query: NewsQuery) => Promise<NewsResult>} fetch
 */

export const CAUSAL_CUE =
  /\b(after|amid|as|because|due to|following|on the back of|driven by|sparked by|triggered by|weighs on|pressured by)\b/i;

/** Map World symbols to the ticker a news vendor is likely to use. */
export function newsTicker(symbol) {
  const raw = String(symbol || "")
    .trim()
    .toUpperCase();
  const stripped = raw.replace(/\.B$/i, "");
  const aliases = {
    WETH: "ETH",
    WBTC: "BTC",
    BTCB: "BTC",
    "BTC.B": "BTC",
    WSTETH: "ETH",
    CBETH: "ETH",
  };
  return aliases[stripped] || aliases[raw] || stripped;
}

/**
 * A headline attributes a cause only when it names the asset *and* uses a
 * causal cue. The cause string is the headline itself (vendor-authored).
 */
export function causeFromHeadline(title, ticker) {
  if (!title || !ticker) return null;
  const named = new RegExp(`\\b${escapeRegExp(ticker)}\\b`, "i").test(title);
  if (!named) return null;
  if (!CAUSAL_CUE.test(title)) return null;
  return title.trim();
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
