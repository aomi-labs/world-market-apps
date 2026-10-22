//! Order type and limit price for one venue order.
//!
//! World books take limit orders only. A "market" intent is expressed as a
//! limit at the live mark moved by the slippage tolerance in the taker's
//! favour of filling; the preview reports which of the two the user asked for.

use rust_decimal::Decimal;

pub(crate) const DEFAULT_SLIPPAGE: Decimal = Decimal::from_parts(5, 0, 0, false, 3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderIntent {
    /// `market` or `limit` — what the user asked for.
    pub(crate) order_type: &'static str,
    /// The price the order word carries. For `market` this is the mark
    /// shifted by `slippage`.
    pub(crate) limit_price: Decimal,
    /// Slippage tolerance applied to a market intent (decimal, 0.005 = 0.5%).
    pub(crate) slippage: Decimal,
}

impl OrderIntent {
    /// Explicit `price` → limit at that price. Otherwise market-as-limit at
    /// `mark × (1 + slippage)` for a buy and `mark × (1 − slippage)` for a
    /// sell. `order_type = "limit"` without a price is an error.
    pub(crate) fn resolve(
        named: Option<&str>,
        price: Option<Decimal>,
        slippage: Option<Decimal>,
        side: &str,
        mark: Decimal,
    ) -> Result<Self, String> {
        let named = named.map(|value| value.trim().to_ascii_lowercase());
        let slippage = slippage.unwrap_or(DEFAULT_SLIPPAGE);
        if slippage.is_sign_negative() || slippage >= Decimal::ONE {
            return Err(format!(
                "[world-markets] slippage {slippage} must be a decimal in [0, 1)"
            ));
        }
        match (named.as_deref(), price) {
            (Some("market"), _) | (None, None) => {
                let factor = if matches!(side, "buy" | "long") {
                    Decimal::ONE + slippage
                } else {
                    Decimal::ONE - slippage
                };
                let limit_price = mark.checked_mul(factor).ok_or_else(|| {
                    "[world-markets] limit price exceeds numeric range".to_string()
                })?;
                Ok(Self {
                    order_type: "market",
                    limit_price,
                    slippage,
                })
            }
            (Some("limit") | None, Some(price)) => {
                if price <= Decimal::ZERO {
                    return Err("[world-markets] limit price must be greater than zero".to_string());
                }
                Ok(Self {
                    order_type: "limit",
                    limit_price: price,
                    slippage,
                })
            }
            (Some("limit"), None) => {
                Err("[world-markets] order_type limit needs a price".to_string())
            }
            (Some(other), _) => Err(format!(
                "[world-markets] unsupported order_type {other:?}; use market or limit"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(raw: &str) -> Decimal {
        Decimal::from_str(raw).unwrap()
    }

    #[test]
    fn explicit_price_is_limit() {
        let intent =
            OrderIntent::resolve(None, Some(d("2000")), None, "buy", d("2465.71")).unwrap();
        assert_eq!(intent.order_type, "limit");
        assert_eq!(intent.limit_price, d("2000"));
        assert_eq!(intent.slippage, DEFAULT_SLIPPAGE);
    }

    #[test]
    fn market_is_limit_at_mark_moved_by_slippage() {
        let buy = OrderIntent::resolve(None, None, None, "buy", d("2000")).unwrap();
        assert_eq!(buy.order_type, "market");
        assert_eq!(buy.limit_price, d("2010"));
        let sell =
            OrderIntent::resolve(Some("market"), None, Some(d("0.01")), "sell", d("2000")).unwrap();
        assert_eq!(sell.limit_price, d("1980"));
        assert_eq!(sell.slippage, d("0.01"));
    }

    #[test]
    fn market_named_with_price_stays_market() {
        let intent =
            OrderIntent::resolve(Some("market"), Some(d("1")), None, "buy", d("2000")).unwrap();
        assert_eq!(intent.order_type, "market");
        assert_eq!(intent.limit_price, d("2010"));
    }

    #[test]
    fn limit_without_price_and_bad_inputs_error() {
        assert!(OrderIntent::resolve(Some("limit"), None, None, "buy", d("2000")).is_err());
        assert!(OrderIntent::resolve(Some("twap"), None, None, "buy", d("2000")).is_err());
        assert!(OrderIntent::resolve(None, Some(d("0")), None, "buy", d("2000")).is_err());
        assert!(OrderIntent::resolve(None, None, Some(d("1")), "buy", d("2000")).is_err());
    }
}
