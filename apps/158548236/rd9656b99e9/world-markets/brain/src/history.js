import { filePath, readJson, writeJson } from "./store.js";

const MAX_POINTS = 2_000;

function historyPath() {
  return filePath("history.json");
}

function empty() {
  return { marks: {}, funding: {}, accounts: {} };
}

export function loadHistory() {
  const data = readJson(historyPath(), empty());
  if (!data.marks) data.marks = {};
  if (!data.funding) data.funding = {};
  if (!data.accounts) data.accounts = {};
  return data;
}

function save(data) {
  writeJson(historyPath(), data);
}

function pushCapped(list, point) {
  list.push(point);
  if (list.length > MAX_POINTS) list.splice(0, list.length - MAX_POINTS);
}

export function recordMark({ symbol, token_id, mark, ts }) {
  if (!symbol || mark == null) return;
  const data = loadHistory();
  const key = String(symbol).toUpperCase();
  if (!data.marks[key]) data.marks[key] = [];
  pushCapped(data.marks[key], {
    ts: Number(ts) || Math.floor(Date.now() / 1000),
    mark: String(mark),
    token_id: token_id ?? null,
  });
  save(data);
}

export function recordFunding({ symbol, rate, ts }) {
  if (!symbol || rate == null) return;
  const data = loadHistory();
  const key = String(symbol).toUpperCase();
  if (!data.funding[key]) data.funding[key] = [];
  pushCapped(data.funding[key], {
    ts: Number(ts) || Math.floor(Date.now() / 1000),
    rate: String(rate),
  });
  save(data);
}

export function recordAccount(sample) {
  const accountId = String(sample.account_id ?? "");
  if (!accountId) return;
  const data = loadHistory();
  if (!data.accounts[accountId]) data.accounts[accountId] = [];
  pushCapped(data.accounts[accountId], {
    ts: Number(sample.ts) || Math.floor(Date.now() / 1000),
    rapv: sample.rapv ?? null,
    liquidation_risk: sample.liquidation_risk ?? null,
    idle_quote: sample.idle_quote ?? null,
    loan_fingerprints: sample.loan_fingerprints ?? [],
  });
  save(data);
}

export function latestMark(symbol) {
  const key = String(symbol || "").toUpperCase();
  const series = loadHistory().marks[key] || [];
  return series.length ? series[series.length - 1] : null;
}

export function markAtOrBefore(symbol, unix) {
  const key = String(symbol || "").toUpperCase();
  const series = loadHistory().marks[key] || [];
  let found = null;
  for (const point of series) {
    if (point.ts <= unix) found = point;
  }
  return found;
}

export function markSeries(symbol) {
  const key = String(symbol || "").toUpperCase();
  return loadHistory().marks[key] || [];
}

export function latestFunding(symbol) {
  const key = String(symbol || "").toUpperCase();
  const series = loadHistory().funding[key] || [];
  return series.length ? series[series.length - 1] : null;
}

export function latestAccount(accountId) {
  const series = loadHistory().accounts[String(accountId)] || [];
  return series.length ? series[series.length - 1] : null;
}

export function movePct(symbol, windowSecs, now = Math.floor(Date.now() / 1000)) {
  const latest = latestMark(symbol);
  const then = markAtOrBefore(symbol, now - windowSecs);
  if (!latest || !then) return null;
  const nowVal = Number(latest.mark);
  const thenVal = Number(then.mark);
  if (!Number.isFinite(nowVal) || !Number.isFinite(thenVal) || thenVal === 0) {
    return null;
  }
  const pct = ((nowVal - thenVal) / thenVal) * 100;
  return {
    pct: pct.toFixed(2),
    mark_now: latest.mark,
    mark_then: then.mark,
    then_ts: then.ts,
    now_ts: latest.ts,
  };
}

export function watchedTokenIds(watches) {
  const ids = new Set();
  for (const watch of watches) {
    const id = watch.predicate?.token_id;
    if (id != null) ids.add(Number(id));
  }
  return [...ids];
}
