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

The plugin owns copy and numbers: `render_guest_surface` and `apply_guest_upgrade` return a fully filled `message` + `controls`. The model pastes them verbatim. Paper sessions persist as JSON (same interim pattern as the PnL ledger: `WORLD_GUEST_DIR`, else XDG). Showcase/drill figures currently come from `FixtureReporting::demo_book` (and `ZeroEdgeReporting` for null-result tests).

The sending-side introduction is `render_share` / `render_lookup` share intent → brain JSON codes (`ref_{code}`) and M10. `/start` payload routing is still a host duty: pass chat identity as `guest_id` and the payload as `start_payload`. Attribution is silent (brain JSON only).

Door order (`WORLD_FUNNEL_DOOR_ORDER`) and conversion timing (`WORLD_FUNNEL_CONVERSION_TIMING`) are switchable. Defaults: basis-first, day-N window of 3, paper start `$100`, recommended first deposit `$20` / "clears transaction minimums".

### Missing host / product dependencies (do not fake these)

- **TODO:** Telegram `?start=` / `?startapp=` delivery. The Aomi host must deliver the start payload into `render_guest_surface.start_payload` (or `render_lookup` text) and chat identity into `guest_id`. `ref_{code}` writes silent attribution; `g_` remains the guest session token.
- **TODO:** Canonical live demo book (`WORLD_DEMO_ACCOUNT_ID` or equivalent) so showcase and fire drill run the real tools at live rates. Fixture figures are a stand-in; if live tools fail, render `demo_unavailable` — never fabricate.
- **TODO:** Guest session persistence in the Aomi host store. File JSON is interim, same as PnL.
- **TODO:** Inline keyboards from structured `controls` (label + action, plus `url` on M10). Keep-looking must be send-nothing, not a follow-up. Host should send `messages[]` as separate Telegram messages when present.
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

## Candlestick charts (Telegram photo delivery)

### Current state

`render_market_chart` fetches OHLC from a pluggable feed (Yahoo by default), writes a compact PNG under `WORLD_CHART_DIR` (auto-pruned), and returns `caption` plus `image` / `image_status` / `image_path` — the same field names as share cards — plus optional `controls` / `mini_app` for **Open chart**. `photo_action` is `viewer`: tapping the PNG must not open the Mini App. Local `aomi-run` can `open` the file when `WORLD_CHART_OPEN=1`. The plugin does not call Telegram `sendPhoto`.

### Observations

Aomi tools return JSON only. Hosted Telegram will not show a photo until the runtime attaches `image_path` (or equivalent bytes) the same way it is supposed to attach share-card PNGs.

### Future intention

Host sends the generated PNG via Telegram `sendPhoto` (or a documented image-attachment envelope). Tapping the photo opens Telegram's media viewer only. The Mini App is optional: one inline `web_app` button **under** the photo (`Open chart` from `mini_app.path` / `mini_app.startapp`). Do not bind `web_app` to the photo. Keep plugin charts ephemeral: do not archive bytes in the host transcript.

### When to revisit

When Aomi documents image/photo delivery from tool results, or alongside the share-card PNG renderer.


---

## Terse lookup short-circuit (500ms)

### Current state

`render_lookup` fills the one-line `b`/`p`/`r`/`a`/`d`/`?` templates in Rust and returns `{ skip_llm, message }`. `warm_account` prefetches the live account into the 8s RPC cache. Skill copy tells the model to paste `message` verbatim.

### Observations

Two LLM hops plus the 8k skill still dominate Telegram latency. Plugin-side cache hits are milliseconds; a cold public-RPC account read is already hundreds of ms. The 500ms budget only closes if the host skips the model on whole-message tokens.

### Future intention

Hosted Aomi (not `aomi-run`) should call `render_lookup` with `text` = the user message **on every inbound chat**, not only terse tokens. Unmatched messages prefetch the account in the background. The plugin also refreshes every 60s while the session is active and rebuilds the cache after trades. When `skip_llm` is true, send `message` and do not invoke the LLM.

### When to revisit

When staging Telegram is wired to this contract, or when measuring p50 time-to-first-byte on terse tokens. Do not cache RAPV/NAV across blocks as "live" without an age qualifier.


---

## News sources (brain sidecar)

### Current state

Research, watches, mark history, and preferences live in an unsigned Node process (`brain/`, default `http://127.0.0.1:8788`). It never holds `WORLD_PRIVATE_KEY` and never places an order. The plugin is an HTTP client (`src/brain.rs`).

News is a **registry**. Each source is `brain/src/news/<id>.js` implementing `{ id, fetch({ symbol, windowSecs, now }) }`. Register it in `brain/src/news/index.js` (`SOURCES`) and enable with `WORLD_NEWS_SOURCES` (comma-separated). Contract: `brain/src/news/source.js`.

**Default source today:** CryptoCompare's public news list (`https://min-api.cryptocompare.com/data/v2/news/`, no API key). `cause_established` is true only when a headline names the asset *and* uses a causal cue (`after`, `amid`, `due to`, …). Headlines without a cause still appear in `sources[]` but do not flip the flag.

Portfolio before→after risk / dollarpower is `POST /v1/portfolio-impact`. Live `before` (RAPV, liquidation risk) is passed through from `get_world_account` via `get_world_research.portfolio_now`. `after` stays absent unless `WORLD_IMPACT_SDK_MODULE` points at an adapter that calls `@composite/sdk` with before/after snapshots. Never invent a post-move score in the message layer.

The Aomi host must drain `POST /v1/outbound/drain` (or `drain_world_outbound`) to deliver watch fires. Watch fires are solicited and must not share accounting with the weekly digest.

### How to add a news source (TODO — owner)

1. Add `brain/src/news/<id>.js` that default-exports `{ id, async fetch(query) }`. Return `{ status: "ok"|"timeout"|"unavailable", items: [{ name, url, ts, title, cause }] }`. Leave `cause` null unless the outlet attributed one — never invent a cause or a price.
2. Import and list it in `SOURCES` in `brain/src/news/index.js`.
3. Set `WORLD_NEWS_SOURCES=cryptocompare,<id>` (or replace the default).
4. Add a unit test in `brain/test/` that the module is registered.
5. Prefer a licensed wire, World-operated feed, or a source that returns structured causes over another headline dump.

Do **not** scrape paywalled articles, and do not let the model fill `cause_established`.

### Future intention

- Replace / supplement CryptoCompare with the in-house news process once it exists.
- Wire `@composite/sdk` (or `@wcm-inc/sdk`) into `brain/src/impact.js` so research can cite live before→after risk and dollarpower for a stated mark move.
- Host Telegram delivery of the outbound queue, 24/7, independent of a chat session.

### When to revisit

When a news vendor is chosen, when Composite SDK snapshot-in functions are available, or when hosted Aomi can drain outbound pushes.


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
