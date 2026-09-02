//! Sending-side introduction: share-intent parser, M10 templates, start payloads.
//!
//! Referral codes and attribution live in the brain JSON store. This module
//! never interpolates account data into the forwarded message: M10 templates
//! accept only `{first_name}` and `{ref_link}`.

use std::collections::BTreeSet;

/// Framing line. Shown once per user, ever.
pub(crate) const HINT: &str = "Forward the next message to them — and a voice note from you on top beats anything I could say.";

/// M10 with the sharer's first name. Slots: `{first_name}`, `{ref_link}`.
#[allow(dead_code)]
pub(crate) const M10_WITH_NAME: &str = "I'm aomi — an AI on a recorded line. I watch, I execute inside signed limits, and I can do nothing my owner hasn't allowed.\n\n{first_name} thought you should meet me.\n\nTry me on paper — pick a number, nothing is real, you sign nothing.\n{ref_link}";

/// M10 without a name. Slots: `{ref_link}`.
#[allow(dead_code)]
pub(crate) const M10_ANON: &str = "I'm aomi — an AI on a recorded line. I watch, I execute inside signed limits, and I can do nothing my owner hasn't allowed.\n\nA friend thought you should meet me.\n\nTry me on paper — pick a number, nothing is real, you sign nothing.\n{ref_link}";

#[allow(dead_code)]
pub(crate) const NAME_ASK: &str = "With your first name on it, or without?";
pub(crate) const ALREADY_USER: &str = "You two already know each other — this account is live.";
#[allow(dead_code)]
pub(crate) const REVOKE_ACK: &str = "Old invite link is dead. Here's your new one.";
#[allow(dead_code)]
pub(crate) const WHO_ASKED: &str = "I don't track who opens it — that stays between you and them.";
#[allow(dead_code)]
pub(crate) const WITHOUT_NAME: &str = "without my name";
pub(crate) const PAPER_BTN: &str = "Try it on paper ↗";
pub(crate) const CANT_BTN: &str = "What can't you do?";
#[allow(dead_code)]
pub(crate) const RATE_LIMITED: &str = "Three new invite links a day. The current one still works.";
#[allow(dead_code)]
pub(crate) const INTRODUCE_ROW: &str = "Introduce aomi to a friend ›";
pub(crate) const INTENT: &str = "introduce yourself to my friend";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShareIntent {
    Introduce,
    WithoutName,
    WithName,
    Revoke,
    Who,
}

impl ShareIntent {
    pub(crate) fn action(self) -> &'static str {
        match self {
            Self::Introduce => "introduce",
            Self::WithoutName => "without_name",
            Self::WithName => "with_name",
            Self::Revoke => "revoke",
            Self::Who => "who",
        }
    }
}

fn collapse(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_start(raw: &str) -> &str {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("/start ") {
        let idx = trimmed.len() - rest.len();
        return trimmed[idx..].trim();
    }
    if let Some(rest) = lower.strip_prefix("start=") {
        let idx = trimmed.len() - rest.len();
        return trimmed[idx..].trim();
    }
    trimmed
}

/// Non-money intent. Voice transcript or typed text. Never a trade.
pub(crate) fn parse_share_intent(raw: &str) -> Option<ShareIntent> {
    let collapsed = collapse(raw);
    if collapsed.is_empty() {
        return None;
    }
    let body = strip_start(&collapsed);
    let lower = body.to_ascii_lowercase();
    if looks_like_equity_share(&lower) {
        return None;
    }
    if is_who(&lower) {
        return Some(ShareIntent::Who);
    }
    if is_revoke(&lower) {
        return Some(ShareIntent::Revoke);
    }
    if is_without_name(&lower) {
        return Some(ShareIntent::WithoutName);
    }
    if is_with_name(&lower) {
        return Some(ShareIntent::WithName);
    }
    if is_introduce(&lower) {
        return Some(ShareIntent::Introduce);
    }
    None
}

fn looks_like_equity_share(lower: &str) -> bool {
    lower.contains("shares of")
        || lower.contains("share of")
        || lower.contains("buy shares")
        || lower.contains("sell shares")
        || (lower
            .split(|c: char| !c.is_ascii_alphabetic())
            .any(|w| w == "shares")
            && lower.bytes().any(|b| b.is_ascii_digit()))
}

fn is_who(lower: &str) -> bool {
    lower == "who did i share with"
        || lower == "who did i share this with"
        || lower == "who opened it"
        || lower == "who used my invite"
        || lower.contains("who did i share with")
        || lower.contains("who opened my invite")
}

fn is_revoke(lower: &str) -> bool {
    lower == "kill my invite link"
        || lower == "new invite link"
        || lower == "kill invite link"
        || lower == "revoke my invite"
        || lower == "revoke invite link"
}

fn is_without_name(lower: &str) -> bool {
    lower == "without my name" || lower == "without the name" || lower == "no name"
}

fn is_with_name(lower: &str) -> bool {
    lower == "with my name" || lower == "with your first name" || lower == "with my first name"
}

fn is_introduce(lower: &str) -> bool {
    if lower == "share" || lower == "introduce" || lower == "show a friend" {
        return true;
    }
    if lower == "share_intro" {
        return true;
    }
    lower.contains("introduce yourself")
        || lower.contains("introduce aomi")
        || lower.contains("share this with")
        || lower.contains("how do i show this")
        || lower.contains("how do i show aomi")
        || lower.contains("show this to someone")
        || lower.contains("show aomi to")
}

/// `ref_{code}` from a Telegram start / startapp payload.
pub(crate) fn ref_code_from_start(payload: &str) -> Option<String> {
    let raw = strip_start(payload);
    let lower = raw.to_ascii_lowercase();
    let rest = lower.strip_prefix("ref_")?;
    let code: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if code.is_empty() { None } else { Some(code) }
}

#[allow(dead_code)]
pub(crate) fn template_slots(template: &str) -> BTreeSet<String> {
    let mut slots = BTreeSet::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            break;
        };
        slots.insert(after[..close].to_string());
        rest = &after[close + 1..];
    }
    slots
}

#[allow(dead_code)]
pub(crate) fn fill_m10(template: &str, first_name: Option<&str>, ref_link: &str) -> String {
    let mut out = template.replace("{ref_link}", ref_link);
    if let Some(name) = first_name {
        out = out.replace("{first_name}", name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m10_templates_allow_only_name_and_link() {
        let with_name = template_slots(M10_WITH_NAME);
        assert_eq!(
            with_name,
            ["first_name", "ref_link"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );
        let anon = template_slots(M10_ANON);
        assert_eq!(anon, ["ref_link"].into_iter().map(str::to_string).collect());
        for banned in ["pnl", "balance", "position", "equity", "nav", "pnl"] {
            assert!(!M10_WITH_NAME.contains(&format!("{{{banned}}}")));
            assert!(!M10_ANON.contains(&format!("{{{banned}}}")));
        }
        let rendered = fill_m10(
            M10_WITH_NAME,
            Some("Ada"),
            "https://t.me/WorldMarketsBot?start=ref_abc",
        );
        assert!(rendered.contains("Ada thought you should meet me."));
        assert!(!rendered.contains('{'));
    }

    #[test]
    fn rust_copy_matches_brain_copy_module() {
        let js = include_str!("../brain/src/copy.js");
        for needle in [
            HINT,
            NAME_ASK,
            ALREADY_USER,
            REVOKE_ACK,
            WHO_ASKED,
            WITHOUT_NAME,
            PAPER_BTN,
            CANT_BTN,
            INTENT,
            INTRODUCE_ROW,
        ] {
            assert!(js.contains(needle), "missing {needle:?}");
        }
        assert!(js.contains("m10_with_name"));
        assert!(js.contains("{first_name} thought you should meet me."));
        assert!(js.contains("A friend thought you should meet me."));
    }

    #[test]
    fn parse_share_catches_spoken_and_typed_phrases() {
        for input in [
            "introduce yourself to my friend",
            "Introduce yourself to my friend",
            "share this with Maya",
            "how do I show this to someone",
            "share",
            "show a friend",
            "introduce aomi to a friend",
            "/start share",
            "kill my invite link",
            "new invite link",
            "who did I share with",
            "without my name",
        ] {
            assert!(parse_share_intent(input).is_some(), "{input}");
        }
        assert_eq!(
            parse_share_intent("introduce yourself to my friend"),
            Some(ShareIntent::Introduce)
        );
        assert_eq!(
            parse_share_intent("kill my invite link"),
            Some(ShareIntent::Revoke)
        );
        assert_eq!(
            parse_share_intent("who did I share with"),
            Some(ShareIntent::Who)
        );
        assert_eq!(
            parse_share_intent("without my name"),
            Some(ShareIntent::WithoutName)
        );
    }

    #[test]
    fn parse_share_does_not_steal_trades_or_lookups() {
        for input in [
            "buy 10 shares of WETH",
            "sell shares",
            "what's my balance",
            "paper",
            "b",
            "help",
            "",
        ] {
            assert_eq!(parse_share_intent(input), None, "{input}");
        }
    }

    #[test]
    fn start_payload_extracts_ref_code() {
        assert_eq!(
            ref_code_from_start("ref_ab12cd34ef").as_deref(),
            Some("ab12cd34ef")
        );
        assert_eq!(
            ref_code_from_start("/start ref_ab12cd34ef").as_deref(),
            Some("ab12cd34ef")
        );
        assert_eq!(ref_code_from_start("start=ref_zz").as_deref(), Some("zz"));
        assert_eq!(ref_code_from_start("g_7K9X2Q"), None);
        assert_eq!(ref_code_from_start("share"), None);
        assert_eq!(ref_code_from_start("refresh"), None);
    }

    #[test]
    fn copy_has_no_exclamation() {
        for line in [
            HINT,
            M10_WITH_NAME,
            M10_ANON,
            NAME_ASK,
            ALREADY_USER,
            REVOKE_ACK,
            WHO_ASKED,
            RATE_LIMITED,
        ] {
            assert!(!line.contains('!'), "{line}");
        }
    }

    #[test]
    fn no_code_path_notifies_the_sharer() {
        let brain = include_str!("../brain/src/share.js");
        let copy = include_str!("../brain/src/copy.js");
        let rust = include_str!("share.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        assert!(!brain.contains("enqueue("));
        assert!(!brain.contains("outbound"));
        for src in [brain, copy, rust] {
            let lower = src.to_lowercase();
            assert!(!lower.contains("friend joined"));
            assert!(!lower.contains("opened your invite"));
            assert!(!lower.contains("notify_sharer"));
        }
    }
}
