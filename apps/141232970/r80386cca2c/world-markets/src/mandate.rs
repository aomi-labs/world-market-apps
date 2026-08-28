use std::str::FromStr;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mandate {
    pub(crate) version: u64,
    pub(crate) markets: Vec<MarketPermission>,
    pub(crate) max_position_notional: AmountLimit,
    pub(crate) max_leverage: String,
    pub(crate) min_risk_adjusted_portfolio_value: AmountLimit,
    pub(crate) halt_if_eligible_for_liquidation: bool,
    pub(crate) can_withdraw: bool,
    /// Interim transport bridge until bot-core delivers the handover account
    /// reference beside the mandate.
    #[serde(default, rename = "account")]
    pub(crate) _account: Option<MandateAccount>,
    /// Standing guidance is carried here only as an interim transport bridge.
    /// It never participates in policy evaluation.
    #[serde(default, rename = "brief")]
    pub(crate) _brief: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MandateAccount {
    #[serde(rename = "id")]
    pub(crate) _id: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarketPermission {
    pub(crate) product: String,
    pub(crate) base: String,
    pub(crate) quote: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AmountLimit {
    pub(crate) amount: String,
    pub(crate) quote: String,
}

#[derive(Debug, Clone)]
pub(crate) struct TradeFacts<'a> {
    pub(crate) product: &'a str,
    pub(crate) side: &'a str,
    pub(crate) base: &'a str,
    pub(crate) quote: &'a str,
    pub(crate) quantity: Decimal,
    pub(crate) mark_price: Decimal,
    pub(crate) current_position_quantity: Decimal,
    pub(crate) risk_adjusted_portfolio_value: Decimal,
    pub(crate) post_trade_risk_adjusted_portfolio_value: Option<Decimal>,
    pub(crate) eligible_for_liquidation: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Verdict {
    pub(crate) status: &'static str,
    pub(crate) rule: &'static str,
    pub(crate) detail: String,
}

impl Verdict {
    fn allow(detail: impl Into<String>) -> Self {
        Self {
            status: "allow",
            rule: "mandate_v1",
            detail: detail.into(),
        }
    }

    fn deny(rule: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: "deny",
            rule,
            detail: detail.into(),
        }
    }

    pub(crate) fn is_allow(&self) -> bool {
        self.status == "allow"
    }
}

/// Bundled aomi-run placeholder (`mandate.dev.example.json`). Used when
/// handover is stubbed and `WORLD_MANDATE_PATH` is unset, empty, or `placeholder`.
const PLACEHOLDER_MANDATE_JSON: &str = include_str!("../mandate.dev.example.json");

/// Canonical detail for the mandate-absent family. One string; the message
/// layer renders it verbatim and does not choose among variants.
pub(crate) const MANDATE_ABSENT_DETAIL: &str = "No mandate is bound to this account.";

#[allow(dead_code)]
pub(crate) const MANDATE_ABSENT_RULES: [&str; 4] = [
    "missing_mandate",
    "unknown_mandate_key",
    "invalid_mandate",
    "unsupported_mandate_version",
];

#[allow(dead_code)]
pub(crate) fn is_mandate_absent(rule: &str) -> bool {
    MANDATE_ABSENT_RULES.contains(&rule)
}

fn mandate_absent(rule: &'static str) -> Verdict {
    Verdict::deny(rule, MANDATE_ABSENT_DETAIL)
}

fn is_placeholder_mandate_path(path: &str) -> bool {
    matches!(
        path.trim().to_ascii_lowercase().as_str(),
        "" | "placeholder" | "dev" | "-"
    )
}

fn is_mandate_absent_path(path: &str) -> bool {
    matches!(
        path.trim().to_ascii_lowercase().as_str(),
        "none" | "off" | "missing"
    )
}

impl Mandate {
    /// Hosted: `handover_mandate`. Local aomi-run stubs that to None, so this
    /// falls back to `WORLD_MANDATE_JSON`, a file at `WORLD_MANDATE_PATH`, or
    /// the bundled placeholder when the path is unset / `placeholder`.
    /// Set `WORLD_MANDATE_PATH=none` to keep the fail-closed handshake locally.
    pub(crate) fn bound(handover: Option<&Value>) -> Result<Self, Verdict> {
        if handover.is_some() {
            return Self::parse(handover);
        }
        if let Ok(raw) = std::env::var("WORLD_MANDATE_JSON")
            && !raw.trim().is_empty()
        {
            return Self::parse_json(&raw);
        }
        match std::env::var("WORLD_MANDATE_PATH") {
            Ok(path) if is_mandate_absent_path(&path) => Self::parse(None),
            Ok(path) if is_placeholder_mandate_path(&path) => {
                Self::parse_json(PLACEHOLDER_MANDATE_JSON)
            }
            Ok(path) => {
                let raw = std::fs::read_to_string(&path)
                    .map_err(|_| mandate_absent("invalid_mandate"))?;
                Self::parse_json(&raw)
            }
            Err(_) => Self::parse_json(PLACEHOLDER_MANDATE_JSON),
        }
    }

    fn parse_json(raw: &str) -> Result<Self, Verdict> {
        let value: Value =
            serde_json::from_str(raw).map_err(|_| mandate_absent("invalid_mandate"))?;
        Self::parse(Some(&value))
    }

    pub(crate) fn parse(value: Option<&Value>) -> Result<Self, Verdict> {
        let Some(value) = value else {
            return Err(mandate_absent("missing_mandate"));
        };
        serde_json::from_value(value.clone()).map_err(|error| {
            let detail = error.to_string();
            let rule = if detail.contains("unknown field") {
                "unknown_mandate_key"
            } else {
                "invalid_mandate"
            };
            mandate_absent(rule)
        })
    }

    pub(crate) fn evaluate(&self, facts: &TradeFacts<'_>) -> Verdict {
        if self.version != 1 {
            return mandate_absent("unsupported_mandate_version");
        }
        if self.can_withdraw {
            return Verdict::deny(
                "withdraw_not_supported",
                "World delegated trading authority can never grant withdrawal permission.",
            );
        }

        let product = normalize_product(facts.product);
        let permitted = self.markets.iter().any(|market| {
            normalize_product(&market.product) == product
                && market.base.eq_ignore_ascii_case(facts.base)
                && market.quote.eq_ignore_ascii_case(facts.quote)
        });
        if !permitted {
            return Verdict::deny(
                "market_not_permitted",
                format!(
                    "{product} {}/{} is not present in the mandate's markets.",
                    facts.base, facts.quote
                ),
            );
        }

        if facts.eligible_for_liquidation && self.halt_if_eligible_for_liquidation {
            return Verdict::deny(
                "liquidatable",
                "The live World account is eligible for liquidation and this mandate requires a halt.",
            );
        }

        let floor = match amount(&self.min_risk_adjusted_portfolio_value, facts.quote) {
            Ok(value) => value,
            Err(verdict) => return verdict,
        };
        if facts.risk_adjusted_portfolio_value < floor {
            return Verdict::deny(
                "portfolio_floor",
                format!(
                    "Live risk-adjusted portfolio value {} {} is below the mandate floor {} {}.",
                    facts.risk_adjusted_portfolio_value, facts.quote, floor, facts.quote
                ),
            );
        }

        let signed_quantity = if is_long_side(facts.side) {
            facts.quantity
        } else if is_short_side(facts.side) {
            -facts.quantity
        } else {
            return Verdict::deny(
                "invalid_side",
                format!(
                    "Unsupported trade side {:?}; expected buy/sell, long/short, or lend/borrow.",
                    facts.side
                ),
            );
        };
        let Some(projected_quantity) = facts.current_position_quantity.checked_add(signed_quantity)
        else {
            return Verdict::deny(
                "numeric_overflow",
                "Projected position quantity exceeds the supported numeric range.",
            );
        };
        if product == "spot" && projected_quantity.is_sign_negative() {
            return Verdict::deny(
                "insufficient_spot_balance",
                format!(
                    "The proposed sell would move the live {} spot balance below zero.",
                    facts.base
                ),
            );
        }
        let Some(projected_notional) = projected_quantity.abs().checked_mul(facts.mark_price)
        else {
            return Verdict::deny(
                "numeric_overflow",
                "Projected position notional exceeds the supported numeric range.",
            );
        };
        let notional_cap = match amount(&self.max_position_notional, facts.quote) {
            Ok(value) => value,
            Err(verdict) => return verdict,
        };
        if projected_notional > notional_cap {
            return Verdict::deny(
                "position_notional",
                format!(
                    "Projected {} position notional {} {} exceeds the mandate cap {} {}.",
                    facts.base, projected_notional, facts.quote, notional_cap, facts.quote
                ),
            );
        }

        let Some(post_trade_rapv) = facts.post_trade_risk_adjusted_portfolio_value else {
            return Verdict::deny(
                "post_trade_risk_unavailable",
                "The app cannot yet prove the post-trade risk-adjusted portfolio value, so the portfolio floor fails closed.",
            );
        };
        if post_trade_rapv < floor {
            return Verdict::deny(
                "post_trade_portfolio_floor",
                format!(
                    "Projected post-trade risk-adjusted portfolio value {} {} is below the mandate floor {} {}.",
                    post_trade_rapv, facts.quote, floor, facts.quote
                ),
            );
        }

        let max_leverage = match parse_positive(&self.max_leverage, "max_leverage") {
            Ok(value) => value,
            Err(verdict) => return verdict,
        };
        if post_trade_rapv <= Decimal::ZERO {
            return Verdict::deny(
                "leverage",
                "Leverage cannot be approved while risk-adjusted portfolio value is non-positive.",
            );
        }
        let Some(projected_leverage) = projected_notional.checked_div(post_trade_rapv) else {
            return Verdict::deny(
                "numeric_overflow",
                "Projected leverage exceeds the supported numeric range.",
            );
        };
        if projected_leverage > max_leverage {
            return Verdict::deny(
                "leverage",
                format!(
                    "Projected leverage {} exceeds the mandate cap {}.",
                    projected_leverage, max_leverage
                ),
            );
        }

        Verdict::allow(format!(
            "Mandate v1 permits {product} {} {} {} at live mark {}, with projected notional {} {} and leverage {}.",
            facts.side,
            facts.quantity,
            facts.base,
            facts.mark_price,
            projected_notional,
            facts.quote,
            projected_leverage
        ))
    }
}

pub(crate) fn parse_decimal(value: &str, field: &'static str) -> Result<Decimal, Verdict> {
    Decimal::from_str(value).map_err(|error| {
        Verdict::deny(
            "invalid_numeric_value",
            format!("{field} must be a decimal string: {error}"),
        )
    })
}

fn parse_positive(value: &str, field: &'static str) -> Result<Decimal, Verdict> {
    let value = parse_decimal(value, field)?;
    if value <= Decimal::ZERO {
        return Err(Verdict::deny(
            "invalid_numeric_value",
            format!("{field} must be greater than zero."),
        ));
    }
    Ok(value)
}

fn amount(limit: &AmountLimit, expected_quote: &str) -> Result<Decimal, Verdict> {
    if !limit.quote.eq_ignore_ascii_case(expected_quote) {
        return Err(Verdict::deny(
            "quote_mismatch",
            format!(
                "Mandate limit quote {} does not match market quote {expected_quote}.",
                limit.quote
            ),
        ));
    }
    parse_positive(&limit.amount, "mandate amount")
}

fn is_long_side(side: &str) -> bool {
    matches!(
        side.to_ascii_lowercase().as_str(),
        "buy" | "long" | "borrow"
    )
}

fn is_short_side(side: &str) -> bool {
    matches!(
        side.to_ascii_lowercase().as_str(),
        "sell" | "short" | "lend"
    )
}

fn normalize_product(product: &str) -> &str {
    match product {
        "perpetual" => "perp",
        "lending" => "lend",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mandate() -> Mandate {
        Mandate::parse(Some(&json!({
            "version": 1,
            "markets": [{ "product": "perp", "base": "WETH", "quote": "USDT" }],
            "max_position_notional": { "amount": "25000", "quote": "USDT" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDT" },
            "halt_if_eligible_for_liquidation": true,
            "can_withdraw": false
        })))
        .unwrap()
    }

    fn facts() -> TradeFacts<'static> {
        TradeFacts {
            product: "perp",
            side: "buy",
            base: "WETH",
            quote: "USDT",
            quantity: Decimal::new(1, 0),
            mark_price: Decimal::new(2_000, 0),
            current_position_quantity: Decimal::ZERO,
            risk_adjusted_portfolio_value: Decimal::new(10_000, 0),
            post_trade_risk_adjusted_portfolio_value: Some(Decimal::new(9_000, 0)),
            eligible_for_liquidation: false,
        }
    }

    #[test]
    fn allows_trade_inside_every_limit() {
        assert!(mandate().evaluate(&facts()).is_allow());
    }

    #[test]
    fn allows_long_and_borrow_side_aliases() {
        let mut trade = facts();
        trade.side = "long";
        assert!(mandate().evaluate(&trade).is_allow());

        let lend = Mandate::parse(Some(&json!({
            "version": 1,
            "markets": [{ "product": "lend", "base": "WETH", "quote": "USDT" }],
            "max_position_notional": { "amount": "25000", "quote": "USDT" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDT" },
            "halt_if_eligible_for_liquidation": true,
            "can_withdraw": false
        })))
        .unwrap();
        let mut trade = facts();
        trade.product = "lend";
        trade.side = "borrow";
        assert!(lend.evaluate(&trade).is_allow());
    }

    #[test]
    fn bound_local_env_fallbacks_are_serial() {
        // Env mutation is process-global; keep the three local-fallback cases
        // in one test so they cannot race with each other.
        let previous_json = std::env::var("WORLD_MANDATE_JSON").ok();
        let previous_path = std::env::var("WORLD_MANDATE_PATH").ok();

        let json = r#"{
            "version": 1,
            "markets": [{ "product": "perp", "base": "WETH", "quote": "USDT" }],
            "max_position_notional": { "amount": "25000", "quote": "USDT" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDT" },
            "halt_if_eligible_for_liquidation": true,
            "can_withdraw": false
        }"#;
        unsafe { std::env::set_var("WORLD_MANDATE_JSON", json) };
        let from_json = Mandate::bound(None).unwrap();
        assert_eq!(from_json.version, 1);

        unsafe { std::env::remove_var("WORLD_MANDATE_JSON") };
        unsafe { std::env::remove_var("WORLD_MANDATE_PATH") };
        let unset = Mandate::bound(None).unwrap();
        unsafe { std::env::set_var("WORLD_MANDATE_PATH", "placeholder") };
        let sentinel = Mandate::bound(None).unwrap();
        assert_eq!(unset.version, 1);
        assert!(unset.markets.iter().any(|market| market.base == "WETH"));
        assert_eq!(sentinel.markets.len(), unset.markets.len());

        unsafe { std::env::set_var("WORLD_MANDATE_PATH", "none") };
        let verdict = Mandate::bound(None).unwrap_err();
        assert_eq!(verdict.rule, "missing_mandate");

        match previous_json {
            Some(value) => unsafe { std::env::set_var("WORLD_MANDATE_JSON", value) },
            None => unsafe { std::env::remove_var("WORLD_MANDATE_JSON") },
        }
        match previous_path {
            Some(value) => unsafe { std::env::set_var("WORLD_MANDATE_PATH", value) },
            None => unsafe { std::env::remove_var("WORLD_MANDATE_PATH") },
        }
    }

    #[test]
    fn mandate_absent_family_shares_canonical_detail() {
        let missing = Mandate::parse(None).unwrap_err();
        assert_eq!(missing.rule, "missing_mandate");
        assert_eq!(missing.detail, MANDATE_ABSENT_DETAIL);

        let unknown = Mandate::parse(Some(&json!({
            "version": 1,
            "markets": [],
            "max_position_notional": { "amount": "25000", "quote": "USDT" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDT" },
            "halt_if_eligible_for_liquidation": true,
            "can_withdraw": false,
            "max_daily_loss": "10"
        })))
        .unwrap_err();
        assert_eq!(unknown.rule, "unknown_mandate_key");
        assert_eq!(unknown.detail, MANDATE_ABSENT_DETAIL);

        let invalid = Mandate::parse(Some(&json!("nope"))).unwrap_err();
        assert_eq!(invalid.rule, "invalid_mandate");
        assert_eq!(invalid.detail, MANDATE_ABSENT_DETAIL);

        let mut mandate = mandate();
        mandate.version = 2;
        let version = mandate.evaluate(&facts());
        assert_eq!(version.rule, "unsupported_mandate_version");
        assert_eq!(version.detail, MANDATE_ABSENT_DETAIL);

        assert!(
            !MANDATE_ABSENT_DETAIL.chars().any(|c| c.is_ascii_digit()),
            "mandate-absent detail must carry zero numbers"
        );
        for rule in MANDATE_ABSENT_RULES {
            assert!(is_mandate_absent(rule), "{rule}");
        }
    }

    #[test]
    fn rejects_unknown_key() {
        let value = json!({
            "version": 1,
            "markets": [],
            "max_position_notional": { "amount": "25000", "quote": "USDT" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDT" },
            "halt_if_eligible_for_liquidation": true,
            "can_withdraw": false,
            "max_daily_loss": "10"
        });
        assert_eq!(
            Mandate::parse(Some(&value)).unwrap_err().rule,
            "unknown_mandate_key"
        );
    }

    #[test]
    fn rejects_unknown_version() {
        let mut mandate = mandate();
        mandate.version = 2;
        assert_eq!(
            mandate.evaluate(&facts()).rule,
            "unsupported_mandate_version"
        );
    }

    #[test]
    fn rejects_when_post_trade_risk_cannot_be_proven() {
        let mut trade = facts();
        trade.post_trade_risk_adjusted_portfolio_value = None;
        assert_eq!(
            mandate().evaluate(&trade).rule,
            "post_trade_risk_unavailable"
        );

        trade.post_trade_risk_adjusted_portfolio_value = Some(Decimal::new(4_999, 0));
        assert_eq!(
            mandate().evaluate(&trade).rule,
            "post_trade_portfolio_floor"
        );
    }

    #[test]
    fn allows_when_post_trade_rapv_is_proven_and_clears_floor() {
        let mut trade = facts();
        trade.post_trade_risk_adjusted_portfolio_value = Some(Decimal::new(5_000, 0));
        let verdict = mandate().evaluate(&trade);
        assert!(verdict.is_allow(), "{}", verdict.detail);
        assert_eq!(verdict.rule, "mandate_v1");
    }

    #[test]
    fn rejects_unlisted_market_liquidation_and_limits() {
        let mut trade = facts();
        trade.base = "BTC.b";
        assert_eq!(mandate().evaluate(&trade).rule, "market_not_permitted");

        let mut trade = facts();
        trade.eligible_for_liquidation = true;
        assert_eq!(mandate().evaluate(&trade).rule, "liquidatable");

        let mut trade = facts();
        trade.quantity = Decimal::new(20, 0);
        assert_eq!(mandate().evaluate(&trade).rule, "position_notional");

        let mut trade = facts();
        trade.risk_adjusted_portfolio_value = Decimal::new(1_000, 0);
        assert_eq!(mandate().evaluate(&trade).rule, "portfolio_floor");
    }

    #[test]
    fn numeric_overflow_fails_closed() {
        let mut trade = facts();
        trade.quantity = Decimal::MAX;
        trade.mark_price = Decimal::MAX;
        assert_eq!(mandate().evaluate(&trade).rule, "numeric_overflow");
    }

    #[test]
    fn interim_account_and_brief_are_transport_only() {
        let value = json!({
            "version": 1,
            "markets": [{ "product": "perp", "base": "WETH", "quote": "USDT" }],
            "max_position_notional": { "amount": "25000", "quote": "USDT" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDT" },
            "halt_if_eligible_for_liquidation": true,
            "can_withdraw": false,
            "account": { "id": 42 },
            "brief": { "objective": "watch risk" }
        });
        let mandate = Mandate::parse(Some(&value)).unwrap();
        assert_eq!(mandate._account.unwrap()._id, 42);
        assert!(mandate._brief.is_some());
    }
}
