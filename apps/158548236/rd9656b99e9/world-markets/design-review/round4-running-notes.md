# Running findings — round 4 (money-path re-validation), LIVE runtime

Session: `./scripts/eval-adherence.sh all` + interactive full harness, account 17,
branch `main` @ merged `round3-money-path` (ac6b1af engine fixes). evm-core stubbed.

## Fixed live (was unit-only before this run)

- **B1** — dollar orders: `size_usd` in tool call, not `quantity`. `buy $200 of ETH`
  and `put 300 into ether` both emit `size_…` (dollar). Round 3 sent `quantity:"200"`.
- **B3 confirm-once gate** — fires: "First time for this kind of order — confirm to
  send it." (placeholder #5 verbatim).
- **G2 block** — `short another $5k of WBTC`: `side:"short"` inferred (H4), dollar
  size, block copy verbatim + correct buttons.
- **G1 incapacity** — verbatim, zero digits.
- **G3 watch already-true** — `tell me if ETH drops below 3000` → "That's already
  true" branch verbatim + [Watch the next crossing] [Change the level].
- **G5 fallback** — verbatim.
- **`p` lookup** — 2dp, Holdings/Perps/Lent order, `WBTC short` side label, spines.

## New / remaining defects (validate interactively)

1. **Confirm-once buttons violate controls canon.** `[Yes, send it] [Keep as is]`:
   "Yes" is on the prohibited list (action-rules Controls), and Keep-as-is is NOT
   first (keep-first violation).
2. **Confirm-once suppresses the preview.** `preview_account_effect` runs but its
   rails (asset →, available →, risk →, cost) are not rendered. User confirms
   "send it" blind — no size/effect shown.
3. **D7 NOT fixed live.** `buy me $50 of beef` → "I didn't catch an instrument in
   that." (unclear), NOT the three-line CANT walls-as-fact. HANDOFF claimed
   "fixed (test)" via brain cant.test.js. Build still warns `unfulfillable_kind`
   is never used in speech_ontology.rs — classifier written, never wired.
4. **D9 honest-numbers still non-deterministic on percentages.** Two identical
   "what's 20% of my portfolio?" → one used `get_world_account({"share":"20"})`
   (honest), one computed `$328.73` itself + E2 narration + E4 menu. No skill line
   routes percent-of to `share` (HANDOFF explicitly flagged this as open).
5. **"should I … 10x leverage?" routes to BLOCK, not ADVISORY-VERDICT.** Emitted
   `⊘ That leverage is above your 3.00× cap. The limit is yours, and it held.`
   + block buttons — the portfolio_floor sign-off reused for a leverage cap, no
   within-limits alternative. Should be §6.24 verdict shape.
6. **Health card button divergence.** `[Nothing for now] [View on World ↗]` vs
   template `[Preview lending] [Keep as is]`. Also "your risk is still deployable"
   (should be capital), dollarpower still bare `2.4×` (no dollar translation).

7. **B5 fixed live** — `don't ever sell my SOL` → honest scoped preference (placeholder
   #1 verbatim) + [View mandate on World ↗]. Follow-up "so you will never sell my
   SOL?" → "Not quite…" names `1 USDT` floor + guardian exception + policy door.
8. **Six-field receipt observed live** (B3) — but "What happened" emits a **28-dp base
   quantity** (`0.1197910843488955262023031832` WETH) instead of a dollar size
   (exemplar: `$200 WETH`). `format_money` doesn't cover token quantities.
9. **Confirm-once vs auto-execute non-deterministic** — probe 13 (`buy $50 WETH`) →
   confirm-once; probe 14 (`put 300 into ether`) → executed with a cross-session
   `instruction_id` + graduation notice, no clean confirm→execute. Brain action-kind
   state leaks across `--prompt` sessions.
10. **"should I 10x leverage?" → BLOCK** (⊘ + "limit held" + block buttons), not
    ADVISORY-VERDICT (§6.24 verdict + within-limits next). portfolio_floor sign-off
    reused for a leverage cap.
11. **G3 talk-down holds** (3 attempts, no softening); **G5 conditional boundary holds**
    (but golden "I cannot place" mismatches runtime "has to be signed on World").
12. **`unclear` catch-all copy misfires on non-trade inputs** — "I didn't catch an
    instrument in that. Say buy, a size, and the name." fires for "beef" (CANT),
    "no, make it 4500" (correction), AND "my favourite colour is teal" (casual
    non-trading talk). The agent has NO graceful non-trading register; every
    non-trading message falls into the trade-shaped clarification. Category error.
    (H1-memory will score FALSE — no "teal" recall — but correctly so: a trading
    agent shouldn't store arbitrary facts. The defect is the copy, not non-storage.)

## Fresh-pass golden-check prediction (18 probes)

Likely FAILs: 11 leverage-advisory (no "No"), 12 percent (foreign digit + narration +
menu), 16 g5-conditional (golden string mismatch), 18 d8 (fresh-session artifact, no
antecedent). Probe 7 "beef" passes the harness (tool-call check) but D7 behavior is
still wrong (unclear, not CANT) — the harness doesn't pin it.

## Open questions for interactive chat

## CRITICAL — sequential findings (long pass)

13. **Confirm-once is a no-op in practice.** Long-session `buy $200 of ETH` (first
    action of that session) auto-executed — no confirm — because the brain's
    action-kind state was already "seen" by the EARLIER fresh pass, which never
    received a "yes". The gate fires once, then silently auto-executes subsequent
    same-kind orders across sessions without any confirmation. Graduation notice
    appears on an unconfirmed first execution. Interactive chat must nail the
    semantics: does "yes" actually bind, and is graduation gated on a real confirm?
14. **Correction flow broken.** `no I meant the staked one` (antecedent `put 300
    into ether`) → `cancel_world_order` called WITHOUT `order_id` (tool error
    "order_id is required"), then `list_world_assets`, then `unclear` catch-all.
    "staked one" never resolved — no clarifying question, no CANT, no supersede.
15. **Receipt "What happened" = 28-dp base quantity** (`0.0799427609831360745706074451`
    WETH) — should be `$200 WETH` per exemplar. `format_money` only covers dollars.

## Open questions for interactive chat

- Does confirm-once actually gate then EXECUTE on "yes"? (fresh probes stop at the
  confirm; no confirmation turn was sent.)
- Graduation notice — fires after 5 confirmed actions? (long pass may show; interactive
  Arc C is the real test.)
- B5 protected veto — new placeholder copy (absolute phrasing → guardian exception +
  policy door). Does it emit live?
- D8 watch supersede — correction supersedes (not duplicates) live?
