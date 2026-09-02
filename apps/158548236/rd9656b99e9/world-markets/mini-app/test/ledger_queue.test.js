import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const {
  FILL_MS,
  remainingQueueSecs,
  remainingDisplaySecs,
  touchFillUx,
  presentQueue,
  isQueueHot,
  rowStampUnix,
  fmtLocalTime,
  fmtLocalDate,
  fmtLocalTimeWithZone,
  rowStampLabel,
  isDoneToday,
} = require("../static/ledger_queue.js");

const EXECUTE_AT = 1_700_000_003;
const T0 = (EXECUTE_AT - 3) * 1000;

function trade(status, extra = {}) {
  return {
    instruction_id: "t1",
    status,
    kind: "trade",
    fire_kind: "act",
    execute_at: EXECUTE_AT,
    delay_secs: 3,
    slice_n: 1,
    ...extra,
  };
}

test("a staged trade sits in queued for 3 seconds", () => {
  const row = trade("pending_execute");
  const ux = touchFillUx(row, null, T0, {});
  const view = presentQueue(row, ux, T0, {});
  assert.equal(view.zone, "queued");
  assert.equal(view.phase, "wait");
  assert.equal(view.remainingDisplay, 3);
  assert.equal(view.fillPct, 0);
  assert.equal(view.cancellable, true);
  assert.equal(remainingQueueSecs(row, T0 + 2500), 0.5);
  assert.equal(remainingDisplaySecs(row, T0 + 2500), 1);
  const still = presentQueue(row, touchFillUx(row, ux, T0 + 2500, {}), T0 + 2500, {});
  assert.equal(still.phase, "wait");
  assert.equal(still.fillPct, 0);
});

test("after the sit, a 1s bar fills before an instant done row leaves queued", () => {
  const pending = trade("pending_execute");
  let ux = touchFillUx(pending, null, T0, {});
  ux = touchFillUx(pending, ux, T0 + 3000, {});
  assert.ok(ux.fillStartedAt);
  const atStart = presentQueue(pending, ux, ux.fillStartedAt, {});
  assert.equal(atStart.phase, "fill");
  assert.equal(atStart.zone, "queued");
  assert.equal(atStart.fillPct, 0);

  const done = trade("done", { status_changed_at: EXECUTE_AT });
  ux = touchFillUx(done, ux, ux.fillStartedAt + 500, {});
  const mid = presentQueue(done, ux, ux.fillStartedAt + 500, {});
  assert.equal(mid.zone, "queued");
  assert.equal(mid.phase, "fill");
  assert.equal(mid.fillPct, 50);
  assert.equal(mid.cancellable, false);
  assert.equal(isQueueHot(done, ux, ux.fillStartedAt + 500, {}), true);

  ux = touchFillUx(done, ux, ux.fillStartedAt + FILL_MS, {});
  assert.equal(ux.revealed, true);
  const shown = presentQueue(done, ux, ux.fillStartedAt + FILL_MS, {});
  assert.equal(shown.zone, "done");
  assert.equal(shown.phase, "done");
  assert.equal(isQueueHot(done, ux, ux.fillStartedAt + FILL_MS, {}), false);
});

test("a just-completed instant fill still plays the 1s bar", () => {
  const now = EXECUTE_AT * 1000;
  const row = trade("done", { status_changed_at: EXECUTE_AT });
  const ux = touchFillUx(row, null, now, {});
  assert.equal(ux.revealed, false);
  const view = presentQueue(row, ux, now, {});
  assert.equal(view.zone, "queued");
  assert.equal(view.phase, "fill");
});

test("historical done rows skip the queue animation", () => {
  const row = trade("done", { status_changed_at: EXECUTE_AT - 60 });
  const ux = touchFillUx(row, null, EXECUTE_AT * 1000, {});
  assert.equal(ux.revealed, true);
  const view = presentQueue(row, ux, EXECUTE_AT * 1000, {});
  assert.equal(view.zone, "done");
});

test("sliced fills keep real progress until they are actually done", () => {
  const row = trade("executing", { slice_n: 5, order_type: "twap", progress_pct: 40 });
  const ux = touchFillUx(row, null, T0 + 4000, {});
  const view = presentQueue(row, ux, T0 + 4000, {});
  assert.equal(view.zone, "queued");
  assert.equal(view.phase, "sliced");
  assert.equal(view.fillPct, 40);
  const later = touchFillUx(row, ux, T0 + 4000 + FILL_MS, {});
  assert.equal(later.revealed, false);
  const done = trade("done", { slice_n: 5, order_type: "twap", status_changed_at: EXECUTE_AT + 10 });
  const revealed = touchFillUx(done, later, (EXECUTE_AT + 10) * 1000, {});
  assert.equal(revealed.revealed, true);
});

test("a just-recorded with_aomi task is queued immediately", () => {
  const row = {
    instruction_id: "opt",
    status: "with_aomi",
    kind: "voice",
    created_at: EXECUTE_AT - 3,
    delay_secs: 3,
  };
  const ux = touchFillUx(row, null, T0, {});
  const view = presentQueue(row, ux, T0, {});
  assert.equal(view.zone, "queued");
  assert.equal(view.phase, "wait");
  assert.equal(view.remainingDisplay, 3);
});

test("the 3s clock does not restart when the server later stages the same task", () => {
  const heard = {
    instruction_id: "opt",
    status: "with_aomi",
    created_at: EXECUTE_AT - 3,
    delay_secs: 3,
  };
  let ux = touchFillUx(heard, null, T0, {});
  ux = touchFillUx(heard, ux, T0 + 3000, {});
  assert.ok(ux.fillStartedAt);
  const staged = trade("pending_execute", { instruction_id: "server", execute_at: EXECUTE_AT + 60 });
  ux = touchFillUx(staged, ux, T0 + 3100, {});
  const view = presentQueue(staged, ux, T0 + 3100, {});
  assert.equal(view.zone, "queued");
  assert.equal(view.phase, "fill");
});

test("reduced motion keeps the 3s sit and skips the 1s hold", () => {
  const pending = trade("pending_execute");
  const wait = presentQueue(pending, touchFillUx(pending, null, T0, { reduceMotion: true }), T0, {
    reduceMotion: true,
  });
  assert.equal(wait.phase, "wait");
  const done = trade("done", { status_changed_at: EXECUTE_AT });
  const ux = touchFillUx(done, null, EXECUTE_AT * 1000, { reduceMotion: true });
  assert.equal(ux.revealed, true);
  const view = presentQueue(done, ux, EXECUTE_AT * 1000, { reduceMotion: true });
  assert.equal(view.zone, "done");
});

test("ledger stamps use the machine's local timezone", () => {
  const local = new Date(2026, 7, 25, 14, 32, 0);
  const at = Math.floor(local.getTime() / 1000);
  const row = { created_at: at };
  const sameYear = new Date(2026, 7, 28, 12, 0, 0).getTime();
  const nextYear = new Date(2027, 0, 2, 12, 0, 0).getTime();
  assert.equal(rowStampUnix(row), at);
  assert.equal(fmtLocalTime(at), "14:32");
  assert.equal(fmtLocalDate(at, sameYear), "Aug 25");
  assert.equal(fmtLocalDate(at, nextYear), "Aug 25, 2026");
  assert.equal(rowStampLabel(row, false), "14:32");
  assert.equal(rowStampLabel(row, true, sameYear), "Aug 25");
  assert.equal(rowStampLabel({ status_changed_at: at }, true, sameYear), "Aug 25");
  assert.equal(rowStampLabel({}, false), "");
  const zoned = fmtLocalTimeWithZone(at);
  assert.match(zoned, /^14:32( [A-Z][A-Za-z0-9+_:-]+)?$/);
});

test("done today is since local midnight, not a rolling 24 hours", () => {
  const now = new Date(2026, 7, 29, 10, 52, 0);
  const nowMs = now.getTime();
  const thisMorning = Math.floor(new Date(2026, 7, 29, 0, 5, 0).getTime() / 1000);
  const lastNight = Math.floor(new Date(2026, 7, 28, 23, 50, 0).getTime() / 1000);
  const hours23ago = Math.floor((nowMs - 23 * 3600 * 1000) / 1000);
  assert.equal(isDoneToday({ status: "done", status_changed_at: thisMorning }, nowMs), true);
  assert.equal(isDoneToday({ status: "done", status_changed_at: lastNight }, nowMs), false);
  assert.equal(isDoneToday({ status: "done", status_changed_at: hours23ago }, nowMs), false);
  assert.equal(isDoneToday({ status: "cant", status_changed_at: thisMorning }, nowMs), false);
});
