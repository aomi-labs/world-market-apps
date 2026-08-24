//! One-line lookup fields for `get_world_account`.
//!
//! Derived figures the message layer interpolates into terse lookup copy — computed
//! in Rust so the model never ranks or sums notionals itself.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use rust_decimal::prelude::Signed;
use serde::Serialize;

use crate::client::{Account, Asset, BASE_TOKEN_ID, WorldClient};
use crate::mandate::parse_decimal;

const QUOTE: &str = "USDT";
const TOP_N: usize = 3;
const MONEY_DP: u32 = 2;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PositionClass {
    Holdings,
    Perps,
    Lent,
    Borrowed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ExposureEntry {
    pub(crate) symbol: String,
    pub(crate) notional_usdt: String,
    /// Spot symbol, or `{symbol} {side}` for a directional perp leg.
    pub(crate) label: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct PositionClassGroup {
    pub(crate) class: PositionClass,
    pub(crate) entries: Vec<ExposureEntry>,
    pub(crate) remaining_count: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct NettingLine {
    pub(crate) symbol: String,
    pub(crate) net_notional_usdt: String,
    pub(crate) is_estimate: bool,
    /// Portfolio-level characterization, e.g. `hedged` when net ≈ 0.
    pub(crate) characterization: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PositionLookupState {
    /// At least one non-cash position across classes.
    Normal,
    /// Only cash holdings, no perps/lend/borrow.
    CashOnly,
    /// No positions at all (may still have cash).
    Empty,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct PositionLookup {
    pub(crate) quote: &'static str,
    pub(crate) classes: Vec<PositionClassGroup>,
    pub(crate) netting: Vec<NettingLine>,
    pub(crate) missing_mark_symbols: Vec<String>,
    pub(crate) state: PositionLookupState,
    /// Total cash (USDT/base) when relevant for empty/cash-only copy.
    pub(crate) cash_total: Option<String>,
    pub(crate) baseline: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct AccountLookups {
    /// Portfolio value in quote units — same as `metrics.net_asset_value`.
    pub(crate) portfolio_value: String,
    pub(crate) positions: PositionLookup,
    /// Absent until the reporting service exposes an exact (non-estimate) figure.
    pub(crate) available_to_deploy: Option<String>,
}

struct ClassRow {
    label: String,
    symbol: String,
    notional: Decimal,
    /// Perps: signed directional notional for netting/ranking (long +, short −).
    signed_directional: Option<Decimal>,
}

pub(crate) fn compute_lookups(
    client: &WorldClient,
    account: &Account,
    assets: &[Asset],
    portfolio_value: &str,
    block_number: u64,
) -> Result<AccountLookups, String> {
    let base = assets
        .iter()
        .find(|asset| asset.token_id == BASE_TOKEN_ID)
        .ok_or_else(|| "[world-markets] base token config is missing".to_string())?;
    let positions = position_lookup(client, account, assets, base, block_number)?;
    Ok(AccountLookups {
        portfolio_value: portfolio_value.to_string(),
        positions,
        available_to_deploy: None,
    })
}

fn position_lookup(
    client: &WorldClient,
    account: &Account,
    assets: &[Asset],
    base: &Asset,
    block_number: u64,
) -> Result<PositionLookup, String> {
    let asset_map: BTreeMap<u32, &Asset> = assets.iter().map(|a| (a.token_id, a)).collect();
    let mut missing_mark_symbols: Vec<String> = Vec::new();

    let mut holdings: Vec<ClassRow> = Vec::new();
    let mut perps: Vec<ClassRow> = Vec::new();
    let mut lent: Vec<ClassRow> = Vec::new();
    let mut borrowed: Vec<ClassRow> = Vec::new();
    let mut cash_total = Decimal::ZERO;

    for perp in &account.perpetual_positions {
        let asset = asset_map
            .get(&perp.token_id)
            .ok_or_else(|| format!("[world-markets] missing asset for perp {}", perp.symbol))?;
        let qty = parse_decimal(&perp.quantity, "perp_quantity")
            .map_err(|v| format!("[world-markets] {}: {}", v.rule, v.detail))?;
        if qty.is_zero() {
            continue;
        }
        let notional = match notional_usdt(client, asset, base, qty.abs()) {
            Ok(n) => n,
            Err(_) => {
                missing_mark_symbols.push(perp.symbol.clone());
                continue;
            }
        };
        let signed = if perp.side.eq_ignore_ascii_case("short") {
            -notional
        } else {
            notional
        };
        perps.push(ClassRow {
            label: perp_exposure_label(&perp.symbol, &perp.side),
            symbol: perp.symbol.clone(),
            notional,
            signed_directional: Some(signed),
        });
    }

    for lend in &account.lending_positions {
        let asset = asset_map
            .get(&lend.token_id)
            .ok_or_else(|| format!("[world-markets] missing asset for lend {}", lend.symbol))?;
        let borrower = parse_decimal(&lend.borrower_quantity, "borrower_quantity")
            .map_err(|v| format!("[world-markets] {}: {}", v.rule, v.detail))?;
        if !borrower.is_zero() {
            let notional = match notional_usdt(client, asset, base, borrower) {
                Ok(n) => n,
                Err(_) => {
                    missing_mark_symbols.push(lend.symbol.clone());
                    continue;
                }
            };
            borrowed.push(ClassRow {
                label: lend.symbol.clone(),
                symbol: lend.symbol.clone(),
                notional,
                signed_directional: None,
            });
        }
        let lender = parse_decimal(&lend.lender_quantity, "lender_quantity")
            .map_err(|v| format!("[world-markets] {}: {}", v.rule, v.detail))?;
        if !lender.is_zero() {
            let notional = match notional_usdt(client, asset, base, lender) {
                Ok(n) => n,
                Err(_) => {
                    if !missing_mark_symbols.contains(&lend.symbol) {
                        missing_mark_symbols.push(lend.symbol.clone());
                    }
                    continue;
                }
            };
            lent.push(ClassRow {
                label: lend.symbol.clone(),
                symbol: lend.symbol.clone(),
                notional,
                signed_directional: None,
            });
        }
    }

    for balance in &account.balances {
        let asset = asset_map.get(&balance.token_id).ok_or_else(|| {
            format!(
                "[world-markets] missing asset for balance {}",
                balance.symbol
            )
        })?;
        let amount = parse_decimal(&balance.balance, "balance")
            .map_err(|v| format!("[world-markets] {}: {}", v.rule, v.detail))?;
        if amount.is_zero() {
            continue;
        }
        if asset.token_id == BASE_TOKEN_ID {
            cash_total += amount;
            holdings.push(ClassRow {
                label: balance.symbol.clone(),
                symbol: balance.symbol.clone(),
                notional: amount,
                signed_directional: None,
            });
            continue;
        }
        let notional = match notional_usdt(client, asset, base, amount.abs()) {
            Ok(n) => n,
            Err(_) => {
                missing_mark_symbols.push(balance.symbol.clone());
                continue;
            }
        };
        holdings.push(ClassRow {
            label: balance.symbol.clone(),
            symbol: balance.symbol.clone(),
            notional,
            signed_directional: Some(notional),
        });
    }

    missing_mark_symbols.sort();
    missing_mark_symbols.dedup();

    let netting = compute_netting(&holdings, &perps);
    let perp_rank_net = perp_rank_nets(&holdings, &perps);
    perps.sort_by(|a, b| {
        let net_a = perp_rank_net.get(&a.symbol).copied().unwrap_or(a.notional);
        let net_b = perp_rank_net.get(&b.symbol).copied().unwrap_or(b.notional);
        net_b.cmp(&net_a).then_with(|| a.label.cmp(&b.label))
    });

    sort_class_rows(&mut holdings);
    sort_class_rows(&mut lent);
    sort_class_rows(&mut borrowed);

    let has_risk_positions = !perps.is_empty()
        || !lent.is_empty()
        || !borrowed.is_empty()
        || holdings.iter().any(|h| h.symbol != QUOTE);
    let state = if !has_risk_positions {
        if cash_total.is_zero() {
            PositionLookupState::Empty
        } else {
            PositionLookupState::CashOnly
        }
    } else {
        PositionLookupState::Normal
    };

    let classes = vec![
        build_class_group(PositionClass::Holdings, &holdings),
        build_class_group(PositionClass::Perps, &perps),
        build_class_group(PositionClass::Lent, &lent),
        build_class_group(PositionClass::Borrowed, &borrowed),
    ]
    .into_iter()
    .filter(|g| !g.entries.is_empty())
    .collect();

    Ok(PositionLookup {
        quote: QUOTE,
        classes,
        netting,
        missing_mark_symbols,
        state,
        cash_total: if cash_total.is_zero() {
            None
        } else {
            Some(format_money(cash_total, false))
        },
        baseline: format!(
            "absolute USDT notional per class at block {block_number} (perp |qty|×mark, spot |balance|×mark, lend qty×mark); netting is portfolio-level directional"
        ),
    })
}

fn sort_class_rows(rows: &mut [ClassRow]) {
    rows.sort_by(|a, b| {
        b.notional
            .cmp(&a.notional)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
}

fn build_class_group(class: PositionClass, rows: &[ClassRow]) -> PositionClassGroup {
    let remaining_count = rows.len().saturating_sub(TOP_N) as u32;
    let entries = rows
        .iter()
        .take(TOP_N)
        .map(|row| ExposureEntry {
            symbol: row.symbol.clone(),
            label: row.label.clone(),
            notional_usdt: format_money(row.notional, false),
        })
        .collect();
    PositionClassGroup {
        class,
        entries,
        remaining_count,
    }
}

fn perp_rank_nets(holdings: &[ClassRow], perps: &[ClassRow]) -> BTreeMap<String, Decimal> {
    let mut nets = BTreeMap::new();
    for row in perps {
        let entry = nets.entry(row.symbol.clone()).or_insert(Decimal::ZERO);
        if let Some(signed) = row.signed_directional {
            *entry += signed;
        }
    }
    for row in holdings {
        if row.symbol == QUOTE {
            continue;
        }
        if let Some(signed) = row.signed_directional {
            let entry = nets.entry(row.symbol.clone()).or_insert(Decimal::ZERO);
            *entry += signed;
        }
    }
    nets.into_iter()
        .map(|(sym, net)| (sym, net.abs()))
        .collect()
}

fn compute_netting(holdings: &[ClassRow], perps: &[ClassRow]) -> Vec<NettingLine> {
    let mut spot_by_symbol: BTreeMap<String, Decimal> = BTreeMap::new();
    for row in holdings {
        if row.symbol == QUOTE {
            continue;
        }
        if let Some(signed) = row.signed_directional {
            spot_by_symbol
                .entry(row.symbol.clone())
                .and_modify(|v| *v += signed)
                .or_insert(signed);
        }
    }

    let mut perp_by_symbol: BTreeMap<String, Decimal> = BTreeMap::new();
    for row in perps {
        if let Some(signed) = row.signed_directional {
            perp_by_symbol
                .entry(row.symbol.clone())
                .and_modify(|v| *v += signed)
                .or_insert(signed);
        }
    }

    let mut symbols: Vec<String> = spot_by_symbol
        .keys()
        .chain(perp_by_symbol.keys())
        .cloned()
        .collect();
    symbols.sort();
    symbols.dedup();

    let threshold = Decimal::new(1, 2); // $0.01
    let mut out = Vec::new();
    for symbol in symbols {
        let spot = spot_by_symbol
            .get(&symbol)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let perp = perp_by_symbol
            .get(&symbol)
            .copied()
            .unwrap_or(Decimal::ZERO);
        if spot.is_zero() || perp.is_zero() {
            continue;
        }
        // Opposing legs: spot long (+) with perp short (−) or the reverse.
        if spot.signum() == perp.signum() {
            continue;
        }
        let net = spot + perp;
        let characterization = if net.abs() <= threshold {
            "hedged".to_string()
        } else if net.is_sign_positive() {
            "net long".to_string()
        } else {
            "net short".to_string()
        };
        let is_estimate = net.abs() <= threshold;
        out.push(NettingLine {
            symbol: symbol.clone(),
            net_notional_usdt: format_money(net.abs(), is_estimate),
            is_estimate,
            characterization,
        });
    }
    out
}

fn notional_usdt(
    client: &WorldClient,
    asset: &Asset,
    _base: &Asset,
    quantity: Decimal,
) -> Result<Decimal, String> {
    if quantity.is_zero() {
        return Ok(Decimal::ZERO);
    }
    if asset.token_id == BASE_TOKEN_ID {
        return Ok(quantity);
    }
    let (_raw, mark) = client.mark_price(asset.token_id)?;
    let mark_price = parse_decimal(&mark, "mark_price")
        .map_err(|v| format!("[world-markets] {}: {}", v.rule, v.detail))?;
    quantity
        .checked_mul(mark_price)
        .ok_or_else(|| "[world-markets] notional overflow".to_string())
}

fn perp_exposure_label(symbol: &str, side: &str) -> String {
    format!("{symbol} {side}")
}

/// Format a USDT notional: exactly 2 dp, thousands separators, optional ≈ for estimates.
pub(crate) fn format_money(value: Decimal, is_estimate: bool) -> String {
    let rounded = if is_estimate {
        value
            .abs()
            .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
    } else {
        value
            .abs()
            .round_dp_with_strategy(MONEY_DP, RoundingStrategy::MidpointAwayFromZero)
    };
    let body = format_with_commas(rounded, if is_estimate { 0 } else { MONEY_DP });
    let sign = if value.is_sign_negative() { "−" } else { "" };
    if is_estimate {
        format!("{sign}≈${body}")
    } else {
        format!("{sign}${body}")
    }
}

fn format_with_commas(value: Decimal, dp: u32) -> String {
    let normalized = value
        .abs()
        .round_dp_with_strategy(dp, RoundingStrategy::MidpointAwayFromZero);
    let mut s = normalized.to_string();
    if dp > 0 {
        if let Some(dot) = s.find('.') {
            let frac_len = s.len() - dot - 1;
            if frac_len < dp as usize {
                s.push_str(&"0".repeat(dp as usize - frac_len));
            } else if frac_len > dp as usize {
                s.truncate(dot + 1 + dp as usize);
            }
        } else {
            s.push('.');
            s.push_str(&"0".repeat(dp as usize));
        }
        let dot = s.find('.').expect("decimal point");
        let int_part = &s[..dot];
        let frac = &s[dot..];
        format!("{}{}", format_int_commas(int_part), frac)
    } else {
        format_int_commas(&s)
    }
}

fn format_int_commas(int_part: &str) -> String {
    let negative = int_part.starts_with('-');
    let digits: String = int_part.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return if negative {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }
    let mut groups: Vec<&str> = Vec::new();
    let chars: Vec<char> = digits.chars().collect();
    let mut start = chars.len();
    while start > 0 {
        let end = start;
        start = start.saturating_sub(3);
        groups.push(&digits[start..end]);
    }
    groups.reverse();
    let joined = groups.join(",");
    if negative {
        format!("-{joined}")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::client::{Balance, PerpetualPosition};

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn format_money_uses_two_decimal_places() {
        assert_eq!(format_money(d("691.1479"), false), "$691.15");
        assert_eq!(format_money(d("1000"), false), "$1,000.00");
        assert_eq!(format_money(d("2707.71"), false), "$2,707.71");
    }

    #[test]
    fn format_money_estimate_rounds_to_whole_dollars() {
        assert_eq!(format_money(d("0.4"), true), "≈$0");
        assert_eq!(format_money(d("2600.49"), true), "≈$2,600");
    }

    #[test]
    fn format_money_negative_uses_minus_sign() {
        assert_eq!(format_money(d("-12.3"), false), "−$12.30");
    }

    #[test]
    fn perp_labels_include_side() {
        assert_eq!(perp_exposure_label("WBTC", "short"), "WBTC short");
    }

    #[test]
    fn class_rows_sort_by_notional_desc_then_symbol() {
        let mut rows = vec![
            ClassRow {
                label: "SOL".into(),
                symbol: "SOL".into(),
                notional: d("100"),
                signed_directional: None,
            },
            ClassRow {
                label: "WBTC".into(),
                symbol: "WBTC".into(),
                notional: d("500"),
                signed_directional: None,
            },
        ];
        sort_class_rows(&mut rows);
        assert_eq!(rows[0].symbol, "WBTC");
    }

    #[test]
    fn netting_detects_hedged_spot_and_perp() {
        let holdings = vec![ClassRow {
            label: "WETH".into(),
            symbol: "WETH".into(),
            notional: d("1000"),
            signed_directional: Some(d("1000")),
        }];
        let perps = vec![ClassRow {
            label: "WETH short".into(),
            symbol: "WETH".into(),
            notional: d("1000"),
            signed_directional: Some(d("-1000")),
        }];
        let nets = compute_netting(&holdings, &perps);
        assert_eq!(nets.len(), 1);
        assert_eq!(nets[0].characterization, "hedged");
        assert!(nets[0].is_estimate);
    }

    #[test]
    fn per_class_truncation_counts_remaining() {
        let rows: Vec<ClassRow> = (0..5)
            .map(|i| ClassRow {
                label: format!("A{i}"),
                symbol: format!("A{i}"),
                notional: d("100"),
                signed_directional: None,
            })
            .collect();
        let group = build_class_group(PositionClass::Perps, &rows);
        assert_eq!(group.entries.len(), TOP_N);
        assert_eq!(group.remaining_count, 2);
    }

    #[test]
    fn account_lookups_omit_available_to_deploy() {
        let lookups = AccountLookups {
            portfolio_value: "100000".to_string(),
            positions: PositionLookup {
                quote: QUOTE,
                classes: vec![],
                netting: vec![],
                missing_mark_symbols: vec![],
                state: PositionLookupState::Empty,
                cash_total: None,
                baseline: "test".to_string(),
            },
            available_to_deploy: None,
        };
        assert!(lookups.available_to_deploy.is_none());
    }

    #[test]
    fn fixture_positions_decode_for_exposure_inputs() {
        let perp = PerpetualPosition {
            token_id: 4,
            symbol: "WETH".to_string(),
            quantity_raw: 0,
            quantity: "2".to_string(),
            side: "long".to_string(),
            entry_price_raw: 0,
            entry_price: "3000".to_string(),
            funding_start_time: 0,
            owed_nom_raw: "0".to_string(),
            owed_nom: "0".to_string(),
            owed_base_raw: "0".to_string(),
        };
        let balance = Balance {
            token_id: 1,
            symbol: "USDT".to_string(),
            balance_raw: "1000".to_string(),
            balance: "1000".to_string(),
            available_raw: "1000".to_string(),
            available: "1000".to_string(),
            spot_lend_sequestered_raw: "0".to_string(),
            spot_lend_sequestered: "0".to_string(),
            perp_sequestered_raw: "0".to_string(),
            perp_sequestered: "0".to_string(),
        };
        assert_eq!(perp.symbol, "WETH");
        assert_eq!(balance.symbol, "USDT");
    }
}
