# The Ledger as a First-Class Runtime Object — Product Design Spec

**Date:** 2026-08-26
**Author:** design (world-markets-design)
**Grounded in:** `SPEC-task-centric-redesign.md` (Task object, 5 types, 7 statuses,
the board), `AOMI_PRODUCT_HANDOFF.md` §3 (the loop, the trail, the `can't` row),
`FINDINGS-round1.md` (the observed runtime gap).
**Scope:** UX / interaction / product-surface changes ONLY. No code, no schema,
no architecture. Every change below is a change to what the user experiences.

---

## 0 · The problem this solves (one paragraph)

We already designed a beautiful Task Board (`w9-task-board.html`) — the surface
you *open* to inspect your instructions. But dogfooding round 1 showed the
runtime doesn't yet treat the ledger as its operating memory: the agent answers
each message from scratch, refusals and unfulfillable requests dissolve into
chat scroll, corrections aren't visibly captured, and there is no moment in any
turn where the user sees "this became a durable thing you can find later." The
Task Board is a **destination**; what's missing is the ledger as a **reflex** —
the thing the agent writes to on *every* turn and reads from *before* every
turn. This spec defines the UX changes that make the ledger first-class at
runtime: not a screen you visit, but the visible spine of every interaction.

**The one-line reframe:** *the board is where the ledger is inspected; this spec
is about making the ledger the thing the agent lives inside.*

---

## 1 · The governing principle: every turn produces a row

Today, only *some* turns leave a trace (a watch, a successful trade). The
first-class change is a single rule the user can feel:

> **Nothing the user says to aomi disappears. Every instruction, every refusal,
> every "I can't," every correction becomes a durable, findable row — or updates
> one that already exists.**

This is the costly-signal (Alchemy) that the entire product thesis rests on:
"everything you've asked me lives here." A ledger that silently drops even one
request can't carry that promise. The UX consequence is that **six turn-types
that currently vanish must now visibly land as rows.** They are the whole of
this spec (§4).

The user never has to *ask* for this. It is not a feature they turn on. It is
the ambient behavior of the counterpart — the way a good broker's blotter simply
*has* everything on it.

---

## 2 · The three moments where the ledger becomes visible in chat

The board is one surface. But "first-class runtime object" means the ledger is
present **in the chat itself**, at three moments in every turn, so the user
never has to leave the conversation to feel it working. These are the load-
bearing UX additions.

### 2.1 · The heard-echo (top of every actionable turn) — NEW, highest priority
Before any tool runs, before any block, aomi quotes back what it heard in one
line. This is the missing primitive from round 1 (D1). It is *the row being
opened, out loud.*

- **Copy shape:** `Heard: "{verbatim user words}."` — then a beat, then the action.
- On a **block**, the echo comes *first*, so the user is never told "no" before
  being shown they were understood.
- On **voice input**, the echo is the transcript — doubling as the "did it hear
  me right?" repair point (reply-to-correct, §4.4).

The heard-echo is not conversational filler ("I'll help you with that, let me
just…"). It is a record event rendered as a sentence. It replaces the chatty
preamble round 1 flagged (D2). **One line, then work.**

### 2.2 · The landing line (bottom of every turn that creates/changes a row)
After the action, one line names the row that now exists and how to find it:

- `→ On your ledger as "{short task name}." [View ↗]`
- For a refusal: `→ Logged as something World can't do yet. [View ↗]`
- For a correction: `→ Updated. Your instruction now reads "{new}."`

This is the Norman feedback loop closed at the ledger level: the user sees the
*durable consequence* of their sentence, not just the immediate action. The
`[View ↗]` deep-links to the exact row on the board — the bridge from operating
surface (chat) to inspection surface (board), stated in the spec as "say it in
chat, see it on the board."

### 2.3 · The pull-status reflex (any time, zero notification cost)
The user can, at any moment, type a bare `?` / `status` / `what's open?` and get
the board's summary strip *inline in chat* — never a push, always a pull:

> `3 active · 2 waiting · 1 needs you · 0 blocked.`
> `Needs you: "Buy $500 BTC if it drops 5%" — blocked at your floor.`
> `[Open your ledger ↗]`

This is what makes the fixed push budget (§7) feel generous instead of sparse:
the status is *always one word away*, so the agent never has to interrupt to
deliver it. The board is pulled, never pushed.

---

## 3 · What a row shows in chat vs. on the board (division of labor)

The spec's rule stands: **chat captures and notifies; the board inspects and
controls.** The first-class-runtime change is that the *same row* is now legible
in both, at different densities:

| | In chat (the reflex) | On the board (the inspection) |
|---|---|---|
| **Heard-echo** | verbatim quote, one line | the `What` field, quoted at top of the row |
| **Structured form** | one mono line: `When · Do · Within · Expires` | same, in the row's form block |
| **Status** | a word in the landing line | the colored pill |
| **Trail / provenance** | not shown (kept quiet) | full: *because you said it → what aomi did → result*, voice replayable |
| **Controls** | "reply to change it" | Pause · Edit-in-chat · Remove · History buttons |

**Design consequence:** the chat never renders the full trail or control buttons
— that is the board's job (Krug: don't make the user read a blotter in a chat
bubble). Chat shows the *minimum that proves the row exists and is correct*; the
board holds the *full record and the controls*. The deep-link is the seam.

---

## 4 · The six turn-types that must now land as rows

These are the concrete UX behaviors. Each is a turn that today vanishes and must
now produce or update a visible row. Ordered by how badly round 1 showed them
missing.

### 4.1 · The refusal / unfulfillable request → a terminal `can't` row [D3 — worst gap]
**Today:** "buy me $50 of beef" → generic "please provide a symbol." Vanishes.
**First-class behavior:**
1. **Heard-echo:** `Heard: "buy me $50 of beef."`
2. **Near-match check, shown only if it fires:** if "beef" phonetically collides
   with something tradeable (BIFI-class), aomi asks *once*, attribute-based:
   `Did you mean BIFI, or is "beef" not a market you meant?` — never auto-maps.
3. **Walls-as-fact reply (three lines, screenshot-safe):**
   > I heard "beef."
   > World doesn't trade beef, commodities, or physical goods.
   > World trades crypto spot, perps, and lending.
   (Category-level only. **Never** a substitute suggestion — no "did you mean
   BTC?" as a nudge to trade something else.)
4. **Landing line:** `→ Logged as something World can't do yet. [View ↗]`
5. **The row:** a terminal **`can't` row**, in its own zone on the board (never
   in Active/Waiting, never counted in the summary strip or heartbeat), **deduped
   per entity** (asking for "beef" five times = one row). Status word: `can't`.

**Why this is the highest-value change:** it is simultaneously (a) the trust
promise ("everything I asked is here, even the no's"), (b) the product-gap
roadmap for free (accumulated `can't` rows = what users want that World doesn't
offer), and (c) the fix for the single worst round-1 divergence. It is the
clearest expression of "first-class runtime object": *even a refusal is a row.*

### 4.2 · A clear instruction → an execute row with a receipt that names its task [D1/D7]
**Today:** trade jumps to a tool call; on success (untested in dev) the receipt
stands alone.
**First-class behavior:**
1. Heard-echo.
2. Execute (inside mandate, allowed-means-allowed — no friction, no substituted
   parameters, at most one concern line).
3. **Receipt restates the instruction AND names the row:**
   `Done — bought $200 WETH at $2,466. → On your ledger as "Buy $200 WETH." [View ↗]`
4. The row exists even for a one-shot immediate trade: it lands in **Done**, with
   the receipt as its first (and only) trail entry. A fire-and-forget trade is
   still a record.

**Design point:** a receipt that doesn't name a row is an orphan event — the old
"trading terminal" behavior. Naming the row is what converts an *event* into a
*task with a history*.

### 4.3 · A conditional instruction → a Waiting row, stated as tell-only or armed [G3, refined]
**Today:** watch creation works but carries a broken number line and chatty
preamble.
**First-class behavior:**
1. Heard-echo.
2. Row created in **Waiting**, with the honest-numbers form:
   `Watching WETH. Trigger: mark ≤ $3,000. Now $2,466.`
3. **The tell-only guarantee stays verbatim** (it's canonical): `This is a
   heads-up, not a trade. I won't buy or sell anything.`
4. **NEW — armed-vs-already-true check:** if the condition is *already true* at
   creation, the row does not silently wait 30 days. aomi says so:
   `That's already true now — ETH is at $2,466, below $3,000. Want me to tell you
   the next time it crosses back down, or did you mean a different level?` This is
   a Norman feedback fix: the user's mental model ("I'll hear when it drops") must
   match the system's ("it's already there").
5. Landing line names the row.

### 4.4 · A correction → the row is REPLACED, and the pair is captured [D-corrections]
**Today:** corrections are handled conversationally; nothing shows the user their
instruction changed on the ledger.
**First-class behavior (this is the spec's "fresh statement, never an edit"):**
1. `no, I meant $300` produces a **new statement**, not an in-place edit of the
   old one.
2. **The row visibly supersedes:** the old row moves to a struck/`superseded`
   state in its trail; the new row is current. On the board, History shows both,
   so the user sees *"you said 500, then corrected to 300."*
3. **Landing line:** `→ Updated. Your instruction now reads "buy $300 WETH." The
   $500 version is in this task's history.`
4. The say/do chain is thereby visible to the user as *their own* record — which
   is also, by construction, the correction-pair the data edge needs. The user
   never sees the word "training"; they just see an honest history.

**Reply-to-correct (voice + text):** because the heard-echo is a quotable message,
the user corrects by *replying to it* ("no, the staked one") — Telegram-native
reply-quote. The reply attaches to the same row; it does not start a new thread.

### 4.5 · The guardian acts → a row appears AFTER the fact, pre-explained [guardian inversion]
**Today:** not observed (stubbed), but the ledger implication is the point.
**First-class behavior:** the guardian is itself an **always-Active row** on the
board ("Keep my portfolio above my floor") — making the one place the agent acts
*first* and confirms *after* permanently visible. When it fires:
1. It acts (the one canon exception to confirm-first).
2. A row lands in the guardian task's trail, and a single push (guardian is
   exempt from all bundling) explains: `Your risk crossed your floor. I closed
   the smallest position that fixed it — sold $X WBTC. Risk now back to Y.
   → On your ledger. [View ↗]`
3. The row shows the cheapest-safe unwind that was chosen, so the user can audit
   *why that leg.* First-class means even the autonomous action is a legible row,
   not a mysterious after-the-fact alert.

### 4.6 · Gibberish / unparseable → NOT a row (the one exception) [G5]
**Today:** clean fallback ("I didn't catch that"). Keep it.
**First-class behavior:** unparseable input does **not** create a row. This is
the deliberate boundary that keeps the ledger meaningful — a ledger that logs
noise stops being "everything I *asked.*" The fallback stays: `I didn't catch
that — say what you'd like to do, or type ? for your open tasks.` (Note: unify on
one status verb — `?` not `/p` — per the round-1 slash inconsistency.)

---

## 5 · The read side: the agent operates FROM the ledger

First-class means the ledger is not just written to — it is the agent's working
memory, and the user can feel that it remembers. Three UX behaviors:

### 5.1 · Context-carry across turns
When the user says "make that $300" or "cancel the ETH one," the agent resolves
the referent **against the open ledger**, not against chat scroll. The UX proof:
the user can refer to a task by its plain-language name days later
(`pause the rebalance one`) and it resolves. This is the "relationship, not an
app" thesis made testable — a broker who remembers your standing orders.

### 5.2 · Duplicate/conflict surfacing at capture time
If a new instruction duplicates or contradicts an open row, aomi says so at
capture, referencing the existing row:
> You already have "Buy $500 BTC if it drops 5%" waiting. Replace it, or add this
> as a second trigger?
This is a Nudge structured-choice: it prevents the board becoming a graveyard of
near-duplicate rows (the anti-pattern the task spec §10 guards against).

### 5.3 · The brief reads the ledger, not the market
The daily/Sunday brief is reorganized as a **task-status digest** (already
specified): per-row — what fired, what's waiting, what needs you — never opening
with P&L. First-class runtime means the brief is *generated by walking the
ledger*, so "1,204 checks, nothing met your conditions, so I did nothing" is
literally the ledger reporting its own quiet. The labor line ("checks this week")
lives on each row and aggregates into the brief.

---

## 6 · Surface inventory — what changes on each product surface

| Surface | Change | New or modified |
|---|---|---|
| **Chat — every actionable turn** | Heard-echo line at top | NEW |
| **Chat — every row-creating turn** | Landing line + `[View ↗]` deep-link at bottom | NEW |
| **Chat — refusals** | Three-line walls-as-fact reply + `can't` landing line | NEW (replaces generic clarify) |
| **Chat — corrections** | "Updated. Now reads … / old in history" line | NEW |
| **Chat — any time** | `?` pull-status inline summary strip | NEW |
| **Chat — fallback** | Unify status verb to `?`; no row for gibberish | MODIFIED |
| **Board — zones** | Add a **`can't` zone** below Done: terminal, deduped, uncounted | NEW |
| **Board — row trail** | Correction history (superseded statements), guardian unwind rationale | EXTENDED |
| **Board — guardian** | "Keep my portfolio above my floor" as an always-Active row | NEW (per §12 Q5 recommendation) |
| **Board — empty state** | "You haven't asked me to do anything yet. Here's what I can do." | per task spec §10 |
| **Notifications** | Every receipt/alert ends with `→ On your ledger. [View ↗]`; digest walks the ledger | MODIFIED |

**No new standalone screen is required.** First-class-runtime is achieved almost
entirely through **chat-side reflexes + the existing board + one new board zone.**
That is deliberate: the ledger becomes first-class by being *everywhere in the
conversation*, not by being a bigger destination.

---

## 7 · What this must NOT become (anti-pattern guard, inherited + extended)

Carried from task-spec §10, plus new ones specific to runtime-visibility:

- **No row-count theater.** The landing line names the row; it never says "that's
  your 12th task!" No progress framing. The desired feeling on seeing a row land
  is *"good, it's recorded,"* not *"I'm being productive."*
- **The heard-echo is not chit-chat.** It is a quoted record, one line. It never
  editorializes ("great choice!"), never adds a second sentence of warmth.
- **The `can't` row never suggests a substitute.** Walls-as-fact names categories
  World trades; it must not become a cross-sell ("we don't trade beef, but ETH is
  up today"). That would convert a trust surface into a sales surface.
- **The pull-status is silent by default.** `?` is a pull. The agent never pushes
  status to demonstrate the ledger is working — visible restraint is the signal.
- **Landing lines don't fire on gibberish or on pure lookups** (`p`, `?`). Only
  turns that create or change durable intent get a landing line, or the chat fills
  with `→ on your ledger` noise and the signal dies.

---

## 8 · Behavioral principles → fears solved (this spec's specific claims)

| Principle | Where it lands here | Fear it solves |
|---|---|---|
| **Alchemy — costly signal** | Even refusals become rows; nothing is hidden or dropped | "Is it quietly ignoring things I asked?" |
| **Norman — feedback** | Heard-echo (understood) + landing line (recorded) close two gaps per turn | "Did it hear me, and did it keep it?" |
| **Norman — conceptual model** | One object (the row) for trades, watches, plans, refusals | "What kind of thing did I just create?" |
| **Nudge — reversibility** | Every row is pausable/removable; corrections supersede visibly | "Can I take it back / change my mind?" |
| **Nudge — mapping** | Heard-echo → structured `When · Do · Within` maps words to consequence | "What exactly will it do?" |
| **Krug — don't-make-me-think** | `?` answers "what's open?" in one line; chat stays minimal, board holds detail | "What's it up to right now?" |
| **Hooked — investment (non-manipulative)** | The accumulating ledger is the user's own record that makes the agent more valuable — no streaks, no rewards | "Why would I keep using this?" |

---

## 9 · The single test that proves it's first-class

A design is done when this sequence works end to end, in chat, with no screen
visit required:

1. User: *"buy me $50 of beef"* → three-line wall + `→ Logged as something World
   can't do yet.`
2. User: *"actually put $300 into ETH"* → heard-echo → (execute or block) →
   `→ On your ledger as "Buy $300 ETH."`
3. User: *"no, make it $500"* → `→ Updated. Now reads "$500." The $300 version is
   in history.`
4. User, an hour later: *"?"* → `1 done · 0 waiting · 0 needs you. 1 can't
   (beef).`
5. User: *"cancel the ETH one"* → resolves against the ledger, not chat scroll →
   `→ Removed "Buy $500 ETH."`

If every one of those turns leaves a correct, findable row — including the beef —
the ledger is a first-class runtime object. Round 1 failed steps 1, 3, and 5.

---

## 10 · Open questions for the owner

1. **Heard-echo on trivial turns.** Should a bare `p` lookup get a heard-echo? Recommend **no** — echo only on turns that create/change intent; lookups stay instant. (Prevents echo-noise.)
2. **`can't` row visibility to the user.** Should the user see the `can't` zone by default, or is it a quiet "earlier / more" expand? Recommend **visible but collapsed** — present enough to honor the promise, quiet enough not to read as a list of failures.
3. **Correction depth in chat.** The landing line names the immediate supersede. Should chat ever show more than one level of correction history, or always defer to the board's History? Recommend **defer to board** (chat stays one line).
4. **Pull-status verb.** `?` vs `status` vs `open` — recommend `?` as primary (fastest), with `status` as an accepted alias. Kill the `/p` vs `p` slash inconsistency from round 1 in the same pass.
5. **Does the guardian row show its floor number?** The guardian row is always-Active; showing "above 6.0" makes the floor legible but risks anchoring. Recommend **show it** — the floor is the user's own number and legibility outranks anchoring here.

---

*End of spec. Next: the specific per-turn copy set and the updated board mockup
adding the `can't` zone + guardian row + correction-history trail. Reconcile
against the owner before building.*
