import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "aomi-ledger-"));
process.env.WORLD_BRAIN_DIR = dir;

import {
  composeDraft,
  confirmInstruction,
  getInstruction,
  listInstructions,
  listDueTrades,
  openInstructions,
  pauseInstruction,
  resumeInstruction,
  stageTrade,
  beginExecute,
  claimSlice,
  recordSlice,
  completeExecute,
  summary,
  transition,
  upsertCant,
  archiveInstruction,
} from "../src/instructions.js";
import { cancelTask } from "../src/watches.js";

test("compose creates with_aomi and questions do not", () => {
  const account = "17";
  const recorded = composeDraft(account, {
    kind: "conditional",
    message: "If ETH touches 3400, close half the perp",
    correlation_id: "c-1",
  });
  assert.equal(recorded.ok, true);
  assert.equal(recorded.recorded, true);
  assert.equal(recorded.instruction.status, "with_aomi");
  const q = composeDraft(account, { kind: "question", message: "Walk me through ETH" });
  assert.equal(q.recorded, false);
  const listed = listInstructions(account);
  assert.equal(listed.filter((row) => row.status === "with_aomi").length, 1);
});

test("double compose with the same id does not duplicate", () => {
  const account = "18";
  const first = composeDraft(account, {
    instruction_id: "same-id",
    kind: "watch",
    message: "If funding turns positive, tell me",
    correlation_id: "c-2",
  });
  const second = composeDraft(account, {
    instruction_id: "same-id",
    kind: "watch",
    message: "If funding turns positive, tell me",
    correlation_id: "c-2",
  });
  assert.equal(first.ok, true);
  assert.equal(second.duplicate, true);
  assert.equal(listInstructions(account).length, 1);
});

test("confirm then pause then resume is the only legal path", () => {
  const account = "19";
  const drafted = composeDraft(account, {
    kind: "watch",
    message: "If ETH drops 5% in a day, tell me",
    correlation_id: "c-3",
  });
  const id = drafted.instruction.instruction_id;
  confirmInstruction(account, {
    instruction_id: id,
    watch_id: "w-1",
    confirm_ref: "w-1",
  });
  assert.equal(getInstruction(account, id).status, "watching");
  pauseInstruction(account, id);
  assert.equal(getInstruction(account, id).status, "paused");
  resumeInstruction(account, id);
  assert.equal(getInstruction(account, id).status, "watching");
  assert.throws(() => {
    const item = { status: "done" };
    transition(item, "watching", 1);
  });
});

test("summary holding counts needs-you and heartbeat fields", () => {
  const account = "20";
  composeDraft(account, {
    kind: "conditional",
    message: "Roll the lend at maturity",
    correlation_id: "c-4",
  });
  const got = summary(account);
  assert.equal(got.holding, 1);
  assert.equal(got.needs_you, 1);
  assert.equal(got.last_check_at, null);
});

test("pause without a watching row fails", () => {
  const account = "21";
  const drafted = composeDraft(account, {
    kind: "watch",
    message: "If ETH touches 2000, tell me",
    correlation_id: "c-5",
  });
  assert.throws(() => pauseInstruction(account, drafted.instruction.instruction_id));
});

test("each instruction has a task_id and cancel drops it without a second confirm", () => {
  const account = "23";
  const drafted = composeDraft(account, {
    kind: "watch",
    message: "If ETH drops 5% in a day, tell me",
    correlation_id: "c-6",
  });
  const taskId = drafted.instruction.task_id;
  assert.equal(typeof taskId, "string");
  assert.equal(taskId.length, 6);
  const id = drafted.instruction.instruction_id;
  confirmInstruction(account, {
    instruction_id: id,
    watch_id: "w-cancel",
    confirm_ref: "w-cancel",
  });
  const cancelled = cancelTask(account, taskId);
  assert.equal(cancelled.ok, true);
  assert.equal(cancelled.task_id, taskId);
  assert.equal(cancelled.command, `cancel task ${taskId}`);
  assert.match(cancelled.reply, /cancelled /);
  assert.equal(listInstructions(account).length, 0);
  const missing = cancelTask(account, "no-such-task");
  assert.equal(missing.ok, false);
  assert.equal(missing.error, "not_found");
});

test("stage trade shows the whole sentence and cancel works before execute", () => {
  const account = "24";
  const sentence = "Buy 0.1 ETH spot at market, the whole instruction";
  const t0 = 1_700_000_000;
  const staged = stageTrade(
    account,
    { sentence, instrument: "ETH", params: { side: "buy", quantity: "0.1" } },
    t0,
  );
  assert.equal(staged.ok, true);
  assert.equal(staged.instruction.status, "pending_execute");
  assert.equal(staged.instruction.sentence, sentence);
  assert.equal(staged.instruction.execute_at, t0 + 3);
  assert.equal(staged.instruction.remaining_secs, 3);
  const tooSoon = beginExecute(account, staged.instruction.instruction_id, t0 + 1);
  assert.equal(tooSoon.ok, false);
  assert.equal(tooSoon.error, "too_soon");
  const cancelled = cancelTask(account, staged.instruction.task_id);
  assert.equal(cancelled.ok, true);
  const after = beginExecute(account, staged.instruction.instruction_id, t0 + 5);
  assert.equal(after.ok, false);
  assert.equal(after.error, "cancelled");
});

test("after the delay begin then complete fills the staged trade", () => {
  const account = "25";
  const sentence = "Sell 1 WETH perp";
  const t0 = 1_700_000_100;
  const staged = stageTrade(account, { sentence, instrument: "WETH" }, t0);
  const id = staged.instruction.instruction_id;
  const begun = beginExecute(account, id, t0 + 3);
  assert.equal(begun.ok, true);
  assert.equal(begun.instruction.status, "executing");
  assert.equal(begun.instruction.progress_pct, 8);
  assert.equal(begun.instruction.slice_i, 1);
  const done = completeExecute(
    account,
    id,
    { receipt: "filled · 0xabc", avg_price: "12" },
    t0 + 4,
  );
  assert.equal(done.ok, true);
  assert.equal(done.instruction.status, "done");
  assert.equal(done.instruction.sentence, sentence);
  assert.equal(done.instruction.progress_pct, 100);
  assert.equal(done.instruction.avg_price, "12");
});

test("cant is terminal, visible, and omitted from holding", () => {
  const account = "24-cant";
  const recorded = upsertCant(account, {
    asked_entity: "beef",
    heard: "buy fifty of beef",
    sentence: "Buy fifty of beef.",
    cant_kind: "no_market",
  });
  assert.equal(recorded.ok, true);
  assert.equal(recorded.instruction.status, "cant");
  assert.equal(recorded.instruction.display_status, "can't");
  const got = summary(account);
  assert.equal(got.holding, 0);
  assert.equal(got.needs_you, 0);
  assert.equal(listInstructions(account).filter((row) => row.status === "cant").length, 1);
  assert.throws(() => {
    transition({ status: "cant" }, "watching", 1);
  });
});

test("archive hides done and cant from the ledger", () => {
  const account = "25-archive";
  const cant = upsertCant(account, {
    asked_entity: "soy",
    heard: "buy soy",
    sentence: "Buy soy.",
    cant_kind: "no_market",
  });
  const id = cant.instruction.instruction_id;
  assert.equal(listInstructions(account).some((row) => row.instruction_id === id), true);
  const watching = composeDraft(account, {
    kind: "watch",
    message: "If ETH drops, tell me",
    correlation_id: "c-arch",
  });
  assert.equal(archiveInstruction(account, watching.instruction.instruction_id).ok, false);
  const archived = archiveInstruction(account, id);
  assert.equal(archived.ok, true);
  assert.equal(listInstructions(account).some((row) => row.instruction_id === id), false);
  const again = archiveInstruction(account, id);
  assert.equal(again.ok, true);
  assert.equal(again.already, true);
});

test("openInstructions is compact drafts and pending pause, not watching", () => {
  const account = "26-open";
  assert.deepEqual(openInstructions(account), []);
  const drafted = composeDraft(account, {
    kind: "conditional",
    message: "If ETH touches 3400, close half the perp",
    correlation_id: "c-open",
    instrument: "ETH",
  });
  const id = drafted.instruction.instruction_id;
  const open = openInstructions(account);
  assert.equal(open.length, 1);
  assert.equal(open[0].instruction_id, id);
  assert.equal(open[0].status, "with_aomi");
  assert.equal(open[0].sentence, "If ETH touches 3400, close half the perp");
  assert.equal(open[0].correlation_id, "c-open");
  assert.equal("trail" in open[0], false);
  confirmInstruction(account, {
    instruction_id: id,
    watch_id: "w-open",
    confirm_ref: "w-open",
  });
  assert.deepEqual(openInstructions(account), []);
  composeDraft(account, {
    kind: "pause",
    instruction_id: id,
    message: "pause it",
    correlation_id: "c-pause",
  });
  const paused = openInstructions(account);
  assert.equal(paused.length, 1);
  assert.equal(paused[0].instruction_id, id);
  assert.equal(paused[0].pending, "pause");
});

test("TWAP stage keeps slice_n and recordSlice stays executing until the last fill", () => {
  const account = "27-twap";
  const t0 = 1_700_000_200;
  const staged = stageTrade(
    account,
    {
      sentence: "Buy 1000 ETH",
      instrument: "ETH",
      params: {
        order_type: "twap",
        quantity: "1000",
        schedule: { slices: 3, interval_secs: 60, quantity_per_slice: "333.333333", filled_quantity: "0" },
      },
    },
    t0,
  );
  assert.equal(staged.instruction.slice_n, 3);
  assert.equal(staged.instruction.order_type, "twap");
  const id = staged.instruction.instruction_id;
  const claimed = claimSlice(account, id, t0 + 3);
  assert.equal(claimed.ok, true);
  assert.equal(claimed.first, true);
  assert.equal(claimed.slice_i, 1);
  assert.equal(claimed.last, false);
  const mid = recordSlice(
    account,
    id,
    {
      slice_i: 1,
      avg_price: "2400",
      filled_quantity: "333.333333",
      fill: { hash: "0x1", quantity: "333.333333", price: "2400" },
      receipt: "slice 1 of 3",
    },
    t0 + 4,
  );
  assert.equal(mid.ok, true);
  assert.equal(mid.more, true);
  assert.equal(mid.instruction.status, "executing");
  assert.equal(mid.instruction.slice_i, 1);
  assert.equal(mid.instruction.child_fills.length, 1);
  assert.equal(mid.next_slice_at, t0 + 4 + 60);
  const tooSoon = claimSlice(account, id, t0 + 10);
  assert.equal(tooSoon.ok, false);
  assert.equal(tooSoon.error, "too_soon");
  const second = claimSlice(account, id, t0 + 4 + 60);
  assert.equal(second.ok, true);
  assert.equal(second.slice_i, 2);
  recordSlice(
    account,
    id,
    {
      slice_i: 2,
      avg_price: "2410",
      filled_quantity: "666.666666",
      fill: { hash: "0x2", quantity: "333.333333", price: "2420" },
    },
    t0 + 64,
  );
  const lastClaim = claimSlice(account, id, t0 + 64 + 60);
  assert.equal(lastClaim.last, true);
  const last = recordSlice(
    account,
    id,
    {
      slice_i: 3,
      avg_price: "2415",
      filled_quantity: "1000",
      fill: { hash: "0x3", quantity: "333.333334", price: "2425" },
    },
    t0 + 124,
  );
  assert.equal(last.more, false);
  const done = completeExecute(
    account,
    id,
    { receipt: "filled · 0x3", avg_price: "2415" },
    t0 + 125,
  );
  assert.equal(done.instruction.status, "done");
  assert.equal(done.instruction.progress_pct, 100);
});

test("cancel drops an executing TWAP so remaining slices do not fire", () => {
  const account = "28-twap-cancel";
  const t0 = 1_700_000_300;
  const staged = stageTrade(
    account,
    {
      sentence: "Buy 10 ETH over time",
      params: { order_type: "twap", schedule: { slices: 4, interval_secs: 30 } },
    },
    t0,
  );
  const id = staged.instruction.instruction_id;
  assert.equal(beginExecute(account, id, t0 + 3).ok, true);
  const cancelled = cancelTask(account, staged.instruction.task_id);
  assert.equal(cancelled.ok, true);
  const next = claimSlice(account, id, t0 + 40);
  assert.equal(next.ok, false);
  assert.equal(next.error, "cancelled");
});

test("listDueTrades returns pending after the delay and executing when the next slice is due", () => {
  const account = "29-due";
  const t0 = 1_700_000_400;
  const staged = stageTrade(
    account,
    {
      sentence: "DCA 7 ETH",
      params: { order_type: "dca", schedule: { slices: 2, interval_secs: 10, cadence: "daily" } },
    },
    t0,
  );
  const id = staged.instruction.instruction_id;
  assert.equal(listDueTrades(account, t0 + 1).length, 0);
  assert.equal(listDueTrades(account, t0 + 3).some((row) => row.instruction_id === id), true);
  claimSlice(account, id, t0 + 3);
  recordSlice(
    account,
    id,
    { slice_i: 1, fill: { hash: "0xa", quantity: "3.5", price: "1" }, filled_quantity: "3.5" },
    t0 + 3,
  );
  assert.equal(listDueTrades(account, t0 + 4).length, 0);
  assert.equal(listDueTrades(account, t0 + 3 + 10).some((row) => row.instruction_id === id), true);
});
