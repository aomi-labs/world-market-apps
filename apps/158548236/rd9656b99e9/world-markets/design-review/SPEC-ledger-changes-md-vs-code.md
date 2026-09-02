# Ledger-First-Class — Change Spec: `.md` prompt files vs. codebase

**Date:** 2026-08-26 · **Commit observed:** 9cb9846
**Inputs:** `FINDINGS-round1.md` (runtime divergences), `SPEC-ledger-first-class-runtime.md`
(the UX target), v7 design zip (`design_handoff_voice_home_v7/`), v6 base
(`TASK_LEDGER_V6_*`), and a read of the live prompt + source tree.

**Purpose:** state exactly what changes go in the aomi skill `.md` prompt files
(design-agent remit — content, not code) and what changes go in the codebase
(coding-agent remit — Rust in `src/*.rs`, brain, tools). Every item cites the
round-1 finding it fixes and the canon it must satisfy.

---

## 0 · The key discovery that reframes everything

**Most round-1 divergences are the model failing to follow prompt rules that
already exist — not missing rules.** The runtime prompt (`src/preamble.rs::COMPOSED`
concatenates all `src/skill/*.md`) already specifies: report-by-default with no
tap (`instructions.md` Voice), no process narration (`instructions.md` E2), the
canonical incapacity answer (`workflows.md` §6.1), the block grammar (§6.6), the
watch tell-only line (§6.25), the unfulfillable `can't` report (§6.21), and
`get_world_tasks` as "first tool on every non-lookup turn" (§6.25).

So the work splits three ways, not two:
- **(A) Prompt reinforcement** — the rule exists but the model drifts; tighten/relocate it.
- **(B) Prompt gap** — a genuinely new behavior the prompt never specifies (heard-echo, already-true watch check).
- **(C) Code gap** — a tool/brain path that must exist or be reachable before any prompt can invoke it (the beef routing, the garbled watch number, risk-score drift).

A change that is (C) cannot be fixed in `.md` at all — and shipping only the
`.md` half would look like a fix while regressing on the next model run.

---

## PART 1 — CHANGES TO THE `.md` PROMPT FILES (design-agent remit)

> These are edits I can make directly. All obey the register rules
> (`instructions.md`: ≤160-char prose, no exclamation, statuses lowercase,
> screenshot-safe) and the character budgets. Illustrative numbers match the
> account-17 set.

### M-1 · Kill the chatty preamble (reinforce E2) — fixes D2
**Where:** `instructions.md` §Concise lint, and `action-rules.md` §Message anatomy.
**Change:** E2 currently reads "no process narration." It is being violated
("I'll help you short $5k… Let me first refresh your account and preview this
trade."). Make it unmissable and give the model the exact anti-pattern:
> **E2 — no process narration.** Never announce tool calls, plans, or steps
> ("let me…", "I'll first…", "let me refresh…"). The user sees the result, never
> the procedure. First token of an action turn is the conclusion or the echo —
> never a promise to act.
**Also** add one line to `action-rules.md` §Three action classes → Execute:
> No preamble before the tool. Do not say you are about to act; act, then report.

### M-2 · Add the heard-echo as an opt-in on ambiguous/high-stakes turns only — addresses D1 (with a caveat) [PROMPT GAP — needs owner call]
**Where:** `workflows.md`, new §6.2b; and reconcile `instructions.md` E5.
**Tension to resolve first:** E5 today says "no restated intent (receipt 'Why'
is the only restatement)." A universal heard-echo *contradicts* E5 and would add
a line to every turn — which the character budgets and the calm-by-construction
canon push against. The v7 design already resolves this on the Mini App side:
the **live transcript during LISTENING is the heard-echo for voice** (`README.md`
§button LISTENING — the transcript renders streaming ASR, and the drafted row
*is* the echo). So a chat-side text echo is only needed where the Mini App
transcript isn't the surface — i.e. **typed text and post-voice parse ambiguity.**
**Recommended scoped rule (not universal):**
> **Heard-echo — only when you are about to *block, refuse, or ask*.** If the
> next thing you do is deny, wall, or clarify, open with one line quoting what
> you heard, verbatim, before the no: `Heard: "{user words}."` On a clean
> allowed execution, do NOT echo — the receipt's `Why` line is the restatement
> (E5 holds). Never editorialize in the echo; one line, then the verdict.
**Why scoped:** it puts the trust primitive exactly where round 1 showed it
hurt most (the user was told "no" with no proof of hearing) without adding a
line to every happy-path trade or breaking E5/budgets. **Owner question O-1
below asks whether to go universal instead.**

### M-3 · Route unknown-asset asks through the heard-path BEFORE trade parsing — fixes D3 (prompt half)
**Where:** `instructions.md` (new bullet under Terse lookups / routing), and
`workflows.md` §6.21.
**Change:** the prompt never tells the model that a *natural-language trade
sentence naming a non-universe thing* ("buy me $50 of beef") must go through the
heard-path. It only wires `render_lookup`/`cant` to whole-message tokens. Add:
> **Unknown asset in a trade ask.** Before parsing any buy/sell/short whose asset
> is not in `list_world_assets` / the lexicon, call the heard-path
> (`render_lookup` with the user text). If it returns `cant` / `near_match`,
> paste `message` + `controls` verbatim (§6.21) — do NOT ask the user to supply a
> symbol, and never suggest a substitute asset. Only parse as a trade once the
> asset resolves to the universe.
**And** replace the observed failure copy. §6.21 must carry the three-line
walls-as-fact template explicitly (today it only describes the tool contract):
> `cant` report (paste from tool; shape):
> > I heard "{heard}."
> > World doesn't trade {category}.
> > World trades crypto spot, perps, and lending.
(Category-level only. No "did you mean BTC?" nudge.)
**Caveat:** this only works if the tool actually classifies "beef" as `cant`
(see C-1). The prompt change is necessary but not sufficient.

### M-4 · Watch: forbid the improvised comparison line; require the tool predicate verbatim — fixes D5 (garbled watch line)
**Where:** `workflows.md` §6.25 `set_world_watch`.
**Change:** the garbled "Now `2465.71`, so that's `3000`." is the model
inventing a comparison. Tighten the template and ban the improvisation:
> Paste the predicate and current mark from the tool, verbatim, in this shape
> only: `Watching [SYM] for [predicate]. Now [#]. I won't buy or sell anything.`
> Never compose your own comparison between the trigger and the mark. If the tool
> did not return a `now` mark, omit it — do not compute one.

### M-5 · Watch: already-true condition must be surfaced — fixes D5 (already-true watch) [PROMPT GAP, depends on C-2]
**Where:** `workflows.md` §6.25.
**Change:** add the state the prompt has no branch for. This can only render if
the tool reports it (C-2), so the prompt line is conditional on a tool field:
> If the tool reports the condition is already true at creation
> (`already_true: true`), do not arm silently. Say so and offer the real choice:
> > That's already true — [SYM] is at `[#]`, past your `[#]` level. Want the next
> > crossing, or a different level?
> > [Watch the next crossing] [Change the level]

### M-6 · dollarpower: always pair the ratio with its dollar translation — fixes D5 (bare 2.4×)
**Where:** `workflows.md` §6.13 (health) and §6.15 (dollarpower).
**Change:** §6.15 already gives the full translation, but §6.13 health emits a
bare `dollarpower [#]×`. Add to §6.13:
> When you cite dollarpower in the health card, keep it to the ratio only if the
> full segregated-÷-World translation is not in this turn's tool result; if it
> is, append the one-clause translation. Never gamify; never propose raising it.
(Leaves the honest-numbers law intact — no computed translation if the tool
didn't return it.)

### M-7 · Unify the status verb; kill the `/p` vs `p` inconsistency — fixes G5 note
**Where:** `instructions.md` §Terse lookups, `workflows.md` §6.19/§6.20, `lookups.md`.
**Change:** the fallback says try `/p` while the working token is `p`. Pick one
displayed form for the fallback/index and use it everywhere. Recommend bare
`p` (matches what actually works in-thread; the leading `/` is "accepted and
ignored" per instructions.md, so showing the slash mis-teaches). One-line sweep
across the three files.

### M-8 · The landing line (chat → ledger bridge) — supports the UX spec, PROMPT half
**Where:** `workflows.md` §6.5 receipt Next line, §6.21, §6.25.
**Change:** the v6/v7 design already makes the Mini App row the durable record,
and the receipt already ends with silence conditions. To make the *chat* carry
the "it's on your ledger" proof (UX spec §2.2) without new sends, append the
ledger reference to the existing final line where a row was created/changed:
> Receipts (§6.5) and watch/`can't` reports may end their existing Next/close
> line with `· on your ledger` (no new line, no button in-thread — the Mini App
> is where it's opened). Never on lookups or the fallback.
**Note:** this is copy-only; the deep-link/`[View ↗]` button is a Mini App
concern (v7 already renders rows), so no in-thread button is added — consistent
with "Mini App buttons never submit / chat = escalation only."

### M-9 · Correction copy: name the supersede in one line — supports UX spec §4.4
**Where:** `workflows.md`, extend §6.25 / add a §6.26 correction note.
**Change:** the trail already models "corrections are new events, never edits"
(v6 COPY sheet_footer). Give the chat side one line so the user sees it:
> On a correction ("no, make it $300"), do not silently re-parse. Confirm the
> supersede in one line: `Updated — now $300. The $500 version is in this
> task's history.` Then proceed under the new statement. One line; the full
> history lives on the ledger, not in chat.

**`.md` files touched:** `instructions.md`, `action-rules.md`, `workflows.md`,
`lookups.md`. All within design-agent remit (prompt copy, not code). I can draft
these as exact patches on your go.

---

## PART 2 — CHANGES TO THE CODEBASE (coding-agent remit)

> These cannot be done in `.md`. Each is the code half of a finding whose prompt
> half is above. Handoff to the coding agent; design finalizes the copy first.

### C-1 · Make the unfulfillable heard-path reachable from a natural-language trade — the code half of D3/M-3
**Files (indicative):** `src/cant.rs`, `src/speech_ontology.rs`, `src/tasks.rs`,
the brain `heard` endpoint, and wherever a trade intent is first classified.
**Problem:** `try_heard` exists and calls `speech_ontology::normalize_utterance`,
but it is only invoked on the token / share / cancel paths. "buy me $50 of beef"
is shaped like a trade, so it never reaches the heard-path; the LLM handles it
conversationally. Two things needed:
1. A pre-parse hook: when a trade intent's asset does not resolve to the
   universe/lexicon, run `try_heard` and, if it returns `cant`/`near_match`,
   short-circuit to the report (with `skip_llm` so copy is deterministic).
2. Confirm the ontology classifies out-of-universe food/commodity words as
   `cant` (and near-homophones like "beef"→BIFI as `near_match`), not `unmatched`
   (which returns `None` and falls through to the LLM). The 9cb9846 "skip-llm
   path for unfulfillable names" is the start; it needs to fire on this shape.
**Acceptance:** "buy me $50 of beef" returns the three-line wall + a terminal
`can't` ledger row (deduped per entity, uncounted in the heartbeat), with no LLM
substitute suggestion — matches v6 (the `can't`/terminal-row model) and the UX
spec §4.1.

### C-2 · Watch tool must report the current mark and an already-true flag — the code half of D5/M-4/M-5
**Files:** `src/tasks.rs` (`set_world_watch`), the watch/predicate evaluator,
possibly `src/reporting.rs`.
**Problem:** the tool returns a predicate the model then garbles into "Now X, so
that's Y", and there is no already-true signal, so a watch whose condition is
already met is armed for 30 days silently (round 1: ETH 2465 vs. ≤3000 trigger).
**Needed:** `set_world_watch` returns (a) a clean `now` mark field the copy
interpolates verbatim, and (b) `already_true: bool` computed at creation. The
prompt (M-4/M-5) renders both; it cannot compute either (honest-numbers law).
**Acceptance:** creating a watch whose condition is already true yields the
already-true branch copy, not a silent 30-day arm.

### C-3 · Stabilize the liquidation-risk figure across reads — the code half of D5 (drift)
**Files:** `src/liquidation_risk.rs`, `src/reporting.rs`, `src/lookups.rs`.
**Problem:** same account, minutes apart, returned risk `4.1` then `4.2` (and the
skill already flagged `$161.54` vs `$161.9159` for `p`). If the score jitters on
identical state, every number in the product inherits the distrust — fatal in a
financial tool. This is a determinism/rounding bug, not a copy issue.
**Needed:** a single deterministic source and fixed rounding for the 0–10 score
(and for lookup notionals). The prompt cannot fix this — the model only pastes
what the tool returns.
**Acceptance:** repeated reads on unchanged state return byte-identical figures.

### C-4 · RAPV fail-closed masks the happy path in dev — blocks verifying D1/D7, and Allowed-means-allowed
**Files:** `src/execution.rs`, `src/mandate.rs`, the RAPV/`preview_account_effect`
path, `evm-core` stub. (See existing `dev_artifacts/design-review/SPEC-DEFECT-B-post-trade-rapv.md`.)
**Problem:** every spot buy returned "cannot prove post-trade RAPV → floor fails
closed," so a successful execute→receipt was never observable in dev. We cannot
validate the receipt grammar, the confirm-once graduation notice (§6.4), or
size-based escalation until RAPV computes in the dev runtime.
**Needed:** a dev path where RAPV resolves (un-stub or seed evm-core, or a
computable post-trade state for the dev harness). Purely enabling — no
user-facing copy change.
**Acceptance:** "buy $200 WETH" in dev produces a receipt with all six §6.5
fields and (on first instance) the graduation line.

### C-5 · Landing-line / correction data on the ledger record — the code half of M-8/M-9 (mostly already present)
**Files:** `src/tasks.rs`, the ledger record + trail.
**Status:** LARGELY DONE. v6/v7 already model the trail ("BECAUSE YOU SAID IT"),
corrections-as-new-events, and the optimistic `with_aomi` row. Verify only that:
(a) a `can't` outcome writes a terminal row deduped per entity and excluded from
`summary.heldN`/the heartbeat (UX spec §4.1, v7 README heartbeat = needs_you +
executing + watching + paused — `can't` must not count); (b) a correction writes
a superseding event visible in History. If both hold, no code change — just
confirm in review.

---

## PART 3 — What v7 already implements (keep as suggestions, do not rebuild)

Per your note. My earlier `SPEC-ledger-first-class-runtime.md` proposed several
things v7/v6 already ship — recording so nobody re-builds them:
- **Heard-echo for voice** → v7 LISTENING live transcript + drafted row (README §button). My chat-side echo (M-2) is only the *typed/refusal* residue.
- **Pull-status / summary** → v6/v7 heartbeat strip + zone counts + launch fold ("SINCE YOU LOOKED"). A chat `?` summary is a nice-to-have, not required.
- **The trail / provenance** → v6 instruction sheet "TRAIL — BECAUSE YOU SAID IT", corrections-as-events. Done.
- **`can't` as a terminal row** → modeled in the ledger status set; the gap is *reachability* (C-1), not design.
- **Guardian visible** → v6 risk card "at a breach the guardian acts first, confirms after"; the always-Active guardian *row* remains a suggestion (owner Q5 in the UX spec).
- **Landing-line deep link** → v7 rows are already the durable record; in-thread we only append "· on your ledger" copy (M-8), no button (chat ≠ confirm surface).

---

## PART 4 — Owner questions (genuine trade-offs only)

- **O-1 · Heard-echo scope.** Scoped to block/refuse/ask turns (M-2, recommended — protects E5 and the budget), or universal on every action turn (stronger trust primitive, but adds a line everywhere and overrides E5)? Mutually exclusive.
- **O-2 · Typed-send register (inherits v7 Q-V7-a).** v7 shows voice drafts as `awaiting confirm` (warn) but typed sends as `with aomi` (accent). Unify typed to the same warn register, or keep the split? Affects M-8/M-9 copy.
- **O-3 · `· on your ledger` tag.** Append it to every row-creating receipt/report (M-8), or only to the first of each kind per conversation (quieter)? Trade-off: reassurance vs. repetition.

Everything else in Parts 1–2 is additive and I've taken it silently.

---

## Summary table

| # | Finding | Fix type | Where | Owner |
|---|---|---|---|---|
| M-1 | D2 chatty preamble | (A) reinforce | instructions.md, action-rules.md | design |
| M-2 | D1 no heard-echo | (B) gap | workflows.md, instructions.md | design + O-1 |
| M-3 | D3 beef routing (copy) | (A)+(B) | instructions.md, workflows.md | design |
| M-4 | D5 garbled watch line | (A) reinforce | workflows.md | design |
| M-5 | D5 already-true watch | (B) gap | workflows.md | design (needs C-2) |
| M-6 | D5 bare dollarpower | (A) reinforce | workflows.md | design |
| M-7 | G5 `/p` vs `p` | (A) sweep | instructions/workflows/lookups.md | design |
| M-8 | UX §2.2 landing line | (A) copy | workflows.md | design + O-3 |
| M-9 | UX §4.4 correction | (A) copy | workflows.md | design |
| C-1 | D3 beef routing (code) | (C) code | cant.rs, speech_ontology.rs, tasks.rs, brain | coding |
| C-2 | D5 watch mark + already-true | (C) code | tasks.rs, reporting.rs | coding |
| C-3 | D5 risk-score drift | (C) code | liquidation_risk.rs, reporting.rs, lookups.rs | coding |
| C-4 | D1/D7 RAPV fail-closed in dev | (C) code | execution.rs, mandate.rs, evm-core stub | coding |
| C-5 | UX §4.1 can't-row / correction record | (C) verify | tasks.rs | coding (likely done) |

*End. On your go I'll draft the exact M-1…M-9 patches against the `.md` files
(design-agent remit) and hand C-1…C-5 to the coding agent with acceptance tests.*
