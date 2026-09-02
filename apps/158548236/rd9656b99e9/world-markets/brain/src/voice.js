/**
 * Voice loop records (F10–F19). Unsigned. Never places an order.
 * Audio blobs live under the brain data dir — not Graphiti, not Telegram.
 */

import { randomBytes } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { filePath, readJson, writeJson } from "./store.js";
import { kindRank, ontologyKeyterms } from "./ontology.js";
import { recordCandidateOutcome, recordFromUtterance } from "./ontology_stats.js";

const EPISODE_GAP_SECS = 90;
const MAX_UTTERANCES = 400;
const MAX_CORRECTIONS = 400;
const MAX_KEYTERMS = 75;

function nowSecs() {
  return Math.floor(Date.now() / 1000);
}

function newId(prefix) {
  return `${prefix}_${randomBytes(6).toString("hex")}`;
}

function voicePath(accountId) {
  return filePath("voice", `${accountId}.json`);
}

function loadVoice(accountId) {
  return readJson(voicePath(accountId), {
    lexicon: [],
    utterances: [],
    consents: [],
    episodes: [],
    corrections: [],
    flags: { long_note_norm_sent: false },
  });
}

function saveVoice(accountId, data) {
  writeJson(voicePath(accountId), data);
}

function cap(list, max) {
  if (list.length <= max) return list;
  return list.slice(list.length - max);
}

export function keyterms(accountId, extra = []) {
  const data = loadVoice(accountId);
  const seen = new Set();
  const out = [];
  const push = (surface) => {
    const term = String(surface || "").trim();
    if (term.length < 2) return;
    const key = term.toLowerCase();
    if (seen.has(key)) return;
    seen.add(key);
    out.push(term);
  };
  for (const row of extra) push(row);
  const ranked = [...data.lexicon].sort((a, b) => {
    const d = kindRank(a.kind) - kindRank(b.kind);
    if (d !== 0) return d;
    return (b.confidence || 0) - (a.confidence || 0);
  });
  for (const row of ranked) {
    if (row.kind === "confusable") continue;
    push(row.surface_form);
  }
  for (const term of ontologyKeyterms()) push(term);
  return out.slice(0, MAX_KEYTERMS);
}

function applyLexicon(data, entries, now) {
  const list = Array.isArray(entries) ? entries : entries ? [entries] : [];
  for (const raw of list) {
    const surface = String(raw.surface_form || raw.surface || "").trim();
    const target = String(raw.normalized_target || raw.target || surface).trim();
    if (!surface) continue;
    const kind = raw.kind || "phrase";
    if (kind === "confusable") continue;
    const source = raw.source || "auto";
    const existing = data.lexicon.find(
      (row) =>
        row.surface_form.toLowerCase() === surface.toLowerCase() &&
        row.normalized_target.toLowerCase() === target.toLowerCase(),
    );
    if (existing) {
      existing.last_used = now;
      existing.confidence = Math.min(
        1,
        (existing.confidence || 0.4) + (source === "confirmed" ? 0.3 : 0.1),
      );
      if (source === "confirmed") existing.source = "confirmed";
      continue;
    }
    data.lexicon.push({
      surface_form: surface,
      normalized_target: target,
      kind,
      confidence: source === "confirmed" ? 0.9 : 0.5,
      first_seen: now,
      last_used: now,
      source,
    });
  }
}

export function lexiconOf(accountId) {
  return loadVoice(accountId).lexicon || [];
}

export function upsertLexicon(accountId, entries, now = nowSecs()) {
  const data = loadVoice(accountId);
  applyLexicon(data, entries, now);
  saveVoice(accountId, data);
  return { ok: true, lexicon: data.lexicon };
}

function persistAudio(utteranceId, audioBase64) {
  if (!audioBase64) return null;
  const buf = Buffer.from(String(audioBase64), "base64");
  if (!buf.length) return null;
  const dir = filePath("audio");
  mkdirSync(dir, { recursive: true });
  const rel = `audio/${utteranceId}.bin`;
  writeFileSync(filePath("audio", `${utteranceId}.bin`), buf);
  return { ref: rel, bytes: buf.length };
}

function openOrJoinEpisode(data, utteranceId, now) {
  const open = [...data.episodes].reverse().find((ep) => ep.state === "open");
  if (open && now - (open.last_ts || open.opened_at) <= EPISODE_GAP_SECS) {
    open.exchanges.push(utteranceId);
    open.last_ts = now;
    return open;
  }
  if (open && open.state === "open") {
    open.state = "recapped";
    open.closed_at = now;
    open.close_reason = "silence";
  }
  const episode = {
    id: newId("ep"),
    opened_at: now,
    last_ts: now,
    exchanges: [utteranceId],
    instructions_touched: [],
    state: "open",
  };
  data.episodes.push(episode);
  return episode;
}

export function ingestUtterance(accountId, body, now = nowSecs()) {
  const text = String(body.text || body.transcript || "").trim();
  if (!text) return { ok: false, error: "transcript_required" };
  const data = loadVoice(accountId);
  data.flags = data.flags || { long_note_norm_sent: false };
  const utteranceId = body.utterance_id || newId("utt");
  const audio = persistAudio(utteranceId, body.audio_base64);
  const duration = Number(body.duration_secs) || 0;
  const longNote = duration > 60;
  const splitParse = duration > 300;
  let longNoteLine = false;
  if (longNote && !data.flags.long_note_norm_sent) {
    data.flags.long_note_norm_sent = true;
    longNoteLine = true;
  }
  const utterance = {
    id: utteranceId,
    text,
    repaired_from: body.repaired_from || null,
    words: Array.isArray(body.words) ? body.words : [],
    lang: body.lang || "en",
    stt_version: body.stt_version || null,
    keyterm_applied: Boolean(body.keyterm_applied),
    duration_secs: duration || null,
    audio_ref: audio ? audio.ref : body.audio_ref || null,
    audio_bytes: audio ? audio.bytes : null,
    mean_confidence: meanConf(body.words),
    ts: now,
    source: body.source || "mini_app",
    foreign: Boolean(body.foreign),
    channel: body.channel || null,
    ontology_version: body.ontology_version ?? null,
    slots: Array.isArray(body.slots) ? body.slots : [],
    proposals: Array.isArray(body.proposals)
      ? body.proposals
      : Array.isArray(body.proposed_confusables)
        ? body.proposed_confusables
        : [],
    grammar: body.grammar || null,
    action_ir: body.action_ir || null,
    lexicon_hits: Array.isArray(body.lexicon_hits) ? body.lexicon_hits : [],
    unknown_instruments: Array.isArray(body.unknown_instruments)
      ? body.unknown_instruments
      : [],
  };
  data.utterances = cap([...data.utterances, utterance], MAX_UTTERANCES);
  const episode = body.foreign
    ? null
    : openOrJoinEpisode(data, utteranceId, now);
  if (Array.isArray(body.lexicon_hits) && body.lexicon_hits.length) {
    applyLexicon(data, body.lexicon_hits, now);
  }
  saveVoice(accountId, data);
  const training = (data.consents || []).some(
    (c) => c.kind === "training_use" && c.status === "granted",
  );
  recordFromUtterance(accountId, utterance, training);
  return {
    ok: true,
    utterance,
    episode,
    heard_echo: text,
    long_note: longNote,
    long_note_line: longNoteLine,
    split_parse: splitParse,
  };
}

export function stampUtterance(accountId, utteranceRef, patch) {
  if (!utteranceRef) return null;
  const data = loadVoice(accountId);
  const row = (data.utterances || []).find((u) => u.id === utteranceRef);
  if (!row) return null;
  Object.assign(row, patch || {});
  saveVoice(accountId, data);
  return row;
}

function meanConf(words) {
  if (!Array.isArray(words) || !words.length) return null;
  const nums = words
    .map((w) => Number(w.conf ?? w.confidence))
    .filter((n) => Number.isFinite(n));
  if (!nums.length) return null;
  return nums.reduce((a, b) => a + b, 0) / nums.length;
}

export function recordCorrection(accountId, body, now = nowSecs()) {
  const data = loadVoice(accountId);
  const row = {
    id: newId("corr"),
    utterance_ref: body.utterance_ref || null,
    rejected_intent: body.rejected_intent || null,
    rejected_readback: body.rejected_readback || null,
    correction_utterance_ref: body.correction_utterance_ref || null,
    accepted_intent: body.accepted_intent || null,
    accepted_readback: body.accepted_readback || null,
    signed: false,
    ts: now,
    versions: body.versions || null,
  };
  data.corrections = cap([...data.corrections, row], MAX_CORRECTIONS);
  if (body.lexicon_rename) {
    applyLexicon(data, { ...body.lexicon_rename, source: "confirmed" }, now);
  }
  saveVoice(accountId, data);
  if (body.utterance_ref) {
    const training = (data.consents || []).some(
      (c) => c.kind === "training_use" && c.status === "granted",
    );
    const utt = (data.utterances || []).find((u) => u.id === body.utterance_ref);
    const acceptedSym = body.accepted_intent?.symbol;
    const rejectedSym = body.rejected_intent?.symbol;
    for (const proposal of utt?.proposals || []) {
      const outcome =
        acceptedSym && String(proposal.target) === String(acceptedSym)
          ? "accepted"
          : "rejected";
      recordCandidateOutcome(accountId, {
        surface: proposal.surface,
        target: proposal.target,
        slotKind: proposal.kind || "confusable",
        channel: utt?.channel,
        outcome,
        trainingUse: training,
      });
    }
    if (rejectedSym && !utt?.proposals?.length) {
      recordCandidateOutcome(accountId, {
        surface: rejectedSym,
        target: acceptedSym || rejectedSym,
        slotKind: "instrument",
        channel: utt?.channel,
        outcome: acceptedSym ? "accepted" : "rejected",
        trainingUse: training,
      });
    }
  }
  return { ok: true, correction: row };
}

export function setConsent(accountId, body, now = nowSecs()) {
  const kind = String(body.kind || "").trim();
  if (!kind) return { ok: false, error: "kind_required" };
  const data = loadVoice(accountId);
  const row = {
    user: accountId,
    kind,
    status: body.status || "granted",
    wording_version: body.wording_version || "v1",
    ts: now,
    msg_ref: body.msg_ref || null,
  };
  data.consents.push(row);
  saveVoice(accountId, data);
  return { ok: true, consent: row, consents: data.consents };
}

export function closeEpisode(accountId, body, now = nowSecs()) {
  const data = loadVoice(accountId);
  const open = [...data.episodes].reverse().find((ep) => ep.state === "open");
  if (!open) return { ok: true, episode: null };
  open.state = "recapped";
  open.closed_at = now;
  open.close_reason = body.reason || "done_for_now";
  saveVoice(accountId, data);
  return { ok: true, episode: open };
}

export function exportEval(accountId) {
  const data = loadVoice(accountId);
  const training = data.consents.some(
    (c) => c.kind === "training_use" && c.status === "granted",
  );
  const pairs = data.corrections.map((row) => ({
    prompt: row.rejected_readback,
    rejected: row.rejected_intent,
    chosen: row.accepted_intent,
    utterance_ref: row.utterance_ref,
  }));
  const utterances = training
    ? data.utterances.map((u) => ({
        ...u,
        channel: u.channel || null,
        raw: u.repaired_from || u.text,
        normalized: u.text,
        proposals: u.proposals || [],
        grammar: u.grammar || null,
        action_ir: u.action_ir || null,
        ontology_version: u.ontology_version ?? null,
      }))
    : [];
  return {
    ok: true,
    training_use: training,
    utterances,
    pairs,
  };
}

export function voiceContext(accountId) {
  const data = loadVoice(accountId);
  const episode = [...data.episodes].reverse().find((ep) => ep.state === "open") || null;
  const consents = {};
  for (const row of data.consents) consents[row.kind] = row.status;
  return {
    lexicon: data.lexicon,
    open_episode: episode,
    consents,
    last_utterance: data.utterances[data.utterances.length - 1] || null,
    correction_count: data.corrections.length,
  };
}
