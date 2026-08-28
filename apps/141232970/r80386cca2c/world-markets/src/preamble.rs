//! Composed system prompt for LLM runtimes that read only [`DynManifest::preamble`].
//!
//! `aomi-run` (aomi-sdk 4.0.0) does **not** fold `manifest.skill` sections into the
//! agent prompt — only this string is sent. The hosted Aomi backend composes
//! skill sections separately, so we keep the `skill = { ... }` block in `lib.rs`
//! for release validation and staging.

const SEP: &str = "\n\n---\n\n";

macro_rules! role_header {
    () => {
        "You are the World Markets Agent: a precise financial operator working inside rules the user signs on World. You run the portfolio on World Markets (UniFi testnet CLOB, chain ID 2092151908), primarily via Telegram.\n\
For a century the best trading interface money could buy was a person — a broker on a recorded line who knew your book, watched the market while you lived your life, and acted on your word inside agreed limits. You are that counterpart, not a grid of buttons and not a chatbot attached to an exchange.\n\
Never an assistant, influencer, salesperson, or narrator.\n\
Tools supply every live fact and every mandate check. The deterministic policy engine — never you — decides what executes.\n\
The turn contract at the end of this prompt is the last word on every message: classify the turn, call tools in silence, send one message from the classified flow's template, and never write a number a tool did not return."
    };
}

/// Role header for `aomi-run`. Hosted composition has no independent copy of this
/// string (allowed COMPOSED-only difference; see the parity test in `lib.rs`).
#[cfg(test)]
pub(crate) const ROLE_HEADER_FOR_TEST: &str = role_header!();

/// Shared core section names, in compose order, shared by `COMPOSED` and the hosted
/// `skill = { ... }` list. `turn_contract` is last in both runtimes.
///
/// Allowed differences (documented here so the parity test does not paper over them):
/// - role header: COMPOSED-only (`ROLE_HEADER`)
/// - `guest.md` / `share.md`: COMPOSED-only; hosted omits them (pre-existing)
#[cfg(test)]
pub(crate) const SHARED_CORE_SECTION_NAMES: &[&str] = &[
    "instructions",
    "lookups",
    "workflows",
    "action_rules",
    "exemplars",
    "safety",
    "atlas",
    "products",
    "account_model",
    "venue",
    "dollarpower",
    "guardian",
    "notifications",
    "strategy_brain",
];

/// Hosted `skill.sections` names in compose order. `turn_contract` is last.
#[cfg(test)]
pub(crate) const HOSTED_SKILL_SECTION_NAMES: &[&str] = &[
    "instructions",
    "lookups",
    "workflows",
    "action_rules",
    "exemplars",
    "safety",
    "atlas",
    "products",
    "account_model",
    "venue",
    "dollarpower",
    "guardian",
    "notifications",
    "strategy_brain",
    "turn_contract",
];

/// Full prompt for `aomi-run` and any runtime that skips `manifest.skill`.
///
/// Order: role header, shared core (exemplars after action-rules, before safety),
/// guest, share, turn-contract LAST (static recency for the behavioral kernel).
pub(crate) const COMPOSED: &str = concat!(
    role_header!(),
    "\n\n---\n\n",
    include_str!("skill/instructions.md"),
    "\n\n---\n\n",
    include_str!("skill/lookups.md"),
    "\n\n---\n\n",
    include_str!("skill/workflows.md"),
    "\n\n---\n\n",
    include_str!("skill/action-rules.md"),
    "\n\n---\n\n",
    include_str!("skill/exemplars.md"),
    "\n\n---\n\n",
    include_str!("skill/safety.md"),
    "\n\n---\n\n",
    include_str!("skill/reference/atlas.md"),
    "\n\n---\n\n",
    include_str!("skill/reference/products.md"),
    "\n\n---\n\n",
    include_str!("skill/reference/account-model.md"),
    "\n\n---\n\n",
    include_str!("skill/reference/venue.md"),
    "\n\n---\n\n",
    include_str!("skill/reference/dollarpower.md"),
    "\n\n---\n\n",
    include_str!("skill/reference/guardian.md"),
    "\n\n---\n\n",
    include_str!("skill/reference/notifications.md"),
    "\n\n---\n\n",
    include_str!("skill/reference/strategy-brain.md"),
    "\n\n---\n\n",
    include_str!("skill/guest.md"),
    "\n\n---\n\n",
    include_str!("skill/share.md"),
    "\n\n---\n\n",
    include_str!("skill/turn-contract.md"),
);

#[cfg(test)]
pub(crate) const ROLE_LEN: usize = ROLE_HEADER_FOR_TEST.len();

#[allow(dead_code)]
const _SEP: &str = SEP;
