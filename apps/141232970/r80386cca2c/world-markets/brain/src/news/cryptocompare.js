/**
 * Public CryptoCompare news feed (no API key required for the free list).
 * Docs: https://min-api.cryptocompare.com/documentation?key=News&cat=allNews
 *
 * This is the default source until a licensed / in-house wire is wired in.
 * See docs/FUTURE-WORK.md ("News sources").
 */

import { causeFromHeadline, newsTicker } from "./source.js";

const DEFAULT_URL = "https://min-api.cryptocompare.com/data/v2/news/";
const TIMEOUT_MS = 8_000;

export default {
  id: "cryptocompare",

  async fetch({ symbol, windowSecs, now }) {
    const ticker = newsTicker(symbol);
    const url = new URL(process.env.WORLD_NEWS_CRYPTOCOMPARE_URL || DEFAULT_URL);
    url.searchParams.set("lang", "EN");
    url.searchParams.set("categories", ticker);
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
    try {
      const response = await fetch(url, {
        headers: { Accept: "application/json" },
        signal: controller.signal,
      });
      if (!response.ok) {
        return { status: "unavailable", items: [] };
      }
      const body = await response.json();
      const rows = Array.isArray(body?.Data) ? body.Data : [];
      const cutoff = now - windowSecs;
      const items = [];
      for (const row of rows) {
        const published = Number(row.published_on) || 0;
        if (published && published < cutoff) continue;
        const title = String(row.title || "").trim();
        const name =
          row.source_info?.name || row.source || "CryptoCompare";
        const link = String(row.url || row.guid || "").trim();
        if (!link) continue;
        items.push({
          name: String(name),
          url: link,
          ts: published ? new Date(published * 1000).toISOString() : "",
          title,
          cause: causeFromHeadline(title, ticker),
        });
      }
      return { status: "ok", items };
    } catch (error) {
      const aborted = error?.name === "AbortError";
      return { status: aborted ? "timeout" : "unavailable", items: [] };
    } finally {
      clearTimeout(timer);
    }
  },
};
