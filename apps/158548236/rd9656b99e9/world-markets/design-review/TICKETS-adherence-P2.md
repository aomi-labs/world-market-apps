# Adherence P2 tickets

Recorded 2026-08-26 with the adherence-wiring pass. Build later.

---

## P2-1 · Hosted per-turn injection of `turn-contract.md` (stage 2)

**What.** Inject the verbatim contents of `src/skill/turn-contract.md` (679 tokens / ≤ 2,700 B) as the **final pre-generation content on every turn**, after history and tool results, **in addition to** its static last-position in the hosted `skill = { ... }` section list.

**Why.** Round 1's clearest failures were **turn-1-after-`/reset`**, not late-session decay. Tool schemas (~35 K tokens) and live state sit between the rulebook and generation at every depth. Static last-position (this PR, stage 1) is the weaker recency fix that round 2 will actually test. If local probes 6/9 still narrate under static recency, that is direct evidence to prioritize this ticket — not a reason to reopen the `.md` payload.

**Acceptance.**
- Dump one hosted Telegram request (staging) and confirm the contract is the final pre-generation content.
- Injection is verbatim (no paraphrase, no truncation).
- Applies on every turn, including turn 1 after `/reset`.
- Does not replace the static last-position copy until this injection is verified hosted-side.

This repo (plugin, aomi-sdk 4.0.0) has no per-generation suffix hook (`skill.hooks` is empty). The work lives in the hosted backend.

---

## P2-2 · History hygiene

**What.** When a turn completes, replace its raw tool JSON in hosted history with a one-line digest (tool name + key figures), keeping the rendered message.

**Why.** The rules forbid reusing stale figures, so nothing usable is lost. This removes both dilution and stale-number temptation while the ledger carries relational memory. (DIAGNOSIS C-b.)

**Acceptance.** Hosted history after a completed turn contains the user message, the rendered assistant message, and a digest line per tool call — not the raw JSON payload.

---

## P2-3 · Dollar-denominated order args

**What.** Accept dollar size on order tools (or a server-side `quote_size` step) so the model never computes `quantity = dollars ÷ mark`.

**Why.** Today every action turn *begins* with model arithmetic the honest-numbers law forbids in prose, and it leaks (round-1 probe 11: `$15,410`, `7.7%`). (DIAGNOSIS C-c / S2.)

**Acceptance.** `buy $200 of WETH` reaches execute without the model dividing by the mark; quantity is derived deterministically server-side.

---

## P2-4 · Conditional exemplar injection

**What.** Class-conditional injection of the single matching exemplar from `exemplars.md`. Exemplars may be removed from the static payload **only after** conditional injection is verified in *both* runtimes.

**Why.** Static exemplars cost payload. Conditional injection is tighter. The **dev REPL reads only `COMPOSED`**, so a hosted-only injection would silently drop exemplars from the runtime where probes re-run.

**Acceptance.** Both `aomi-run` (`COMPOSED`) and hosted inject the matching exemplar; a probe on each runtime shows the exemplar present; only then is `exemplars.md` removed from the static lists.

---

## P2-5 · Session-typed composition

**What.** Compose `guest.md` / `share.md` (~4 KB of different-register copy) only for guest-capable sessions. Funded sessions skip them.

**Why.** DIAGNOSIS B-2. Also: **hosted currently omits `guest.md`/`share.md` entirely** (pre-existing). Telegram is where `start=g_` / `start=ref_` guests arrive. Owner to confirm whether guest copy is composed elsewhere hosted-side — if not, this is a production gap, not just a payload-savings ticket.

**Acceptance.** Funded sessions do not receive guest/share copy; guest-capable sessions do, in both runtimes.

---

## P2-6 · C-3 risk-score determinism

**What.** Same live state must not yield `4.1` vs `4.2` minutes apart. Tracked in the SPEC; not part of the wiring pass.

**Why.** Blocks trusting eval 4a-2 on risk figures until fixed. Round-1 D5.

**Acceptance.** Two `r` lookups on unchanged state return the identical 0–10 score string.

---

## P2-7 · aomi-sdk app-skill token budget vs payload

**What.** aomi-sdk 4.0.0 `AppSkillManifest::validate` refuses skills over 8,000 tokens (chars/4). The design-agent payload is already ~10,067 on the pre-wiring hosted list (workflows.md alone is ~4,828). Wiring `exemplars.md` + `turn-contract.md` is ~11,028. `aomi-build compile` will refuse the hosted artifact until the budget is raised or the payload is cut.

**Why.** P0 requires those two sections in the hosted list. Copy cannot be edited in this pass. The budget is an SDK constant (`APP_SKILL_TOKEN_BUDGET`).

**Acceptance.** Either (a) aomi-sdk raises the app-skill budget above the composed payload, or (b) the design agent cuts the static payload under 8,000 hosted tokens *without* dropping `turn-contract.md` last-position or `exemplars.md` until P2-4 lands. Hosted compile is green either way.

