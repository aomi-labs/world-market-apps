# Handoff — design agent → prompt author (restructure execution report)

**Date:** 2026-08-26 · **From:** design agent (content remit: `src/skill/*.md`)
**To:** the agent that authored `PROMPT-adherence-structural-problem.md` and the
restructure brief · **Purpose:** you asked for the `.md` restructure; it is done
and verified. This is what I did, what I found wrong in the brief, what I
discovered mid-edit, and the implementation material you need to compose the
coding-agent prompt. **I did not write that prompt — §7 is raw material for it.**

**Remit boundary observed:** no `.rs`, tool, schema, or runtime file touched.
Everything below marked "code-side" is yours to route.

---

## 1 · Status: complete, all form gates pass

The owner withdrew the brief's "no net growth" gate mid-session (see §3.1) and
replaced it with five form gates. Measured, not estimated:

| Gate | Target | Actual | |
|---|---|---|---|
| `turn-contract.md` size | ≤ 800 tokens | **679** (cl100k) | PASS |
| Static skill payload | ≤ 50 KB | **49,081 B / 47.9 KB / 12,890 tok** | PASS |
| Tier 2 (`reference/*`) net growth | zero | **4,751 → 4,751 B (+0)** | PASS |
| Per-flow header block | ≤ 4 lines | **max 4** (PREVIEW); 20 of 28 are ≤ 2 | PASS |
| Growth is header/template/exemplar only | no free prose | held — see §2.4 | PASS |

Zero dangling cross-references. All 28 flows carry WHEN/DO/MODE/BUDGET. All five
round-1 failure inputs classify unambiguously (§4.2).

---

## 2 · What changed, file by file

### 2.1 New files (both written to disk, both dead weight until wired)

| File | Size | Contents |
|---|---|---|
| `src/skill/turn-contract.md` | 2,621 B / 679 tok | persona + anti-personas; 4-step turn algorithm; first-match-wins routing table; five lint rules in do-form; precedence ladder |
| `src/skill/exemplars.md` | 2,167 B / 591 tok | 4 worked turns: ACTION-happy (RECEIPT), ACTION-blocked, CANT, WATCH |

Every element the brief listed as mandatory survived. Two deliberate deviations
from the brief's A-1 draft, both additive:

- Routing table splits ADVISORY into three explicit destinations
  (VERDICT / EXPLAIN / SIM) instead of one `ADVISORY` label, because the brief's
  own A-4 creates three separate flows with different MODEs and DO sequences. A
  single `ADVISORY` label would have re-created the retrieval ambiguity (H3) the
  split exists to remove.
- Step 2 names `get_world_tasks` as the first tool on non-lookup turns, so the
  relational-memory rule (brief §2, "do not delete") is stated in the kernel
  rather than only in the flow file it was previously buried in.

### 2.2 `workflows.md` — rebuilt (12,921 → 19,311 B)

- **28 flows**, each `## SLUG (§6.x) — title` + a 1–4 line
  `WHEN: · DO: · MODE: · BUDGET:` header above the **unchanged** canonical
  template. Old §-numbers retained as parenthetical aliases for one release.
- **§6.25 split** into `RESEARCH (§6.25a)` / `WATCH (§6.25b)` / `TASKS (§6.25c)`,
  one MODE each. This was the garbled-watch-line root cause.
- **Three advisory flows restored** — `ADVISORY-EXPLAIN (§6.22)`,
  `ADVISORY-SIM (§6.23)`, `ADVISORY-VERDICT (§6.24)`. They were referenced from
  `action-rules.md` and absent from the file.
- **`CORRECTION (§6.26)` added** (M-9).
- **CANT (§6.21)** now carries the three-line wall body explicitly; previously it
  only described the tool contract, which is why the model improvised.

### 2.3 Tier 1 edits

- `instructions.md` (4,873 → 5,030): terse-lookup section deleted (deduped to
  `lookups.md`); pointer to the turn contract added; **M-1** (E2 anti-pattern
  verbatim: "never 'let me…' / 'I'll first…'") and **M-2** (E5 scoped heard-echo)
  folded into the lint rules; single tier-level reference banner added here.
- `lookups.md` (4,453 → 5,426): now the **single home** for terse-token dispatch,
  lookup formats, capability index, fallback string, chart/cancel tokens, and the
  voice-submit rule. The +973 B is the role-header plumbing arriving per A-3, not
  new prose.
- `action-rules.md` (4,799 → 4,988): fixed the wrong first-contact pointer
  (§6.21 → FIRST-CONTACT/§6.1); repointed §6.22/23/24 to the advisory slugs;
  added M-1's Execute-class line ("act, then report").
- `reference/*.md` ×8: **unchanged, zero bytes.** Banners were applied, then
  removed per the owner's amendment; the one tier-level banner lives in
  `instructions.md` §References.

### 2.4 Growth accounting (all mandated content, no free prose)

```
turn-contract.md + exemplars.md (new files)   +4,788
workflows.md (3 advisory flows, CORRECTION,
  §6.25 split, 28 header blocks)              +6,390
lookups.md (role-header dispatch relocation)    +973
action-rules.md (M-1 line + slug repointing)    +189
instructions.md (M-1/M-2 inline, net of dedup)  +157
reference/*.md                                     +0
                                       total  +12,497
```

---

## 3 · Criticisms of the brief — things you should fix in the next one

### 3.1 The size arithmetic was unachievable (owner accepted; gate withdrawn)

Brief §2 budgeted "+2.5 KB for contract + exemplars, find 2.5 KB in dedup,
end ≤ 36.6 KB." Brief §3 then ordered ~11 KB of *new* content: three restored
flows, a new CORRECTION flow, a three-way split, and headers on 28 flows. No
editing discipline closes an 11 KB mandate with a 2.5 KB dedup budget. The
constraint and the work items contradicted each other. **Lesson for the next
brief: budget additions from the work items, not from a target you set first.**

### 3.2 A-6's premise is wrong: dedup and relocation are opposite operations

A-6 says dedup "pays for A-1/A-2." It cannot, because A-3 in the same brief
orders the role-header plumbing *relocated into* `lookups.md`. Measured:

- Structural goal **achieved** — 6 previously-duplicated items are now
  single-homed (capability index ×3→1, fallback ×2→1, terse dispatch ×4→1, plus
  handshake copy, chart dispatch, voice-submit rule each ×1).
- Byte goal **missed** — net **+346 B** across the three deduped files excluding
  the relocation, **+1,319 B** including it. The savings were consumed by M-item
  copy (M-1, M-2) landing in the same files.

Both facts are true simultaneously. If a future brief wants bytes back, it must
name a *deletion* target, not a *consolidation* target.

### 3.3 M-7 collides with frozen typography canon — unresolved, needs your call

M-7 recommends displaying bare `p` in the fallback/index. `instructions.md`
§Typography (frozen canon) mandates shortcuts are **always** `` `/letter` `` in a
code entity on lookup/index/fallback surfaces. These are mutually exclusive.

**I kept `` `/p` ``** (canon outranks an M-item recommendation; my standing
instruction is to resolve against the owner before violating canon). The
underlying G5 finding is cosmetic anyway: a leading `/` is "accepted and ignored"
for matching, so `` `/p` `` mis-teaches nothing functionally. The genuine
inconsistency M-7 spotted — fallback and index disagreeing with each other — did
not exist; both already used `/p`. **Recommend: drop M-7, or amend the typography
canon explicitly. Do not leave it as a silent conflict.**

### 3.4 §6.24 was silently repurposed — confirm this is intended

Old `action-rules.md` cited §6.24 as *"Ranking ask → refuse, then one idle fact
from account + rates."* Your brief's slug list assigns §6.24 to
ADVISORY-VERDICT. I followed the brief. The ranking-refusal behavior is now
covered only implicitly by RECOMMEND (§6.2, "compare only on request") and is no
longer stated as an explicit rule anywhere. **Either confirm it is intentionally
absorbed, or give it a home.** This is the one behavior that lost its explicit
statement in this restructure.

### 3.5 Filename drift

The brief cites `design-review/DIAGNOSIS-adherence-structural.md` as parent. No
such file exists; the actual document is
`design-review/PROMPT-adherence-structural-problem.md`. Your DIAGNOSIS §-numbers
(C-a, C-e, §3) therefore don't resolve against anything on disk — I mapped them
by content. Fix the citation or ship the missing file.

### 3.6 Premise correction for the record — C-a is not about late-session decay

Your §4 H1 frames the fix as counteracting attention decay at high context
("in = 121,656 tokens", "lost in the middle"). **The transcript does not support
that as the primary mechanism.** Probes 6 (chatty preamble), 7 (beef → improvised
clarify), and 9 (garbled watch line) all failed on `[after /reset]` — turn 1 of a
fresh session, minimal context. The failures cluster by **turn type** (tool-calling
and open-ended), not by depth.

The correct justification for per-turn injection is **structural distance**: tool
schemas and live account state sit between the rulebook and the generation point
on *every* turn, at *every* depth. That gap is constant, not cumulative. This
matters practically — a fix justified by decay would be tested with long sessions
and might look unnecessary at turn 1, where the failures actually are. **Justify
C-a on structural distance; the round-2 probe set should include turn-1 probes.**

---

## 4 · Insights and traps discovered mid-edit

### 4.1 CANT must precede ACTION in the routing table, or "beef" gets trade-parsed

The ordering is load-bearing, not cosmetic: a trade-shaped sentence naming an
out-of-universe asset matches *both* rules. Verified in the shipped file — CANT
at char 604, ACTION at 677. Any future edit that reorders the routing table
silently reintroduces the round-1 beef failure. Worth an assertion (§6.3).

Same for ADVISORY-VERDICT before ADVISORY-EXPLAIN (703 < 742): "should I go all
in…" matches both "should I" and "explain/compare" shapes, and VERDICT is the
landing pad probe 11 lacked.

### 4.2 Routing verification, on paper

| Input | Class | First-match rule |
|---|---|---|
| `short another $5k of WBTC` | ACTION → PREVIEW/BLOCK | trade/size instruction |
| `buy me $50 of beef` | CANT | trade-shaped, asset not in universe |
| `tell me if ETH drops below 3000` | WATCH | "tell me if / when…" |
| `should I go all in on ETH with 10x leverage?` | ADVISORY-VERDICT | "should I…" |
| `asdkjfhaslkdjf` | FALLBACK | unparseable |

(ACTION is a kernel *class* that dispatches to PREVIEW/RECEIPT/BLOCK; there is
deliberately no flow named ACTION.)

### 4.3 The real account-17 watch figure would have taught the bug

Round-1 probe 9 set "ETH below 3000" when the live mark was `2465.71` — **the
condition was already true at creation.** Using the observed figure in the WATCH
exemplar would have modeled a silent 30-day arm of an already-true watch, i.e.
canonized D5. I used `3418.22` (illustrative, clearly marked) so the exemplar
teaches the clean arm, and cross-referenced the already-true branch beneath it.
**Any exemplar built from a transcript inherits that transcript's defects — check
each figure against the rule it is supposed to demonstrate.**

### 4.4 WATCH is `MODE: PASTE` but the already-true branch is a choice

The already-true branch (M-5) requires the model to select between two shapes
based on a tool flag — a compose-like decision inside a paste flow. I kept both
shapes fully verbatim-pasteable so PASTE still holds literally, and the selection
is stated as a tool-field condition, not a judgment. Flagging because your
acceptance check says "no flow mixes PASTE and COMPOSE" — this one is the
boundary case, and it passes only under that reading.

### 4.5 Two PASTE flows depend on tool behavior that does not exist yet

- **CANT** assumes `render_lookup` classifies "beef" as `cant`/`near_match` and
  returns a `message`. Per SPEC C-1 it currently returns `unmatched` → `None` and
  falls through to the LLM. **Until C-1 lands, CANT is unreachable from a
  trade-shaped sentence and both the flow and its exemplar are aspirational.**
- **WATCH** renders `now` and `already_true` (SPEC C-2). Neither field exists.
  The `now` line is conditional ("if the tool did not return a `now` mark, omit
  it"); the already-true branch cannot fire at all until C-2 ships.

### 4.6 Per-flow BUDGET lines duplicate the canonical budget table

`instructions.md` holds the canonical character-budget table; the 28 flow headers
now restate the relevant number. This is deliberate redundancy for routing
salience, but it *is* duplication and it will drift if one side is edited alone.
Your call whether to keep it. If kept, the budget table should be labeled the
source of truth and the header numbers treated as derived.

### 4.7 Exemplars must stay in the static payload (owner ruling)

I proposed moving exemplars to class-conditional per-turn injection to recover
2.2 KB. The owner rejected it on a correct premise I had missed: **`aomi-run`
reads only `COMPOSED`**, so conditional injection may never reach the runtime
where round-2 probes re-run. Exemplars stay static. Class-conditional injection
is specced as a follow-up in §5.3 with an explicit removal condition.

---

## 5 · Implementation material for the coding-agent prompt

Raw material, not a finished prompt. Each item: what · where · why · verify.

### 5.1 BLOCKING — composition (my files are inert without these)

**(a) Add both new sections to `COMPOSED` in `src/preamble.rs`.**
`exemplars.md` after `action-rules.md` and before `reference/*`;
`turn-contract.md` **last, after `share.md`** — it is designed to be the final
thing read before generation.
*Verify:* both filenames appear in `COMPOSED`; `cargo build` succeeds (an
`include_str!` on a missing path is a compile error, so a green build proves
inclusion).

**(b) Replace the inline role header.** Current text is ~946 chars of dispatch
trivia in the highest-attention position. Replacement (verbatim):

```
You are the World Markets Agent: a precise financial operator working inside rules the user signs on World. You run the portfolio on World Markets (UniFi testnet CLOB, chain ID 2092151908), primarily via Telegram.
For a century the best trading interface money could buy was a person — a broker on a recorded line who knew your book, watched the market while you lived your life, and acted on your word inside agreed limits. You are that counterpart, not a grid of buttons and not a chatbot attached to an exchange.
Never an assistant, influencer, salesperson, or narrator.
Tools supply every live fact and every mandate check. The deterministic policy engine — never you — decides what executes.
The turn contract at the end of this prompt is the last word on every message: classify the turn, call tools in silence, send one message from the classified flow's template, and never write a number a tool did not return.
```

*Why:* DIAGNOSIS H2 (no salience gradient) and the persona-collapse thesis — the
first position should carry the persona, not terse-token dispatch.
*Verify:* header contains no tool names and no token-dispatch rules; every
relocated rule resolves per §5.2.

**(c) `ROLE_LEN` must be updated in the same edit.** `preamble.rs` has a
`#[cfg(test)] pub(crate) const ROLE_LEN` that **duplicates the role-header string
literally**. Replacing the header without updating this second copy leaves a test
constant measuring text that is no longer in the prompt. *Verify:* `cargo test`
green; the two literals are identical, or better, refactor to a single `const`
referenced by both.

### 5.2 Where each relocated role-header rule landed (audit trail for (b))

| Rule removed from header | New home |
|---|---|
| terse tokens `b p r a d` + word forms | `lookups.md` §Terse tokens |
| `{ticker} d\|w\|m` chart, lone-`d`, `clear charts` | `lookups.md` §Chart & cancel tokens |
| `cancel task {id}` skip-LLM | `lookups.md` §Chart & cancel tokens |
| voice/text submits, Mini App never submits | `lookups.md` §Chart & cancel tokens |
| `open_instructions` before confirm/buy/sell/watch | `lookups.md` (same section) + kernel step 2 |
| "never ask what they meant" / "never list capabilities" | `turn-contract.md` precedence ladder (scoped to LOOKUP) + `lookups.md` §Hard rules |
| `/help` is host-reserved | `lookups.md` §Capability index |

Nothing was dropped. The `/help` and never-ask rules are now *scoped* rather than
global — see the precedence ladder's last clause, which resolves a real conflict:
"never ask what they meant" was previously stated globally but must not block an
ACTION turn's one legitimate clarifying question.

### 5.3 Runtime — per-turn injection (the highest-value item)

Inject `turn-contract.md` immediately before generation, every turn, in the
hosted backend, in addition to its static position.

*Why (corrected justification — do not cite late-session decay):* tool schemas
and live account state sit between the rulebook and the generation point on every
turn at every depth. Round-1's clearest failures were at **turn 1 after
`/reset`** (probes 6, 7, 9). The contract is 679 tokens precisely so this is
affordable per turn. **State plainly in the coding prompt: without this, the
contract is just one more section at the top of a 13 K-token rulebook and loses
most of its value.**

*Follow-up, not now:* class-conditional injection of `exemplars.md` (send only
the exemplar matching the classified turn). **Condition for removing exemplars
from the static payload: injection verified working in BOTH runtimes** — the dev
REPL reads only `COMPOSED`, so a hosted-only injection would silently drop
exemplars from the runtime where probes re-run.

### 5.4 Tool-contract dependencies I consumed (route to C-1/C-2)

| Flow | Field / behavior needed | SPEC item | Status if absent |
|---|---|---|---|
| WATCH (§6.25b) | `set_world_watch.now` | C-2 | `now` clause omitted; template still valid |
| WATCH (§6.25b) | `set_world_watch.already_true` | C-2 | already-true branch **cannot fire**; silent-arm bug persists |
| CANT (§6.21) | `render_lookup` → `cant`/`near_match` + `message` on a trade-shaped sentence | C-1 | flow **unreachable**; model improvises (round-1 probe 7) |
| RECEIPT (§6.5) | six-field receipt observable in dev | C-4 | receipt grammar + GRADUATION notice **unverifiable** |

RECEIPT's exemplar was written from spec, not observation, because RAPV
fail-closed masked every successful execution in dev (C-4). **It is the one
exemplar no one has seen the runtime produce** — treat it as unvalidated until
C-4 lands.

### 5.5 Owner decisions already taken (do not re-litigate)

- **M-8** ships in the *quiet* variant: `· on your ledger` appended to the `Next`
  line of the **first row-creating receipt of each kind per conversation** only.
  (O-3 resolved.)
- **M-9** ships as drafted.
- **M-2 / O-1** resolved to the **scoped** heard-echo (block/refuse/ask only),
  per the brief's E5 line. O-1 is closed.
- **O-2** (typed-send register) remains open but no longer blocks anything in the
  `.md` payload.

---

## 6 · Verification the coding agent can re-run mechanically

### 6.1 Form gates (exact commands)

```sh
cd /Users/lucas/Desktop/World/aomi/src
# Gate: static payload <= 50 KB  (expect 49081)
find skill -name '*.md' -exec cat {} + | wc -c
# Gate: Tier 2 zero growth       (expect 4751)
cat skill/reference/*.md | wc -c
# Gate: every flow fully headed  (expect 28 28 28 28)
grep -c '^## ' skill/workflows.md; for k in WHEN MODE BUDGET; do grep -c "$k:" skill/workflows.md; done
```

`turn-contract.md` ≤ 800 tokens (expect **679**, cl100k) needs a tokenizer:
`python -c "import tiktoken;print(len(tiktoken.get_encoding('cl100k_base').encode(open('skill/turn-contract.md').read())))"`

### 6.2 Reference resolution (expect empty output)

```sh
# every §6.x cited anywhere must exist as a workflows.md header
comm -23 \
  <(grep -rhoE '§6\.[0-9]+[a-c]?' skill/ | sort -u) \
  <(grep -oE '§6\.[0-9]+[a-c]?' skill/workflows.md | sort -u)
```

### 6.3 Assertions worth adding as tests

1. **First output token on an action turn is a tool call.** Given a trade-shaped
   in-mandate input, assert the first emitted element is a tool call, not text.
   Fails on round-1 probe 6's "I'll help you short $5k…". *This is the single
   highest-value regression test in the set.*
2. **No digits absent from this turn's tool results.** Extract every numeric
   literal from the final message; assert each appears verbatim in some tool
   result from the same turn. Fails on probe 11's fabricated `$15,410` and
   `7.7%`. Allow-list the shortcut tokens (`/b`…`/d`) and glyph counts.
3. **Routing-order invariant:** in `turn-contract.md`, `index_of("CANT") <
   index_of("ACTION")` and `index_of("ADVISORY-VERDICT") <
   index_of("ADVISORY-EXPLAIN")`. Cheap, and it guards §4.1 against future edits.
4. **Header integrity:** every `^## ` in `workflows.md` is followed within 4
   non-blank lines by a line containing all of `WHEN:`, `MODE:`, `BUDGET:`.

---

## 7 · What I recommend you put in the coding-agent prompt, in priority order

1. §5.1 (a)(b)(c) — composition + role header + `ROLE_LEN`. **BLOCKING.**
2. §5.3 — per-turn contract injection, justified on structural distance.
3. C-1 and C-2 (§5.4) — without them, CANT and the already-true watch are dead
   copy, and round 2 will "fail" on flows that cannot physically fire.
4. §6.3 assertions 1 and 2 — they encode the two failure classes that motivated
   the whole exercise.
5. C-4 — so the receipt path becomes observable and §5.4's unvalidated exemplar
   can be confirmed.

**Do not ask the coding agent to change any `.md` copy.** If something in the
payload is wrong, route it back to me — the copy is canon-bound and several
sentences are verbatim-frozen.

---

## 8 · Open questions for you (I did not decide these)

- **Q1 · M-7** — drop it, or amend the typography canon to permit bare `p`?
  (§3.3. I kept canon.)
- **Q2 · §6.24 ranking-refusal** — intentionally absorbed into RECOMMEND, or does
  it need its own home? (§3.4. The one behavior that lost an explicit statement.)
- **Q3 · Per-flow BUDGET restatement** — keep the redundancy for salience, or
  point to the canonical table? (§4.6.)
- **Q4 · Round-2 probe set** — should include turn-1-after-reset probes for the
  three flows that failed there, plus a long-session control to test whether
  depth matters at all. Right now we have no evidence either way, and §3.6 says
  the depth hypothesis was never the load-bearing one.
