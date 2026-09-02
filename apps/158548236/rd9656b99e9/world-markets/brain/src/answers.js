/**
 * Mini App answer-sheet projection. Thread messages remain authoritative;
 * this is a GET-able index keyed by correlation id, not a second conversation.
 */

import { filePath, readJson, writeJson } from "./store.js";

const MAX_ANSWERS = 80;

function nowSecs() {
  return Math.floor(Date.now() / 1000);
}

function answersPath(accountId) {
  return filePath("answers", `${accountId}.json`);
}

function loadAnswers(accountId) {
  return readJson(answersPath(accountId), { items: [] });
}

function saveAnswers(accountId, data) {
  data.items = (data.items || []).slice(-MAX_ANSWERS);
  writeJson(answersPath(accountId), data);
}

function findItem(data, correlationId) {
  const id = String(correlationId || "");
  if (!id) return null;
  return (data.items || []).find((row) => row.correlation_id === id) || null;
}

export function getAnswer(accountId, correlationId) {
  const data = loadAnswers(accountId);
  const item = findItem(data, correlationId);
  if (!item) {
    return { ok: true, found: false, correlation_id: correlationId || null };
  }
  return { ok: true, found: true, ...item };
}

export function upsertAnswer(accountId, body, now = nowSecs()) {
  const correlationId = String(body.correlation_id || "").trim();
  if (!correlationId) {
    return { ok: false, error: "correlation_id required" };
  }
  const data = loadAnswers(accountId);
  let item = findItem(data, correlationId);
  if (!item) {
    item = {
      correlation_id: correlationId,
      account_id: Number(accountId) || accountId,
      question: "",
      heard_echo: "",
      referent: null,
      status: "working",
      answer: null,
      controls: [],
      voice_note_url: null,
      parent_correlation_id: body.parent_correlation_id || null,
      context_ref: body.context_ref || null,
      created_at: now,
      updated_at: now,
    };
    data.items.push(item);
  }
  if (body.question != null) item.question = String(body.question);
  if (body.heard_echo != null) item.heard_echo = String(body.heard_echo);
  if (body.referent !== undefined) item.referent = body.referent || null;
  if (body.status) item.status = String(body.status);
  if (body.answer != null) {
    item.answer = String(body.answer);
    if (!body.status) item.status = "answered";
  }
  if (Array.isArray(body.controls)) item.controls = body.controls;
  if (body.voice_note_url !== undefined) {
    item.voice_note_url = body.voice_note_url || null;
  }
  if (body.parent_correlation_id !== undefined) {
    item.parent_correlation_id = body.parent_correlation_id || null;
  }
  if (body.context_ref !== undefined) item.context_ref = body.context_ref || null;
  item.updated_at = now;
  saveAnswers(accountId, data);
  return { ok: true, ...item };
}

export function latestWorking(accountId) {
  const data = loadAnswers(accountId);
  const items = data.items || [];
  for (let i = items.length - 1; i >= 0; i -= 1) {
    if (items[i].status === "working") return items[i];
  }
  return null;
}
