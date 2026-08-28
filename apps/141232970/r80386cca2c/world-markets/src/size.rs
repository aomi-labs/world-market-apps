//! Server-side size grammar and mark resolution (P-A).
//!
//! The model never converts dollars to base quantity. `speech_ontology` classifies
//! the user's sentence; this module turns that into a base quantity from the same
//! mark the preview uses.

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde_json::{Value, json};
use std::str::FromStr;

use crate::lookups::format_money;
use crate::speech_ontology::{self, SizeKind, SizeSpan};

/// Preview→execute mark drift: 50 bps of notional. Venue ticks on ETH/BTC are
/// ~0.4–4 bps; 50 bps covers a few seconds of mark movement without letting a
/// stale conversion through.
pub(crate) const DRIFT_TOLERANCE_BPS: u32 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Size {
    Quote(Decimal),
    Base(Decimal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedSize {
    pub input: String,
    pub denomination: &'static str,
    pub mark: Decimal,
    pub base_qty: Decimal,
    pub notional: Decimal,
    pub size: Size,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SizeError {
    Ambiguous {
        span: SizeSpan,
        mark: Option<Decimal>,
    },
    Missing,
    Mismatch {
        sentence: String,
        size_usd: String,
    },
    Invalid(String),
    Drift {
        expected: Decimal,
        actual: Decimal,
        bps: Decimal,
    },
}

impl SizeError {
    pub(crate) fn to_json(&self) -> Value {
        match self {
            SizeError::Ambiguous { span, mark } => {
                let (as_quote, as_base) = ambiguous_notionals(&span.amount, *mark);
                json!({
                    "error": "size_ambiguous",
                    "retry_with": "ask",
                    "message": ambiguous_ask(&span.amount, mark.as_ref(), as_quote.as_deref(), as_base.as_deref()),
                    "reply_verbatim": true,
                    "skip_llm": true,
                    "interpretations": [
                        { "denomination": "quote", "input": format!("${}", span.amount), "notional": as_quote },
                        { "denomination": "base", "input": span.amount, "notional": as_base },
                    ],
                    "executable": false,
                })
            }
            SizeError::Missing => json!({
                "error": "size_missing",
                "detail": "pass size_usd for a dollar amount, size_base for an asset amount, or the user's whole sentence",
                "executable": false,
            }),
            SizeError::Mismatch { sentence, size_usd } => json!({
                "error": "size_denomination_mismatch",
                "retry_with": { "size_usd": size_usd, "size_base": null, "text": sentence },
                "detail": "sentence is quote-denominated; resend with size_usd, not size_base/quantity",
                "executable": false,
            }),
            SizeError::Invalid(detail) => json!({
                "error": "invalid_size",
                "detail": detail,
                "executable": false,
            }),
            SizeError::Drift {
                expected,
                actual,
                bps,
            } => json!({
                "error": "size_mark_drift",
                "detail": format!(
                    "re-resolved base qty {actual} drifted {bps} bps from preview {expected}; re-preview"
                ),
                "executable": false,
            }),
        }
    }

    pub(crate) fn as_err(&self) -> Result<Value, String> {
        Ok(self.to_json())
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SizeInput<'a> {
    pub sentence: Option<&'a str>,
    pub size_usd: Option<&'a str>,
    pub size_base: Option<&'a str>,
    pub quantity: Option<&'a str>,
    pub instrument: Option<&'a str>,
}

pub(crate) fn classify(input: &SizeInput<'_>) -> Result<Size, SizeError> {
    let sentence = input.sentence.map(str::trim).filter(|s| !s.is_empty());
    let span = sentence.map(|s| speech_ontology::parse_size(s, input.instrument));

    if let Some(span) = span.as_ref() {
        match span.kind {
            SizeKind::Quote => {
                if model_passed_base_only(input) {
                    eprintln!(
                        "[world-markets] size_mismatch_rejected sentence={:?} quantity={:?} size_base={:?}",
                        sentence, input.quantity, input.size_base
                    );
                    let amount = parse_amount(&span.amount).or_else(|| {
                        input
                            .size_usd
                            .and_then(parse_amount)
                            .or_else(|| parse_amount(&span.surface))
                    });
                    let usd = amount
                        .map(|d| d.normalize().to_string())
                        .unwrap_or_else(|| span.amount.clone());
                    return Err(SizeError::Mismatch {
                        sentence: sentence.unwrap_or("").to_string(),
                        size_usd: usd,
                    });
                }
                let amount = input
                    .size_usd
                    .and_then(parse_amount)
                    .or_else(|| parse_amount(&span.amount))
                    .ok_or_else(|| SizeError::Invalid("quote size is not a number".into()))?;
                return Ok(Size::Quote(amount));
            }
            SizeKind::Base => {
                let amount = input
                    .size_base
                    .and_then(parse_amount)
                    .or_else(|| input.quantity.and_then(parse_amount))
                    .or_else(|| parse_amount(&span.amount))
                    .ok_or_else(|| SizeError::Invalid("base size is not a number".into()))?;
                return Ok(Size::Base(amount));
            }
            SizeKind::Ambiguous => {
                return Err(SizeError::Ambiguous {
                    span: span.clone(),
                    mark: None,
                });
            }
            SizeKind::None => {}
        }
    }

    if let Some(usd) = input.size_usd.and_then(parse_amount) {
        return Ok(Size::Quote(usd));
    }
    if let Some(base) = input
        .size_base
        .and_then(parse_amount)
        .or_else(|| input.quantity.and_then(parse_amount))
    {
        return Ok(Size::Base(base));
    }
    Err(SizeError::Missing)
}

pub(crate) fn resolve(size: &Size, mark: Decimal) -> Result<ResolvedSize, SizeError> {
    if mark <= Decimal::ZERO {
        return Err(SizeError::Invalid("mark must be greater than zero".into()));
    }
    match size {
        Size::Quote(usd) => {
            let base_qty = usd
                .checked_div(mark)
                .ok_or_else(|| SizeError::Invalid("quote size exceeds numeric range".into()))?;
            let notional = base_qty
                .checked_mul(mark)
                .ok_or_else(|| SizeError::Invalid("notional exceeds numeric range".into()))?;
            Ok(ResolvedSize {
                input: format!("${}", usd.normalize()),
                denomination: "quote",
                mark,
                base_qty,
                notional,
                size: size.clone(),
            })
        }
        Size::Base(qty) => {
            let notional = qty
                .checked_mul(mark)
                .ok_or_else(|| SizeError::Invalid("notional exceeds numeric range".into()))?;
            Ok(ResolvedSize {
                input: qty.normalize().to_string(),
                denomination: "base",
                mark,
                base_qty: *qty,
                notional,
                size: size.clone(),
            })
        }
    }
}

pub(crate) fn classify_and_resolve(
    input: &SizeInput<'_>,
    mark: Decimal,
) -> Result<ResolvedSize, SizeError> {
    match classify(input) {
        Ok(size) => resolve(&size, mark),
        Err(SizeError::Ambiguous { span, .. }) => Err(SizeError::Ambiguous {
            span,
            mark: Some(mark),
        }),
        Err(err) => Err(err),
    }
}

pub(crate) fn reject_drift(preview_qty: Decimal, live_qty: Decimal) -> Result<(), SizeError> {
    if preview_qty <= Decimal::ZERO {
        return Ok(());
    }
    let delta = (live_qty - preview_qty).abs();
    let bps = (delta / preview_qty) * Decimal::new(10_000, 0);
    if bps > Decimal::from(DRIFT_TOLERANCE_BPS) {
        return Err(SizeError::Drift {
            expected: preview_qty,
            actual: live_qty,
            bps,
        });
    }
    Ok(())
}

impl ResolvedSize {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "input": self.input,
            "denomination": self.denomination,
            "mark": self.mark.normalize().to_string(),
            "base_qty": format_base_qty(self.base_qty),
            "notional": self.notional.normalize().to_string(),
            "notional_rendered": format!("`{}`", format_money(self.notional, false)),
        })
    }
}

fn model_passed_base_only(input: &SizeInput<'_>) -> bool {
    let has_base = input
        .size_base
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some()
        || input
            .quantity
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_some();
    let has_usd = input
        .size_usd
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_some();
    has_base && !has_usd
}

pub(crate) fn parse_amount(raw: &str) -> Option<Decimal> {
    speech_ontology::parse_amount_token(raw).and_then(|s| Decimal::from_str(&s).ok())
}

pub(crate) fn format_base_qty(qty: Decimal) -> String {
    qty.round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero)
        .normalize()
        .to_string()
}

/// Receipt-facing base quantity: ≤4 decimal places, trailing zeros stripped.
pub(crate) fn format_qty_human(qty: Decimal) -> String {
    qty.round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero)
        .normalize()
        .to_string()
}

fn ambiguous_notionals(amount: &str, mark: Option<Decimal>) -> (Option<String>, Option<String>) {
    let Some(n) = parse_amount(amount) else {
        return (None, None);
    };
    let Some(mark) = mark.filter(|m| *m > Decimal::ZERO) else {
        return (Some(format_money(n, false)), None);
    };
    let as_quote = format_money(n, false);
    let as_base = format_money(n * mark, false);
    (Some(as_quote), Some(as_base))
}

fn ambiguous_ask(
    amount: &str,
    _mark: Option<&Decimal>,
    as_quote: Option<&str>,
    as_base: Option<&str>,
) -> String {
    match (as_quote, as_base) {
        (Some(q), Some(b)) => {
            format!("Did you mean `{q}` of it, or `{amount}` units (about `{b}` at the mark)?")
        }
        _ => format!("Did you mean `${amount}` worth, or `{amount}` units?"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark() -> Decimal {
        Decimal::from_str("2465.71").unwrap()
    }

    #[test]
    fn dollar_sentence_is_quote() {
        let size = classify(&SizeInput {
            sentence: Some("buy $200 of ETH"),
            instrument: Some("ETH"),
            ..SizeInput::default()
        })
        .unwrap();
        assert_eq!(size, Size::Quote(Decimal::from(200)));
    }

    #[test]
    fn put_300_into_ether_is_quote() {
        let size = classify(&SizeInput {
            sentence: Some("put 300 into ether"),
            instrument: Some("ETH"),
            ..SizeInput::default()
        })
        .unwrap();
        assert_eq!(size, Size::Quote(Decimal::from(300)));
    }

    #[test]
    fn base_unit_sentence_is_base() {
        let size = classify(&SizeInput {
            sentence: Some("buy 0.02 WETH"),
            instrument: Some("WETH"),
            ..SizeInput::default()
        })
        .unwrap();
        assert_eq!(size, Size::Base(Decimal::from_str("0.02").unwrap()));
    }

    #[test]
    fn buy_200_weth_is_base() {
        let size = classify(&SizeInput {
            sentence: Some("buy 200 WETH"),
            instrument: Some("WETH"),
            ..SizeInput::default()
        })
        .unwrap();
        assert_eq!(size, Size::Base(Decimal::from(200)));
    }

    #[test]
    fn buy_200_alone_is_ambiguous() {
        let err = classify(&SizeInput {
            sentence: Some("buy 200"),
            ..SizeInput::default()
        })
        .unwrap_err();
        assert!(matches!(err, SizeError::Ambiguous { .. }));
    }

    #[test]
    fn size_base_with_dollar_sentence_is_rejected() {
        let err = classify(&SizeInput {
            sentence: Some("buy $50 of WETH"),
            quantity: Some("50"),
            instrument: Some("WETH"),
            ..SizeInput::default()
        })
        .unwrap_err();
        match err {
            SizeError::Mismatch { size_usd, .. } => assert_eq!(size_usd, "50"),
            other => panic!("expected mismatch, got {other:?}"),
        }
    }

    #[test]
    fn quote_resolves_from_mark() {
        let resolved = classify_and_resolve(
            &SizeInput {
                sentence: Some("buy $200 of ETH"),
                size_usd: Some("200"),
                instrument: Some("ETH"),
                ..SizeInput::default()
            },
            mark(),
        )
        .unwrap();
        assert_eq!(resolved.denomination, "quote");
        let expected = Decimal::from(200) / mark();
        assert!((resolved.base_qty - expected).abs() < Decimal::new(1, 8));
        assert!(
            resolved.to_json()["notional_rendered"]
                .as_str()
                .unwrap()
                .contains("$")
        );
    }

    #[test]
    fn mismatch_json_is_machine_actionable() {
        let err = SizeError::Mismatch {
            sentence: "buy $50 of WETH".into(),
            size_usd: "50".into(),
        };
        let value = err.to_json();
        assert_eq!(value["error"], "size_denomination_mismatch");
        assert_eq!(value["retry_with"]["size_usd"], "50");
        assert_eq!(value["executable"], false);
    }

    #[test]
    fn drift_within_50bps_passes() {
        let qty = Decimal::from_str("0.081102").unwrap();
        let live = qty * Decimal::from_str("1.004").unwrap();
        reject_drift(qty, live).unwrap();
    }

    #[test]
    fn drift_over_50bps_rejects() {
        let qty = Decimal::from_str("0.081102").unwrap();
        let live = qty * Decimal::from_str("1.01").unwrap();
        assert!(reject_drift(qty, live).is_err());
    }

    #[test]
    fn format_qty_human_caps_at_four_decimals() {
        let raw = Decimal::from_str("0.0799427609831360745706074451").unwrap();
        let rendered = format_qty_human(raw);
        let frac = rendered.split('.').nth(1).unwrap_or("");
        assert!(frac.len() <= 4, "{rendered}");
        assert_eq!(rendered, "0.0799");
        assert_eq!(
            format_qty_human(Decimal::from_str("0.0800").unwrap()),
            "0.08"
        );
    }

    #[test]
    fn format_base_qty_allows_six_decimals_for_readback() {
        let raw = Decimal::from_str("0.0799427609831360745706074451").unwrap();
        let rendered = format_base_qty(raw);
        let frac = rendered.split('.').nth(1).unwrap_or("");
        assert!(frac.len() <= 6, "{rendered}");
    }
}
