import { filePath, readJson, writeJson } from "./store.js";

function prefsPath(accountId) {
  return filePath("preferences", `${accountId}.json`);
}

export function listPreferences(accountId) {
  const data = readJson(prefsPath(accountId), { items: [] });
  return (data.items || []).map((item) => ({
    ...item,
    on_chain: false,
  }));
}

export function classifyProtectedVeto(text) {
  const lower = String(text || "").toLowerCase();
  const protect =
    lower.includes("never sell") ||
    lower.includes("don't ever sell") ||
    lower.includes("dont ever sell") ||
    lower.includes("do not ever sell") ||
    lower.includes("protect my") ||
    lower.includes("don't sell my") ||
    lower.includes("dont sell my") ||
    (lower.includes("never") && lower.includes("sell"));
  if (!protect) return null;
  const absolute =
    lower.includes("never") ||
    lower.includes("ever") ||
    lower.includes("no matter what") ||
    lower.includes("under any circumstance");
  const asset = String(text || "")
    .split(/[\s,.:;!?]+/)
    .reverse()
    .find((t) => {
      const w = t.toLowerCase();
      return (
        w &&
        !["never", "sell", "ever", "dont", "don't", "do", "not", "my", "the", "protect", "please", "stack", "position", "holdings"].includes(w)
      );
    });
  return { asset: (asset || "that").toUpperCase(), absolute };
}

export function vetoMessage(veto) {
  const asset = veto.asset || "that";
  const base = `Stored: I'll avoid selling your ${asset}. One exception you've already signed: if your portfolio breaches your floor and ${asset} is the only way back above it, the guardian may sell some — your mandate outranks this preference. To make it absolute, change your policies on World.`;
  if (veto.absolute) {
    return `${base} [View mandate on World ↗]`;
  }
  return base;
}

export function upsertPreference(accountId, item) {
  const data = readJson(prefsPath(accountId), { items: [] });
  const now = Math.floor(Date.now() / 1000);
  const id = item.id || `p-${accountId}-${now}`;
  const veto = classifyProtectedVeto(item.text);
  const next = {
    id,
    text: String(item.text || "").trim(),
    created_at: item.created_at || now,
    on_chain: false,
    kind: veto ? "guardian_preference" : "preference",
    override_scope: veto
      ? "guardian may sell if the signed floor requires it and this is the only path back above"
      : null,
    asset: veto ? veto.asset : null,
    absolute: veto ? veto.absolute : false,
  };
  if (!next.text) {
    return { ok: false, error: "empty_preference" };
  }
  data.items = data.items.filter((row) => row.id !== id);
  data.items.push(next);
  writeJson(prefsPath(accountId), data);
  const out = { ok: true, item: next };
  if (veto) {
    out.message = vetoMessage(veto);
    out.reply_verbatim = true;
    out.categorical_veto = false;
  }
  return out;
}

export function cancelPreference(accountId, id) {
  const data = readJson(prefsPath(accountId), { items: [] });
  const before = data.items.length;
  const removed = data.items.find((row) => row.id === id);
  data.items = data.items.filter((row) => row.id !== id);
  if (data.items.length === before) {
    return { ok: false, error: "not_found" };
  }
  writeJson(prefsPath(accountId), data);
  return { ok: true, item: removed, remaining: data.items.length };
}

export function seedBrief(accountId, brief) {
  if (!brief) return;
  const text =
    typeof brief === "string"
      ? brief
      : brief.objective
        ? String(brief.objective)
        : JSON.stringify(brief);
  if (!text.trim()) return;
  const existing = listPreferences(accountId);
  if (existing.some((item) => item.text === text)) return;
  upsertPreference(accountId, { text });
}
