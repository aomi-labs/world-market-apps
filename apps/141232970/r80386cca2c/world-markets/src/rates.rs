//! Live comparable rates for the strategy brain.
//!
//! All annualization and spread composition happens here in Rust. The model
//! must quote these decimal strings verbatim and must never add `lend_apr` to
//! `native_yield_apy`.

use std::collections::BTreeMap;
use std::str::FromStr;

use rust_decimal::Decimal;
use serde::Serialize;

use crate::client::{Asset, BASE_TOKEN_ID, CHAIN_ID, LendBookRates, WorldClient, asset_by_symbol};

/// 365 × 3 eight-hour funding intervals. Simple, non-compounded.
pub(crate) const FUNDING_PERIODS_PER_YEAR: i64 = 1095;

pub(crate) const RATES_DESCRIPTION: &str = "\
Live per-asset venue rates and the two composed spreads the strategy brain ranks on. \
Classic basis: borrow quote at borrow_apr, buy spot, short perp at funding_annualized; net = basis_spread_apr. \
Yield-bearing basis: hold yield-bearing spot (native_yield_apy), short perp, optionally borrow; net = yield_basis_spread_apr. \
Do not sum lend_apr with native_yield_apy. Funding per-8h annualizes ×1095 (simple annualization, not compounded). \
Native yield is WORLD_NATIVE_YIELDS or null. borrow_apr is quote taker-borrow; lend_apr is this asset's taker-lend. Never executes.";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AssetRates {
    pub(crate) base_symbol: String,
    pub(crate) quote_symbol: String,
    pub(crate) funding_rate_8h: Option<String>,
    pub(crate) funding_annualized: Option<String>,
    pub(crate) lend_apr: Option<String>,
    pub(crate) borrow_apr: Option<String>,
    pub(crate) native_yield_apy: Option<String>,
    pub(crate) native_yield_source: &'static str,
    pub(crate) basis_spread_apr: Option<String>,
    pub(crate) yield_basis_spread_apr: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RatesSnapshot {
    pub(crate) source: &'static str,
    pub(crate) chain_id: u64,
    pub(crate) exchange: String,
    pub(crate) block_number: String,
    pub(crate) executable: bool,
    pub(crate) quote_symbol: String,
    pub(crate) native_yield_note: &'static str,
    pub(crate) rates: Vec<AssetRates>,
}

pub(crate) fn parse_rate(raw: &str) -> Result<Decimal, String> {
    Decimal::from_str(raw.trim())
        .map_err(|e| format!("[world-markets] invalid decimal rate {raw:?}: {e}"))
}

pub(crate) fn annualize_funding_8h(rate_8h: Decimal) -> Decimal {
    rate_8h * Decimal::from(FUNDING_PERIODS_PER_YEAR)
}

/// 8h funding as a percent (0.0001 → `"0.01"`), matching watch `rate_pct`.
pub(crate) fn eight_hour_rate_as_pct(rate_8h: &str) -> Option<String> {
    let rate = parse_rate(rate_8h).ok()?;
    Some((rate * Decimal::from(100)).normalize().to_string())
}

pub(crate) fn classic_basis_spread(funding_annualized: Decimal, borrow_apr: Decimal) -> Decimal {
    funding_annualized - borrow_apr
}

pub(crate) fn yield_basis_spread(
    funding_annualized: Decimal,
    native_yield_apy: Decimal,
    borrow_apr: Decimal,
) -> Decimal {
    funding_annualized + native_yield_apy - borrow_apr
}

pub(crate) fn daily_carry_from_annual(spread_apr: Decimal) -> Decimal {
    spread_apr / Decimal::from(365)
}

pub(crate) fn load_native_yields() -> BTreeMap<String, String> {
    if let Ok(raw) = std::env::var("WORLD_NATIVE_YIELDS")
        && let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(&raw)
    {
        return normalize_yield_map(map);
    }
    if let Ok(path) = std::env::var("WORLD_NATIVE_YIELDS_PATH")
        && let Ok(raw) = std::fs::read_to_string(path)
        && let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(&raw)
    {
        return normalize_yield_map(map);
    }
    BTreeMap::new()
}

fn normalize_yield_map(map: BTreeMap<String, String>) -> BTreeMap<String, String> {
    map.into_iter()
        .map(|(k, v)| (k.to_ascii_uppercase(), v))
        .collect()
}

fn native_for(symbol: &str, table: &BTreeMap<String, String>) -> (Option<String>, &'static str) {
    match table.get(&symbol.to_ascii_uppercase()) {
        Some(value) if !value.is_empty() => (Some(value.clone()), "config"),
        _ => (None, "none"),
    }
}

fn dec_string(value: Decimal) -> String {
    value.normalize().to_string()
}

pub(crate) fn snapshot(
    client: &WorldClient,
    symbols: Option<&[String]>,
) -> Result<RatesSnapshot, String> {
    let assets = client.assets()?;
    let selected = select_assets(&assets, symbols)?;
    let quote = assets
        .iter()
        .find(|asset| asset.token_id == BASE_TOKEN_ID)
        .cloned()
        .ok_or_else(|| "[world-markets] quote token is missing from the asset list".to_string())?;
    let quote_borrow = client.lend_book_rates(quote.token_id)?.borrow_apr;
    let native_table = load_native_yields();
    let now = client.block_timestamp()?;
    let token_ids: Vec<u32> = selected.iter().map(|asset| asset.token_id).collect();
    let funding_ids: Vec<u32> = token_ids
        .iter()
        .copied()
        .filter(|id| *id != BASE_TOKEN_ID)
        .collect();
    let from = now.saturating_sub(8 * 3600);
    let _ = client.funding_rate_histories(from, now, &funding_ids)?;
    let books = client.lend_book_rates_many(&token_ids)?;
    let mut rates = Vec::new();
    for (asset, book) in selected.into_iter().zip(books) {
        rates.push(compose_asset_with_book(
            client,
            &asset,
            &quote.symbol,
            quote_borrow.as_deref(),
            &native_table,
            book,
        )?);
    }
    Ok(RatesSnapshot {
        source: "world-markets-contract",
        chain_id: CHAIN_ID,
        exchange: client.exchange(),
        block_number: client.block_number()?.to_string(),
        executable: false,
        quote_symbol: quote.symbol,
        native_yield_note: "native_yield_apy is operator config via WORLD_NATIVE_YIELDS; null when unknown. Illustrative ETH≈3% / SOL≈7.5% figures in strategy docs are not live.",
        rates,
    })
}

fn select_assets(assets: &[Asset], symbols: Option<&[String]>) -> Result<Vec<Asset>, String> {
    match symbols {
        None => Ok(assets.to_vec()),
        Some([]) => Ok(assets.to_vec()),
        Some(wanted) => {
            let mut out = Vec::new();
            for symbol in wanted {
                out.push(asset_by_symbol(assets, symbol)?);
            }
            Ok(out)
        }
    }
}

fn compose_asset_with_book(
    client: &WorldClient,
    asset: &Asset,
    quote_symbol: &str,
    quote_borrow_apr: Option<&str>,
    native_table: &BTreeMap<String, String>,
    book: LendBookRates,
) -> Result<AssetRates, String> {
    let funding_rate_8h = client.current_funding_rate_8h(asset.token_id)?;
    let funding_annualized = funding_rate_8h
        .as_deref()
        .map(parse_rate)
        .transpose()?
        .map(annualize_funding_8h)
        .map(dec_string);
    let lend_apr = book.lend_apr;
    let borrow_apr = quote_borrow_apr.map(ToString::to_string);
    let (native_yield_apy, native_yield_source) = native_for(&asset.symbol, native_table);

    let basis_spread_apr = match (funding_annualized.as_deref(), borrow_apr.as_deref()) {
        (Some(funding), Some(borrow)) => Some(dec_string(classic_basis_spread(
            parse_rate(funding)?,
            parse_rate(borrow)?,
        ))),
        _ => None,
    };
    let yield_basis_spread_apr = match (
        funding_annualized.as_deref(),
        native_yield_apy.as_deref(),
        borrow_apr.as_deref(),
    ) {
        (Some(funding), Some(native), Some(borrow)) => Some(dec_string(yield_basis_spread(
            parse_rate(funding)?,
            parse_rate(native)?,
            parse_rate(borrow)?,
        ))),
        _ => None,
    };

    Ok(AssetRates {
        base_symbol: asset.symbol.clone(),
        quote_symbol: quote_symbol.to_string(),
        funding_rate_8h,
        funding_annualized,
        lend_apr,
        borrow_apr,
        native_yield_apy,
        native_yield_source,
        basis_spread_apr,
        yield_basis_spread_apr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annualizes_eight_hour_funding_simple_non_compounded() {
        let eight_h = parse_rate("0.0001").unwrap();
        assert_eq!(
            annualize_funding_8h(eight_h).normalize().to_string(),
            "0.1095"
        );
    }

    #[test]
    fn eight_hour_rate_as_pct_matches_watch_rate_pct() {
        assert_eq!(eight_hour_rate_as_pct("0.0001").as_deref(), Some("0.01"));
    }

    #[test]
    fn classic_basis_is_funding_minus_borrow_only() {
        let spread =
            classic_basis_spread(parse_rate("0.12").unwrap(), parse_rate("0.055").unwrap());
        assert_eq!(spread.normalize().to_string(), "0.065");
    }

    #[test]
    fn yield_basis_adds_native_not_lend() {
        let spread = yield_basis_spread(
            parse_rate("0.12").unwrap(),
            parse_rate("0.03").unwrap(),
            parse_rate("0.055").unwrap(),
        );
        assert_eq!(spread.normalize().to_string(), "0.095");
        let with_lend_wrongly = spread + parse_rate("0.04").unwrap();
        assert_ne!(
            with_lend_wrongly.normalize().to_string(),
            spread.normalize().to_string()
        );
    }

    #[test]
    fn missing_native_does_not_default_to_illustrative_anchors() {
        let table = load_native_yields();
        let (value, source) = native_for("WETH", &table);
        assert!(value.is_none());
        assert_eq!(source, "none");
        assert!(!table.contains_key("SOL"));
    }

    #[test]
    fn description_documents_strategies_and_simple_annualization() {
        let d = RATES_DESCRIPTION;
        assert!(d.contains("Classic basis"));
        assert!(d.contains("Yield-bearing basis"));
        assert!(d.contains("×1095"));
        assert!(d.contains("simple, non-compounded") || d.contains("simple annualization"));
        assert!(!d.contains("ETH native ≈ 3%"));
        assert!(!d.contains("7.5%"));
    }
}
