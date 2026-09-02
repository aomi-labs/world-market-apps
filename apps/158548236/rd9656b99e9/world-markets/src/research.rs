//! M7 research composition: live mark + stored move + cited news.
//!
//! Numbers come from tool-sourced fields. This module never calls execution.

use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::str::FromStr;

use crate::brain::BrainClient;
use crate::client::{Account, Asset, WorldClient, asset_by_symbol};
use crate::mandate::Mandate;

pub(crate) fn parse_lookback(raw: &str) -> u64 {
    match raw
        .trim()
        .trim_start_matches('/')
        .to_ascii_lowercase()
        .as_str()
    {
        "1w" | "w" | "week" | "7d" => 7 * 86400,
        "1m" | "m" | "month" | "30d" => 30 * 86400,
        _ => 86400,
    }
}

pub(crate) fn is_out_of_domain(symbol: &str, assets: &[Asset]) -> bool {
    asset_by_symbol(assets, symbol).is_err()
}

pub(crate) fn move_pct(mark_now: &str, mark_then: &str) -> Option<String> {
    let now = Decimal::from_str(mark_now).ok()?;
    let then = Decimal::from_str(mark_then).ok()?;
    if then.is_zero() {
        return None;
    }
    let pct = (now - then) / then * Decimal::from(100);
    Some(pct.round_dp(2).normalize().to_string())
}

pub(crate) struct ActionDoor {
    pub(crate) exposed: bool,
    pub(crate) position_size: Option<String>,
    pub(crate) position_side: Option<String>,
    pub(crate) mandate_bound: bool,
    pub(crate) preview_adjustment_available: bool,
}

pub(crate) fn action_door(
    symbol: &str,
    account: Option<&Account>,
    mandate: Option<&Mandate>,
) -> ActionDoor {
    let mut exposed = false;
    let mut position_size = None;
    let mut position_side = None;
    if let Some(account) = account {
        if let Some(pos) = account
            .perpetual_positions
            .iter()
            .find(|p| p.symbol.eq_ignore_ascii_case(symbol))
        {
            exposed = true;
            position_size = Some(pos.quantity.clone());
            position_side = Some(pos.side.clone());
        } else if let Some(bal) = account
            .balances
            .iter()
            .find(|b| b.symbol.eq_ignore_ascii_case(symbol))
            && bal.balance != "0"
            && bal.balance != "0.0"
        {
            exposed = true;
            position_size = Some(bal.balance.clone());
            position_side = Some("spot".to_string());
        }
    }
    let mandate_bound = mandate
        .map(|m| {
            m.markets.iter().any(|market| {
                market.base.eq_ignore_ascii_case(symbol)
                    || market.quote.eq_ignore_ascii_case(symbol)
            })
        })
        .unwrap_or(false);
    ActionDoor {
        exposed,
        position_size,
        position_side,
        mandate_bound,
        preview_adjustment_available: exposed || mandate_bound,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn compose(
    client: &WorldClient,
    brain: &BrainClient,
    symbol: &str,
    lookback: &str,
    product: &str,
    quote_symbol: &str,
    account: Option<&Account>,
    mandate: Option<&Mandate>,
    assets: &[Asset],
    portfolio_now: Option<&Value>,
) -> Result<Value, String> {
    if is_out_of_domain(symbol, assets) {
        return Ok(json!({
            "source": "world-markets-research",
            "executable": false,
            "domain_supported": false,
            "symbol": symbol,
        }));
    }
    let base = asset_by_symbol(assets, symbol)?;
    let quote = asset_by_symbol(assets, quote_symbol).ok();
    let market = client.market(product, base.clone(), quote)?;
    let window_secs = parse_lookback(lookback);
    let _ = brain.ingest(&json!({
        "symbol": base.symbol,
        "token_id": base.token_id,
        "mark": market.mark_price,
    }));
    let news = match brain.research(&base.symbol, window_secs) {
        Ok(value) => value,
        Err(_) => json!({
            "news_status": "unavailable",
            "sources": [],
            "cause_established": false,
            "attributions": [],
        }),
    };
    let history = brain.history_move(&base.symbol, window_secs).ok();
    let mark_then = history
        .as_ref()
        .and_then(|h| h.pointer("/mark_then/mark"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            news.pointer("/move/mark_then")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let window_move = mark_then
        .as_deref()
        .and_then(|then| move_pct(&market.mark_price, then));
    let door = action_door(&base.symbol, account, mandate);
    let mut impact_body = json!({
        "symbol": base.symbol,
        "mark": market.mark_price,
        "move_pct": window_move,
    });
    if let Some(before) = portfolio_now {
        impact_body["before"] = before.clone();
    }
    let impact = brain
        .portfolio_impact(&impact_body)
        .ok()
        .and_then(|v| v.get("impact").cloned());
    let after_present = impact
        .as_ref()
        .and_then(|v| v.get("after"))
        .is_some_and(|after| !after.is_null());
    let impact_ok = impact
        .as_ref()
        .and_then(|v| v.get("status"))
        .and_then(Value::as_str)
        == Some("ok")
        && after_present;
    let news_status = news
        .get("news_status")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    Ok(json!({
        "source": "world-markets-research",
        "executable": false,
        "domain_supported": true,
        "symbol": base.symbol,
        "product": market.product,
        "lookback": lookback,
        "window_secs": window_secs,
        "mark": {
            "value": market.mark_price,
            "source": "get_world_market",
        },
        "move": window_move.as_ref().map(|pct| json!({
            "pct": pct,
            "window": lookback,
            "mark_then": mark_then,
            "source": "world-markets-history",
        })),
        "sources": news.get("sources").cloned().unwrap_or(json!([])),
        "cause_established": news.get("cause_established").and_then(Value::as_bool).unwrap_or(false),
        "attributions": news.get("attributions").cloned().unwrap_or(json!([])),
        "news_status": news_status,
        "action_door": {
            "exposed": door.exposed,
            "position_size": door.position_size,
            "position_side": door.position_side,
            "mandate_bound": door.mandate_bound,
            "preview_adjustment_available": door.preview_adjustment_available,
            "trade_door": door.exposed || door.mandate_bound,
        },
        "portfolio_now": portfolio_now.cloned(),
        "portfolio_impact": if impact_ok { impact.clone() } else { None::<Value> },
        "portfolio_impact_status": impact.as_ref().and_then(|v| v.get("status")).cloned().unwrap_or(json!("unavailable")),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookback_windows() {
        assert_eq!(parse_lookback("1d"), 86400);
        assert_eq!(parse_lookback("24h"), 86400);
        assert_eq!(parse_lookback("1w"), 7 * 86400);
    }

    #[test]
    fn move_pct_is_tool_arithmetic_from_two_marks() {
        let pct = move_pct("2289", "2180").expect("pct");
        let parsed: f64 = pct.parse().unwrap();
        assert!((parsed - 5.0).abs() < 0.01);
    }

    #[test]
    fn equities_are_out_of_domain() {
        let assets: Vec<Asset> = Vec::new();
        assert!(is_out_of_domain("AAPL", &assets));
        assert!(is_out_of_domain("EURUSD", &assets));
    }
}
