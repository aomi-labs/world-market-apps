# Handback — coding round 3 (money path)

**Date:** 2026-08-26 · **Branch:** `round3-money-path` · **Base:** `d1f6067`

Live round-3 probe re-run (OpenRouter + full stack) was **not** executed in this pass — unit tests and brain tests were. Include a transcript in the PR when `./scripts/eval-adherence.sh probes` is run against a live sidecar.

## What shipped

### P-A — ontology is the authority for size
- `Size::{Quote,Base}` in `src/size.rs`. Trade tools take `size_usd` / `size_base`; `quantity` is a deprecated alias for `size_base`.
- `speech_ontology::parse_size` classifies currency markers, money-verbs (`put/spend/invest/deploy`), and asset units. Bare numbers Ask, they do not guess.
- Quote sizes resolve from the **same mark the preview uses**. Preview→execute drift tolerance: **50 bps of notional** (`DRIFT_TOLERANCE_BPS`).
- Sentence with `$` + model-supplied `size_base`/`quantity` is rejected pre-engine (`size_denomination_mismatch`, `retry_with.size_usd`).

### P-B — money surfaces PASTE
- Receipts: `render_receipt` (six §6.5 fields) on `execute_world_order`. No Keep/Cancel trade buttons on an ordinary fill.
- Denies: `detail_rendered` + `message` from `render_deny` (2 dp, separators, backticks).
- Dollarpower: tool returns a complete `message`. Operands: **effective (separate-venue) ÷ committed (World)**. Fixture 24700 ÷ 10300 = 2.4×; the inverted “10300 ÷ 24700” labeling is gone.
- Protected-veto confirmation is the tool `message` (guardian exception + policy door when phrasing is absolute).
- Percent-of reads: `parse_share_ask` on `render_lookup` / `get_world_account.share`.

### P-C — gates are engine state
- Confirm-once: brain `/v1/action-kinds` + compose `trade_confirm` on the ledger. First-of-kind returns `needs_confirm`; `instruction_id` on the next `execute_world_order` is the yes-binding. Graduation notice is a receipt field.
- Watch correction: `supersedeWatch` cancels-and-replaces and writes the trail **before** the message. `cancel the ETH one` with >1 live match returns an Ask, not newest-wins.
- Guest: `render_guest_surface` hard-rejects any bound account (`already_user`).
- Permission read failure is `read_failed` (not `Err`, not guest).
- Health `needs_attention` is a **live** `ledger/summary` read the same turn; informational controls only (no Preview lending). `scripts/dev-run.sh --wipe` / `WORLD_BRAIN_WIPE=1` deletes `WORLD_BRAIN_DIR`.

### Phase 0 harness
- **H3:** first-tool-call reads stderr + stdout.
- **H2:** 22s pace default; 429/empty = `INFRA_SKIP`.
- **H1 chosen mechanism:** `--long` drives a persistent **PTY REPL**. One-shot `--prompt` stays fresh-per-probe. Memory probe: teal → favourite colour.

### Phase 5 diet
- Preview drops order-book / standing-brief dumps.
- Account inspect is compact (full dump behind `detail=full`).
- `get_world_tasks` omits voice lexicon unless `detail=full`.
- `tool_result_bytes` logged on fat tools.

### Phase 6 goldens
Added `G3_talkdown`, `G5_conditional`, B1/B5/D8/D9 pins in `tests/adherence-eval/probes.json`. Existing `G3` remains the watch already-true golden.

---

## Placeholder copy (design sign-off)

| # | Surface | Copy shipped |
|---|---|---|
| 1 | Protected-veto stored | `Stored: I'll avoid selling your {ASSET}. One exception you've already signed: if your portfolio breaches your floor and {ASSET} is the only way back above it, the guardian may sell some — your mandate outranks this preference. To make it absolute, change your policies on World.` |
| 2 | Same, absolute phrasing | (1) + ` [View mandate on World ↗]` |
| 3 | Permission `read_failed` | `I can't reach your account grant right now — try again in a moment` |
| 4 | Size ambiguous Ask | `Did you mean \`$N\` of it, or \`N\` units…` (see `size.rs` `ambiguous_ask`) |
| 5 | Confirm-once | `First time for this kind of order — confirm to send it. Say yes to place it, or change the size.` |
| 6 | Graduation | `Orders like this now execute automatically. Say \`always ask\` to keep confirmations.` |
| 7 | Watch ambiguity | `Which {SYM}? \`phrase\` · \`phrase\`` |
| 8 | Watch supersede | `Updated — now {phrase}. The previous version is in this task's history.` |
| 9 | Bound-account guest reject | existing `already_user` |
| 10 | Health nothing-needs-you | `Nothing needs you.` |

Skill markdown was **not** edited. Flag one routing line in the payload for percent-of reads (D9).

---

## §9 still open

| Item | This pass |
|---|---|
| Preview→execute drift | **50 bps of notional** proposed (venue ticks ~0.4–4 bps; 50 bps covers a few seconds of mark move). |
| §11.4 two-scale (0–10 vs RAPV) | Untouched. RAPV is never labeled “risk”; user-facing risk is 0–10. |
| Placeholder copy | Table above — design agent. |
| `Size::Pct` (“sell half”) | **Not** implemented. D9 is a read (`share`); trade percent stays next round. |

---

## Findings status

| ID | Status | Evidence |
|---|---|---|
| H1 | fixed | PTY REPL `--long`; memory probe in harness |
| H2 | fixed | `INFRA_SKIP` on 429/empty; `PACE_SECONDS=22` |
| H3 | fixed | `check_first_output_is_tool_call(stdout, stderr)` |
| H4 | fixed | `infer_side` from short/long; `missing_side` is machine-actionable |
| H5 | fixed (path) | same Quote/Base path for spot and perp |
| B1 | fixed (unit) | size tests: `$` / `put 300` / mismatch reject. Live transcript pending |
| B2 | fixed (unit) | `render_deny` + `detail_rendered` |
| B3 | fixed (unit) | `render_receipt` + confirm-once ledger gate |
| B4 | fixed (unit) | concern line only when 0–10 delta exists |
| B5 | fixed (unit) | preference tool PASTE; no categorical “I won’t sell” |
| D5 | fixed (unit) | dollarpower message: effective ÷ committed; 24700÷10300=2.4 |
| D6 | fixed (unit) | bound account → `already_user`; permission `read_failed` |
| D7 | fixed (test) | `buy me $50 of beef` → `cant` immediately; `buy $50` → unclear. Brain `cant.test.js` |
| D8 | fixed (code) | `supersedeWatch` + match ambiguity. Live state check pending |
| D9 | fixed (unit) | `parse_share_ask("what's 20% of my portfolio?")` |
| D10 | fixed (path) | `format_money` on user-facing figures |
| D11 | fixed (code) | live `needs_you`; wipe script; no trade buttons on health |
| G3 talk-down | pinned | probe 15; live pending |
| G5 conditional | pinned | probe 16; live pending |

---

## Chosen H1 mechanism (PR body)

Drive multi-turn through a PTY `aomi-run` REPL (`ReplSession`). Do **not** persist `--session-id` across `--prompt` (aomi-run does not keep that conversation).
