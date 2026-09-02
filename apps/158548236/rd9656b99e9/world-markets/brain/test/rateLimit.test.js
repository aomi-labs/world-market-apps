import assert from "node:assert/strict";
import test from "node:test";
import { admit, emptyLimiter, flushHeld } from "../src/rateLimit.js";

test("first N fires deliver; N+1 is held and appears in the bundle", () => {
  process.env.WORLD_WATCH_FIRE_CEILING = "2";
  const state = emptyLimiter();
  const now = 1_700_000_000;
  const a = admit(state, { id: "a" }, now);
  const b = admit(state, { id: "b" }, now + 1);
  const c = admit(state, { id: "c" }, now + 2);
  const d = admit(state, { id: "d" }, now + 3);
  assert.equal(a.action, "deliver");
  assert.equal(b.action, "deliver");
  assert.equal(c.action, "hold");
  assert.equal(d.action, "hold");
  const bundle = flushHeld(state, now + 4);
  assert.equal(bundle.length, 2);
  assert.deepEqual(
    bundle.map((item) => item.id),
    ["c", "d"],
  );
  assert.equal(state.held.length, 0);
});
