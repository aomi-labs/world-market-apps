import assert from "node:assert/strict";
import test from "node:test";
import { portfolioImpact } from "../src/impact.js";

test("live before is passed through; after stays unavailable without an SDK adapter", async () => {
  delete process.env.WORLD_IMPACT_SDK_MODULE;
  const got = await portfolioImpact({
    before: { liquidation_risk: "3.2", rapv: "10000" },
  });
  assert.equal(got.status, "ok");
  assert.equal(got.before.liquidation_risk, "3.2");
  assert.equal(got.after, null);
  assert.equal(got.after_status, "unavailable");
});

test("no before and no adapter stays unavailable", async () => {
  delete process.env.WORLD_IMPACT_SDK_MODULE;
  const got = await portfolioImpact({});
  assert.equal(got.status, "unavailable");
  assert.equal(got.reason, "sdk_not_wired");
});
