/** Ledger queue UX: 3s cancel sit, then a 1s fill bar, then Done. */

const QUEUE_DELAY_SECS = 3;
const FILL_MS = 1000;
const RECENT_DONE_MS = 2000;

function isSlicedQueueRow(row) {
  if (!row) return false;
  const n = Number(row.slice_n) || 1;
  const type = String(row.order_type || (row.params && row.params.order_type) || "").toLowerCase();
  return n > 1 || type === "twap" || type === "dca";
}

function isSittingQueueStatus(status) {
  return status === "pending_execute" || status === "with_aomi";
}

function isQueueRow(row) {
  if (!row) return false;
  if (row.status === "pending_execute" || row.status === "executing" || row.status === "with_aomi") {
    return true;
  }
  if (row.status !== "done") return false;
  return (
    row.kind === "trade" ||
    row.fire_kind === "act" ||
    row.execute_at != null ||
    row.delay_secs != null
  );
}

function remainingQueueSecs(row, nowMs) {
  if (!row || !isSittingQueueStatus(row.status)) return null;
  const delay = Number(row.delay_secs) || QUEUE_DELAY_SECS;
  const now = Number(nowMs);
  const clock = Number.isFinite(now) ? now : Date.now();
  if (row.wait_until_ms != null) {
    const rem = (Number(row.wait_until_ms) - clock) / 1000;
    if (!Number.isFinite(rem)) return null;
    return Math.max(0, Math.min(delay, rem));
  }
  if (row.execute_at) {
    const rem = (Number(row.execute_at) * 1000 - clock) / 1000;
    if (!Number.isFinite(rem)) return null;
    return Math.max(0, Math.min(delay, rem));
  }
  if (row.remaining_secs != null) return Math.max(0, Number(row.remaining_secs));
  return null;
}

function remainingDisplaySecs(row, nowMs) {
  const rem = remainingQueueSecs(row, nowMs);
  if (rem == null) return null;
  if (rem <= 0) return 0;
  return Math.max(1, Math.ceil(rem));
}

function doneAgeMs(row, nowMs) {
  if (!row || row.status !== "done") return Infinity;
  const changed = Number(row.status_changed_at || row.updated_at || 0);
  if (!changed) return Infinity;
  return Math.max(0, nowMs - changed * 1000);
}

function touchFillUx(row, ux, nowMs, opts) {
  const next = {
    fillStartedAt: ux && ux.fillStartedAt != null ? ux.fillStartedAt : null,
    revealed: Boolean(ux && ux.revealed),
    toasted: Boolean(ux && ux.toasted),
    waitUntil: ux && ux.waitUntil != null ? ux.waitUntil : null,
  };
  if (!isQueueRow(row)) {
    next.revealed = true;
    return next;
  }
  if (isSittingQueueStatus(row.status) && next.waitUntil == null) {
    const cap = nowMs + QUEUE_DELAY_SECS * 1000;
    let until = cap;
    if (row.execute_at) until = Math.min(until, Number(row.execute_at) * 1000);
    if (row.created_at) until = Math.min(until, Number(row.created_at) * 1000 + QUEUE_DELAY_SECS * 1000);
    next.waitUntil = until;
  }
  if (next.revealed) return next;
  const presented = next.waitUntil != null ? { ...row, wait_until_ms: next.waitUntil } : row;
  const rem = remainingQueueSecs(presented, nowMs);
  const waiting = isSittingQueueStatus(row.status) && rem != null && rem > 0;
  if (waiting) return next;
  if (row.status === "done" && next.fillStartedAt == null) {
    if (isSlicedQueueRow(row) || doneAgeMs(row, nowMs) > RECENT_DONE_MS) {
      next.revealed = true;
      return next;
    }
  }
  if (opts && opts.reduceMotion && row.status === "done") {
    next.revealed = true;
    return next;
  }
  if (next.fillStartedAt == null) next.fillStartedAt = nowMs;
  if (row.status === "done" && isSlicedQueueRow(row)) {
    next.revealed = true;
    return next;
  }
  if (row.status === "done" && nowMs - next.fillStartedAt >= FILL_MS) {
    next.revealed = true;
  }
  return next;
}

function presentQueue(row, ux, nowMs, opts) {
  if (!isQueueRow(row)) return null;
  const reduce = Boolean(opts && opts.reduceMotion);
  const presented = ux && ux.waitUntil != null ? { ...row, wait_until_ms: ux.waitUntil } : row;
  const rem = remainingQueueSecs(presented, nowMs);
  const waiting = isSittingQueueStatus(row.status) && rem != null && rem > 0;
  if (waiting) {
    return {
      zone: "queued",
      phase: "wait",
      remainingDisplay: remainingDisplaySecs(presented, nowMs),
      fillPct: 0,
      showMeter: true,
      showCountdown: true,
      cancellable: true,
    };
  }
  if (ux && ux.revealed && row.status === "done") {
    return {
      zone: "done",
      phase: "done",
      remainingDisplay: null,
      fillPct: 100,
      showMeter: false,
      showCountdown: false,
      cancellable: false,
    };
  }
  if (isSlicedQueueRow(row) && row.status !== "pending_execute") {
    const pct = row.progress_pct != null ? Number(row.progress_pct) : null;
    if (row.status === "done") {
      return {
        zone: "done",
        phase: "done",
        remainingDisplay: null,
        fillPct: 100,
        showMeter: false,
        showCountdown: false,
        cancellable: false,
      };
    }
    return {
      zone: "queued",
      phase: "sliced",
      remainingDisplay: null,
      fillPct: pct,
      showMeter: pct != null,
      showCountdown: false,
      cancellable: true,
    };
  }
  if (reduce) {
    if (row.status === "done") {
      return {
        zone: "done",
        phase: "done",
        remainingDisplay: null,
        fillPct: 100,
        showMeter: false,
        showCountdown: false,
        cancellable: false,
      };
    }
    return {
      zone: "queued",
      phase: "fill",
      remainingDisplay: null,
      fillPct: row.progress_pct != null ? Number(row.progress_pct) : 100,
      showMeter: true,
      showCountdown: false,
      cancellable: row.status !== "done",
    };
  }
  const started = ux && ux.fillStartedAt != null ? ux.fillStartedAt : nowMs;
  const elapsed = Math.max(0, nowMs - started);
  const fillPct = Math.max(0, Math.min(100, Math.round((elapsed / FILL_MS) * 100)));
  if (row.status === "done" && elapsed >= FILL_MS) {
    return {
      zone: "done",
      phase: "done",
      remainingDisplay: null,
      fillPct: 100,
      showMeter: false,
      showCountdown: false,
      cancellable: false,
    };
  }
  return {
    zone: "queued",
    phase: "fill",
    remainingDisplay: null,
    fillPct,
    showMeter: true,
    showCountdown: false,
    cancellable: row.status !== "done",
  };
}

function isQueueHot(row, ux, nowMs, opts) {
  const view = presentQueue(row, ux, nowMs, opts);
  return Boolean(view && view.zone === "queued");
}

if (typeof window !== "undefined") {
  window.QUEUE_DELAY_SECS = QUEUE_DELAY_SECS;
  window.FILL_MS = FILL_MS;
  window.isSlicedQueueRow = isSlicedQueueRow;
  window.isQueueRow = isQueueRow;
  window.isSittingQueueStatus = isSittingQueueStatus;
  window.remainingQueueSecs = remainingQueueSecs;
  window.remainingDisplaySecs = remainingDisplaySecs;
  window.touchFillUx = touchFillUx;
  window.presentQueue = presentQueue;
  window.isQueueHot = isQueueHot;
}
if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    QUEUE_DELAY_SECS,
    FILL_MS,
    RECENT_DONE_MS,
    isSlicedQueueRow,
    isQueueRow,
    isSittingQueueStatus,
    remainingQueueSecs,
    remainingDisplaySecs,
    touchFillUx,
    presentQueue,
    isQueueHot,
  };
}
