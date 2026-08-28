/**
 * Counterfactual risk / dollarpower for a stated market move.
 *
 * Optional adapter: set WORLD_IMPACT_SDK_MODULE to a file that default-exports
 * `async function impact(body)` calling @composite/sdk with before/after
 * snapshots. Until that exists, we pass through live `before` figures from
 * get_world_account and leave `after` absent (never invent a post-move score).
 */

export async function portfolioImpact(body = {}) {
  const adapterPath = process.env.WORLD_IMPACT_SDK_MODULE;
  if (adapterPath) {
    try {
      const mod = await import(adapterPath);
      const fn = mod.default || mod.portfolioImpact;
      if (typeof fn === "function") return fn(body);
    } catch (error) {
      return {
        status: "unavailable",
        reason: "sdk_adapter_failed",
        detail: error?.message || String(error),
      };
    }
  }
  if (body.before && typeof body.before === "object") {
    return {
      status: "ok",
      before: body.before,
      after: null,
      after_status: "unavailable",
      note: "after requires WORLD_IMPACT_SDK_MODULE (@composite/sdk). Live before is from get_world_account.",
    };
  }
  return {
    status: "unavailable",
    reason: "sdk_not_wired",
    note: "Pass live before from get_world_account, or set WORLD_IMPACT_SDK_MODULE.",
  };
}
