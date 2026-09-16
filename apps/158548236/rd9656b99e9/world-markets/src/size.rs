//! Server-side size grammar and mark resolution.
//!
//! The model never converts dollars to base quantity. The sentence is
//! classified here (quote vs base denomination), then turned into a base
//! quantity from the same mark the preview uses.

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use serde_json::{Value, json};
use std::str::FromStr;

use crate::lookups::format_money;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SizeKind {
    Quote,
    Base,
    Ambiguous,
    None,
}

/// The first number in a sentence and how the surrounding words denominate it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SizeSpan {
    pub(crate) kind: SizeKind,
    pub(crate) surface: String,
    pub(crate) amount: String,
}

const MONEY_VERBS: &[&str] = &["put", "spend", "invest", "deploy"];
const CURRENCY_MARKERS: &[&str] = &["usd", "usdt", "dollar", "dollars", "worth", "bucks", "buck"];

impl SizeSpan {
    pub(crate) fn parse(sentence: &str, instrument: Option<&str>) -> Self {
        let lower = sentence.to_ascii_lowercase();
        let mut tokens: Vec<String> = Vec::new();
        let mut cur = String::new();
        for ch in lower.chars() {
            if ch == '.' && !cur.is_empty() && cur.chars().all(|c| c.is_ascii_digit()) {
                cur.push('.');
            } else if ch.is_ascii_alphanumeric() {
                cur.push(ch);
            } else if !cur.is_empty() {
                tokens.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            tokens.push(cur);
        }

        let Some(idx) = tokens.iter().position(|t| parse_amount_token(t).is_some()) else {
            return Self {
                kind: SizeKind::None,
                surface: String::new(),
                amount: String::new(),
            };
        };
        let mut amount = parse_amount_token(&tokens[idx]).unwrap_or_else(|| tokens[idx].clone());
        let mut surface = tokens[idx].clone();
        if tokens.get(idx + 1).map(String::as_str) == Some("hundred")
            && let Ok(n) = amount.parse::<i64>()
        {
            amount = (n * 100).to_string();
            surface = format!("{} hundred", tokens[idx]);
        }
        let window_lo = idx.saturating_sub(3);
        let window_hi = (idx + 4).min(tokens.len());
        let window = &tokens[window_lo..window_hi];
        let has_currency = window
            .iter()
            .any(|t| CURRENCY_MARKERS.iter().any(|m| m == t))
            || tokens.iter().any(|t| {
                CURRENCY_MARKERS.iter().any(|m| m == t) && {
                    let pos = tokens.iter().position(|x| x == t).unwrap_or(0);
                    pos.abs_diff(idx) <= 4
                }
            });
        let money_verb = tokens.iter().any(|t| MONEY_VERBS.iter().any(|v| v == t));
        let inst = instrument
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        let next = tokens.get(idx + 1).map(String::as_str);
        let unit_is_asset = match (next, inst.as_deref()) {
            (Some(n), Some(i)) if n == i || n.eq_ignore_ascii_case(i) => true,
            (Some("eth" | "ether" | "weth" | "btc" | "wbtc" | "sol" | "usdt"), _) => true,
            _ => false,
        };
        let has_of_asset = next == Some("of") && tokens.get(idx + 2).is_some();
        let mut kind = if has_currency || money_verb {
            SizeKind::Quote
        } else if unit_is_asset {
            SizeKind::Base
        } else if has_of_asset {
            SizeKind::Quote
        } else {
            SizeKind::Ambiguous
        };
        if sentence.contains('$') && kind != SizeKind::None {
            kind = SizeKind::Quote;
        }
        Self {
            kind,
            surface,
            amount,
        }
    }
}

/// Parse "200", "0.02", "$50", "5k", "fifty" into a decimal string.
fn parse_amount_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('$').replace(',', "");
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let word = match lower.as_str() {
        "zero" | "oh" => Some(0),
        "one" => Some(1),
        "two" => Some(2),
        "three" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        "eleven" => Some(11),
        "twelve" => Some(12),
        "thirteen" => Some(13),
        "fourteen" => Some(14),
        "fifteen" => Some(15),
        "sixteen" => Some(16),
        "seventeen" => Some(17),
        "eighteen" => Some(18),
        "nineteen" => Some(19),
        "twenty" => Some(20),
        "thirty" => Some(30),
        "forty" => Some(40),
        "fifty" => Some(50),
        "sixty" => Some(60),
        "seventy" => Some(70),
        "eighty" => Some(80),
        "ninety" => Some(90),
        "hundred" => Some(100),
        _ => None,
    };
    if let Some(n) = word {
        return Some(n.to_string());
    }
    let (body, mult) = if let Some(stripped) = lower.strip_suffix('k') {
        (stripped, 1_000i64)
    } else if let Some(stripped) = lower.strip_suffix('m') {
        (stripped, 1_000_000i64)
    } else {
        (lower.as_str(), 1i64)
    };
    if body.is_empty()
        || !body.chars().all(|c| c.is_ascii_digit() || c == '.')
        || body == "."
        || body.chars().filter(|c| *c == '.').count() > 1
    {
        return None;
    }
    if mult == 1 {
        let s = body.trim_start_matches('0');
        return Some(if s.is_empty() || s.starts_with('.') {
            format!("0{s}")
        } else {
            s.to_string()
        });
    }
    let n: f64 = body.parse().ok()?;
    Some(((n * mult as f64).round() as i64).to_string())
}

pub(crate) fn parse_amount(raw: &str) -> Option<Decimal> {
    parse_amount_token(raw).and_then(|s| Decimal::from_str(&s).ok())
}

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
}

impl SizeError {
    pub(crate) fn to_json(&self) -> Value {
        match self {
            SizeError::Ambiguous { span, mark } => {
                let amount = &span.amount;
                let (as_quote, as_base) = match (parse_amount(amount), mark) {
                    (None, _) => (None, None),
                    (Some(n), Some(mark)) if *mark > Decimal::ZERO => (
                        Some(format_money(n, false)),
                        Some(format_money(n * mark, false)),
                    ),
                    (Some(n), _) => (Some(format_money(n, false)), None),
                };
                let message = match (&as_quote, &as_base) {
                    (Some(q), Some(b)) => format!(
                        "Did you mean `{q}` of it, or `{amount}` units (about `{b}` at the mark)?"
                    ),
                    _ => format!("Did you mean `${amount}` worth, or `{amount}` units?"),
                };
                json!({
                    "error": "size_ambiguous",
                    "retry_with": "ask",
                    "message": message,
                    "interpretations": [
                        { "denomination": "quote", "input": format!("${amount}"), "notional": as_quote },
                        { "denomination": "base", "input": amount, "notional": as_base },
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
        }
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

impl SizeInput<'_> {
    fn classify(&self) -> Result<Size, SizeError> {
        let sentence = self.sentence.map(str::trim).filter(|s| !s.is_empty());
        let span = sentence.map(|s| SizeSpan::parse(s, self.instrument));

        if let Some(span) = span.as_ref() {
            match span.kind {
                SizeKind::Quote => {
                    let has_base = self
                        .size_base
                        .or(self.quantity)
                        .map(str::trim)
                        .is_some_and(|s| !s.is_empty());
                    let has_usd = self.size_usd.map(str::trim).is_some_and(|s| !s.is_empty());
                    if has_base && !has_usd {
                        eprintln!(
                            "[world-markets] size_mismatch_rejected sentence={:?} quantity={:?} size_base={:?}",
                            sentence, self.quantity, self.size_base
                        );
                        let amount = parse_amount(&span.amount).or_else(|| {
                            self.size_usd
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
                    let amount = self
                        .size_usd
                        .and_then(parse_amount)
                        .or_else(|| parse_amount(&span.amount))
                        .ok_or_else(|| SizeError::Invalid("quote size is not a number".into()))?;
                    return Ok(Size::Quote(amount));
                }
                SizeKind::Base => {
                    let amount = self
                        .size_base
                        .and_then(parse_amount)
                        .or_else(|| self.quantity.and_then(parse_amount))
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

        if let Some(usd) = self.size_usd.and_then(parse_amount) {
            return Ok(Size::Quote(usd));
        }
        if let Some(base) = self
            .size_base
            .and_then(parse_amount)
            .or_else(|| self.quantity.and_then(parse_amount))
        {
            return Ok(Size::Base(base));
        }
        Err(SizeError::Missing)
    }

    /// Classify the sentence and convert to a base quantity at `mark`.
    pub(crate) fn resolve(&self, mark: Decimal) -> Result<ResolvedSize, SizeError> {
        let size = match self.classify() {
            Ok(size) => size,
            Err(SizeError::Ambiguous { span, .. }) => {
                return Err(SizeError::Ambiguous {
                    span,
                    mark: Some(mark),
                });
            }
            Err(err) => return Err(err),
        };
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
                    base_qty: qty,
                    notional,
                })
            }
        }
    }
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

/// Readback base quantity: ≤6 decimal places, trailing zeros stripped.
pub(crate) fn format_base_qty(qty: Decimal) -> String {
    qty.round_dp_with_strategy(6, RoundingStrategy::MidpointAwayFromZero)
        .normalize()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark() -> Decimal {
        Decimal::from_str("2465.71").unwrap()
    }

    #[test]
    fn dollar_sentence_is_quote() {
        let size = SizeInput {
            sentence: Some("buy $200 of ETH"),
            instrument: Some("ETH"),
            ..SizeInput::default()
        }
        .classify()
        .unwrap();
        assert_eq!(size, Size::Quote(Decimal::from(200)));
    }

    #[test]
    fn put_300_into_ether_is_quote() {
        let size = SizeInput {
            sentence: Some("put 300 into ether"),
            instrument: Some("ETH"),
            ..SizeInput::default()
        }
        .classify()
        .unwrap();
        assert_eq!(size, Size::Quote(Decimal::from(300)));
    }

    #[test]
    fn base_unit_sentence_is_base() {
        let size = SizeInput {
            sentence: Some("buy 0.02 WETH"),
            instrument: Some("WETH"),
            ..SizeInput::default()
        }
        .classify()
        .unwrap();
        assert_eq!(size, Size::Base(Decimal::from_str("0.02").unwrap()));
    }

    #[test]
    fn buy_200_weth_is_base() {
        let size = SizeInput {
            sentence: Some("buy 200 WETH"),
            instrument: Some("WETH"),
            ..SizeInput::default()
        }
        .classify()
        .unwrap();
        assert_eq!(size, Size::Base(Decimal::from(200)));
    }

    #[test]
    fn number_words_and_suffixes_parse() {
        assert_eq!(parse_amount_token("fifty").as_deref(), Some("50"));
        assert_eq!(parse_amount_token("5k").as_deref(), Some("5000"));
        assert_eq!(parse_amount_token("$1,250.5").as_deref(), Some("1250.5"));
        assert_eq!(parse_amount_token(".5").as_deref(), Some("0.5"));
        assert_eq!(parse_amount_token("eth"), None);
        let span = SizeSpan::parse("buy two hundred worth of WETH", Some("WETH"));
        assert_eq!(span.kind, SizeKind::Quote);
        assert_eq!(span.amount, "200");
    }

    #[test]
    fn buy_200_alone_is_ambiguous() {
        let err = SizeInput {
            sentence: Some("buy 200"),
            ..SizeInput::default()
        }
        .classify()
        .unwrap_err();
        assert!(matches!(err, SizeError::Ambiguous { .. }));
        let value = SizeInput {
            sentence: Some("buy 200"),
            ..SizeInput::default()
        }
        .resolve(mark())
        .unwrap_err()
        .to_json();
        assert_eq!(value["error"], "size_ambiguous");
        assert!(value.get("skip_llm").is_none());
        assert!(value.get("reply_verbatim").is_none());
        assert!(value["message"].as_str().unwrap().contains("$200.00"));
    }

    #[test]
    fn size_base_with_dollar_sentence_is_rejected() {
        let err = SizeInput {
            sentence: Some("buy $50 of WETH"),
            quantity: Some("50"),
            instrument: Some("WETH"),
            ..SizeInput::default()
        }
        .classify()
        .unwrap_err();
        match err {
            SizeError::Mismatch { size_usd, .. } => assert_eq!(size_usd, "50"),
            other => panic!("expected mismatch, got {other:?}"),
        }
    }

    #[test]
    fn quote_resolves_from_mark() {
        let resolved = SizeInput {
            sentence: Some("buy $200 of ETH"),
            size_usd: Some("200"),
            instrument: Some("ETH"),
            ..SizeInput::default()
        }
        .resolve(mark())
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
    fn format_base_qty_allows_six_decimals_for_readback() {
        let raw = Decimal::from_str("0.0799427609831360745706074451").unwrap();
        let rendered = format_base_qty(raw);
        let frac = rendered.split('.').nth(1).unwrap_or("");
        assert!(frac.len() <= 6, "{rendered}");
    }
}
