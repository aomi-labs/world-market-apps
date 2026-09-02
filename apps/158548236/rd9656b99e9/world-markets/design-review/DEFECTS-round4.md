# DEFECTS — round 4 (what to fix, and how)

**Date:** 2026-08-26 · **Branch:** `main` @ merged `round3-money-path` (`ac6b1af`)
**Grounded in:** `FINDINGS-round4.md` (this round's live run) + the owner's opt-out resolution.
**Scope:** product/agent defects only. Each entry: what's broken → why it matters → recommended solution, split **copy** (design agent) vs **engine** (coding agent).

**Owner resolution adopted first (overrides R4-1/R4-4 as previously written):**
> Confirmation is **opt-out, not opt-in.** The agent reads back what it understood, waits 3 seconds, then executes. No "yes" required. The user can cancel in the window, but it sends by default.

---

## D1 — Confirm-once must be opt-out (read-back + 3s + default-send), not "say yes". **[P0]**

**What's broken now.** `buy $200 of WETH` → *"First time for this kind of order — confirm to send it. Say yes to place it."* Three defects compound here:
1. It asks for a *yes* — opt-in, contrary to the resolution.
2. It shows **no size/asset/effect** — the user confirms "send it" blind (`preview_account_effect` ran in the same turn but its rails are discarded).
3. The gate advances on **first sight, not on the send** — the next session auto-executes without the user ever having acted (the `instruction_id` leaks across sessions).

**Why it matters.** This is the product's primary control promise ("I remain in control"). In the current form it's an empty gesture: it demands a tap that it then doesn't actually require, while giving the user zero information about what they'd be tapping.

**Recommended solution.**

*Copy (design):* replace the confirm-once message with a read-back that doubles as the 3-second cancel notice, in house register:

> Staging `$200` of WETH spot — `~0.08` WETH at `~$2,500`.
> Sends in 3s if you don't cancel.
> [Cancel]

The read-back restates **size, asset, side, and the derived base quantity + mark** — all sourced from the `preview_account_effect` that already runs, so it fixes blindness without adding a tool call. `[Cancel]` is the only control; keep-first does not apply (there is no "keep" — cancelling is the only opt-out).

*Engine (coding):* the action-kind state must advance on **actual execution** (after the 3s window elapses without a cancel), never on "first sight." The existing "staged … 3s to fill" window that already exists in `execute_world_order` is the natural mechanism — the read-back is displayed *during that window*, and the order sends when it closes. The cross-session `instruction_id` leak (fresh pass triggered confirm-once, next session auto-executed) is rooted here: state advances when `execute_world_order` first returns `staged`, not when the order fills/sends.

---

## D2 — Receipt "What happened" must not emit engine-precision quantities. **[P1]**

**What's broken now.** `Staged `0.0799427609831360745706074451` WETH spot market`. 28 decimal places on the product's primary action. The exemplar says `Bought `$200` WETH`.

**Why it matters.** In a financial product, a 28-digit figure reads as sloppy custody — the SOUL's own line. The user can't sanity-check a number they can't read.

**Recommended solution.** *Tool (coding):* render the base quantity in human units — either the dollar size (`~$200 of WETH`) or a ≤4-dp base quantity (`0.0799 WETH`) — in `render_receipt`. `format_money` covers dollars but not token quantities; add a quantity formatter. *Copy (design):* `What happened · Sent `$200` of WETH spot (~`0.08` WETH), filled at `[#]`.` — matching the exemplar.

---

## D3 — The `unclear` catch-all misfires on non-trade input; CANT was never wired. **[P1]**

**What's broken now.** One string — *"I didn't catch an instrument in that. Say buy, a size, and the name."* — is returned for `buy me $50 of beef` (→ should be CANT walls), `no, make it 4500` (→ correction-aware), and `my favourite colour is teal` (→ non-trading register). The D7 fix was claimed but never reached the runtime: the build still warns `unfulfillable_kind` is **never used** in `speech_ontology.rs` — the classifier was written and never called.

**Why it matters.** The agent has no non-trading register at all. Every message it can't place is answered as if the user tried to buy something — a category error on the most human inputs.

**Recommended solution.** *Engine (coding):* wire `unfulfillable_kind` into `render_lookup` so `buy me $50 of beef` returns `cant`, not `unclear`. *Copy (design):* give the `unclear` branch a non-trade-aware fallback that names the actual situation instead of assuming a buy, e.g.:

> I didn't catch that — I trade crypto spot, perps, and lending. Say what you'd like to do, or `/p` for positions.

(Distinct from the CANT three-line walls, and distinct from FALLBACK §6.20.)

---

## D4 — Percent-of reads don't route through the `share` field → the model does arithmetic. **[P1]**

**What's broken now.** `what's 20% of my portfolio?` → two identical runs: one used `get_world_account({"share":"20"})` (tool computes `$328.73`, honest), the other called `get_world_account({})` and computed `$328.73` itself — with narration (*"I'll check… and calculate 20%"*) and a capability menu. The `share` field exists; **no skill line routes percent-of asks to it** (the handoff flagged this exact line as open).

**Why it matters.** This is the honest-numbers law failing a coin-flip. The model treats "multiply by 0.2" as harmless while refusing "multiply by EUR/USD" — the law is enforced by which conversions *look* hard, not by the law itself.

**Recommended solution.** *Copy (design):* one routing line in the payload — percent/share/fraction-of asks go through `parse_share_ask` → `get_world_account.share`, with the existing "I've left it out rather than guess" as the refusal when no share tool matches. This is a one-line skill edit, and it's the design agent's territory.

---

## D5 — "should I X" routes non-deterministically to BLOCK instead of ADVISORY-VERDICT. **[P2]**

**What's broken now.** Same prompt, two shapes across runs:
- `⊘ That leverage is above your 3.00× cap. The limit is yours, and it held.` + block buttons (the **floor** sign-off borrowed for a **leverage cap**).
- `That's outside your signed leverage cap… Next · Preview a smaller position within your $25,000 limit.` (correct §6.24 verdict).

**Why it matters.** The BLOCK shape drops the constructive within-limits "Next" path §6.24 requires, and reuses copy ("the limit is yours, and it held") that belongs to the *floor* block. Inconsistent routing + inconsistent gate citation reads as sloppy.

**Recommended solution.** *Copy (design):* pin "should I X" → ADVISORY-VERDICT in the routing table, and state that the `portfolio_floor` sign-off is *never* used for a leverage/notional cap. The verdict shape stays: verdict first → one mandate-grounded clause → within-limits "Next" → preview button.

---

## D6 — "Cannot help" turns answer with zero tool calls and a drifted figure. **[P2]**

**What's broken now.** `should I add more money?` → correct refusal, but **no tool call**, citing `portfolio $1,644.52` from memory — `$0.89` off the `$1,643.63` read two turns earlier (and the session had staged orders since).

**Why it matters.** Stale-figure reuse is the honest-numbers failure class round 3 caught, reappearing on the one turn type that feels safe ("I'm just refusing"). A user screenshotting that answer gets a number that was already wrong.

**Recommended solution.** *Copy (design):* on incapacity/refusal turns, cite no figure at all unless the turn read it fresh — "I work with what's already on World" is complete without a portfolio value. If a figure is wanted, the turn calls `get_world_account` first.

---

## D7 — `instruction_id` is reused across different orders. **[engine clarify]**

**What's broken now.** `yes` → `execute(instruction_id=f9b02ef5-…)`. `buy $150 WETH` → `execute(instruction_id=f9b02ef5-…)` — the *same* token for a *different* order.

**Why it matters.** If the id is a per-kind authorization token, this is fine; if it's a per-order binding, it's a bug that would let an order "inherit" another's confirmation. Can't be resolved from observation alone.

**Recommended solution.** *Engine (coding):* clarify and document the `instruction_id` semantics. Under the opt-out model (D1) this token's role changes anyway — state it explicitly in the D1 engine change.

---

## Harness defects (repo, not agent — but they gate the regression net) **[P1]**

- **H1 — `check_no_foreign_digits` is a no-op.** It compares message digits against `tool_blob(stdout) + stdout`, but `stdout` *contains* the message, so every digit matches. The single most important honest-numbers regression test can never fail — which is how probe 12 "passed" while the model was fabricating `$328.73`. Fix: compare against tool output/stderr only, excluding the reply text.
- **H2 — `run.py all` hangs on exit.** An orphaned PTY child survives `ReplSession.close()`; `results.json` is never rewritten. Fix: kill the child process group in `close()`.
- **H3 — `G5_conditional` golden is pinned to the wrong copy** (`"I cannot place"` vs the runtime's correct `"has to be signed on World"`). Fix the golden, not the agent.

---

## Priority order

1. **D1** (P0) — opt-out confirm-once: read-back + 3s + default-send. Trust core.
2. **D2** (P1) — receipt quantity in human units.
3. **D3** (P1) — wire CANT, fix the `unclear` copy.
4. **D4** (P1) — one routing line for percent-of → `share`.
5. **H1** (P1) — fix the foreign-digit regression test (it's guarding everything above).
6. **D5, D6** (P2), **D7** (engine clarify), **H2, H3** (hygiene).

**Design-border note:** D1-copy, D2-copy, D3-copy, D4, D5, D6 are prompt/skill edits (design agent's territory). D1-engine, D2-tool, D3-tool, D7, H1–H3 are Rust/harness (coding agent). Design finalizes copy → owner signs off → coding agent implements engine.