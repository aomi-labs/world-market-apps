//! One-line lookup fields and host-ready reply lines for `get_world_account`.
//!
//! Derived figures the message layer interpolates into terse lookup copy — computed
//! in Rust so the model never ranks or sums notionals itself. `render_lookup` fills
//! the `b`/`p`/`r`/`a`/`d` templates so the host can skip the LLM.

use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use rust_decimal::prelude::Signed;
use serde::Serialize;
use serde_json::{Value, json};

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

pub(crate) fn notional_usdt(
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

/// Mark / price for read-back: thousands separators, no currency prefix.
/// Whole numbers when the mark is ≥ 100 (exemplar `~2,500`); otherwise 2 dp.
pub(crate) fn format_mark_human(mark: Decimal) -> String {
    let abs = mark.abs();
    let dp = if abs >= Decimal::from(100) { 0 } else { 2 };
    format_with_commas(abs, dp)
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

/// Whole-message terse lookup the host can answer without an LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LookupKind {
    Balance,
    Positions,
    Risk,
    Available,
    Dollarpower,
    Index,
}

pub(crate) const INDEX_LINE: &str = "One letter, one answer: `/b` balance · `/p` positions · `/r` risk · `/a` available · `/d` dollarpower. Or say what you want in a sentence.";
pub(crate) const AVAILABLE_UNAVAILABLE: &str = "Available to deploy isn't available from live reads yet — I can't quote it without an exact figure.";
pub(crate) const LEFT_OUT: &str = "I've left it out rather than guess.";
const POSITIONS_BUDGET: usize = 180;

impl LookupKind {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Balance => "b",
            Self::Positions => "p",
            Self::Risk => "r",
            Self::Available => "a",
            Self::Dollarpower => "d",
            Self::Index => "index",
        }
    }
}

/// Classify a whole user message. Does not treat `index` as user text.
pub(crate) fn parse_lookup_text(raw: &str) -> Option<LookupKind> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let lower = collapsed.to_ascii_lowercase();
    let stripped = lower.strip_prefix('/').unwrap_or(lower.as_str());
    match stripped {
        "b" | "balance" => Some(LookupKind::Balance),
        "p" | "positions" => Some(LookupKind::Positions),
        "r" | "risk" => Some(LookupKind::Risk),
        "a" | "available" => Some(LookupKind::Available),
        "d" | "dollarpower" => Some(LookupKind::Dollarpower),
        "?" | "commands" | "shortcuts" => Some(LookupKind::Index),
        "what can you do" | "what can you do?" => Some(LookupKind::Index),
        _ => None,
    }
}

/// Whole-message `cancel task <id>` — host skips the LLM and drops the watch.
pub(crate) fn parse_cancel_task(raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    if !parts[0].eq_ignore_ascii_case("cancel") {
        return None;
    }
    if !parts[1].eq_ignore_ascii_case("task") {
        return None;
    }
    let id = parts[2].trim().trim_start_matches('/');
    if id.is_empty() {
        return None;
    }
    Some(id.to_string())
}

/// Classify an explicit `token` argument from the model or host.
pub(crate) fn parse_lookup_token(raw: &str) -> Option<LookupKind> {
    let lower = raw.trim().to_ascii_lowercase();
    let stripped = lower.strip_prefix('/').unwrap_or(lower.as_str());
    match stripped {
        "index" => Some(LookupKind::Index),
        other => parse_lookup_text(other),
    }
}

pub(crate) fn render_balance(portfolio_value: &str) -> String {
    match money_code(portfolio_value, false) {
        Ok(figure) => format!("Portfolio {figure}."),
        Err(_) => LEFT_OUT.to_string(),
    }
}

pub(crate) fn render_risk(score: &str, eligible_for_liquidation: bool) -> String {
    let parsed = parse_decimal(score, "liquidation_risk").ok();
    let Some(score_dec) = parsed else {
        return LEFT_OUT.to_string();
    };
    let figure = format!("`{score}/10.`");
    if eligible_for_liquidation || score_dec >= Decimal::TEN {
        format!("Eligible for liquidation — liquidation risk {figure}")
    } else if score_dec >= Decimal::new(8, 0) {
        format!("Liquidation risk {figure} — high.")
    } else {
        format!("Liquidation risk {figure}")
    }
}

pub(crate) fn render_available(available_to_deploy: Option<&str>) -> String {
    match available_to_deploy {
        Some(raw) => match money_code(raw, false) {
            Ok(figure) => format!("Available to deploy {figure}."),
            Err(_) => LEFT_OUT.to_string(),
        },
        None => AVAILABLE_UNAVAILABLE.to_string(),
    }
}

pub(crate) fn render_dollarpower(
    ratio: &str,
    committed: &str,
    committed_estimate: bool,
    effective: &str,
    effective_estimate: bool,
) -> String {
    let committed = match money_code(committed, committed_estimate) {
        Ok(figure) => figure,
        Err(_) => return LEFT_OUT.to_string(),
    };
    let effective = match money_code(effective, effective_estimate) {
        Ok(figure) => figure,
        Err(_) => return LEFT_OUT.to_string(),
    };
    format!("Dollarpower `{ratio}`× — your {committed} is doing the work of {effective}.")
}

pub(crate) fn render_positions(positions: &PositionLookup) -> String {
    if positions.state != PositionLookupState::Normal {
        let cash = positions.cash_total.as_deref().unwrap_or("$0.00");
        return format!("No open positions. Cash `{cash}`.");
    }
    let mut parts: Vec<String> = positions.classes.iter().map(format_class_line).collect();
    for net in &positions.netting {
        parts.push(format!("`{}` {}", net.symbol, net.characterization));
    }
    join_within_budget(&parts, POSITIONS_BUDGET)
}

fn format_class_line(group: &PositionClassGroup) -> String {
    let (label, glyph) = match group.class {
        PositionClass::Holdings => ("Holdings", "◆"),
        PositionClass::Perps => ("Perps", "◇"),
        PositionClass::Lent => ("Lent", "◈"),
        PositionClass::Borrowed => ("Borrowed", "◈"),
    };
    let entries = group
        .entries
        .iter()
        .map(|entry| format!("`{} {}`", entry.label, entry.notional_usdt))
        .collect::<Vec<_>>()
        .join(" · ");
    let mut line = format!("**{label}** {glyph} {entries}");
    if group.remaining_count > 0 {
        line.push_str(&format!(" +{}", group.remaining_count));
    }
    line
}

fn join_within_budget(parts: &[String], budget: usize) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for part in parts {
        let mut candidate = kept.clone();
        candidate.push(part.as_str());
        let joined = candidate.join(" · ");
        if joined.chars().count() <= budget || kept.is_empty() {
            kept.push(part.as_str());
        } else {
            break;
        }
    }
    kept.join(" · ")
}

fn money_code(raw: &str, is_estimate: bool) -> Result<String, String> {
    let value = parse_decimal(raw, "money").map_err(|v| v.detail)?;
    Ok(format!("`{}`", format_money(value, is_estimate)))
}

pub(crate) fn format_money_str(raw: &str, is_estimate: bool) -> String {
    parse_decimal(raw, "money")
        .map(|v| format_money(v, is_estimate))
        .unwrap_or_else(|_| raw.to_string())
}

pub(crate) fn format_risk(raw: &str) -> String {
    let Ok(value) = parse_decimal(raw, "risk") else {
        return raw.to_string();
    };
    let rounded = value.round_dp_with_strategy(1, RoundingStrategy::MidpointAwayFromZero);
    let mut s = rounded.normalize().to_string();
    if !s.contains('.') {
        s.push_str(".0");
    }
    s
}

pub(crate) fn render_figure(value: &str, unit: &str, is_estimate: bool) -> String {
    if unit.eq_ignore_ascii_case("USDT") || unit == "$" {
        return format_money_str(value, is_estimate);
    }
    if unit == "×" || unit == "x" {
        return format!("{value}×");
    }
    value.to_string()
}

pub(crate) fn first_money_token(detail: &str) -> Option<String> {
    money_tokens(detail).into_iter().next()
}

pub(crate) fn last_money_token(detail: &str) -> Option<String> {
    money_tokens(detail).into_iter().last()
}

fn money_tokens(detail: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in detail.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            cur.push(ch);
        } else if !cur.is_empty() {
            if let Ok(v) = parse_decimal(&cur, "n") {
                out.push(format!("`{}`", format_money(v, false)));
            }
            cur.clear();
        }
    }
    if !cur.is_empty()
        && let Ok(v) = parse_decimal(&cur, "n")
    {
        out.push(format!("`{}`", format_money(v, false)));
    }
    out
}

pub(crate) fn rewrite_engine_numbers(detail: &str) -> String {
    let mut out = String::new();
    let mut cur = String::new();
    for ch in detail.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            cur.push(ch);
        } else {
            if !cur.is_empty() {
                if let Ok(v) = parse_decimal(&cur, "n") {
                    if cur.contains('.') && cur.split('.').nth(1).map(|f| f.len()).unwrap_or(0) > 2
                    {
                        out.push_str(&format!("`{}`", format_money(v, false)));
                    } else {
                        out.push_str(&cur);
                    }
                } else {
                    out.push_str(&cur);
                }
                cur.clear();
            }
            out.push(ch);
        }
    }
    if !cur.is_empty() {
        if let Ok(v) = parse_decimal(&cur, "n") {
            if cur.contains('.') && cur.split('.').nth(1).map(|f| f.len()).unwrap_or(0) > 2 {
                out.push_str(&format!("`{}`", format_money(v, false)));
            } else {
                out.push_str(&cur);
            }
        } else {
            out.push_str(&cur);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShareAsk {
    pub pct: Decimal,
    pub of: String,
    pub label: String,
}

/// "what's 20% of my portfolio" / "half of my SOL" / "10% of my SOL position"
pub(crate) fn parse_share_ask(raw: &str) -> Option<ShareAsk> {
    let lower = raw.trim().to_ascii_lowercase();
    if [
        "buy ", "sell ", "short ", "long ", "put ", "spend ", "invest ", "deploy ",
    ]
    .iter()
    .any(|verb| lower.contains(verb))
    {
        return None;
    }
    let has_pct = lower.contains('%')
        || lower.contains("percent")
        || lower.contains("half")
        || lower.contains("quarter");
    if !has_pct {
        return None;
    }
    let has_of = lower.contains("of my")
        || lower.contains("of the")
        || lower.contains("portfolio")
        || lower.contains("position")
        || lower.contains("account")
        || lower.contains("nav");
    if !has_of {
        return None;
    }
    let of = if lower.contains("portfolio") || lower.contains("account") || lower.contains("nav") {
        "portfolio".to_string()
    } else if let Some(sym) = share_symbol(&lower) {
        sym
    } else {
        "portfolio".to_string()
    };
    let pct = if lower.contains("half") {
        Decimal::new(50, 0)
    } else if lower.contains("quarter") {
        Decimal::new(25, 0)
    } else {
        regex_pct(&lower)?
    };
    Some(ShareAsk {
        pct,
        of: of.clone(),
        label: format!("{}% of {of}", pct.normalize()),
    })
}

pub(crate) fn share_from_lookups_json(lookups: &Value, ask: &ShareAsk) -> Option<Value> {
    let base = if ask.of == "portfolio" {
        let raw = lookups.get("portfolio_value").and_then(Value::as_str)?;
        parse_money_token(raw)?
    } else {
        position_notional_json(lookups, &ask.of)?
    };
    let amount = compute_share(ask.pct, base);
    let rendered = format_money(amount, false);
    Some(json!({
        "label": ask.label,
        "pct": ask.pct.normalize().to_string(),
        "of": ask.of,
        "base": format_money(base, false),
        "amount": rendered,
        "message": format!("{} is `{rendered}`.", ask.label),
    }))
}

fn parse_money_token(raw: &str) -> Option<Decimal> {
    let cleaned = raw
        .trim()
        .trim_start_matches('≈')
        .trim_start_matches('$')
        .replace(',', "");
    parse_decimal(&cleaned, "money").ok()
}

fn position_notional_json(lookups: &Value, symbol: &str) -> Option<Decimal> {
    let classes = lookups.pointer("/positions/classes")?.as_array()?;
    for group in classes {
        let entries = group.get("entries")?.as_array()?;
        for entry in entries {
            let sym = entry.get("symbol").and_then(Value::as_str).unwrap_or("");
            if sym.eq_ignore_ascii_case(symbol) {
                let raw = entry.get("notional_usdt").and_then(Value::as_str)?;
                return parse_money_token(raw);
            }
        }
    }
    let netting = lookups.pointer("/positions/netting")?.as_array()?;
    for row in netting {
        let sym = row.get("symbol").and_then(Value::as_str).unwrap_or("");
        if sym.eq_ignore_ascii_case(symbol) {
            let raw = row.get("net_notional_usdt").and_then(Value::as_str)?;
            return parse_money_token(raw);
        }
    }
    None
}

fn regex_pct(lower: &str) -> Option<Decimal> {
    let chars: Vec<char> = lower.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let num: String = chars[start..i].iter().collect();
            let rest: String = chars[i..].iter().collect();
            if rest.trim_start().starts_with('%') || rest.trim_start().starts_with("percent") {
                return parse_decimal(&num, "pct").ok();
            }
        }
        i += 1;
    }
    None
}

fn share_symbol(lower: &str) -> Option<String> {
    for needle in ["weth", "eth", "wbtc", "btc", "sol", "usdt"] {
        if lower.contains(needle) {
            return Some(needle.to_ascii_uppercase());
        }
    }
    None
}

pub(crate) fn compute_share(pct: Decimal, base: Decimal) -> Decimal {
    (base * pct) / Decimal::new(100, 0)
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
    fn format_mark_human_uses_thousands_separators() {
        assert_eq!(format_mark_human(d("2500")), "2,500");
        assert_eq!(format_mark_human(d("2465.71")), "2,466");
        assert_eq!(format_mark_human(d("1.23")), "1.23");
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
    fn parse_share_ask_reads_percent_of_portfolio() {
        let ask = parse_share_ask("what's 20% of my portfolio?").unwrap();
        assert_eq!(ask.pct, d("20"));
        assert_eq!(ask.of, "portfolio");
        assert!(parse_share_ask("buy half").is_none());
        assert!(parse_share_ask("sell 20% of my portfolio").is_none());
        let half = parse_share_ask("what's half of my SOL position?").unwrap();
        assert_eq!(half.pct, d("50"));
        assert_eq!(half.of, "SOL");
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

    #[test]
    fn parse_lookup_text_matches_tokens_and_word_forms() {
        for (input, kind) in [
            ("b", LookupKind::Balance),
            ("/b", LookupKind::Balance),
            ("  B  ", LookupKind::Balance),
            ("balance", LookupKind::Balance),
            ("/BALANCE", LookupKind::Balance),
            ("/positions", LookupKind::Positions),
            ("risk", LookupKind::Risk),
            ("/r", LookupKind::Risk),
            ("a", LookupKind::Available),
            ("available", LookupKind::Available),
            ("d", LookupKind::Dollarpower),
            ("D", LookupKind::Dollarpower),
            ("dollarpower", LookupKind::Dollarpower),
            ("?", LookupKind::Index),
            ("/?", LookupKind::Index),
            ("commands", LookupKind::Index),
            ("  shortcuts  ", LookupKind::Index),
            ("what can you do?", LookupKind::Index),
            ("What can you do", LookupKind::Index),
            ("what   can   you   do?", LookupKind::Index),
        ] {
            assert_eq!(parse_lookup_text(input), Some(kind), "{input}");
        }
    }

    #[test]
    fn parse_lookup_text_rejects_prose_and_help() {
        for input in [
            "",
            "   ",
            "what's my balance?",
            "buy",
            "help",
            "/help",
            "index",
            "weth d",
            "how am I doing?",
            "b p",
            "paper",
            "what can you do today",
        ] {
            assert_eq!(parse_lookup_text(input), None, "{input}");
        }
    }

    #[test]
    fn parse_lookup_token_accepts_index() {
        assert_eq!(parse_lookup_token("index"), Some(LookupKind::Index));
        assert_eq!(parse_lookup_token(" /Index "), Some(LookupKind::Index));
        assert_eq!(parse_lookup_token("b"), Some(LookupKind::Balance));
        assert_eq!(parse_lookup_token("nope"), None);
    }

    #[test]
    fn parse_cancel_task_is_whole_message_only() {
        assert_eq!(
            parse_cancel_task("cancel task abc12x"),
            Some("abc12x".into())
        );
        assert_eq!(
            parse_cancel_task("Cancel Task ABC12X"),
            Some("ABC12X".into())
        );
        assert_eq!(
            parse_cancel_task("cancel task i_roll"),
            Some("i_roll".into())
        );
        assert_eq!(parse_cancel_task("please cancel task abc12x"), None);
        assert_eq!(parse_cancel_task("cancel"), None);
        assert_eq!(parse_cancel_task("cancel the task"), None);
        assert_eq!(parse_cancel_task("cancel task"), None);
    }

    #[test]
    fn index_line_matches_skill_copy() {
        assert!(
            include_str!("skill/lookups.md").contains(INDEX_LINE),
            "INDEX_LINE must stay verbatim with lookups.md"
        );
        assert!(include_str!("skill/lookups.md").contains(AVAILABLE_UNAVAILABLE));
        assert!(include_str!("skill/lookups.md").contains(LEFT_OUT));
    }

    #[test]
    fn render_balance_wraps_money() {
        assert_eq!(render_balance("1234.5"), "Portfolio `$1,234.50`.");
        assert_eq!(render_balance("not-a-number"), LEFT_OUT);
    }

    #[test]
    fn render_risk_uses_bands() {
        assert_eq!(render_risk("3.8", false), "Liquidation risk `3.8/10.`");
        assert_eq!(render_risk("7.9", false), "Liquidation risk `7.9/10.`");
        assert_eq!(
            render_risk("8.0", false),
            "Liquidation risk `8.0/10.` — high."
        );
        assert_eq!(
            render_risk("9.9", false),
            "Liquidation risk `9.9/10.` — high."
        );
        assert_eq!(
            render_risk("10.0", false),
            "Eligible for liquidation — liquidation risk `10.0/10.`"
        );
        assert_eq!(
            render_risk("3.8", true),
            "Eligible for liquidation — liquidation risk `3.8/10.`"
        );
        assert_eq!(render_risk("n/a", false), LEFT_OUT);
    }

    #[test]
    fn render_available_refuses_when_absent() {
        assert_eq!(render_available(None), AVAILABLE_UNAVAILABLE);
        assert_eq!(
            render_available(Some("2500")),
            "Available to deploy `$2,500.00`."
        );
        assert_eq!(render_available(Some("nope")), LEFT_OUT);
    }

    #[test]
    fn render_dollarpower_translates_dollars() {
        assert_eq!(
            render_dollarpower("2.4", "10300", true, "24700", true),
            "Dollarpower `2.4`× — your `≈$10,300` is doing the work of `≈$24,700`."
        );
        assert_eq!(
            render_dollarpower("2.4", "10300", false, "24700", false),
            "Dollarpower `2.4`× — your `$10,300.00` is doing the work of `$24,700.00`."
        );
        assert_eq!(
            render_dollarpower("2.4", "bad", true, "24700", true),
            LEFT_OUT
        );
    }

    fn class_group(
        class: PositionClass,
        label: &str,
        notional: &str,
        remaining: u32,
    ) -> PositionClassGroup {
        PositionClassGroup {
            class,
            entries: vec![ExposureEntry {
                symbol: label.to_string(),
                label: label.to_string(),
                notional_usdt: notional.to_string(),
            }],
            remaining_count: remaining,
        }
    }

    #[test]
    fn render_positions_empty_and_classes() {
        let empty = PositionLookup {
            quote: QUOTE,
            classes: vec![],
            netting: vec![],
            missing_mark_symbols: vec![],
            state: PositionLookupState::Empty,
            cash_total: None,
            baseline: "test".to_string(),
        };
        assert_eq!(render_positions(&empty), "No open positions. Cash `$0.00`.");

        let cash_only = PositionLookup {
            quote: QUOTE,
            classes: vec![],
            netting: vec![],
            missing_mark_symbols: vec![],
            state: PositionLookupState::CashOnly,
            cash_total: Some("$1,000.00".into()),
            baseline: "test".to_string(),
        };
        assert_eq!(
            render_positions(&cash_only),
            "No open positions. Cash `$1,000.00`."
        );

        let populated = PositionLookup {
            quote: QUOTE,
            classes: vec![
                class_group(PositionClass::Holdings, "USDT", "$1,000.00", 0),
                class_group(PositionClass::Perps, "WBTC short", "$2,707.71", 1),
                class_group(PositionClass::Lent, "WETH", "$500.00", 0),
                class_group(PositionClass::Borrowed, "USDT", "$200.00", 0),
            ],
            netting: vec![NettingLine {
                symbol: "WETH".into(),
                net_notional_usdt: "≈$0".into(),
                is_estimate: true,
                characterization: "hedged".into(),
            }],
            missing_mark_symbols: vec![],
            state: PositionLookupState::Normal,
            cash_total: Some("$1,000.00".into()),
            baseline: "test".to_string(),
        };
        let line = render_positions(&populated);
        assert!(line.contains("**Holdings** ◆ `USDT $1,000.00`"));
        assert!(line.contains("**Perps** ◇ `WBTC short $2,707.71` +1"));
        assert!(line.contains("**Lent** ◈ `WETH $500.00`"));
        assert!(line.contains("**Borrowed** ◈ `USDT $200.00`"));
        assert!(line.contains("`WETH` hedged"));
        assert!(line.chars().count() <= POSITIONS_BUDGET);
    }

    #[test]
    fn render_positions_drops_later_classes_to_keep_budget() {
        let bulky = "X".repeat(70);
        let positions = PositionLookup {
            quote: QUOTE,
            classes: vec![
                class_group(PositionClass::Holdings, &bulky, "$1.00", 0),
                class_group(PositionClass::Perps, &bulky, "$2.00", 0),
                class_group(PositionClass::Lent, &bulky, "$3.00", 0),
            ],
            netting: vec![],
            missing_mark_symbols: vec![],
            state: PositionLookupState::Normal,
            cash_total: None,
            baseline: "test".to_string(),
        };
        let line = render_positions(&positions);
        assert!(line.contains("**Holdings**"));
        assert!(
            !line.contains("**Lent**"),
            "later classes must yield to the 180-char budget: {line}"
        );
        assert!(line.chars().count() <= POSITIONS_BUDGET, "{line}");
    }

    #[test]
    fn cpu_lookup_render_is_far_under_500ms() {
        use std::time::Instant;
        let start = Instant::now();
        for _ in 0..1_000 {
            let _ = parse_lookup_text(" /balance ");
            let _ = render_balance("12345.67");
            let _ = render_risk("3.8", false);
            let _ = render_available(None);
            let _ = render_dollarpower("2.4", "10300", true, "24700", true);
        }
        let elapsed = start.elapsed();
        println!("1000 cpu lookup renders {elapsed:?}");
        assert!(
            elapsed.as_millis() < 50,
            "pure render path must be << 500ms, took {elapsed:?}"
        );
    }
}
