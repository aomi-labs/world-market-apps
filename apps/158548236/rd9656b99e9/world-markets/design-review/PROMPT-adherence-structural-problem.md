# Prompt for a more powerful agent — aomi prompt-adherence structural problem

You are being asked to diagnose and fix a **prompt-engineering / instruction-adherence
problem** in a production AI trading agent (codename **aomi**, product: World Markets).
The agent's instructions are already correct. The problem is that the runtime model
**does not reliably follow them**, and we need you to find and fix the *structural*
reason, not to rewrite the individual rules.

Treat this as an instruction-architecture problem. Do not just reword the rules that
are being violated — if the structure is what's causing the drift, rewording one rule
moves the problem somewhere else.

---

## 1 · What the product is (minimum context)

aomi is an AI trading agent operated primarily through a Telegram thread. The user
speaks or types instructions ("buy $200 of ETH", "tell me if ETH drops below 3000",
"how am I doing?"); the model parses intent, calls deterministic tools for every
number and every mandate/risk check, and a policy engine — never the model — is the
final authority on what executes. The design canon is unusually strict: calm,
numerically explicit, screenshot-safe copy; no process narration; no hype; a fixed
message budget. Getting the *voice and behavior* exactly right is the product.

## 1.5 · The directional intention — what the prompt architecture is actually serving

Do not optimize for generic "instruction-following" in the abstract. The prompt exists
to produce one specific thing, and the structural fix should be judged against it:

**aomi is a relationship, not an app.** The product thesis, in the owner's words: for a
century the best trading interface money could buy was a *person* — a broker on a
recorded line who knew your book, watched the market while you lived your life, and
acted on your word within agreed limits. Screens replaced that relationship with
self-service vending machines: grids of buy/sell buttons where the user does all the
remembering. aomi gives the relationship back. **You tell it what you want to be true,
it remembers, organizes, watches, executes inside signed limits, and reports back.**
The interface is the exchange itself — speech and text — with screens demoted to what
they are actually good at: verification, records, and shapes.

Everything the prompt asks the model to do is in service of *sounding and behaving like
that specific counterpart* — a **precise financial operator working inside rules the
user controls.** Concretely, the persona is defined as much by what it is NOT: never a
black box, never an AI personality, never a financial influencer, never a salesperson,
never an engagement-maximizing chatbot. The house voice (calm, numerically explicit,
one-conclusion-one-explanation-one-decision, screenshot-safe, no process narration, no
hype, a fixed message budget) is not stylistic garnish — **it is the character of the
broker.** A good broker doesn't narrate that they're about to check the screen, doesn't
pad, doesn't upsell, doesn't celebrate a trade, and doesn't ask you for more money. The
lint rules encode exactly those restraints.

This matters for your diagnosis in a specific way: **when the model drifts off-spec, it
does not drift randomly — it drifts toward the generic assistant.** Every observed
failure is the model reverting to default-LLM behavior: narrating its process ("let me
first refresh your account…"), padding a leverage question into a four-paragraph essay
with a coaching line and a yield pitch, asking the user to supply a ticker instead of
stating a wall. Those are precisely the mannerisms of "a helpful chatbot," and they are
precisely the mannerisms the broker persona forbids. So the structural target is not
"make the model obey more rules" — it is **"make the broker persona the model's
stable, high-context default, so that under load it falls back to the operator, not to
the assistant."** The failure mode is a *persona collapse* under context pressure; the
fix must protect the persona, not just the rule list.

A second consequence: the relationship is **durable and cumulative.** The broker
remembers your standing orders across days; the product's memory is a ledger of every
instruction, and the agent is supposed to operate *from* that memory (resolve "cancel
the ETH one" against the ledger, not the chat scroll). Any fix that trades away the
agent's ability to carry that relational state across a long session to save context
would defeat the point — the long, loaded sessions where adherence currently collapses
are exactly the sessions where the relationship is supposed to be deepest. Solve
adherence *without* amputating memory.

Hold this as the success criterion: the fix works when a user, twenty tool-calls deep
into a session, still gets the same calm, terse, numerically-exact operator they got in
the first message — not a chattier, hedgier, more generic version of it.

## 2 · How the prompt is assembled (the structure under suspicion)

The system prompt is built in Rust at `src/preamble.rs::COMPOSED`. It is a static
`concat!` of a short role header plus, in this fixed order, these markdown files from
`src/skill/`:

```
role header (~1.2 KB, inline in preamble.rs)
instructions.md        (69 lines,  ~4.9 KB)   ← global voice, honest-numbers law, lint rules, budgets
lookups.md             (72 lines,  ~4.5 KB)
workflows.md           (246 lines, ~12.9 KB)  ← §6.1–§6.25: every message template and flow
action-rules.md        (48 lines,  ~4.8 KB)   ← symmetric rule pair, action classes, ladder
safety.md              (~0.8 KB)
reference/*.md          (8 files, ~5.5 KB total: atlas, products, account-model, venue,
                          dollarpower, guardian, notifications, strategy-brain)
guest.md, share.md     (~4 KB)
```

Total instruction payload: **~37 KB (~9–10K tokens)**, sitting at the very top of the
context window, before tool schemas, before live account state, before the
conversation. The order is: global rules first, then the big template file
(workflows.md), then the action rules, then references.

Two runtimes consume this. `aomi-run` (the dev REPL) reads only this single composed
string. The hosted Telegram backend composes the same sections separately. Assume both
see essentially the same instruction content in the same order.

## 3 · The evidence: rules that exist but are not followed

Every rule below is present in the prompt today. We confirmed each by reading the
source. Each is nonetheless violated at runtime. Transcripts are in
`design-review/session-transcript-2026-08-26.md`; the analysis is in
`design-review/FINDINGS-round1.md`. Representative failures:

1. **Process narration, explicitly banned.** `instructions.md` lint rule **E2 = "no
   process narration."** Runtime, on "short another $5k of WBTC," opened with: *"I'll
   help you short $5k of WBTC. Let me first refresh your account and preview this
   trade."* — then made the tool calls. That is exactly what E2 forbids, verbatim.

2. **Report-by-default / "no tap" ignored on the happy path.** `instructions.md` Voice
   §: *"The report is the default. Act on mandate, then report."* `action-rules.md`
   Execute class: *"execute … No tap."* Runtime instead narrates intent before acting
   (see #1) and, on ambiguous asks, silently picks a resolution and jumps to a preview.

3. **Unfulfillable-request flow not triggered.** `workflows.md` §6.21 defines a
   "heard-path" for out-of-universe asks (quote what was heard → wall → what World does
   trade → a terminal `can't` record), and a tool exists to produce it. On "buy me $50
   of beef" the model ignored the flow entirely and improvised a generic clarify: *"I
   need to clarify what you mean by beef … please provide the symbol (like BTC, ETH)."*
   (Note: this one also has a code-reachability component — see §5 — but the model also
   simply did not route to the specified flow.)

4. **Improvised numbers where a verbatim template exists.** `workflows.md` §6.25 gives
   the watch template: `Watching [SYM] for [predicate]. Now [#]. I won't buy or sell
   anything.` Runtime emitted a garbled comparison the template does not contain: *"Now
   `2465.71`, so that's `3000`."* The tell-only guarantee line (which IS in the template)
   came through fine; the model added an invented clause around it.

5. **Length/register drift on advisory turns.** The voice is specified as "one
   conclusion + one explanation + one next decision," with hard per-message character
   budgets in `instructions.md`. On "should I go all in on ETH with 10x leverage?" the
   model produced a 4-paragraph essay with a coaching line ("Your strategy focus should
   be…") and an unprompted yield pitch — well past the budget and drifting toward the
   salesperson persona the prompt explicitly bans.

The pattern: **the more contextually loaded and open-ended the turn, the more the model
falls back on generic-assistant behavior and abandons the house rules.** Simple,
early-in-session, single-shot answers (the incapacity answer §6.1, the block copy §6.6,
the fallback) came through almost verbatim. The failures cluster on (a) action turns
that call tools, and (b) turns that arrive later in the session.

## 4 · The structural hypotheses we want you to test

We believe this is an instruction-architecture failure, not a wording failure. Named
hypotheses, in rough priority — confirm, refute, or replace them:

- **H1 · Instruction distance + context dilution.** The ~10K-token rulebook sits at the
  very top of the window. A live turn observed at **in = 121,656 tokens** (≈110K of
  accumulated tool JSON + history between the rules and the user's actual message). The
  rules that survive are the ones for turns that happen early (low context) and/or need
  no tools; the rules that fail are for tool-calling turns deep in the session. This is
  the classic "lost in the middle" / instruction-attention-decay signature. If true, the
  fix is structural: move the load-bearing behavioral contract closer to the point of
  generation (e.g. a compact always-on contract injected just before the model responds,
  a per-turn reminder, or re-emitting the relevant §6.x template into the turn), not
  longer rules at the top.

- **H2 · The rulebook is too large and too uniform to attend to.** 587 lines / 37KB of
  dense, equally-weighted prose, much of it conditional templates the model must
  *select among* on every turn (workflows.md alone is 25 numbered flows). There is no
  salience gradient — E2 (a hard voice rule) reads with the same weight as a reference
  footnote. The model may be pattern-matching to "a trading assistant" rather than
  executing this specific spec because the spec is too big to hold as an active
  constraint. If true, the fix is compression + a small, sharply-prioritized core
  contract that is always obeyed, with the long tail demoted to on-demand reference.

- **H3 · Retrieval/selection failure, not obedience failure.** Several failures are the
  model not *finding* the right template (didn't route "beef" to §6.21; didn't use the
  §6.25 watch template) rather than knowingly disobeying. The 25 flows in workflows.md
  are addressed by number, with trigger conditions embedded in prose. The model may not
  be reliably matching a user turn to the correct flow. If true, the fix is a cleaner
  routing layer: an explicit "classify the turn, then apply exactly this template"
  step, or moving flow-selection out of free-form prose into something the model can
  deterministically key on.

- **H4 · Conflicting / self-cancelling directives.** At least one real tension exists:
  E5 says "no restated intent (the receipt 'Why' line is the only restatement)," which
  argues *against* the "heard-echo" (quote-what-you-heard-before-acting) primitive the
  product wants for trust. Where rules pull in opposite directions, the model resolves
  the conflict unpredictably. Audit the full rule set for other silent conflicts, and
  for rules stated as prohibitions ("never narrate") with no positive replacement
  behavior specified (models follow "do X instead of Y" far better than "don't do Y").

- **H5 · The prohibitions lack teeth because there's no worked positive exemplar.** The
  rules are largely declarative ("concise, calm, numerically explicit"; "no process
  narration"). There are templates, but few full, correct end-to-end *examples of a
  complete turn* (user message → tool calls → exact final message) for the model to
  imitate. Models comply far more reliably with a handful of gold exemplars than with a
  page of adjectives. If true, the fix is a small set of canonical few-shot turns
  covering the action classes.

You are not limited to these. If the real cause is something else (tool-schema
crowding, the two-runtime composition drift, ordering, temperature, a prompt-vs-
tool-result register clash), say so.

## 5 · Scope boundary — what is NOT your problem

Some round-1 failures have a genuine *code* cause and are being handled separately; do
not spend effort on them except where they interact with adherence:

- The "beef" flow is also partly unreachable in code (the heard-path isn't invoked from
  a trade-shaped sentence). The routing fix is code-side. **Your** concern is only: given
  the flow IS reachable, why doesn't the model choose it — i.e. H3.
- A liquidation-risk figure that jitters (4.1 vs 4.2 on identical state) is a
  determinism bug in the tool, not adherence.
- In the dev runtime, spot buys hit a fail-closed risk gate (a stubbed dependency), which
  masked the successful-execution path. That's an environment gap, not adherence.

## 6 · What we want back from you

1. A **root-cause diagnosis**: which hypothesis (or combination) actually explains the
   clustering of failures — especially the "simple/early turns comply, complex/late
   turns don't" pattern. Argue from the structure, not the symptom list.
2. A **structural fix to the prompt architecture**: concretely, how the instruction
   payload should be organized, weighted, sized, positioned, and/or re-injected per-turn
   so the load-bearing rules survive at high context. If you recommend a compact always-
   on core contract, draft it. If you recommend per-turn template re-emission or a
   routing step, specify the mechanism.
3. A **prioritized, checkable change list**, separating (a) changes to the `.md` /
   composed-prompt content and ordering from (b) changes to how `preamble.rs` assembles
   or injects it, from (c) anything that needs runtime plumbing (per-turn context
   management, mid-context reminders).
4. If the honest answer is "the rules are fine but the delivery mechanism is wrong,"
   say that plainly and fix the mechanism.

## 7 · Where to read (all paths under `/Users/lucas/Desktop/World/aomi/`)

- `src/preamble.rs` — how the prompt is composed (the `COMPOSED` concat and file order).
- `src/skill/instructions.md` — global voice, honest-numbers law, the E1–E5 lint rules,
  character budgets. **Read the E-rules and Voice section first.**
- `src/skill/workflows.md` — §6.1–§6.25, every message template and flow. This is the
  245-line file the model must select from every turn.
- `src/skill/action-rules.md` — the symmetric rule pair, the three action classes, the
  autonomy ladder, message anatomy.
- `src/skill/lookups.md`, `safety.md`, `reference/*.md`, `guest.md`, `share.md` — the rest
  of the payload.
- `design-review/session-transcript-2026-08-26.md` — the actual runtime transcript with
  token counts per turn (the in=45,545 → in=121,656 growth is the H1 evidence).
- `design-review/FINDINGS-round1.md` — the symptom analysis (what diverged, ranked).
- `design-review/SPEC-ledger-changes-md-vs-code.md` — the change catalogue that treated
  these as individual rule fixes; your job is to find the structural cause beneath it.

The design canon (voice, the action classes, honest-numbers, screenshot-safe copy, the
message budgets) is fixed and correct — do not relitigate *what* the rules say. Your
mandate is *why the model isn't obeying rules it demonstrably has, and how to
restructure the prompt so it does.*
