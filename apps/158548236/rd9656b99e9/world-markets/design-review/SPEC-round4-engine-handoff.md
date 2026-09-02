# SPEC — round 4 engine + harness work (coding agent)

**For:** the coding agent implementing the Rust/harness half of DEFECTS-round4.
**Date:** 2026-08-27 · **Branch base:** `main` @ `ac6b1af` (merged `round3-money-path`).
**Companion (already done, do not re-do):** the copy half of D1–D6 is landed in
`src/skill/*.md` (workflows.md, turn-contract.md, action-rules.md, exemplars.md).
Read those first — your engine changes must make the runtime emit the copy those
files now govern. **Do not edit `src/skill/*.md`.** That is the design agent's remit.

**Grounded in:** `design-review/FINDINGS-round4.md` (live run) and the code cited
inline below (file:line as of `ac6b1af`; re-locate if it drifted).

**Owner resolution this spec implements (overrides the old opt-in confirm-once):**
> Confirmation is **opt-out, not opt-in.** The agent reads back what it understood,
> waits 3 seconds, then executes. No "yes" required. The user can cancel in the
> window, but it sends by default.

**Process gate:** design finalized → owner signs off → you implement. Every claim
below is verifiable against the tree; verify, don't trust.

---

## Priority order

1. **E1 (P0)** — opt-out confirm-once: collapse the `needs_confirm` gate into the
   staged 3s window; advance the action-kind on the *send*, not on sight.
2. **E2 (P1)** — receipt/staged "What happened" quantity in human units (kill 28-dp).
3. **E3 (P1)** — wire `unfulfillable_kind`; split `unclear` from `cant`/`near_match`.
4. **H1 (P1)** — fix `check_no_foreign_digits` (the honest-numbers regression net).
5. **E4 (P2 / clarify)** — `instruction_id` semantics (per-kind vs per-order).
6. **H2, H3 (hygiene)** — PTY orphan on `run.py all` exit; G5_conditional golden.

---

## E1 — Opt-out confirm-once [P0, trust core]

### The defect, precisely (from FINDINGS R4-1 + R4-4)

Two code paths exist for a first-instance order and they are the wrong two:

- **`confirm_once_gate`** (`src/tool.rs:1779`) short-circuits the *first* instance of
  an action kind. It calls `app.brain.compose({kind:"trade_confirm", …})` to draft an
  `instruction_id`, then returns a `needs_confirm` payload
  (`src/tool.rs:1824–1843`) whose `message` is the static
  `CONFIRM_ONCE_MESSAGE` (`src/reporting.rs:587`,
  *"First time for this kind of order — confirm to send it. Say yes to place it…"*)
  and whose controls are `[Yes, send it]` / `[Keep as is]`. This is **opt-in** — it
  asks for a "yes" — and it shows **no size/asset/effect** even though `resolved_size`
  (with `notional`, `base_qty`, `mark`) is right there in the same payload
  (`src/tool.rs:1830`).
- The **staged 3s path** (`src/tool.rs:1562–1616`) is only reached *after*
  `confirm_once_gate` returns `None` — i.e. on the second-and-later instances, or
  when an `instruction_id` is passed. It stages via `stage_and_schedule`, renders
  `"Staged … — 3s to fill."`, and calls `mark_kind_confirmed`
  (`src/tool.rs:1573`) to graduate the kind.

The result (observed live): first "buy" → opt-in gate, **no send, kind not marked**;
next "buy" (even seconds later, even next session) → `confirm_once_gate` returns
`None` (because `action_kind_status` now reports the kind as seen/durable) → **silent
auto-execute**. The user never said "yes" to anything and the confirm-once gate
provided zero protection. That is the R4-1 cross-session leak.

### Target behavior

**Every first-instance order takes the staged 3s read-back path.** There is one path,
not two. The 3s window *is* the confirmation. The kind graduates when the order
**sends** (window elapsed uncancelled), never on sight and never on a separate "yes".

Concretely:

1. **Delete the opt-in branch.** `confirm_once_gate` must no longer return a
   `needs_confirm` / `[Yes, send it]` payload. Either remove the early-return gate
   entirely, or repurpose it to only *tag* the staged result as "first instance of
   this kind" (so the staged render knows to append the graduation notice after the
   send). The first-instance signal is still needed — it's what decides whether the
   GRADUATION notice rides the resulting receipt — but it must no longer block or ask.

2. **First instance stages exactly like any other order.** Route first-instance
   through `stage_and_schedule` (`src/staged.rs:stage_and_schedule`) with the 3s
   cancel window. `resolved_size` is already computed before the gate
   (`src/tool.rs:1539` passes `&resolved`); use it for the read-back.

3. **The staged read-back message is the CONFIRM-ONCE copy, not the old
   `"Staged … — 3s to fill."` string.** The canonical template now lives in
   `src/skill/workflows.md` → `## CONFIRM-ONCE (§6.4a)`:

   ```
   Staging `$200` of WETH spot — `~0.08` WETH at `~2,500`.
   Sends in 3s if you don't cancel.
   [Cancel]
   ```

   Replace `CONFIRM_ONCE_MESSAGE` (`src/reporting.rs:587`). Because the read-back
   carries live figures (size, base qty, mark) it can no longer be a `const &str` —
   make it a formatter, e.g.:

   ```rust
   pub(crate) fn render_confirm_once_readback(resolved: &ResolvedSize,
                                               asset: &str, product: &str) -> String
   ```

   sourcing size from `resolved.notional` via `format_money` (→ `$200`), base qty from
   `format_base_qty(resolved.base_qty)` (≤6 dp; already exists, `src/size.rs:285`),
   and mark from `resolved.mark`. The base-qty+mark clause is prefixed `~` (they are
   estimates at stage time). Controls: a single `[Cancel]` (`action:"cancel"`), **no**
   `[Yes, send it]`, **no** keep-first pair. The `[Cancel]` must actually cancel the
   staged order in the window — wire it to `flush`/cancel of the staged instruction
   (`src/staged.rs` already has the cancel field, `src/staged.rs:81`).

4. **Graduate on the send, not on sight.** `mark_kind_confirmed`
   (`src/tool.rs:1848`) currently runs at stage time (`src/tool.rs:1573`). Move the
   graduation so the kind is marked confirmed **only when the staged order actually
   fills/sends** — i.e. inside the flush path (`flush_staged_trade`,
   `src/staged.rs:97`) after a successful claim, not when `stage_and_schedule` returns
   `staged:true`. If the user cancels in the window, the kind must **not** graduate,
   and the next same-kind order gets the read-back again. This is the core of the
   fix: *state advances on execution, never on first sight.*

5. **The GRADUATION notice moves to the fill receipt.** Today it is appended in the
   staged render (`src/tool.rs:1589`, `graduating.then_some(GRADUATION_NOTICE)`).
   Under the new model the read-back does not carry it (there is no send yet). Append
   `GRADUATION_NOTICE` to the **receipt that reports the fill** (the message emitted
   when the staged order completes), and only on the first fill of each kind. Copy
   unchanged (`src/reporting.rs:584`).

### Acceptance (E1)

- `buy $200 of WETH` on a fresh account → a read-back message containing `$200`,
  `WETH`, a `~[base qty]` ≤6 dp, and `~[mark]`; a single `[Cancel]` control; **no**
  "yes", **no** `[Yes, send it]`, **no** `CONFIRM_ONCE_MESSAGE` text.
- No cancel within 3s → order sends → receipt carries the GRADUATION notice.
- Cancel within 3s → order does not send → kind not graduated → a second
  `buy $200 of WETH` shows the read-back again.
- A fresh session's first same-kind order still shows the read-back (no cross-session
  auto-execute). Add a test that seeds `action_kind_status` as "seen but not
  confirmed" and asserts the next order still stages with a read-back.
- The b1-quote-weth golden (which failed because the confirm-once message had no `$`)
  now passes: the read-back contains the dollar figure.

---

## E2 — Human-unit quantity in the receipt / staged line [P1]

### The defect (FINDINGS R4-3 / DEFECTS D2)

`Staged `0.0799427609831360745706074451` WETH` — 28 decimal places. Root cause is
`src/tool.rs:1578`: the staged `happened` string interpolates
`resolved.base_qty.normalize()` **raw**, bypassing the formatter that already exists.

### Fix

- In the staged/receipt `happened` string (`src/tool.rs:1575–1591`), never emit a raw
  `Decimal`. Render **either** the dollar size (preferred, matches the exemplar
  `Bought $200 WETH`) via `format_money(resolved.notional, false)` → `$200`, **or** a
  ≤4-dp base quantity. Note `format_base_qty` (`src/size.rs:285`) rounds to **6** dp;
  the receipt copy calls for **≤4** dp. Add a receipt-facing quantity formatter (4 dp,
  trailing-zero-stripped) rather than reusing the 6-dp one, e.g.:

  ```rust
  pub(crate) fn format_qty_human(qty: Decimal) -> String  // ≤4 dp, normalized
  ```

  and use it wherever a base quantity reaches a user-facing string.
- The design copy (workflows.md → RECEIPT) now specifies:
  `What happened · Sent `$200` of WETH spot (~`0.08` WETH), filled at `[#]`.` — dollar
  size primary, base qty parenthetical and optional. Match that shape. Fills/marks
  are **prices**, render as the tool gives them; only *quantities* get the human
  formatter.
- `resolved_size.to_json()` (`src/size.rs:250`) already exposes `notional_rendered`
  (a formatted `` `$X` ``) and a 6-dp `base_qty`; consider having the render read from
  there for a single source of truth.

### Acceptance (E2)

- No user-facing message ever contains a base quantity with more than 4 dp. Add a
  unit test on `render_receipt`/staged render asserting the digit-after-decimal count
  ≤ 4 for the quantity token, and that the dollar size is present.

---

## E3 — Wire `unfulfillable_kind`; split `unclear` from `cant`/`near_match` [P1]

### The defect (FINDINGS R4-8 / DEFECTS D3)

The build still warns `unfulfillable_kind` is **never used** in
`src/speech_ontology.rs` — the CANT classifier was written and never called. So
`render_lookup` returns the single `unclear` string
(*"I didn't catch an instrument in that. Say buy, a size, and the name."*) for three
distinct situations:

- `buy me $50 of beef` → should be **CANT** (out-of-universe asset → three-line wall).
- `no, make it 4500` → should be **CORRECTION** (amend a still-open instruction).
- `my favourite colour is teal` → should be **UNCLEAR** (non-trade register).

### Fix

1. **Call `unfulfillable_kind`** from `render_lookup` (wherever `render_lookup`
   classifies input — `src/lookups.rs` / `src/tool.rs render_lookup`). A trade-shaped
   ask naming an out-of-universe asset must classify as `cant`, returning the
   three-line wall (design copy: workflows.md → CANT §6.21), **not** `unclear`.
2. **Give `unclear` its own distinct string** — the non-trade register, canonical in
   workflows.md → `## UNCLEAR (§6.21a)`:

   ```
   I didn't catch that — I trade crypto spot, perps, and lending on World.
   Say what you'd like to do, or `/p` for positions.
   ```

   Replace the trade-shaped `"Say buy, a size, and the name."` string wherever
   `render_lookup` emits it for the `unclear` branch. `unclear` must never assume the
   user tried to buy something.
3. **`cant` / `near_match` keep the three-line wall** (unchanged copy). Only the
   `unclear` fall-through gets the new non-trade string.
4. Corrections (`no, make it 4500`) should route to the correction path, not
   `unclear` — verify the existing `looks_like_watch_correction`
   (`src/tool.rs:1636`) / correction handling covers a bare amend to an open trade
   instruction; if it doesn't, note it, but the primary fix here is (1)+(2).

### Acceptance (E3)

- `cargo build` emits **no** `unfulfillable_kind is never used` warning.
- `buy me $50 of beef` → CANT three-line wall (quote · category · what World trades),
  not the trade-shaped clarification.
- `my favourite colour is teal` → the UNCLEAR non-trade string, containing
  "I trade crypto spot, perps, and lending" and offering `/p` — never "say buy, a
  size, and the name."

---

## E4 — `instruction_id` semantics [P2, clarify + document]

### The observation (FINDINGS R4-2 / DEFECTS D7)

`yes` → `execute_world_order(instruction_id=f9b02ef5-…)`; then `buy $150 WETH` →
`execute_world_order(instruction_id=f9b02ef5-…)` — the **same** token for a
**different** order. If it's a per-kind authorization token, fine; if it's a
per-order binding, an order could inherit another's confirmation.

### What to do

Under the E1 opt-out model this token's role changes: there is no "yes" turn to bind,
so a model-supplied `instruction_id` should no longer be the thing that skips the
gate. Decide and **document in code comments + the round-4 changelog**:

- Is `instruction_id` per-kind (authorization) or per-order (binding)?
- After E1, what (if anything) still consumes a model-passed `instruction_id`?
  Today `confirm_once_gate` early-returns `None` when one is present
  (`src/tool.rs:1787–1795`) — once the gate is gone, remove or repurpose that check so
  a stale/mismatched id can't silently skip the read-back.
- Ensure the staged order's cancel/flush is keyed to the **specific staged instruction**
  (`src/staged.rs`), so `[Cancel]` cancels *that* order and nothing else.

### Acceptance (E4)

- A comment at the `instruction_id` field (`src/tool.rs:284`) states its scope.
- No path lets a model-supplied `instruction_id` skip the 3s read-back for a new order.

---

## H1 — `check_no_foreign_digits` is a no-op [P1, regression net]

The harness's most important honest-numbers test compares the message's digits
against `tool_blob(stdout) + stdout` — but `stdout` **contains** the message, so every
message digit trivially matches itself. It can never flag a fabricated figure; this is
why probe 12 "passed" while the model invented `$328.73` (R4-6 run B).

**Fix:** compare the reply's digits against **tool output only** (the structured tool
results / tool stdout channel), **excluding the reply text itself**. Any digit in the
user-facing reply that does not appear in a tool result for that turn is a foreign
digit → fail. Locate the reply/tool-output split in the harness
(`tests/` / `scripts/eval-adherence.sh` / `run.py`) and pass the two as separate
inputs. Add a self-test: a synthetic turn whose reply contains a digit absent from tool
output must FAIL the check (guards against the no-op regressing).

**Acceptance:** re-running the R4-6 fabricating case (model computes `$328.73` from
`get_world_account({})` without the `share` field) is flagged as a foreign-digit
failure.

---

## H2 — `run.py all` hangs on exit [P1, hygiene]

An orphaned PTY child (`aomi-run --max-turns 80`) survives `ReplSession.close()`;
`results.json` is never rewritten (stuck at stale content; results live only in the
log). **Fix:** in `ReplSession.close()`, hard-kill the child **process group**
(`os.killpg(os.getpgid(pid), SIGKILL)` after a SIGTERM grace), and ensure
`results.json` is flushed before exit. **Acceptance:** `run.py all` exits cleanly and
`results.json` reflects the run just completed.

---

## H3 — G5_conditional golden pinned to wrong copy [P1, hygiene]

Golden asserts `["I can watch", "I cannot place"]`; the runtime correctly says
*"…has to be signed on World"*. The **golden** is wrong, not the agent. **Fix:** update
the G5_conditional golden to match the correct runtime copy (align it with the current
G5/conditional boundary string). Do not change the agent to match a stale golden.
**Acceptance:** g5-conditional passes against the correct runtime string.

---

## Verification checklist (run before opening the PR)

- [ ] `cargo build` — zero warnings, specifically no `unfulfillable_kind is never used`.
- [ ] `cargo test` green, including the new E1/E2/E3 tests.
- [ ] Live REPL (`aomi-run … --provider openrouter`), fresh account:
      first `buy $200 of WETH` → read-back with `$200` + `~qty` + `~mark` + `[Cancel]`,
      no "yes"; no-cancel → fill receipt with graduation notice; cancel → next same-kind
      order re-shows the read-back.
- [ ] `buy me $50 of beef` → CANT wall; `my favourite colour is teal` → UNCLEAR string.
- [ ] `what's 20% of my portfolio?` (companion copy already routes to `share`) — confirm
      the `share` field path returns the tool-computed figure; H1 now flags the
      manual-arithmetic run.
- [ ] Receipt/staged line never shows >4-dp quantity.
- [ ] `run.py all` exits clean; `results.json` rewritten.

## Payload note (design side, FYI)

The copy half grew the skill payload from **49,081 → 54,340 bytes** (+5,259): six
additive blocks (CONFIRM-ONCE flow, UNCLEAR flow, floor-sign-off guard, percent→share
routing line, opt-out Execute-class note, refusal-freshness rule, plus a CONFIRM-ONCE
exemplar). No net-shrink was in scope for round 4. Flagging for budget tracking.
