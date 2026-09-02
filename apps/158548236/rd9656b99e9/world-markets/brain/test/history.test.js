import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "aomi-history-"));
process.env.WORLD_BRAIN_DIR = dir;

const { markSeries, recordMark } = await import("../src/history.js");

test("markSeries returns recorded marks for a symbol", () => {
  recordMark({ symbol: "WETH", mark: "3800", ts: 1_700_000_000 });
  const series = markSeries("weth");
  assert.ok(series.length >= 1);
  assert.equal(series[series.length - 1].mark, "3800");
});
