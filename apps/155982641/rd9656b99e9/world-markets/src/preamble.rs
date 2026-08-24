//! Composed system prompt for LLM runtimes that read only [`DynManifest::preamble`].
//!
//! `aomi-run` (aomi-sdk 4.0.0) does **not** fold `manifest.skill` sections into the
//! agent prompt — only this string is sent. The hosted Aomi backend composes
//! skill sections separately, so we keep the `skill = { ... }` block in `lib.rs`
//! for release validation and staging.

const SEP: &str = "\n\n---\n\n";

/// Full prompt for `aomi-run` and any runtime that skips `manifest.skill`.
pub(crate) const COMPOSED: &str = concat!(
    "You are the World Markets Agent on UniFi testnet. ",
    "Terse lookups (whole message only): when the user sends exactly one token — ",
    "b, p, r, a, d, paper, or balance, positions, risk, available, dollarpower ",
    "(case-insensitive, nothing else) — call the mapped tool immediately and reply ",
    "with exactly one line. Never ask what they meant. Never list capabilities. ",
    "/help is the host REPL only; you do not register slash commands.",
    "\n\n---\n\n",
    include_str!("skill/instructions.md"),
    "\n\n---\n\n",
    include_str!("skill/lookups.md"),
    "\n\n---\n\n",
    include_str!("skill/workflows.md"),
    "\n\n---\n\n",
    include_str!("skill/action-rules.md"),
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
);

#[cfg(test)]
pub(crate) const ROLE_LEN: usize = concat!(
    "You are the World Markets Agent on UniFi testnet. ",
    "Terse lookups (whole message only): when the user sends exactly one token — ",
    "b, p, r, a, d, paper, or balance, positions, risk, available, dollarpower ",
    "(case-insensitive, nothing else) — call the mapped tool immediately and reply ",
    "with exactly one line. Never ask what they meant. Never list capabilities. ",
    "/help is the host REPL only; you do not register slash commands.",
)
.len();

#[allow(dead_code)]
const _SEP: &str = SEP;
