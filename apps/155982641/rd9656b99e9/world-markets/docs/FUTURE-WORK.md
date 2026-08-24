# Future work & observations

Living notes for architectural decisions that are good enough for now but should be revisited.

---

## Liquidation risk (`liquidationRisk`)

### Current state

`liquidation_risk` is computed in Rust (`src/liquidation_risk.rs`) and returned from `get_world_account` under `metrics`. The algorithm is a port of `Portfolio.calculateLiquidationRisk` from the Composite frontend (`@composite/sdk` in the `frontend` monorepo).

### Observations

- **Two implementations.** The canonical logic lives in TypeScript (`frontend/web/packages/sdk/src/contracts/portfolio.ts` and `@wcm/tools`). The Rust port can drift when the SDK changes risk bounds, funding history, token decoding, or the health-score mapping.
- **Why Rust for now.** The Aomi plugin is a Rust `cdylib` with no Node runtime. A local port was the fastest path to ship the 0–10 score the Telegram agent and Composite UI both need, without a new deployable or subprocess dependency.
- **RPC overlap is acceptable today.** `WorldClient` already reads chain state for account tools; the metrics module reuses that data and adds `block_timestamp` + `readFundingRateHistory` calls. A future service could own all reads or accept a snapshot from Rust—TBD when we build (A).
- **Skill contract is stable.** Action rules and §6.13 health templates point at `get_world_account` → `metrics`. Changing the backend should not require copy changes if field names and semantics stay the same.

### Future intention: Option A — metrics sidecar (canonical SDK)

**Target:** Replace the Rust port with a small HTTP service (sidecar or shared World service) that depends on `@composite/sdk` and exposes portfolio metrics, including `liquidationRisk`.

Rough shape:

```
POST /portfolio-metrics
  → { nav, prv, liquidationRisk, liquidation_risk_band, … }
```

The plugin would call this service from `get_world_account` (or a dedicated tool) instead of `liquidation_risk::compute_metrics`.

**Why this over other options:**

| Option | Verdict |
|--------|---------|
| **A — Sidecar / microservice** | **Chosen target.** Single source of truth; SDK upgrades = redeploy service. |
| B — Node CLI subprocess | Acceptable for local dev only; awkward in production (spawn latency, `node_modules` on host). |
| C — Rust port + CI parity tests | **Current interim.** Reduces surprise drift but does not eliminate duplication. |
| D — Hosted World reporting API | Use if/when World exposes the same fields officially—could subsume the sidecar. |
| E — Shared WASM/native lib | High cost; SDK is Node/ethers-centric today. |

**Open questions when implementing (A):**

1. Who fetches chain state—the service (duplicate RPC) or Rust (pass a portfolio snapshot)?
2. Where does the sidecar live—`frontend` repo package, separate deploy, or World infrastructure?
3. Versioning: pin `@composite/sdk` in the service; how does the plugin handle version skew?
4. Failure mode: fallback to Rust, fail closed, or omit `metrics` if the service is down?

**When to revisit:** Before production Telegram traffic at scale, or on the first `@composite/sdk` change that touches `calculateLiquidationRisk`, `evaluate`, or related `@wcm/tools` helpers—whichever comes first.

### Interim maintenance (until A)

- When the SDK risk logic changes, update `src/liquidation_risk.rs` in the same PR (or immediately after) and note the SDK commit/version in the PR description.
- Consider adding CI parity tests (Rust vs Node on fixture accounts) if drift becomes painful before (A) ships.

---

## Guest referral, paper book, and deposit funnel

### Current state

The plugin owns copy and numbers: `render_share`, `render_guest_surface`, and `apply_guest_upgrade` return a fully filled `message` + `controls`. The model pastes them verbatim. Paper sessions persist as JSON (same interim pattern as the PnL ledger: `WORLD_GUEST_DIR`, else XDG). Showcase/drill figures currently come from `FixtureReporting::demo_book` (and `ZeroEdgeReporting` for null-result tests). Share images are off unless `WORLD_SHARE_CARD_RENDERER=1`, which still does not produce a PNG — it only flips the status field.

Door order (`WORLD_FUNNEL_DOOR_ORDER`) and conversion timing (`WORLD_FUNNEL_CONVERSION_TIMING`) are switchable. Defaults: basis-first, day-N window of 3, paper start `$100`, recommended first deposit `$20` / "clears transaction minimums".

### Missing host / product dependencies (do not fake these)

- **TODO:** Telegram `?start=g_<token>` routing. The Aomi host must deliver the start payload (or chat identity) into `render_guest_surface.guest_id`. Aggregate funnel attribution only — no referrer on any user-visible surface.
- **TODO:** 1200×1200 share-card PNG renderer (house style: `#0a0a0c`, IBM Plex Sans, JetBrains Mono, accent `#b388ff`, tabular figures, QR = guest deep link). Until it exists, the tool returns the link-only fallback.
- **TODO:** Canonical live demo book (`WORLD_DEMO_ACCOUNT_ID` or equivalent) so showcase and fire drill run the real tools at live rates. Fixture figures are a stand-in; if live tools fail, render `demo_unavailable` — never fabricate.
- **TODO:** Guest session persistence in the Aomi host store. File JSON is interim, same as PnL.
- **TODO:** Inline keyboards from structured `controls` (label + action). Keep-looking must be send-nothing, not a follow-up.
- **TODO:** `upgrade_event` from world.inc grant-key completion + `tg_chat` handoff, calling `apply_guest_upgrade` exactly once in the existing thread.
- **TODO:** world.inc `/?from=guest` first-deposit step (web team). Deposit mechanics (assets, gas, true minimums) are unverified at spec time; copy names a recommendation, not a gate.

### When to revisit

When Telegram start-payload routing, the share-card renderer, or the live demo-book account land. Do not ship unlabeled fixture rates as live.

---

## Local execution sidecar

### Current state

`sidecar/` is a Node process wrapping `@wcm-inc/sdk` (run via `tsx` because `@wcm-inc/abi` ships TypeScript factories). It signs with `WORLD_PRIVATE_KEY` from `.env` and exposes `/v1/orders`, `/v1/orders/cancel`, `/v1/swaps`, `/v1/loans/renew`. The Rust plugin (`src/execution.rs`) is an HTTP client only. The swap point for hosted Aomi signing is that HTTP client, not a new plugin tool surface.

Local mandate arrives via `WORLD_MANDATE_JSON`, `WORLD_MANDATE_PATH`, or the bundled `mandate.dev.example.json` placeholder because `aomi-run` stubs `handover_mandate`. Set `WORLD_MANDATE_PATH=none` to test the fail-closed handshake. Post-trade RAPV is a pre-trade stand-in.

### Future intention

Replace the sidecar process with whatever Aomi uses to hold a key and broadcast — keep the request types in `src/execution.rs`. Then delete local `.env` key usage.

### When to revisit

When Aomi documents host signing, or when swapping the rest of the Rust contract reads to the same SDK service.


---

## Template for new entries

```markdown
### <topic>

#### Current state
…

#### Observations
…

#### Future intention
…

#### When to revisit
…
```
