import { filePath, readJson, writeJson } from "./store.js";

function kindsPath(accountId) {
  return filePath("action_kinds", `${accountId}.json`);
}

export function status(accountId, kind) {
  const data = readJson(kindsPath(accountId), { confirmed: [] });
  const confirmed = (data.confirmed || []).includes(kind);
  return { ok: true, kind, confirmed };
}

export function confirm(accountId, kind) {
  const data = readJson(kindsPath(accountId), { confirmed: [] });
  data.confirmed = data.confirmed || [];
  const graduating = !data.confirmed.includes(kind);
  if (graduating) data.confirmed.push(kind);
  writeJson(kindsPath(accountId), data);
  return { ok: true, kind, confirmed: true, graduating };
}
