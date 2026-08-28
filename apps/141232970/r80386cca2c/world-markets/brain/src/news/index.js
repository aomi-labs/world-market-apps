/**
 * News source registry.
 *
 * Add a source: implement `src/news/<id>.js` (see `source.js` for the
 * contract), import it here, and put the id in `SOURCES`. Enable via
 * `WORLD_NEWS_SOURCES=cryptocompare,yourid` (comma-separated).
 */

import cryptocompare from "./cryptocompare.js";

/** @type {Record<string, import("./source.js").NewsSource>} */
export const SOURCES = {
  cryptocompare,
};

const DEFAULT_IDS = ["cryptocompare"];
const CACHE_TTL_MS = 5 * 60 * 1000;

const cache = new Map();

export function enabledSourceIds() {
  const raw = process.env.WORLD_NEWS_SOURCES || "";
  const ids = raw
    .split(",")
    .map((id) => id.trim().toLowerCase())
    .filter(Boolean);
  return ids.length > 0 ? ids : DEFAULT_IDS;
}

export async function gatherNews(query) {
  const ids = enabledSourceIds();
  const key = `${ids.join("+")}:${query.symbol}:${query.windowSecs}`;
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < CACHE_TTL_MS) {
    return hit.value;
  }
  const results = await Promise.all(
    ids.map(async (id) => {
      const source = SOURCES[id];
      if (!source) {
        return { id, status: "unavailable", items: [] };
      }
      try {
        const result = await source.fetch(query);
        return { id, ...result, items: result.items || [] };
      } catch {
        return { id, status: "unavailable", items: [] };
      }
    }),
  );
  const items = [];
  const seen = new Set();
  let anyOk = false;
  let anyTimeout = false;
  for (const result of results) {
    if (result.status === "ok") anyOk = true;
    if (result.status === "timeout") anyTimeout = true;
    for (const item of result.items) {
      const dedupe = item.url || `${item.name}:${item.ts}`;
      if (seen.has(dedupe)) continue;
      seen.add(dedupe);
      items.push({ ...item, source_id: result.id });
    }
  }
  let news_status = "ok";
  if (!anyOk && anyTimeout) news_status = "timeout";
  else if (!anyOk) news_status = "unavailable";
  const value = {
    news_status,
    sources: items.map((item) => ({
      name: item.name,
      url: item.url,
      ts: item.ts,
    })),
    cause_established: items.some((item) => Boolean(item.cause)),
    attributions: items
      .filter((item) => item.cause)
      .map((item) => ({
        name: item.name,
        url: item.url,
        ts: item.ts,
        cause: item.cause,
      })),
    source_ids: ids,
  };
  cache.set(key, { at: Date.now(), value });
  return value;
}
