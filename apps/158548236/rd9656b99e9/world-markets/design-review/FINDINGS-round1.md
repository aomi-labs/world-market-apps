# aomi dogfooding — FINDINGS round 1

**Date:** 2026-08-26 · **Commit:** 9cb9846 · **Session:** df6deb51
**Transcript:** `design-review/session-transcript-2026-08-26.md`
**Method:** live `dev-run.sh` stack (brain + sidecar + REPL), account 17, 11 probes.

The question this round answers: **how does the running agent behave vs. the
ideal in `AOMI_PRODUCT_HANDOFF.md`, and where is the biggest gap / biggest
opportunity.** Behavioral changes and feature list come next round.

---

## The one-paragraph verdict

The agent's **static, single-shot messages are already excellent** — the
incapacity answer, the block copy, the watch tell-only guarantee are close to
canonical. What is largely **missing is the *loop* itself**: the R1 core loop
(heard-echo → parse-with-lexicon → act/one-question/escalate → receipt, with
everything landing as a ledger row) is not the runtime's operating shape. The
agent behaves like a **very good chatbot answering each message from scratch**,
not like a **counterpart running a relationship with memory, a ledger, and a
disciplined grammar.** That is the gap, and it is also the opportunity: the
product's entire thesis ("a relationship, not an app") lives in the loop, and
the loop is the part that isn't there yet.

---

## What is already right (protect these — they will regress if unspecified)

| # | Behavior | Why it's right | Status |
|---|----------|----------------|--------|
| G1 | **Incapacity message** (probe 8) | Trade-only key, unapproved markets, can't-change-own-rules, prompt-injection resistance, policy engine final. No numbers. Screenshot-safe. | **Canonical.** Lift verbatim into spec as the reference string. |
| G2 | **Unsigned-market block** (probe 6) | Names exact gate, one boundary, `[View mandate on World ↗] [Keep as is]`. *Blocked means blocked.* | Strong. |
| G3 | **Watch tell-only line** (probe 9) | "This is a heads-up, not a trade. I won't buy or sell anything." + exits. | Correct — *watches never trade* landed. |
| G4 | **Health card "Working, not stuck"** (probe 2) | Echoes the north-star feeling; one improvement line; exits. | Strong skeleton. |
| G5 | **Gibberish fallback** (probe 10) | Short, no capability-menu leak, screenshot-safe. | Good. |
| G6 | **Leverage "No" grounded in mandate** (probe 11) | Refuses without moralising; no banned hype vocab; offers a within-limits next step. | Good spine, over-long body (see D6). |

**These are model-improvised, not spec-guaranteed.** Per the dogfooding
discipline: correct-but-unspecified behavior *will* regress on the next model
swap. G1–G6 need to become pinned reference strings + golden-eval anchors.

---

## Divergences, ranked by product impact

### D1 — The heard-echo is missing from the money path  **[highest impact]**
**Spec:** R1 loop opens with aomi quoting what it heard (~2s) — "proof of
hearing is what makes both action and refusal trustworthy" (§3, §5-nothing).
**Runtime:** every trade probe jumps straight to a tool call. No echo before
action (probes 3, 4, 6). On a *block*, the user never even gets confirmation the
agent understood them before being told no — the single worst place to omit it.
**Consequence:** the trust primitive the whole voice loop is built on is absent.
This is the load-bearing gap.

### D2 — Chatty preamble replaces the statement grammar  **[high]**
**Spec:** statement grammar — one conclusion + one explanation + one next
decision; ≤160-char blocks; screenshot-safe; no filler.
**Runtime:** "I'll help you short $5k of WBTC. Let me first refresh your account
and preview this trade." (probe 6) and "I'll set up a watch… Let me store that
for you." (probe 9). This is assistant-chatbot register, not counterpart
register. It also *narrates tool-calling*, which the user should never see as prose.
**Consequence:** breaks screenshot-safe canon and the "precise financial
operator" persona. Cheap to fix (prompt), high visibility.

### D3 — The unfulfillable-request flow is not firing  **[high]**
**Spec:** phonetic near-match → three-line walls-as-fact reply (heard · wall ·
what World *does* trade, category-level) → terminal `can't` ledger row.
**Runtime:** "buy me $50 of beef" → generic "I need to clarify what you mean by
beef… please provide the symbol (like BTC, ETH)." (probe 7). No near-match, no
walls-as-fact, no `can't` row, and it pushes the *user* to supply a symbol —
the inverse of the intended direction.
**Note:** commit 9cb9846 added "a skip-llm path for unfulfillable names" — the
path exists but did not engage for "beef"; the LLM handled it conversationally.
**Consequence:** the `can't`-row roadmap (product-gap capture) is not being
built, and the ledger's "everything you asked lives here" promise is broken.

### D4 — Ambiguity → one-question repair does not fire  **[medium, confounded]**
**Spec:** attribute-based single question ("which ether — spot, perp, or
staked?"), max two rounds.
**Runtime:** "put 300 into ether" silently resolved to WETH spot (probe 4).
**Confound:** this testnet universe has only WETH — there is genuinely no
stETH/ETH-perp to disambiguate against (surfaced in probe 5). So the *silence*
may be correct here. But the correction probe shows the repair, when it does
come, is open-ended ("or were you looking for a different asset?") rather than
attribute-based. Re-test on a real multi-ETH universe before calling it a defect.

### D5 — Number-layer defects  **[medium — trust-critical]**
- **Broken watch line:** "Now `2465.71`, so that's `3000`." (probe 9) — garbled
  template; nonsense to a user.
- **Already-true watch not detected:** ETH is 2465.71, the trigger is ≤3000, so
  the watch fires immediately in principle; the agent set a 30-day watch as if
  the condition were in the future.
- **Liquidation-risk drift:** `4.1` (probe 2) vs `4.2` (probe 11), same account,
  minutes apart — the instability the skill already flagged for `p`.
- **dollarpower `2.4×` with no dollar translation** (probe 2) — canon: *always
  pair with the dollar translation.*
**Consequence:** the honest-numbers layer is the product's trust spine; garbled
and drifting numbers are disproportionately damaging in a financial product.

### D6 — Advisory answers over-run the voice  **[medium]**
**Spec:** one conclusion + one explanation + one next decision; agent never
sells capital deployment as coaching.
**Runtime:** the leverage answer (probe 11) is a 4-paragraph essay with a
"Your strategy focus should be…" coaching line and an unprompted basis-trade
yield pitch ("13.2% funding vs 5.5% borrow = 7.7% net"). Edges toward
salesmanship and prescription. Also carries the "dangerous levels" hand-wave
next to a precise 4.2 (mixes vague + precise risk language).
**Consequence:** persona drift toward advisor/salesperson; length breaks the
screenshot-safe / calm-by-construction stance.

### D7 — Fail-closed RAPV block masks the happy path  **[env, not a defect —
but it hid the most important test]**
Every spot-buy hit "cannot prove post-trade RAPV → floor fails closed" (probes
3, 4). This is almost certainly the stubbed evm-core, not a product block. It
means **we never observed a successful execute → receipt** — the positive half
of *Allowed means allowed* went untested. Need a runtime where RAPV computes to
verify the receipt grammar and whether the confirm-once/escalation classes fire
on material size. **Top of next round's setup list.**

---

## Where the biggest improvement is (my read)

**Biggest gap:** **D1 + D2 together — the loop grammar.** The agent has the
right *words* for the hard edges (blocks, incapacity, watches) but not the right
*shape* for the ordinary turn. Heard-echo is absent; tool-narration filler is
present. Fixing this converts a good chatbot into the counterpart the product
thesis promises, and it is mostly prompt-and-render work, not new systems.

**Biggest opportunity:** **the ledger as a first-class runtime object (D3 +
G-series persistence).** Right now each message is answered from scratch; the
`can't` row isn't written, corrections aren't visibly captured as pairs, and
there's no evidence the say/do chain is being assembled at runtime. The ledger
is the product's spine *and* its data edge (compliance archive = training
corpus). Making every turn — including refusals and unfulfillable requests —
land as a durable, provenance-carrying row is the single change that most
advances both the experience ("everything I asked lives here") and the moat
(lexicons + correction pairs + say/do chain). It is also the thing no competitor
copies.

**One-line priority for the next round:**
> Specify the loop grammar (heard-echo + statement register + ledger-row-on-every-turn)
> before adding any new capability — the words are good, the shape is missing.

---

## Setup debt for round 2
1. A runtime where **RAPV computes** (un-stub or seed evm-core) so execute→receipt
   and size-based escalation can be observed (D7).
2. A universe with **multiple ETH instruments** to fairly test the one-question
   repair (D4).
3. Confirm whether a **ledger store** is being written at runtime (`get_world_tasks`
   / brain JSON) — needed to test D3's `can't` row and the say/do chain.
