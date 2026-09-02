import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const { COPY } = require("../static/copy.js");

test("answer copy register has no bangs and 160-char blocks", () => {
  const keys = [
    "heard_with_ref",
    "working",
    "footer_thread",
    "handoff_command",
    "handoff_escalation",
    "clarify",
  ];
  for (const key of keys) {
    const value = COPY.answer[key];
    assert.equal(typeof value, "string", key);
    assert.equal(value.includes("!"), false, value);
    assert.ok([...value].length <= 160, value);
  }
});
