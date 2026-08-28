import { readdirSync } from "node:fs";
import { filePath, readJson, writeJson } from "./store.js";
import {
  latestAccount,
  latestFunding,
  latestMark,
  loadHistory,
  markAtOrBefore,
} from "./history.js";
import { resolvePredicate } from "./resolve.js";
import { admit, dueForDailyFlush, emptyLimiter, flushHeld } from "./rateLimit.js";
import { enqueue } from "./outbound.js";
import {
  appendEvent,
  attachWatch,
  cancelInstruction,
  onWatchExpired,
  onWatchFired,
  recordCheck,
  pauseInstruction,
  resumeInstruction,
} from "./instructions.js";
import {
  attachBundleCopy,
  attachExpireCopy,
  attachFireCopy,
  attachSetCopy,
} from "./copy.js";

export const DEFAULT_TTL_SECS = 30 * 24 * 60 * 60;

function watchesPath(accountId) {
  return filePath("watches", `${accountId}.json`);
}

function limiterPath(accountId) {
  return filePath("limiter", `${accountId}.json`);
}

function loadWatches(accountId) {
  return readJson(watchesPath(accountId), { items: [] });
}

function saveWatches(accountId, data) {
  writeJson(watchesPath(accountId), data);
}

function loadLimiter(accountId) {
  return readJson(limiterPath(accountId), emptyLimiter());
}

function saveLimiter(accountId, state) {
  writeJson(limiterPath(accountId), state);
}

export function watchedAccounts() {
  try {
    return readdirSync(filePath("watches"))
      .filter((name) => name.endsWith(".json"))
      .map((name) => name.replace(/\.json$/, ""));
  } catch {
    return [];
  }
}

export function listWatches(accountId) {
  return (loadWatches(accountId).items || [])
    .filter((item) => item.status !== "superseded")
    .map((item) => ({
      ...item,
      on_chain: false,
    }));
}

export function allActiveWatches() {
  const out = [];
  for (const accountId of watchedAccounts()) {
    for (const watch of listWatches(accountId)) {
      if (watch.status === "active") {
        out.push({ ...watch, account_id: Number(accountId) || accountId });
      }
    }
  }
  return out;
}

export function setWatch(accountId, body) {
  const now = Math.floor(Date.now() / 1000);
  let predicate = body.predicate;
  if (!predicate) {
    const resolved = resolvePredicate({
      phrase: body.phrase,
      symbol: body.symbol,
      token_id: body.token_id,
    });
    if (resolved.execution_folded) {
      return attachSetCopy({
        ok: true,
        stored: false,
        execution_folded: true,
        symbol: body.symbol,
      });
    }
    if (resolved.needs_clarification) {
      return attachSetCopy({
        ok: true,
        stored: false,
        needs_clarification: true,
        options: resolved.options,
        symbol: body.symbol,
      });
    }
    if (!resolved.ok) {
      return { ok: false, stored: false, error: resolved.error || "unresolved_predicate" };
    }
    predicate = resolved.predicate;
  }
  const fireMode = body.fire_mode === "repeats" ? "repeats" : "once";
  const fireOnTransition = Boolean(body.fire_on_transition);
  const watch = {
    id:
      body.id ||
      `w-${accountId}-${now}-${Math.random().toString(16).slice(2, 6)}`,
    account_id: Number(accountId),
    original_phrase: String(body.phrase || predicate.resolved || ""),
    predicate,
    fire_mode: fireMode,
    fire_on_transition: fireOnTransition,
    created_at: now,
    expires_at: Number(body.expires_at) || now + DEFAULT_TTL_SECS,
    status: "active",
    last_fired_at: null,
    predicate_was_false: true,
    mark_at_set: body.mark_at_set ?? null,
    instruction_id: body.instruction_id || null,
    correlation_id: body.correlation_id || null,
  };
  const atSet = evaluatePredicate(watch, now);
  const liveNow = body.mark_at_set ?? atSet.live ?? null;
  const alreadyTrue = Boolean(atSet.ready && atSet.true);
  if (alreadyTrue && !fireOnTransition) {
    return attachSetCopy({
      ok: true,
      stored: false,
      already_true: true,
      now: liveNow,
      symbol: body.symbol,
      predicate,
      watch: { ...watch, status: "not_armed" },
    });
  }
  watch.predicate_was_false = fireOnTransition ? false : !alreadyTrue;
  const data = loadWatches(accountId);
  data.items = data.items || [];
  data.items.push(watch);
  saveWatches(accountId, data);
  attachWatch(accountId, watch, now);
  return attachSetCopy({
    ok: true,
    stored: true,
    already_true: false,
    now: liveNow,
    watch,
  });
}

export function cancelWatch(accountId, id) {
  const data = loadWatches(accountId);
  const removed = (data.items || []).find((row) => row.id === id);
  if (!removed) return { ok: false, error: "not_found" };
  data.items = data.items.filter((row) => row.id !== id);
  saveWatches(accountId, data);
  return { ok: true, item: removed, remaining: data.items.length };
}

export function matchWatches(accountId, symbol) {
  const key = String(symbol || "").trim().toUpperCase();
  const live = (loadWatches(accountId).items || []).filter((row) => {
    if (row.status && row.status !== "active") return false;
    const sym = String(row.predicate?.symbol || row.symbol || "").toUpperCase();
    return !key || sym === key || sym.includes(key) || key.includes(sym);
  });
  if (live.length > 1) {
    return {
      ok: true,
      ambiguous: true,
      message: `Which ${key || "watch"}? ${live
        .map((w) => `\`${w.original_phrase || w.id}\``)
        .join(" · ")}`,
      reply_verbatim: true,
      candidates: live.map((w) => ({
        id: w.id,
        phrase: w.original_phrase,
        symbol: w.predicate?.symbol || w.symbol,
      })),
    };
  }
  return { ok: true, ambiguous: false, matches: live };
}

export function supersedeWatch(accountId, body) {
  const now = Math.floor(Date.now() / 1000);
  const matches = matchWatches(accountId, body.symbol || body.referent);
  if (matches.ambiguous) return matches;
  const old = (matches.matches || [])[0];
  if (old) {
    const data = loadWatches(accountId);
    const item = (data.items || []).find((row) => row.id === old.id);
    if (item) {
      item.status = "superseded";
      item.superseded_at = now;
      saveWatches(accountId, data);
      if (item.instruction_id) {
        appendEvent(
          accountId,
          item.instruction_id,
          "superseded",
          body.phrase || "updated",
          now,
          { actor: "you" },
        );
      }
    }
  }
  const created = setWatch(accountId, {
    ...body,
    phrase: body.phrase,
    symbol: body.symbol,
  });
  created.superseded_id = old?.id || null;
  created.message =
    created.message ||
    `Updated — now ${body.phrase || body.symbol}. The previous version is in this task's history.`;
  created.reply_verbatim = true;
  return created;
}

function findWatch(accountId, id) {
  const data = loadWatches(accountId);
  return (data.items || []).find((row) => row.id === id);
}

function setWatchStatus(accountId, id, status) {
  const data = loadWatches(accountId);
  const watch = (data.items || []).find((row) => row.id === id);
  if (!watch) return { ok: false, error: "not_found" };
  watch.status = status;
  saveWatches(accountId, data);
  return { ok: true, watch };
}

export function pauseWatch(accountId, id, instructionId) {
  const now = Math.floor(Date.now() / 1000);
  if (instructionId) {
    const result = pauseInstruction(accountId, instructionId, now);
    if (!result.ok) return result;
    if (result.watch_id) setWatchStatus(accountId, result.watch_id, "paused");
    return result;
  }
  if (!id) return { ok: false, error: "not_found" };
  return setWatchStatus(accountId, id, "paused");
}

export function resumeWatch(accountId, id, instructionId) {
  const now = Math.floor(Date.now() / 1000);
  if (instructionId) {
    const result = resumeInstruction(accountId, instructionId, now);
    if (!result.ok) return result;
    if (result.watch_id) setWatchStatus(accountId, result.watch_id, "active");
    return result;
  }
  if (!id) return { ok: false, error: "not_found" };
  return setWatchStatus(accountId, id, "active");
}

export function cancelTask(accountId, id) {
  const result = cancelInstruction(accountId, id);
  if (!result.ok) return result;
  if (result.watch_id) {
    const dropped = cancelWatch(accountId, result.watch_id);
    if (!dropped.ok && dropped.error !== "not_found") return dropped;
  }
  const command = result.command || `cancel task ${result.task_id}`;
  const reply = result.reply || `cancelled ${result.task_id}`;
  enqueue({
    account_id: Number(accountId) || accountId,
    kind: "user_command",
    message: command,
    instruction_id: result.instruction?.instruction_id || null,
  });
  enqueue({
    account_id: Number(accountId) || accountId,
    kind: "notice",
    message: reply,
    instruction_id: result.instruction?.instruction_id || null,
  });
  return { ...result, command, reply, thread: { command, reply } };
}

export { findWatch };

function cmp(op, left, right) {
  if (op === "gt") return left > right;
  if (op === "gte") return left >= right;
  if (op === "lt") return left < right;
  if (op === "lte") return left <= right;
  return left >= right;
}

function num(value) {
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

function idleStart(accountId) {
  const series = loadHistory().accounts[String(accountId)] || [];
  let start = null;
  for (const point of series) {
    const idle = num(point.idle_quote);
    if (idle != null && idle > 0) {
      if (start == null) start = point.ts;
    } else {
      start = null;
    }
  }
  return start;
}

function evaluatePredicate(watch, now) {
  const p = watch.predicate || {};
  switch (p.kind) {
    case "price_level": {
      const mark = latestMark(p.symbol);
      const live = mark ? num(mark.mark) : num(watch.mark_at_set);
      const level = num(p.level);
      if (live == null || level == null) return { ready: false };
      return {
        ready: true,
        true: cmp(p.op, live, level),
        live: mark ? mark.mark : watch.mark_at_set,
        live_ts: mark ? mark.ts : null,
      };
    }
    case "pct_move": {
      const window = p.window === "1w" ? 7 * 86400 : 86400;
      const latest = latestMark(p.symbol);
      const then = markAtOrBefore(p.symbol, now - window);
      if (!latest || !then) return { ready: false };
      const nowVal = num(latest.mark);
      const thenVal = num(then.mark);
      if (nowVal == null || thenVal == null || thenVal === 0) {
        return { ready: false };
      }
      const pct = ((nowVal - thenVal) / thenVal) * 100;
      const threshold = num(p.pct);
      if (threshold == null) return { ready: false };
      const hit =
        p.op === "lte"
          ? pct <= -Math.abs(threshold)
          : pct >= Math.abs(threshold);
      return {
        ready: true,
        true: hit,
        live: latest.mark,
        live_ts: latest.ts,
        move_pct: pct.toFixed(2),
      };
    }
    case "funding": {
      const sample = latestFunding(p.symbol);
      const live = sample ? num(sample.rate) : null;
      const level = num(p.rate_pct);
      if (live == null || level == null) return { ready: false };
      return {
        ready: true,
        true: cmp(p.op || "gt", live, level),
        live: sample.rate,
        live_ts: sample.ts,
      };
    }
    case "risk": {
      const sample = latestAccount(watch.account_id);
      const live = sample ? num(sample.liquidation_risk) : null;
      const level = num(p.level);
      if (live == null || level == null) return { ready: false };
      return {
        ready: true,
        true: cmp(p.op || "gte", live, level),
        live: sample.liquidation_risk,
        live_ts: sample.ts,
      };
    }
    case "rapv": {
      const sample = latestAccount(watch.account_id);
      const live = sample ? num(sample.rapv) : null;
      const level = num(p.level);
      if (live == null || level == null) return { ready: false };
      return {
        ready: true,
        true: cmp(p.op || "gte", live, level),
        live: sample.rapv,
        live_ts: sample.ts,
      };
    }
    case "idle": {
      const sample = latestAccount(watch.account_id);
      if (!sample || sample.idle_quote == null) return { ready: false };
      const idle = num(sample.idle_quote);
      if (idle == null || idle <= 0) {
        return {
          ready: true,
          true: false,
          live: sample.idle_quote,
          live_ts: sample.ts,
        };
      }
      const start = idleStart(watch.account_id);
      const days = start == null ? 0 : (now - start) / 86400;
      return {
        ready: true,
        true: days > Number(p.days),
        live: sample.idle_quote,
        live_ts: sample.ts,
      };
    }
    case "loan_renewal": {
      const sample = latestAccount(watch.account_id);
      if (!sample) return { ready: false };
      const nowFp = JSON.stringify(sample.loan_fingerprints || []);
      const prev = watch.last_loan_fingerprints;
      watch.last_loan_fingerprints = nowFp;
      return {
        ready: Boolean(prev),
        true: Boolean(prev) && prev !== nowFp,
        live: nowFp,
        live_ts: sample.ts,
      };
    }
    default:
      return { ready: false };
  }
}

function firePayload(watch, result, now) {
  return {
    kind: "watch_fired",
    account_id: watch.account_id,
    watch_id: watch.id,
    original_phrase: watch.original_phrase,
    created_at: watch.created_at,
    predicate: watch.predicate,
    fire_mode: watch.fire_mode,
    live: result.live,
    live_ts: result.live_ts,
    move_pct: result.move_pct || null,
    spent: watch.fire_mode === "once",
    fired_at: now,
  };
}

function expirePayload(watch, now) {
  return {
    kind: "watch_expired",
    account_id: watch.account_id,
    watch_id: watch.id,
    original_phrase: watch.original_phrase,
    created_at: watch.created_at,
    predicate: watch.predicate,
    expired_at: now,
  };
}

export function evaluateAccount(
  accountId,
  now = Math.floor(Date.now() / 1000),
) {
  const data = loadWatches(accountId);
  let changed = false;
  const limiter = loadLimiter(accountId);
  for (const watch of data.items || []) {
    if (watch.status === "active" && watch.expires_at && now >= watch.expires_at) {
      watch.status = "expired";
      changed = true;
      enqueue({
        channel: "watch_expired",
        ...attachExpireCopy(expirePayload(watch, now)),
      });
      onWatchExpired(accountId, watch, now);
      continue;
    }
    if (watch.status !== "active") continue;
    const result = evaluatePredicate(
      { ...watch, account_id: watch.account_id || accountId },
      now,
    );
    if (!result.ready) continue;
    recordCheck(accountId, watch, result, now);
    if (!result.true) {
      watch.predicate_was_false = true;
      changed = true;
      continue;
    }
    const mayFire = watch.fire_on_transition
      ? Boolean(watch.predicate_was_false)
      : watch.fire_mode === "once"
        ? true
        : Boolean(watch.predicate_was_false);
    if (!mayFire) continue;
    watch.predicate_was_false = false;
    watch.last_fired_at = now;
    if (watch.fire_mode === "once") watch.status = "spent";
    changed = true;
    const fire = firePayload(watch, result, now);
    onWatchFired(accountId, watch, result, now);
    const decision = admit(limiter, fire, now);
    if (decision.action === "deliver") {
      enqueue({ channel: "watch", ...attachFireCopy(fire) });
      const bundled = flushHeld(limiter, now);
      if (bundled?.length) {
        enqueue(
          attachBundleCopy({
            channel: "watch_bundle",
            kind: "watch_bundle",
            account_id: Number(accountId),
            fires: bundled,
          }),
        );
      }
    }
  }
  if (dueForDailyFlush(limiter, now)) {
    const bundle = flushHeld(limiter, now);
    if (bundle?.length) {
      enqueue(
        attachBundleCopy({
          channel: "watch_bundle",
          kind: "watch_bundle",
          account_id: Number(accountId),
          fires: bundle,
        }),
      );
    }
  }
  saveLimiter(accountId, limiter);
  if (changed) saveWatches(accountId, data);
}

export function evaluateAll(now = Math.floor(Date.now() / 1000)) {
  for (const accountId of watchedAccounts()) {
    evaluateAccount(accountId, now);
  }
}
