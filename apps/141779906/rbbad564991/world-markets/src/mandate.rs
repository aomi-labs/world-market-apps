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

impl Mandate {
    pub(crate) fn parse(value: Option<&Value>) -> Result<Self, Verdict> {
        let Some(value) = value else {
            return Err(Verdict::deny(
                "missing_mandate",
                "No handover_mandate is bound to this turn.",
            ));
        };
        serde_json::from_value(value.clone()).map_err(|error| {
            let detail = error.to_string();
            let rule = if detail.contains("unknown field") {
                "unknown_mandate_key"
            } else {
                "invalid_mandate"
            };
            Verdict::deny(rule, detail)
        })
    }

    pub(crate) fn evaluate(&self, facts: &TradeFacts<'_>) -> Verdict {
        if self.version != 1 {
            return Verdict::deny(
                "unsupported_mandate_version",
                format!(
                    "Mandate version {} is not supported; expected version 1.",
                    self.version
                ),
            );
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

        let signed_quantity = if facts.side.eq_ignore_ascii_case("buy") {
            facts.quantity
        } else if facts.side.eq_ignore_ascii_case("sell") {
            -facts.quantity
        } else {
            return Verdict::deny(
                "invalid_side",
                format!(
                    "Unsupported trade side {:?}; expected buy or sell.",
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

fn normalize_product(product: &str) -> &str {
    match product {
        "perpetual" => "perp",
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
            "markets": [{ "product": "perp", "base": "WETH", "quote": "USDm" }],
            "max_position_notional": { "amount": "25000", "quote": "USDm" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDm" },
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
            quote: "USDm",
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
    fn rejects_unknown_key() {
        let value = json!({
            "version": 1,
            "markets": [],
            "max_position_notional": { "amount": "25000", "quote": "USDm" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDm" },
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
            "markets": [{ "product": "perp", "base": "WETH", "quote": "USDm" }],
            "max_position_notional": { "amount": "25000", "quote": "USDm" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDm" },
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
