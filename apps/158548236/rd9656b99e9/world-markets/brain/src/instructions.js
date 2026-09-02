import { readdirSync } from "node:fs";
import { filePath, readJson, writeJson } from "./store.js";
import { latestMark } from "./history.js";

export const VISIBILITY_SECS = 90 * 24 * 60 * 60;
export const TRADE_DELAY_SECS = 3;
const NEAR_MISS_BAND = 0.02;

const STATUSES = new Set([
  "with_aomi",
  "watching",
  "triggered",
  "awaiting_confirm",
  "pending_execute",
  "executing",
  "done",
  "declined",
  "paused",
  "expired",
  "revoked",
  "cant",
]);

const ALLOWED = {
  with_aomi: ["watching", "declined", "expired", "revoked"],
  watching: [
    "triggered",
    "awaiting_confirm",
    "paused",
    "expired",
    "revoked",
    "done",
    "executing",
  ],
  triggered: ["awaiting_confirm", "executing", "done", "watching", "revoked"],
  awaiting_confirm: ["executing", "declined", "watching", "done", "revoked"],
  pending_execute: ["executing", "revoked", "declined"],
  executing: ["done", "revoked", "paused"],
  paused: ["watching", "revoked", "expired", "executing"],
  done: [],
  declined: [],
  expired: [],
  revoked: [],
  cant: [],
};

const EVENT_TYPES = new Set([
  "heard",
  "parsed",
  "sent_to_thread",
  "confirmed",
  "declined",
  "check",
  "near_miss",
  "triggered",
  "confirm_requested",
  "executed",
  "blocked",
  "reported",
  "staged",
  "paused",
  "resumed",
  "expired",
  "revoked",
  "edited",
  "archived",
]);

function itemsPath(accountId) {
  return filePath("instructions", `${accountId}.json`);
}

function eventsPath(accountId) {
  return filePath("instruction_events", `${accountId}.json`);
}

function loadItems(accountId) {
  return readJson(itemsPath(accountId), { items: [] });
}

function saveItems(accountId, data) {
  writeJson(itemsPath(accountId), data);
}

function loadEvents(accountId) {
  return readJson(eventsPath(accountId), { items: [] });
}

function saveEvents(accountId, data) {
  writeJson(eventsPath(accountId), data);
}

export function newId() {
  if (globalThis.crypto?.randomUUID) return globalThis.crypto.randomUUID();
  return `i-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
}

function nowSecs(now) {
  return Number.isFinite(now) ? now : Math.floor(Date.now() / 1000);
}

function scheduleOf(params) {
  const raw = params && typeof params === "object" ? params.schedule : null;
  return raw && typeof raw === "object" ? raw : null;
}

function sliceCount(params, fallback = 1) {
  const scheduled = Number(scheduleOf(params)?.slices);
  if (Number.isFinite(scheduled) && scheduled > 0) return Math.floor(scheduled);
  const named = Number(fallback);
  return Number.isFinite(named) && named > 0 ? Math.floor(named) : 1;
}

function intervalSecsOf(params, item) {
  const scheduled = Number(scheduleOf(params)?.interval_secs);
  if (Number.isFinite(scheduled) && scheduled > 0) return Math.floor(scheduled);
  const cadence = String(scheduleOf(params)?.cadence || "").toLowerCase();
  if (cadence === "weekly") return 7 * 86400;
  if (cadence === "daily") return 86400;
  const n = sliceCount(params, item?.slice_n);
  const windowSecs = Number(scheduleOf(params)?.window_secs);
  if (Number.isFinite(windowSecs) && windowSecs > 0 && n > 1) {
    return Math.max(1, Math.floor(windowSecs / n));
  }
  return 60;
}

export function instructionAccounts() {
  try {
    return readdirSync(filePath("instructions"))
      .filter((name) => name.endsWith(".json"))
      .map((name) => name.replace(/\.json$/, ""));
  } catch {
    return [];
  }
}

function findItem(data, instructionId) {
  const raw = String(instructionId || "").trim();
  if (!raw) return undefined;
  const key = raw.startsWith("i_") ? raw.slice(2) : raw;
  return (data.items || []).find(
    (row) =>
      row.instruction_id === key ||
      row.task_id === key ||
      row.watch_id === key,
  );
}

function allocTaskId(data) {
  const used = new Set((data.items || []).map((row) => row.task_id).filter(Boolean));
  const alphabet = "23456789abcdefghijkmnpqrstuvwxyz";
  for (let n = 0; n < 80; n++) {
    let id = "";
    for (let i = 0; i < 6; i++) {
      id += alphabet[Math.floor(Math.random() * alphabet.length)];
    }
    if (!used.has(id)) return id;
  }
  return newId().replace(/-/g, "").slice(0, 8);
}

function ensureTaskId(data, item) {
  if (item.task_id) return item.task_id;
  item.task_id = allocTaskId(data);
  return item.task_id;
}

export function transition(item, next, now) {
  if (!STATUSES.has(next)) {
    throw new Error(`illegal_status:${next}`);
  }
  const allowed = ALLOWED[item.status] || [];
  if (item.status === next) return item;
  if (!allowed.includes(next)) {
    throw new Error(`illegal_transition:${item.status}->${next}`);
  }
  item.status = next;
  item.status_changed_at = now;
  item.updated_at = now;
  return item;
}

export function appendEvent(accountId, instructionId, eventType, detail, now, extra = {}) {
  if (!EVENT_TYPES.has(eventType)) {
    throw new Error(`illegal_event:${eventType}`);
  }
  const data = loadEvents(accountId);
  data.items = data.items || [];
  const prior = data.items.filter((row) => row.instruction_id === instructionId);
  const seq = prior.length + 1;
  const event = {
    instruction_id: instructionId,
    seq,
    event_type: eventType,
    at: nowSecs(now),
    ref: extra.ref || null,
    actor: extra.actor || actorFor(eventType),
    detail: detail || "",
    signed: Boolean(extra.signed),
    origin: extra.origin || null,
  };
  data.items.push(event);
  saveEvents(accountId, data);
  return event;
}

function actorFor(eventType) {
  if (
    eventType === "heard" ||
    eventType === "confirmed" ||
    eventType === "declined" ||
    eventType === "archived"
  ) {
    return "you";
  }
  if (eventType === "check" || eventType === "near_miss") return "watcher";
  if (
    eventType === "executed" ||
    eventType === "triggered" ||
    eventType === "blocked" ||
    eventType === "reported"
  ) {
    return "engine";
  }
  return "aomi";
}

export function composeDraft(accountId, body, now = nowSecs()) {
  const kind = body.kind || "conditional";
  if (kind === "question") {
    return { ok: true, recorded: false, kind };
  }
  const instructionId = body.instruction_id || newId();
  const data = loadItems(accountId);
  data.items = data.items || [];
  let item = findItem(data, instructionId);
  const sentence = String(body.message || body.sentence || "").trim().slice(0, 160);
  if (!sentence) {
    return { ok: false, error: "sentence_required" };
  }

  if (kind === "pause" || kind === "resume") {
    if (!item) return { ok: false, error: "not_found" };
    item.pending = kind;
    item.updated_at = now;
    saveItems(accountId, data);
    appendEvent(
      accountId,
      instructionId,
      "sent_to_thread",
      kind === "pause" ? "pause sent to thread" : "resume sent to thread",
      now,
      { ref: body.correlation_id || null },
    );
    return { ok: true, recorded: true, instruction: cardOf(item, accountId) };
  }

  if (item) {
    return { ok: true, recorded: true, instruction: cardOf(item, accountId), duplicate: true };
  }

  item = {
    instruction_id: instructionId,
    account_id: Number(accountId) || accountId,
    kind: kind === "watch" ? "watch" : kind,
    sentence,
    params: body.params || {},
    status: "with_aomi",
    policy_scope: body.policy_scope || null,
    source_ref: body.correlation_id || body.source_ref || instructionId,
    confirm_ref: null,
    result_ref: null,
    watch_id: null,
    task_id: allocTaskId(data),
    correlation_id: body.correlation_id || instructionId,
    expires_at: body.expires_at || null,
    check_stats: { last_check_at: null, checks_7d: 0 },
    pending: null,
    fire_kind: body.fire_kind || (kind === "watch" ? "tell" : "act"),
    instrument: body.instrument || body.params?.instrument || null,
    created_at: now,
    updated_at: now,
    status_changed_at: now,
  };
  data.items.push(item);
  saveItems(accountId, data);
  appendEvent(accountId, instructionId, "sent_to_thread", sentence, now, {
    ref: item.source_ref,
    actor: "you",
  });
  return { ok: true, recorded: true, instruction: cardOf(item, accountId) };
}

export function stageTrade(accountId, body, now = nowSecs()) {
  const sentence = String(body.sentence || body.message || "").trim().slice(0, 400);
  if (!sentence) return { ok: false, error: "sentence_required" };
  const delay = Math.max(1, Number(body.delay_secs) || TRADE_DELAY_SECS);
  const data = loadItems(accountId);
  data.items = data.items || [];
  const instructionId = body.instruction_id || newId();
  let item = findItem(data, instructionId);
  if (item && item.status === "pending_execute") {
    return { ok: true, recorded: true, instruction: cardOf(item, accountId, now), duplicate: true };
  }
  if (item) {
    return { ok: false, error: "conflict", instruction: cardOf(item, accountId, now) };
  }
  item = {
    instruction_id: instructionId,
    account_id: Number(accountId) || accountId,
    kind: "trade",
    sentence,
    params: body.params || {},
    status: "pending_execute",
    policy_scope: body.policy_scope || null,
    source_ref: body.correlation_id || body.source_ref || instructionId,
    confirm_ref: null,
    result_ref: null,
    watch_id: null,
    task_id: allocTaskId(data),
    correlation_id: body.correlation_id || instructionId,
    expires_at: null,
    check_stats: { last_check_at: null, checks_7d: 0 },
    pending: null,
    fire_kind: "act",
    instrument: body.instrument || body.params?.base_symbol || null,
    execute_at: now + delay,
    delay_secs: delay,
    progress_pct: 0,
    slice_i: null,
    slice_n: sliceCount(body.params, body.slice_n || 1),
    avg_price: null,
    next_slice_at: null,
    child_fills: [],
    slice_inflight: false,
    created_at: now,
    updated_at: now,
    status_changed_at: now,
  };
  data.items.push(item);
  saveItems(accountId, data);
  appendEvent(accountId, instructionId, "staged", sentence, now, { actor: "you" });
  return { ok: true, recorded: true, instruction: cardOf(item, accountId, now) };
}

export function beginExecute(accountId, id, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, id);
  if (!item) return { ok: false, error: "not_found" };
  if (item.status === "revoked" || item.status === "declined") {
    return { ok: false, error: "cancelled", instruction: cardOf(item, accountId, now) };
  }
  if (item.status === "done") {
    return { ok: true, already: true, done: true, instruction: cardOf(item, accountId, now) };
  }
  if (item.status === "executing") {
    return {
      ok: true,
      already: true,
      instruction: cardOf(item, accountId, now),
      params: item.params || {},
    };
  }
  if (item.status !== "pending_execute") {
    return { ok: false, error: "not_pending", instruction: cardOf(item, accountId, now) };
  }
  if (item.execute_at && now < item.execute_at) {
    return { ok: false, error: "too_soon", instruction: cardOf(item, accountId, now) };
  }
  transition(item, "executing", now);
  item.progress_pct = 8;
  item.slice_i = 1;
  item.slice_n = sliceCount(item.params, item.slice_n || 1);
  item.slice_inflight = true;
  item.slice_inflight_at = now;
  item.child_fills = Array.isArray(item.child_fills) ? item.child_fills : [];
  item.updated_at = now;
  saveItems(accountId, data);
  appendEvent(accountId, item.instruction_id, "executed", "sending", now, { actor: "aomi" });
  return { ok: true, instruction: cardOf(item, accountId, now), params: item.params || {} };
}

export function completeExecute(accountId, id, body = {}, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, id);
  if (!item) return { ok: false, error: "not_found" };
  if (item.status === "revoked" || item.status === "declined") {
    return { ok: false, error: "cancelled", instruction: cardOf(item, accountId, now) };
  }
  if (item.status === "pending_execute") {
    transition(item, "executing", now);
  }
  if (item.status !== "executing" && item.status !== "done") {
    return { ok: false, error: "not_executing", instruction: cardOf(item, accountId, now) };
  }
  const failed = Boolean(body.failed || body.error);
  item.progress_pct = failed ? item.progress_pct : 100;
  item.slice_i = item.slice_n || 1;
  item.avg_price = body.avg_price || item.avg_price;
  item.receipt = body.receipt || item.receipt;
  item.result_ref = body.result_ref || item.result_ref;
  item.slice_inflight = false;
  item.next_slice_at = null;
  if (Array.isArray(body.child_fills)) {
    item.child_fills = body.child_fills;
  }
  if (item.status === "executing") transition(item, "done", now);
  item.updated_at = now;
  saveItems(accountId, data);
  appendEvent(
    accountId,
    item.instruction_id,
    failed ? "blocked" : "executed",
    item.receipt || (failed ? String(body.error || "failed") : "filled"),
    now,
    { actor: "aomi" },
  );
  return { ok: true, instruction: cardOf(item, accountId, now) };
}

const INFLIGHT_STALE_SECS = 120;

export function claimSlice(accountId, id, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, id);
  if (!item) return { ok: false, error: "not_found" };
  if (item.status === "revoked" || item.status === "declined") {
    return { ok: false, error: "cancelled", instruction: cardOf(item, accountId, now) };
  }
  if (item.status === "done") {
    return { ok: true, already: true, done: true, instruction: cardOf(item, accountId, now) };
  }
  if (item.status === "pending_execute") {
    const begun = beginExecute(accountId, id, now);
    if (!begun.ok) return begun;
    if (begun.already) {
      return { ok: false, error: "in_flight", instruction: begun.instruction };
    }
    const fresh = loadItems(accountId);
    const row = findItem(fresh, id);
    return {
      ok: true,
      first: true,
      slice_i: row?.slice_i || 1,
      slice_n: row?.slice_n || 1,
      last: (row?.slice_n || 1) <= 1,
      instruction: cardOf(row || item, accountId, now),
      params: row?.params || item.params || {},
    };
  }
  if (item.status !== "executing") {
    return { ok: false, error: "not_executing", instruction: cardOf(item, accountId, now) };
  }
  const fills = Array.isArray(item.child_fills) ? item.child_fills : [];
  const sliceN = sliceCount(item.params, item.slice_n || 1);
  if (fills.length >= sliceN) {
    return { ok: true, already: true, done: true, instruction: cardOf(item, accountId, now) };
  }
  if (item.slice_inflight) {
    const claimedAt = Number(item.slice_inflight_at) || 0;
    if (!claimedAt || now - claimedAt < INFLIGHT_STALE_SECS) {
      return { ok: false, error: "in_flight", instruction: cardOf(item, accountId, now) };
    }
  }
  if (item.next_slice_at && now < item.next_slice_at) {
    return { ok: false, error: "too_soon", instruction: cardOf(item, accountId, now) };
  }
  item.slice_i = fills.length + 1;
  item.slice_n = sliceN;
  item.slice_inflight = true;
  item.slice_inflight_at = now;
  item.updated_at = now;
  saveItems(accountId, data);
  return {
    ok: true,
    first: false,
    slice_i: item.slice_i,
    slice_n: sliceN,
    last: item.slice_i >= sliceN,
    instruction: cardOf(item, accountId, now),
    params: item.params || {},
  };
}

export function recordSlice(accountId, id, body = {}, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, id);
  if (!item) return { ok: false, error: "not_found" };
  if (item.status === "revoked" || item.status === "declined") {
    return { ok: false, error: "cancelled", instruction: cardOf(item, accountId, now) };
  }
  if (item.status === "done") {
    return { ok: true, already: true, done: true, instruction: cardOf(item, accountId, now) };
  }
  if (item.status !== "executing") {
    return { ok: false, error: "not_executing", instruction: cardOf(item, accountId, now) };
  }
  item.child_fills = Array.isArray(item.child_fills) ? item.child_fills : [];
  if (body.fill && typeof body.fill === "object") {
    item.child_fills.push(body.fill);
  }
  const sliceN = sliceCount(item.params, item.slice_n || 1);
  item.slice_n = sliceN;
  item.slice_i = body.slice_i || item.child_fills.length || item.slice_i;
  item.avg_price = body.avg_price || item.avg_price;
  item.receipt = body.receipt || item.receipt;
  item.result_ref = body.result_ref || item.result_ref;
  const filled = item.child_fills.length;
  item.progress_pct = Math.max(
    8,
    Math.min(99, Math.round((filled / Math.max(1, sliceN)) * 100)),
  );
  item.slice_inflight = false;
  item.slice_inflight_at = null;
  if (item.params && typeof item.params === "object") {
    item.params.schedule = item.params.schedule || {};
    if (body.filled_quantity != null) {
      item.params.schedule.filled_quantity = String(body.filled_quantity);
    }
  }
  const more = filled < sliceN;
  if (more) {
    const interval = intervalSecsOf(item.params, item);
    item.next_slice_at = now + interval;
  } else {
    item.next_slice_at = null;
  }
  item.updated_at = now;
  saveItems(accountId, data);
  appendEvent(
    accountId,
    item.instruction_id,
    "executed",
    body.receipt || `slice ${item.slice_i} of ${sliceN}`,
    now,
    { actor: "aomi" },
  );
  return {
    ok: true,
    more,
    next_slice_at: item.next_slice_at,
    interval_secs: more ? intervalSecsOf(item.params, item) : null,
    instruction: cardOf(item, accountId, now),
    params: item.params || {},
  };
}

export function listDueTrades(accountId, now = nowSecs()) {
  const accounts =
    accountId == null || accountId === ""
      ? instructionAccounts()
      : [String(accountId)];
  const out = [];
  for (const id of accounts) {
    const data = loadItems(id);
    for (const item of data.items || []) {
      if (item.kind !== "trade") continue;
      if (item.status === "pending_execute" && item.execute_at && now >= item.execute_at) {
        out.push({
          account_id: item.account_id,
          instruction_id: item.instruction_id,
          status: item.status,
        });
        continue;
      }
      if (item.status !== "executing") continue;
      const fills = Array.isArray(item.child_fills) ? item.child_fills.length : 0;
      const sliceN = sliceCount(item.params, item.slice_n || 1);
      if (fills >= sliceN) continue;
      if (item.slice_inflight) {
        const claimedAt = Number(item.slice_inflight_at) || 0;
        if (claimedAt && now - claimedAt < INFLIGHT_STALE_SECS) continue;
      }
      if (item.next_slice_at && now < item.next_slice_at) continue;
      out.push({
        account_id: item.account_id,
        instruction_id: item.instruction_id,
        status: item.status,
      });
    }
  }
  return out;
}

export function confirmInstruction(accountId, body, now = nowSecs()) {
  const data = loadItems(accountId);
  data.items = data.items || [];
  let item = body.instruction_id ? findItem(data, body.instruction_id) : null;
  if (!item && body.correlation_id) {
    item = data.items.find((row) => row.correlation_id === body.correlation_id);
  }
  if (!item) {
    item = {
      instruction_id: body.instruction_id || newId(),
      account_id: Number(accountId) || accountId,
      kind: body.kind || "watch",
      sentence: String(body.sentence || body.phrase || "").slice(0, 160),
      params: body.params || {},
      status: "with_aomi",
      policy_scope: body.policy_scope || null,
      source_ref: body.source_ref || body.watch_id || body.instruction_id,
      confirm_ref: null,
      result_ref: null,
      watch_id: body.watch_id || null,
      correlation_id: body.correlation_id || null,
      expires_at: body.expires_at || null,
      check_stats: { last_check_at: null, checks_7d: 0 },
      pending: null,
      fire_kind: body.fire_kind || "tell",
      instrument: body.instrument || body.params?.instrument || body.symbol || null,
      created_at: now,
      updated_at: now,
      status_changed_at: now,
    };
    if (!item.sentence || !item.source_ref) {
      return { ok: false, error: "untraceable" };
    }
    data.items.push(item);
  }
  if (item.status === "with_aomi") {
    transition(item, "watching", now);
  }
  item.confirm_ref = body.confirm_ref || body.watch_id || item.confirm_ref;
  item.watch_id = body.watch_id || item.watch_id;
  item.expires_at = body.expires_at || item.expires_at;
  item.params = body.params || item.params;
  item.pending = null;
  item.updated_at = now;
  saveItems(accountId, data);
  appendEvent(accountId, item.instruction_id, "confirmed", "Confirmed.", now, {
    signed: true,
    ref: item.confirm_ref,
    actor: "you",
  });
  return { ok: true, instruction: cardOf(item, accountId) };
}

export function attachWatch(accountId, watch, now = nowSecs()) {
  return confirmInstruction(accountId, {
    instruction_id: watch.instruction_id,
    correlation_id: watch.correlation_id,
    sentence: watch.original_phrase || watch.predicate?.resolved,
    phrase: watch.original_phrase,
    params: watch.predicate,
    watch_id: watch.id,
    source_ref: watch.id,
    expires_at: watch.expires_at,
    kind: "watch",
    fire_kind: "tell",
    instrument: watch.predicate?.symbol || watch.symbol,
    symbol: watch.predicate?.symbol,
  }, now);
}

export function pauseInstruction(accountId, instructionId, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, instructionId);
  if (!item) return { ok: false, error: "not_found" };
  transition(item, "paused", now);
  item.pending = null;
  saveItems(accountId, data);
  appendEvent(accountId, instructionId, "paused", "Paused.", now, {
    signed: true,
    actor: "you",
  });
  return { ok: true, instruction: cardOf(item, accountId), watch_id: item.watch_id };
}

export function resumeInstruction(accountId, instructionId, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, instructionId);
  if (!item) return { ok: false, error: "not_found" };
  transition(item, "watching", now);
  item.pending = null;
  saveItems(accountId, data);
  appendEvent(accountId, instructionId, "resumed", "Resumed.", now, {
    signed: true,
    actor: "you",
  });
  return { ok: true, instruction: cardOf(item, accountId), watch_id: item.watch_id };
}

export function revokeInstruction(accountId, instructionId, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, instructionId);
  if (!item) return { ok: false, error: "not_found" };
  ensureTaskId(data, item);
  if (item.status !== "revoked") transition(item, "revoked", now);
  item.pending = null;
  saveItems(accountId, data);
  appendEvent(accountId, item.instruction_id, "revoked", "Cancelled.", now, { actor: "you" });
  return {
    ok: true,
    instruction: cardOf(item, accountId),
    watch_id: item.watch_id,
    task_id: item.task_id,
  };
}

const CANCELLABLE = new Set([
  "with_aomi",
  "watching",
  "triggered",
  "awaiting_confirm",
  "paused",
  "pending_execute",
  "executing",
]);

export function cancelInstruction(accountId, id, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, id);
  if (!item) return { ok: false, error: "not_found" };
  ensureTaskId(data, item);
  if (item.status === "revoked") {
    return {
      ok: true,
      already: true,
      instruction: cardOf(item, accountId),
      watch_id: item.watch_id,
      task_id: item.task_id,
      reply: `already cancelled ${item.task_id}`,
    };
  }
  if (!CANCELLABLE.has(item.status)) {
    return { ok: false, error: "not_cancellable", task_id: item.task_id };
  }
  const result = revokeInstruction(accountId, item.instruction_id, now);
  const sentence = item.sentence || "";
  const clipped = sentence.length > 80 ? `${sentence.slice(0, 77)}…` : sentence;
  result.reply = clipped
    ? `cancelled ${result.task_id} — "${clipped}"`
    : `cancelled ${result.task_id}`;
  result.command = `cancel task ${result.task_id}`;
  return result;
}

const ARCHIVABLE = new Set(["done", "cant"]);

export function archiveInstruction(accountId, id, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, id);
  if (!item) return { ok: false, error: "not_found" };
  if (item.archived_at) {
    return { ok: true, already: true, instruction: cardOf(item, accountId, now) };
  }
  if (!ARCHIVABLE.has(item.status)) {
    return { ok: false, error: "not_archivable", status: item.status };
  }
  item.archived_at = now;
  item.updated_at = now;
  saveItems(accountId, data);
  appendEvent(accountId, item.instruction_id, "archived", "Archived.", now, { actor: "you" });
  return { ok: true, instruction: cardOf(item, accountId, now) };
}

export function recordCheck(accountId, watch, result, now = nowSecs()) {
  if (!result?.ready) return;
  const data = loadItems(accountId);
  const item = (data.items || []).find((row) => row.watch_id === watch.id);
  if (!item || item.status !== "watching") return;
  item.check_stats = item.check_stats || { last_check_at: null, checks_7d: 0 };
  item.check_stats.last_check_at = now;
  item.check_stats.checks_7d = (item.check_stats.checks_7d || 0) + 1;
  item.updated_at = now;
  if (result.live != null) item.last_mark = String(result.live);
  saveItems(accountId, data);
  appendEvent(accountId, item.instruction_id, "check", "", now, { actor: "watcher" });
  maybeNearMiss(accountId, item, result, now);
}

function maybeNearMiss(accountId, item, result, now) {
  const level = Number(item.params?.level);
  const live = Number(result.live);
  if (!Number.isFinite(level) || !Number.isFinite(live) || level === 0) return;
  const band = Math.abs(live - level) / Math.abs(level);
  if (band > NEAR_MISS_BAND || result.true) return;
  const events = loadEvents(accountId).items || [];
  const recent = events
    .filter((row) => row.instruction_id === item.instruction_id && row.event_type === "near_miss")
    .pop();
  if (recent && now - recent.at < 3600) return;
  appendEvent(
    accountId,
    item.instruction_id,
    "near_miss",
    `Near miss — ${result.live} low, condition not held.`,
    now,
  );
}

export function onWatchFired(accountId, watch, result, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = (data.items || []).find((row) => row.watch_id === watch.id);
  if (!item) return;
  const fireKind = item.fire_kind || "tell";
  if (fireKind === "act") {
    if (item.status === "watching") transition(item, "awaiting_confirm", now);
    appendEvent(
      accountId,
      item.instruction_id,
      "triggered",
      `Condition met — ${result.live ?? ""}. Confirm sent to thread.`,
      now,
    );
    appendEvent(accountId, item.instruction_id, "confirm_requested", "", now);
  } else {
    appendEvent(
      accountId,
      item.instruction_id,
      "triggered",
      `Condition met — ${result.live ?? ""}.`,
      now,
    );
    appendEvent(accountId, item.instruction_id, "reported", "told in the thread", now);
    if (watch.fire_mode === "once" && item.status === "watching") {
      transition(item, "done", now);
      item.result_ref = watch.id;
    }
  }
  item.updated_at = now;
  saveItems(accountId, data);
}

export function onWatchExpired(accountId, watch, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = (data.items || []).find((row) => row.watch_id === watch.id);
  if (!item) return;
  if (item.status === "watching" || item.status === "paused" || item.status === "with_aomi") {
    transition(item, "expired", now);
  }
  saveItems(accountId, data);
  appendEvent(accountId, item.instruction_id, "expired", "expired — never met", now);
}

function displayStatus(status) {
  if (status === "with_aomi") return "with aomi";
  if (status === "triggered" || status === "awaiting_confirm") return "needs you";
  if (status === "pending_execute" || status === "executing") return null;
  if (status === "declined" || status === "revoked") return status === "declined" ? "done" : null;
  if (status === "cant") return "can't";
  return status;
}

function distanceFor(item) {
  const level = Number(item.params?.level);
  const live = Number(item.last_mark);
  if (!Number.isFinite(level) || !Number.isFinite(live) || level === 0) {
    const mark = item.params?.symbol ? latestMark(item.params.symbol) : null;
    const liveMark = mark ? Number(mark.mark) : NaN;
    if (!Number.isFinite(liveMark) || !Number.isFinite(level) || level === 0) return null;
    return {
      mark: String(mark.mark),
      pct: Math.max(0, Math.min(100, Math.round((1 - Math.abs(liveMark - level) / Math.abs(level)) * 100))),
      near: Math.abs(liveMark - level) / Math.abs(level) <= 0.08,
    };
  }
  const pct = Math.max(
    0,
    Math.min(100, Math.round((1 - Math.abs(live - level) / Math.abs(level)) * 100)),
  );
  return { mark: item.last_mark, pct, near: Math.abs(live - level) / Math.abs(level) <= 0.08 };
}

function cardOf(item, accountId, now = nowSecs()) {
  const events = loadEvents(accountId).items || [];
  const mine = events.filter((row) => row.instruction_id === item.instruction_id);
  const last = mine[mine.length - 1];
  const delay = Number(item.delay_secs) || TRADE_DELAY_SECS;
  const remaining =
    item.status === "pending_execute" && item.execute_at
      ? Math.max(0, item.execute_at - now)
      : null;
  const countdownPct =
    remaining == null
      ? null
      : Math.max(0, Math.min(100, Math.round(((delay - remaining) / delay) * 100)));
  return {
    instruction_id: item.instruction_id,
    task_id: item.task_id || null,
    kind: item.kind,
    sentence: item.sentence,
    status: item.status,
    display_status: displayStatus(item.status),
    progress_pct:
      item.status === "pending_execute" ? countdownPct : item.progress_pct ?? null,
    remaining_secs: remaining,
    execute_at: item.execute_at || null,
    delay_secs: item.status === "pending_execute" ? delay : item.delay_secs || null,
    slice_i: item.slice_i ?? null,
    slice_n: item.slice_n ?? null,
    avg_price: item.avg_price ?? null,
    next_slice_at: item.next_slice_at ?? null,
    child_fills: item.child_fills || [],
    order_type: item.params?.order_type || null,
    instrument: item.instrument,
    params: item.params || {},
    expires_at: item.expires_at,
    check_stats: item.check_stats || { last_check_at: null, checks_7d: 0 },
    source_ref: item.source_ref,
    confirm_ref: item.confirm_ref,
    result_ref: item.result_ref,
    pending: item.pending || null,
    fire_kind: item.fire_kind || "tell",
    receipt: item.receipt || null,
    trigger_value: item.trigger_value || null,
    last_event_at: last?.at || item.updated_at,
    created_at: item.created_at,
    updated_at: item.updated_at,
    status_changed_at: item.status_changed_at,
    watch_id: item.watch_id,
    correlation_id: item.correlation_id,
    distance: item.status === "watching" ? distanceFor(item) : null,
    last_mark: item.last_mark || null,
    asked_entity: item.asked_entity || null,
    cant_kind: item.cant_kind || null,
    repeat_count: item.repeat_count || 0,
    sub_line: item.sub_line || null,
  };
}

function trailOf(accountId, instructionId) {
  const events = (loadEvents(accountId).items || []).filter(
    (row) => row.instruction_id === instructionId,
  );
  const checks = events.filter((row) => row.event_type === "check");
  const rest = events.filter((row) => row.event_type !== "check");
  const lines = [];
  for (const event of rest) {
    if (event.event_type === "near_miss" && checks.length && !lines.some((l) => l.event_type === "check_aggregate")) {
      lines.push({
        event_type: "check_aggregate",
        at: event.at,
        actor: "watcher",
        line: `+${checks.length} checks — none met the condition.`,
        signed: false,
        ref: null,
      });
    }
    lines.push({
      event_type: event.event_type,
      at: event.at,
      actor: event.actor,
      line: event.detail || labelFor(event),
      signed: Boolean(event.signed) && Boolean(event.ref || event.event_type === "confirmed" || event.event_type === "paused" || event.event_type === "resumed"),
      ref: event.ref,
      origin: event.origin || null,
    });
  }
  if (checks.length && !lines.some((l) => l.event_type === "check_aggregate")) {
    const lastCheck = checks[checks.length - 1];
    lines.splice(
      Math.max(0, lines.findIndex((l) => l.event_type === "confirmed") + 1),
      0,
      {
        event_type: "check_aggregate",
        at: lastCheck.at,
        actor: "watcher",
        line: `+${checks.length} checks — none met the condition.`,
        signed: false,
        ref: null,
      },
    );
  }
  return lines.filter((row) => row.line);
}

function labelFor(event) {
  if (event.event_type === "confirmed") return "Confirmed.";
  if (event.event_type === "paused") return "Paused.";
  if (event.event_type === "resumed") return "Resumed.";
  if (event.event_type === "sent_to_thread") return event.detail || "";
  return event.detail || "";
}

function isActiveStatus(status) {
  return (
    status === "with_aomi" ||
    status === "watching" ||
    status === "triggered" ||
    status === "awaiting_confirm" ||
    status === "pending_execute" ||
    status === "executing" ||
    status === "paused"
  );
}

function stillVisible(item, now) {
  if (item.status === "revoked") return false;
  if (item.archived_at) return false;
  if (isActiveStatus(item.status)) return true;
  const changed = item.status_changed_at || item.updated_at || item.created_at || 0;
  return now - changed < VISIBILITY_SECS;
}

function sortCards(a, b) {
  const rank = (status) => {
    if (status === "awaiting_confirm" || status === "triggered" || status === "with_aomi") return 0;
    if (status === "pending_execute" || status === "executing") return 1;
    if (status === "watching") return 2;
    if (status === "paused") return 3;
    if (status === "done" || status === "cant") return 4;
    return 5;
  };
  const d = rank(a.status) - rank(b.status);
  if (d !== 0) return d;
  return (b.last_event_at || 0) - (a.last_event_at || 0);
}

export function listInstructions(accountId, now = nowSecs()) {
  const data = loadItems(accountId);
  let dirty = false;
  for (const item of data.items || []) {
    if (!item.task_id) {
      ensureTaskId(data, item);
      dirty = true;
    }
  }
  if (dirty) saveItems(accountId, data);
  const cards = (data.items || [])
    .filter((item) => stillVisible(item, now))
    .map((item) => cardOf(item, accountId, now))
    .sort(sortCards);
  return cards;
}

export function getInstruction(accountId, instructionId, now = nowSecs()) {
  const data = loadItems(accountId);
  const item = findItem(data, instructionId);
  if (!item || !stillVisible(item, now)) return null;
  return {
    ...cardOf(item, accountId, now),
    trail: trailOf(accountId, item.instruction_id),
  };
}

const OPEN_STATUSES = new Set(["with_aomi", "triggered", "awaiting_confirm"]);

function compactOpenCard(card) {
  return {
    instruction_id: card.instruction_id,
    task_id: card.task_id || null,
    status: card.status,
    sentence: card.sentence,
    kind: card.kind,
    instrument: card.instrument || null,
    fire_kind: card.fire_kind || "tell",
    pending: card.pending || null,
    trigger_value: card.trigger_value || null,
    correlation_id: card.correlation_id || null,
  };
}

/** Compact cards the agent must act on: drafts, fired confirms, pending pause/resume. */
export function openInstructions(accountId, now = nowSecs()) {
  return listInstructions(accountId, now)
    .filter(
      (row) =>
        OPEN_STATUSES.has(row.status) || row.pending === "pause" || row.pending === "resume",
    )
    .map(compactOpenCard);
}

export function summary(accountId, now = nowSecs()) {
  const cards = listInstructions(accountId, now);
  const holding = cards.filter((row) =>
    ["with_aomi", "watching", "triggered", "awaiting_confirm", "pending_execute", "executing", "paused"].includes(
      row.status,
    ),
  ).length;
  const needsYou = cards.filter((row) =>
    ["triggered", "awaiting_confirm", "with_aomi"].includes(row.status),
  ).length;
  let lastCheck = null;
  for (const card of cards) {
    const at = card.check_stats?.last_check_at;
    if (at && (lastCheck == null || at > lastCheck)) lastCheck = at;
  }
  return {
    holding,
    needs_you: needsYou,
    last_check_at: lastCheck,
  };
}

export function watchCountsByInstrument(accountId, now = nowSecs()) {
  const counts = {};
  for (const card of listInstructions(accountId, now)) {
    if (card.status !== "watching") continue;
    const key = String(card.instrument || card.params?.symbol || "").toUpperCase();
    if (!key) continue;
    counts[key] = (counts[key] || 0) + 1;
  }
  return counts;
}

export function laborStats(accountId, windowSecs = 7 * 86400, now = nowSecs()) {
  const events = loadEvents(accountId).items || [];
  const since = now - windowSecs;
  const checks = events.filter((row) => row.event_type === "check" && row.at >= since);
  const fires = events.filter((row) => row.event_type === "triggered" && row.at >= since);
  const near = events.filter((row) => row.event_type === "near_miss" && row.at >= since);
  const executed = events.filter((row) => row.event_type === "executed" && row.at >= since);
  return {
    holding: summary(accountId, now).holding,
    checks_window: checks.length,
    fired: fires.length,
    near_miss: near.length,
    executed: executed.length,
  };
}

const CANT_SUBLINE = "World doesn't trade this · kept for the record";

export function upsertCant(accountId, body, now = nowSecs()) {
  const entity = String(body.asked_entity || "").trim().toLowerCase();
  if (!entity) return { ok: false, error: "entity_required" };
  const data = loadItems(accountId);
  data.items = data.items || [];
  const existing = data.items.find(
    (row) =>
      row.status === "cant" &&
      String(row.asked_entity || "").toLowerCase() === entity &&
      stillVisible(row, now),
  );
  const origin = body.origin || null;
  const utteranceRef = body.utterance_ref || body.ref || null;
  const heard = String(body.heard || body.sentence || "").trim();
  const wall = String(body.wall || "").trim();
  const sentence = String(body.sentence || heard || "").trim().slice(0, 160);

  if (existing) {
    existing.repeat_count = (existing.repeat_count || 1) + 1;
    existing.sub_line =
      body.sub_line ||
      (existing.repeat_count === 2
        ? "asked twice · kept for the record"
        : existing.repeat_count === 3
          ? "asked three times · kept for the record"
          : `asked ${existing.repeat_count} times · kept for the record`);
    existing.updated_at = now;
    existing.status_changed_at = now;
    saveItems(accountId, data);
    appendEvent(accountId, existing.instruction_id, "heard", heard, now, {
      ref: utteranceRef,
      origin,
      actor: "you",
    });
    if (wall) {
      appendEvent(accountId, existing.instruction_id, "sent_to_thread", wall, now, {
        ref: utteranceRef,
      });
    }
    return { ok: true, repeat: true, instruction: cardOf(existing, accountId, now) };
  }

  const instructionId = body.instruction_id || newId();
  const item = {
    instruction_id: instructionId,
    account_id: Number(accountId) || accountId,
    kind: "cant",
    sentence,
    params: {
      asked: entity,
      answer: "not tradeable on World",
      world_trades: "crypto — spot, perps, lending",
    },
    status: "cant",
    policy_scope: null,
    source_ref: body.correlation_id || utteranceRef || instructionId,
    confirm_ref: null,
    result_ref: null,
    watch_id: null,
    task_id: allocTaskId(data),
    correlation_id: body.correlation_id || instructionId,
    expires_at: null,
    check_stats: { last_check_at: null, checks_7d: 0 },
    pending: null,
    fire_kind: null,
    instrument: null,
    asked_entity: entity,
    cant_kind: body.cant_kind || "no_market",
    repeat_count: 1,
    sub_line: body.sub_line || CANT_SUBLINE,
    created_at: now,
    updated_at: now,
    status_changed_at: now,
  };
  data.items.push(item);
  saveItems(accountId, data);
  appendEvent(accountId, instructionId, "heard", heard, now, {
    ref: utteranceRef,
    origin,
    actor: "you",
  });
  if (wall) {
    appendEvent(accountId, instructionId, "sent_to_thread", wall, now, {
      ref: utteranceRef,
    });
  }
  return { ok: true, repeat: false, instruction: cardOf(item, accountId, now) };
}
