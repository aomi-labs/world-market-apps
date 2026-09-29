//! Always-active safety and response contract. Detailed operating instructions
//! live in SDK 5 skills. The app fragment must leave room within the backend
//! 32 KB cap for the shared harness, chain context, and model instructions.

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

/// Complete skill section order, including details omitted from `COMPOSED`.
/// The turn contract remains last in both the preamble and the skill list.
///
/// Allowed differences (documented here so the parity test does not paper over them):
/// - role header: COMPOSED-only (`ROLE_HEADER`)
/// - `guest.md` / `share.md`: COMPOSED-only; hosted omits them (pre-existing)
#[cfg(test)]
pub(crate) const SHARED_CORE_SECTION_NAMES: &[&str] = &[
    "instructions",
    "lookups",
    "workflows",
    "workflows_monitoring",
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

/// Hosted `skills` section names in compose order. `turn_contract` is last.
#[cfg(test)]
pub(crate) const HOSTED_SKILL_SECTION_NAMES: &[&str] = &[
    "instructions",
    "lookups",
    "workflows",
    "workflows_monitoring",
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

/// Always-active prompt. SDK 5 skill activation supplies detailed workflows.
///
/// Order: role and skill routing, safety,
/// guest, share, turn-contract LAST (static recency for the behavioral kernel).
pub(crate) const COMPOSED: &str = concat!(
    role_header!(),
    "\n\nBefore any World tool call, activate world-markets/trading and world-markets/reporting to load account rules, lookup dispatch, action rules and response templates. Skill activation precedes the turn contract's first business-tool call. Before handling execution, trade amendments or order management, also activate world-markets/execution. Before watches, health, research, voice or recurring tasks, activate world-markets/monitoring. Before venue, account-model, risk or product explanations, activate world-markets/reference. Follow those instructions before using the relevant tools; never invent an omitted workflow. Safety, guest and share handling, and the turn contract below remain active on every turn. Activate the relevant skills again each serve cycle; never infer omitted rules from memory.",
    "\n\n---\n\n",
    include_str!("skill/safety.md"),
    "\n\n---\n\n",
    include_str!("skill/guest.md"),
    "\n\n---\n\n",
    include_str!("skill/share.md"),
    "\n\n---\n\n",
    include_str!("skill/turn-contract.md"),
);

#[allow(dead_code)]
const _SEP: &str = SEP;
