//! Trade-intent order type: market, limit, TWAP, DCA.
//!
//! Venue child fills stay market/IOC unless a limit price is set. TWAP/DCA are
//! stored on the ledger schedule and executed over time.

use rust_decimal::Decimal;
use serde_json::{Value, json};

fn depth_fraction() -> Decimal {
    Decimal::new(20, 2)
}
const MIN_TWAP_SLICES: u32 = 3;
const MAX_SLICES: u32 = 10;
const DEFAULT_TWAP_INTERVAL_SECS: u64 = 60;
const DEFAULT_DCA_SLICES: u32 = 7;
const DAILY_SECS: u64 = 86_400;
const WEEKLY_SECS: u64 = 7 * DAILY_SECS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionPlan {
    pub(crate) order_type: String,
    pub(crate) slices: u32,
    pub(crate) interval_secs: u64,
    pub(crate) window_secs: Option<u64>,
    pub(crate) cadence: Option<String>,
    pub(crate) quantity_per_slice: String,
}

impl ExecutionPlan {
    pub(crate) fn is_sliced(&self) -> bool {
        matches!(self.order_type.as_str(), "twap" | "dca") && self.slices > 1
    }

    pub(crate) fn to_schedule_json(&self, total_quantity: &str) -> Value {
        json!({
            "slices": self.slices,
            "interval_secs": self.interval_secs,
            "window_secs": self.window_secs,
            "cadence": self.cadence,
            "quantity_per_slice": self.quantity_per_slice,
            "filled_quantity": "0",
            "total_quantity": total_quantity,
        })
    }
}

pub(crate) struct InferInput<'a> {
    pub(crate) named: Option<&'a str>,
    pub(crate) price: Option<&'a str>,
    pub(crate) sentence: Option<&'a str>,
    pub(crate) quantity: Decimal,
    pub(crate) opposite_depth: Option<Decimal>,
    pub(crate) slices: Option<u32>,
    pub(crate) window_minutes: Option<u32>,
    pub(crate) interval_secs: Option<u64>,
    pub(crate) cadence: Option<&'a str>,
}

pub(crate) fn infer_execution_plan(input: InferInput<'_>) -> ExecutionPlan {
    let sentence = input.sentence.unwrap_or("");
    let forced_market = sentence_forces_market(sentence);
    let from_name = named_order_type(input.named);
    let from_sentence = sentence_order_type(sentence);

    let order_type = if forced_market {
        "market".to_string()
    } else if let Some(named) = from_name {
        named.to_string()
    } else if let Some(hint) = from_sentence {
        hint.to_string()
    } else if input
        .price
        .filter(|value| !value.trim().is_empty())
        .is_some()
    {
        "limit".to_string()
    } else if should_twap(input.quantity, input.opposite_depth) {
        "twap".to_string()
    } else {
        "market".to_string()
    };

    match order_type.as_str() {
        "twap" => twap_plan(input),
        "dca" => dca_plan(input),
        other => ExecutionPlan {
            order_type: other.to_string(),
            slices: 1,
            interval_secs: 0,
            window_secs: None,
            cadence: None,
            quantity_per_slice: format_qty(input.quantity),
        },
    }
}

pub(crate) fn venue_order_type(intent: &str, price: Option<&str>) -> String {
    match intent.to_ascii_lowercase().as_str() {
        "limit" => "limit".to_string(),
        "twap" | "dca" if price.filter(|value| !value.trim().is_empty()).is_some() => {
            "limit".to_string()
        }
        "market" | "ioc" | "twap" | "dca" => "market".to_string(),
        _ if price.filter(|value| !value.trim().is_empty()).is_some() => "limit".to_string(),
        _ => "market".to_string(),
    }
}

/// Child-fill quantity for slice `index` (1-based). Last slice takes the remainder.
pub(crate) fn slice_quantity(total: Decimal, filled: Decimal, slices: u32, index: u32) -> Decimal {
    let remaining = (total - filled).max(Decimal::ZERO);
    if remaining <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    if slices == 0 || index >= slices {
        return remaining;
    }
    let per = if slices == 0 {
        remaining
    } else {
        (total / Decimal::from(slices)).normalize()
    };
    if per <= Decimal::ZERO {
        return remaining;
    }
    remaining.min(per)
}

fn twap_plan(input: InferInput<'_>) -> ExecutionPlan {
    let slices = input
        .slices
        .filter(|n| *n > 0)
        .unwrap_or_else(|| twap_slices(input.quantity, input.opposite_depth));
    let window_secs = input
        .window_minutes
        .filter(|n| *n > 0)
        .map(|minutes| u64::from(minutes) * 60);
    let interval_secs = input.interval_secs.filter(|n| *n > 0).unwrap_or_else(|| {
        if let Some(window) = window_secs {
            (window / u64::from(slices.max(1))).max(1)
        } else {
            DEFAULT_TWAP_INTERVAL_SECS
        }
    });
    let window_secs = window_secs.or(Some(interval_secs.saturating_mul(u64::from(slices))));
    ExecutionPlan {
        order_type: "twap".to_string(),
        slices,
        interval_secs,
        window_secs,
        cadence: None,
        quantity_per_slice: format_qty(slice_quantity(input.quantity, Decimal::ZERO, slices, 1)),
    }
}

fn dca_plan(input: InferInput<'_>) -> ExecutionPlan {
    let cadence = input
        .cadence
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .or_else(|| sentence_cadence(input.sentence.unwrap_or("")))
        .unwrap_or_else(|| "daily".to_string());
    let interval_secs = input.interval_secs.filter(|n| *n > 0).unwrap_or_else(|| {
        if cadence == "weekly" {
            WEEKLY_SECS
        } else {
            DAILY_SECS
        }
    });
    let slices = input
        .slices
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_DCA_SLICES);
    ExecutionPlan {
        order_type: "dca".to_string(),
        slices,
        interval_secs,
        window_secs: Some(interval_secs.saturating_mul(u64::from(slices))),
        cadence: Some(cadence),
        quantity_per_slice: format_qty(slice_quantity(input.quantity, Decimal::ZERO, slices, 1)),
    }
}

fn should_twap(quantity: Decimal, opposite_depth: Option<Decimal>) -> bool {
    let Some(depth) = opposite_depth.filter(|value| *value > Decimal::ZERO) else {
        return false;
    };
    quantity > depth * depth_fraction()
}

fn twap_slices(quantity: Decimal, opposite_depth: Option<Decimal>) -> u32 {
    let Some(depth) = opposite_depth.filter(|value| *value > Decimal::ZERO) else {
        return MIN_TWAP_SLICES;
    };
    let cap = depth * depth_fraction();
    if cap <= Decimal::ZERO {
        return MIN_TWAP_SLICES;
    }
    let ratio = quantity / cap;
    let floor = ratio.trunc();
    let extra = if ratio > floor { 1 } else { 0 };
    let parsed = floor
        .trunc()
        .to_string()
        .split('.')
        .next()
        .and_then(|digits| digits.parse::<u32>().ok())
        .unwrap_or(MAX_SLICES);
    parsed
        .saturating_add(extra)
        .clamp(MIN_TWAP_SLICES, MAX_SLICES)
}

fn named_order_type(named: Option<&str>) -> Option<&'static str> {
    match named
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("twap") => Some("twap"),
        Some("dca") => Some("dca"),
        Some("limit") => Some("limit"),
        Some("market") | Some("ioc") => Some("market"),
        _ => None,
    }
}

fn sentence_forces_market(sentence: &str) -> bool {
    let lower = sentence.to_ascii_lowercase();
    contains_phrase(&lower, "all at once")
        || contains_phrase(&lower, "right now")
        || contains_phrase(&lower, "fill now")
        || contains_phrase(&lower, "market order")
        || word_eq(&lower, "now")
        || word_eq(&lower, "immediately")
}

fn sentence_order_type(sentence: &str) -> Option<&'static str> {
    let lower = sentence.to_ascii_lowercase();
    if word_eq(&lower, "dca")
        || contains_phrase(&lower, "dollar cost")
        || contains_phrase(&lower, "dollar-cost")
        || contains_phrase(&lower, "every day")
        || contains_phrase(&lower, "every week")
        || contains_phrase(&lower, "each day")
        || contains_phrase(&lower, "each week")
    {
        return Some("dca");
    }
    if word_eq(&lower, "twap")
        || contains_phrase(&lower, "in slices")
        || contains_phrase(&lower, "over time")
        || contains_phrase(&lower, "over the next")
    {
        return Some("twap");
    }
    None
}

fn sentence_cadence(sentence: &str) -> Option<String> {
    let lower = sentence.to_ascii_lowercase();
    if contains_phrase(&lower, "every week") || contains_phrase(&lower, "each week") {
        return Some("weekly".to_string());
    }
    if contains_phrase(&lower, "every day") || contains_phrase(&lower, "each day") {
        return Some("daily".to_string());
    }
    None
}

fn contains_phrase(haystack: &str, needle: &str) -> bool {
    haystack.contains(needle)
}

fn word_eq(haystack: &str, word: &str) -> bool {
    haystack
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token == word)
}

fn format_qty(value: Decimal) -> String {
    value.normalize().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn qty(raw: &str) -> Decimal {
        raw.parse().unwrap()
    }

    fn infer(
        named: Option<&str>,
        price: Option<&str>,
        sentence: Option<&str>,
        quantity: &str,
        depth: Option<&str>,
    ) -> ExecutionPlan {
        infer_execution_plan(InferInput {
            named,
            price,
            sentence,
            quantity: qty(quantity),
            opposite_depth: depth.map(qty),
            slices: None,
            window_minutes: None,
            interval_secs: None,
            cadence: None,
        })
    }

    #[test]
    fn price_without_algo_is_limit() {
        assert_eq!(
            infer(None, Some("2000"), None, "1", None).order_type,
            "limit"
        );
        assert_eq!(
            infer(Some("market"), Some("2000"), None, "1", None).order_type,
            "market"
        );
    }

    #[test]
    fn thin_book_becomes_twap_unless_user_said_now() {
        let plan = infer(None, None, Some("buy 1000 ETH"), "1000", Some("100"));
        assert_eq!(plan.order_type, "twap");
        assert!(plan.slices >= 3);
        assert!(plan.is_sliced());

        let ample = infer(None, None, Some("buy 1 ETH"), "1", Some("100"));
        assert_eq!(ample.order_type, "market");
        assert!(!ample.is_sliced());

        let now = infer(None, None, Some("buy 1000 ETH now"), "1000", Some("100"));
        assert_eq!(now.order_type, "market");

        let missing_depth = infer(None, None, Some("buy 1000 ETH"), "1000", None);
        assert_eq!(missing_depth.order_type, "market");
    }

    #[test]
    fn sentence_and_named_select_dca_and_twap() {
        let dca = infer(None, None, Some("dca 7 ETH every day"), "7", None);
        assert_eq!(dca.order_type, "dca");
        assert_eq!(dca.cadence.as_deref(), Some("daily"));
        assert_eq!(dca.interval_secs, DAILY_SECS);

        let named = infer(Some("twap"), None, Some("buy 2 ETH"), "2", None);
        assert_eq!(named.order_type, "twap");
        assert_eq!(named.slices, MIN_TWAP_SLICES);
    }

    #[test]
    fn venue_child_fills_are_market_unless_limit() {
        assert_eq!(venue_order_type("twap", None), "market");
        assert_eq!(venue_order_type("dca", None), "market");
        assert_eq!(venue_order_type("limit", Some("2000")), "limit");
        assert_eq!(venue_order_type("market", Some("2000")), "market");
    }

    #[test]
    fn last_slice_takes_remainder() {
        let total = qty("10");
        let first = slice_quantity(total, Decimal::ZERO, 3, 1);
        let second = slice_quantity(total, first, 3, 2);
        let last = slice_quantity(total, first + second, 3, 3);
        assert_eq!(first + second + last, total);
        assert_eq!(last, total - first - second);
    }
}
