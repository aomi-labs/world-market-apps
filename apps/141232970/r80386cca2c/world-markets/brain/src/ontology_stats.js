/**
 * Ontology analytics. All numeric decision thresholds live here — not in HTML.
 * Candidates and snapshots are under WORLD_BRAIN_DIR, never assets/speech_ontology.json.
 */

import { readdirSync } from "node:fs";
import { filePath, readJson, writeJson } from "./store.js";
import {
  ONTOLOGY_VERSION,
  entryCounts,
  ontologyFingerprint,
} from "./ontology.js";

export const PROMOTE_CONFUSABLE_MIN_N = 5;
export const PROMOTE_CONFUSABLE_ACCEPT_RATE = 0.8;
export const NEGATIVE_FIXTURE_REJECT_N = 3;
export const ADD_ALIAS_UNKNOWN_N = 5;
export const STUCK_REPAIR_MIN_N = 5;
export const STUCK_REPAIR_RATE = 0.5;
export const FRAME_GAP_MIN_N = 5;
export const FRAME_GAP_LOOKBACK_SECS = 7 * 24 * 3600;

const CANDIDATE_CAP = 400;

function nowSecs() {
  return Math.floor(Date.now() / 1000);
}

function snapshotsPath() {
  return filePath("ontology", "snapshots.json");
}

function candidatesPath() {
  return filePath("ontology", "candidates.json");
}

function loadSnapshots() {
  return readJson(snapshotsPath(), { items: [] });
}

function loadCandidates() {
  return readJson(candidatesPath(), { global: {}, accounts: {} });
}

function candidateKey(surface, target, slotKind, channel) {
  return [
    String(surface || "").toLowerCase(),
    String(target || "").toLowerCase(),
    String(slotKind || ""),
    String(channel || ""),
  ].join("|");
}

function parseKey(key) {
  const [surface, target, slot_kind, channel] = String(key).split("|");
  return { surface, target, slot_kind, channel };
}

function bump(row, field, n = 1) {
  row[field] = (row[field] || 0) + n;
}

function ensureRow(table, key) {
  if (!table[key]) {
    table[key] = { proposed: 0, accepted: 0, rejected: 0, unknown: 0, size_rule: 0 };
  }
  return table[key];
}

function writeCandidates(data) {
  const trim = (table) => {
    const keys = Object.keys(table);
    if (keys.length <= CANDIDATE_CAP) return table;
    const ranked = keys
      .map((key) => {
        const row = table[key];
        const n =
          (row.proposed || 0) +
          (row.accepted || 0) +
          (row.rejected || 0) +
          (row.unknown || 0) +
          (row.size_rule || 0);
        return { key, n };
      })
      .sort((a, b) => b.n - a.n)
      .slice(0, CANDIDATE_CAP);
    const out = {};
    for (const row of ranked) out[row.key] = table[row.key];
    return out;
  };
  data.global = trim(data.global || {});
  data.accounts = data.accounts || {};
  for (const id of Object.keys(data.accounts)) {
    data.accounts[id] = trim(data.accounts[id] || {});
  }
  writeJson(candidatesPath(), data);
}

export function recordOntologySnapshot(now = nowSecs()) {
  const fingerprint = ontologyFingerprint();
  const counts = entryCounts();
  const data = loadSnapshots();
  data.items = Array.isArray(data.items) ? data.items : [];
  const last = data.items[data.items.length - 1];
  if (
    last &&
    last.version === ONTOLOGY_VERSION &&
    last.fingerprint === fingerprint
  ) {
    return last;
  }
  const row = {
    ts: now,
    version: ONTOLOGY_VERSION,
    fingerprint,
    counts_by_kind: counts.counts_by_kind,
    entry_count: counts.entry_count,
    channels_speech: counts.channels_speech,
    channels_text: counts.channels_text,
  };
  data.items.push(row);
  writeJson(snapshotsPath(), data);
  return row;
}

export function ontologySummary() {
  const snapshots = loadSnapshots();
  const last = snapshots.items[snapshots.items.length - 1] || null;
  const counts = entryCounts();
  const fingerprint = ontologyFingerprint();
  return {
    ok: true,
    version: ONTOLOGY_VERSION,
    fingerprint,
    fingerprint_short: fingerprint.slice(0, 12),
    counts_by_kind: counts.counts_by_kind,
    entry_count: counts.entry_count,
    channels_speech: counts.channels_speech,
    channels_text: counts.channels_text,
    channels_speech_only: counts.channels_speech_only,
    channels_text_only: counts.channels_text_only,
    channels_both: counts.channels_both,
    last_snapshot: last,
    snapshots: snapshots.items,
  };
}

export function recordFromUtterance(accountId, utterance, trainingUse) {
  const channel = String(utterance?.channel || "speech");
  const data = loadCandidates();
  const accountTable = (data.accounts[accountId] = data.accounts[accountId] || {});
  const apply = (key, field) => {
    bump(ensureRow(accountTable, key), field);
    if (trainingUse) bump(ensureRow(data.global, key), field);
  };
  for (const row of utterance?.proposals || []) {
    const key = candidateKey(row.surface, row.target, row.kind || "confusable", channel);
    apply(key, "proposed");
  }
  for (const surface of utterance?.unknown_instruments || []) {
    const key = candidateKey(surface, "", "instrument", channel);
    apply(key, "unknown");
  }
  for (const slot of utterance?.slots || []) {
    if (slot.source === "size_rule" || slot.kind === "size") {
      const key = candidateKey(slot.surface, slot.target, "size", channel);
      apply(key, "size_rule");
    }
  }
  writeCandidates(data);
}

export function recordCandidateOutcome(
  accountId,
  { surface, target, slotKind, channel, outcome, trainingUse },
) {
  if (!surface || (outcome !== "accepted" && outcome !== "rejected")) return;
  const data = loadCandidates();
  const key = candidateKey(surface, target, slotKind || "confusable", channel || "speech");
  const accountTable = (data.accounts[accountId] = data.accounts[accountId] || {});
  bump(ensureRow(accountTable, key), outcome);
  if (trainingUse) bump(ensureRow(data.global, key), outcome);
  writeCandidates(data);
}

function loadVoiceFile(accountId) {
  return readJson(filePath("voice", `${accountId}.json`), {
    lexicon: [],
    utterances: [],
    consents: [],
    episodes: [],
    corrections: [],
  });
}

function trainingUseOf(data) {
  return (data.consents || []).some(
    (c) => c.kind === "training_use" && c.status === "granted",
  );
}

function listVoiceAccountIds() {
  try {
    return readdirSync(filePath("voice"))
      .filter((name) => name.endsWith(".json"))
      .map((name) => name.slice(0, -".json".length));
  } catch {
    return [];
  }
}

function emptyChannelStats() {
  return {
    n: 0,
    proposal_n: 0,
    size_rule_n: 0,
    grammar: { matched: 0, partial: 0, none: 0 },
    none_with_act: 0,
    repaired_n: 0,
    cant_n: 0,
    correction_n: 0,
  };
}

function absorbUtterance(stats, utterance, correctedIds) {
  stats.n += 1;
  const grammar = utterance.grammar || "none";
  if (stats.grammar[grammar] == null) stats.grammar[grammar] = 0;
  stats.grammar[grammar] += 1;
  if (Array.isArray(utterance.proposals) && utterance.proposals.length) {
    stats.proposal_n += 1;
  }
  if ((utterance.slots || []).some((s) => s.source === "size_rule")) {
    stats.size_rule_n += 1;
  }
  if (utterance.repaired_from) stats.repaired_n += 1;
  if (grammar === "none" && utterance.action_ir?.act) {
    stats.none_with_act += 1;
  }
  if (utterance.cant_kind) stats.cant_n += 1;
  if (correctedIds && correctedIds.has(utterance.id)) stats.correction_n += 1;
}

function tableToRows(table) {
  return Object.entries(table || {}).map(([key, counts]) => ({
    ...parseKey(key),
    ...counts,
  }));
}

function decide(rows, utterances, lastSnapshot, now) {
  const decisions = [];
  const seen = new Set();
  const push = (row) => {
    const key = `${row.action}|${row.surface}|${row.target}|${row.channel}`;
    if (seen.has(key)) return;
    seen.add(key);
    decisions.push(row);
  };

  for (const row of rows) {
    const proposed = row.proposed || 0;
    const accepted = row.accepted || 0;
    const rejected = row.rejected || 0;
    const unknown = row.unknown || 0;
    const acceptRate = proposed ? accepted / proposed : 0;
    if (
      row.slot_kind === "confusable" &&
      proposed >= PROMOTE_CONFUSABLE_MIN_N &&
      acceptRate >= PROMOTE_CONFUSABLE_ACCEPT_RATE
    ) {
      push({
        action: "promote_confusable",
        surface: row.surface,
        target: row.target,
        channel: row.channel,
        n: proposed,
        accept_rate: acceptRate,
        threshold: `proposed n≥${PROMOTE_CONFUSABLE_MIN_N} and accept≥${PROMOTE_CONFUSABLE_ACCEPT_RATE * 100}%`,
        suggested_entry: {
          surface_form: row.surface,
          normalized_target: row.target,
          kind: "confusable",
          confidence: 1.0,
          channels: [row.channel || "speech"],
        },
        suggested_test: `confusable_${String(row.surface || "").replace(/\s+/g, "_")}_proposes_${row.target}`,
      });
    }
    if (rejected >= NEGATIVE_FIXTURE_REJECT_N && row.slot_kind === "confusable") {
      const nonInstrument = utterances.some((u) => {
        const hit = (u.proposals || []).some(
          (p) =>
            String(p.surface || "").toLowerCase() === row.surface &&
            u.grammar === "matched" &&
            u.action_ir?.referent,
        );
        return hit;
      });
      if (nonInstrument) {
        push({
          action: "add_negative_fixture",
          surface: row.surface,
          target: row.target,
          channel: row.channel,
          n: rejected,
          threshold: `rejected ≥${NEGATIVE_FIXTURE_REJECT_N} in a non-instrument frame`,
          suggested_test: `cancel_${String(row.surface || "").replace(/\s+/g, "_")}_does_not_propose_${row.target}`,
        });
      }
    }
    if (unknown >= ADD_ALIAS_UNKNOWN_N && row.slot_kind === "instrument") {
      push({
        action: "add_alias",
        surface: row.surface,
        target: row.target || null,
        channel: row.channel,
        n: unknown,
        threshold: `unknown instrument-slot token n≥${ADD_ALIAS_UNKNOWN_N} (speech or text)`,
        suggested_entry: row.target
          ? {
              surface_form: row.surface,
              normalized_target: row.target,
              kind: "instrument",
              confidence: 1.0,
              channels: row.channel ? [row.channel] : ["speech", "text"],
            }
          : null,
        suggested_test: `alias_${String(row.surface || "").replace(/\s+/g, "_")}_resolves`,
      });
    }
  }

  if (lastSnapshot) {
    const after = utterances.filter((u) => (u.ts || 0) >= lastSnapshot.ts);
    const pairCounts = new Map();
    for (const u of after) {
      for (const p of u.proposals || []) {
        const key = `${String(p.surface || "").toLowerCase()}|${String(p.target || "").toLowerCase()}|${u.channel || ""}`;
        const row = pairCounts.get(key) || { n: 0, repaired: 0, surface: p.surface, target: p.target, channel: u.channel };
        row.n += 1;
        if (u.repaired_from) row.repaired += 1;
        pairCounts.set(key, row);
      }
    }
    for (const row of pairCounts.values()) {
      if (row.n < STUCK_REPAIR_MIN_N) continue;
      const rate = row.n ? row.repaired / row.n : 0;
      if (rate >= STUCK_REPAIR_RATE) {
        push({
          action: rate > 0 ? "alias-did-not-land" : "needs_more_n",
          surface: row.surface,
          target: row.target,
          channel: row.channel,
          n: row.n,
          repair_rate: rate,
          threshold: `repair rate stays high after snapshot bump (n≥${STUCK_REPAIR_MIN_N}, rate≥${STUCK_REPAIR_RATE})`,
        });
      } else if (row.n < STUCK_REPAIR_MIN_N) {
        push({
          action: "needs_more_n",
          surface: row.surface,
          target: row.target,
          channel: row.channel,
          n: row.n,
          threshold: `need n≥${STUCK_REPAIR_MIN_N} after snapshot bump`,
        });
      }
    }
  }

  const recentFrom = now - FRAME_GAP_LOOKBACK_SECS;
  const priorFrom = now - 2 * FRAME_GAP_LOOKBACK_SECS;
  const noneAct = (from, to) =>
    utterances.filter(
      (u) =>
        (u.ts || 0) >= from &&
        (u.ts || 0) < to &&
        u.grammar === "none" &&
        u.action_ir?.act,
    ).length;
  const recent = noneAct(recentFrom, now + 1);
  const prior = noneAct(priorFrom, recentFrom);
  if (recent >= FRAME_GAP_MIN_N && recent > prior) {
    push({
      action: "frame_gap",
      surface: null,
      target: null,
      channel: null,
      n: recent,
      prior,
      threshold: `grammar-none + has act rising (n≥${FRAME_GAP_MIN_N} in ${FRAME_GAP_LOOKBACK_SECS}s)`,
    });
  }

  return decisions.slice(0, 20);
}

export const THRESHOLDS = {
  PROMOTE_CONFUSABLE_MIN_N,
  PROMOTE_CONFUSABLE_ACCEPT_RATE,
  NEGATIVE_FIXTURE_REJECT_N,
  ADD_ALIAS_UNKNOWN_N,
  STUCK_REPAIR_MIN_N,
  STUCK_REPAIR_RATE,
  FRAME_GAP_MIN_N,
  FRAME_GAP_LOOKBACK_SECS,
};

export function ontologyStats({ accountId, from, to, all } = {}) {
  const fromTs = from ? Number(from) : null;
  const toTs = to ? Number(to) : null;
  const now = nowSecs();
  const weekFrom = now - FRAME_GAP_LOOKBACK_SECS;
  const ids = all ? listVoiceAccountIds() : accountId ? [String(accountId)] : [];
  const utterances = [];
  const correctedIds = new Set();
  let trainingUse = false;
  for (const id of ids) {
    const data = loadVoiceFile(id);
    if (trainingUseOf(data)) trainingUse = true;
    for (const row of data.utterances || []) {
      if (!inRange(row.ts || 0, fromTs, toTs)) continue;
      utterances.push(row);
    }
    for (const row of data.corrections || []) {
      if (row.utterance_ref) correctedIds.add(row.utterance_ref);
    }
  }

  const split = (fromBound, toBound) => {
    const speech = emptyChannelStats();
    const text = emptyChannelStats();
    for (const u of utterances) {
      if (!inRange(u.ts || 0, fromBound, toBound)) continue;
      absorbUtterance(u.channel === "text" ? text : speech, u, correctedIds);
    }
    return { speech, text };
  };

  const candidates = loadCandidates();
  const table = all && trainingUse
    ? candidates.global
    : accountId
      ? candidates.accounts[String(accountId)] || {}
      : {};
  const rows = tableToRows(table);
  const snapshots = loadSnapshots().items || [];
  const lastSnapshot = snapshots[snapshots.length - 1] || null;
  const decisions = decide(rows, utterances, lastSnapshot, now);

  return {
    ok: true,
    account_id: accountId || null,
    from: fromTs,
    to: toTs,
    operator_local: Boolean(all),
    note: all
      ? "?all=1 iterates voice/*.json on this brain host. Operator-local; do not export other users' utterances without training_use."
      : null,
    training_use: trainingUse,
    thresholds: THRESHOLDS,
    last_7d: split(weekFrom, now + 1),
    all_time: split(fromTs, toTs || now + 1),
    candidates: rows
      .sort(
        (a, b) =>
          (b.proposed || 0) +
          (b.unknown || 0) -
          ((a.proposed || 0) + (a.unknown || 0)),
      )
      .slice(0, 40),
    decisions,
    snapshots,
  };
}

function inRange(ts, from, to) {
  if (from != null && Number.isFinite(from) && ts < from) return false;
  if (to != null && Number.isFinite(to) && ts > to) return false;
  return true;
}
