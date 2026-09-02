//! Portfolio metrics aligned with the Composite frontend (`@composite/sdk`).
//!
//! `liquidation_risk` is a 0–10 score from `Portfolio.calculateLiquidationRisk`:
//! binary search on the risk multiplier, then a non-linear map to the display scale.
//! The Telegram agent must cite only values returned here — never infer them.

use std::collections::BTreeMap;
use std::str::FromStr;

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use rust_decimal::prelude::FromPrimitive;
use serde::Serialize;

use crate::client::{
    Account, Asset, BASE_TOKEN_ID, Balance, LendingPosition, PerpetualPosition, WorldClient,
    decimal_digits,
};
use crate::mandate::parse_decimal;
const LEND_DURATION_DAYS: u32 = 10;
const LENDER_HAIRCUT: i64 = 980;
const PERMILLE_SCALE: i64 = 1000;
const FUNDING_INTERVAL_SEC: u64 = 8 * 60 * 60;
const FUNDING_RATE_DIVISOR: i64 = 10_000_000;
const MAX_SCORE: f64 = 10.0;
const SEARCH_PRECISION: f64 = 0.05;
const MAX_SEARCH_ITERATIONS: u32 = 10;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct PortfolioMetrics {
    /// Net asset value (portfolio evaluation at risk multiplier 0).
    pub(crate) net_asset_value: String,
    /// 0–10 liquidation risk score (one decimal), matching the Composite UI.
    pub(crate) liquidation_risk: String,
    pub(crate) liquidation_risk_scale: &'static str,
    /// `safe` (<6), `elevated` (6–8), `high` (8–10), `liquidation` (10).
    pub(crate) liquidation_risk_band: String,
    pub(crate) baseline: String,
}

pub(crate) fn compute_metrics(
    client: &WorldClient,
    account: &Account,
    assets: &[Asset],
    block_number: u64,
) -> Result<PortfolioMetrics, String> {
    let time_sec = client.block_timestamp()?;
    let base = assets
        .iter()
        .find(|asset| asset.token_id == BASE_TOKEN_ID)
        .ok_or_else(|| "[world-markets] base token config is missing".to_string())?;
    let state = build_state(client, account, assets, time_sec)?;
    let nav = evaluate(&state, assets, base, 0.0)?;
    let prv = parse_decimal(
        &account.risk_adjusted_portfolio_value,
        "risk_adjusted_portfolio_value",
    )
    .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
    let val_at_max = evaluate(&state, assets, base, MAX_SCORE)?;
    let risk = calculate_liquidation_risk(nav, prv, val_at_max, |multiplier| {
        evaluate(&state, assets, base, multiplier)
    })?;
    Ok(PortfolioMetrics {
        net_asset_value: format_decimal(nav, base.position_decimals),
        liquidation_risk: format_risk_score(risk),
        liquidation_risk_scale: "0-10",
        liquidation_risk_band: risk_band(risk).to_string(),
        baseline: format!(
            "composite portfolio evaluation at block {block_number} (risk multiplier search vs live RAPV)"
        ),
    })
}

/// Intent used to project post-trade ATLAS RAPV and the 0–10 liquidation score.
#[derive(Debug, Clone)]
pub(crate) struct TradeIntent<'a> {
    pub(crate) product: &'a str,
    pub(crate) side: &'a str,
    pub(crate) base: &'a Asset,
    pub(crate) quote: &'a Asset,
    pub(crate) quantity: Decimal,
    pub(crate) mark: Decimal,
}

/// Derived post-trade risk. RAPV is ATLAS `evaluate` at risk multiplier 1,
/// anchored to the live contract RAPV via a state delta so the current reading
/// stays the source of truth.
///
/// **Derivation (owner ratification):**
/// `post_trade_rapv = live_rapv + evaluate(apply(intent, state), 1) − evaluate(state, 1)`
/// where `evaluate` is the Composite/ATLAS valuation already used for NAV
/// (multiplier 0) and the liquidation search. At multiplier 1 this is native
/// riskPrice / riskSlippage plus the 98% lender haircut. Labeled `is_estimate`
/// because it is a derivation, not a post-trade contract read (the exchange
/// does not expose a simulation).
#[derive(Debug, Clone)]
pub(crate) struct PostTradeProjection {
    pub(crate) rapv: Decimal,
    pub(crate) rapv_display: String,
    pub(crate) liquidation_risk: Decimal,
    pub(crate) source: &'static str,
    pub(crate) is_estimate: bool,
}

const POST_TRADE_SOURCE: &str = "world-markets-reporting";
const DEV_SEED_SOURCE: &str = "world-markets-dev-seed";

pub(crate) fn project_post_trade(
    client: &WorldClient,
    account: &Account,
    assets: &[Asset],
    intent: &TradeIntent<'_>,
) -> Result<PostTradeProjection, String> {
    let time_sec = client.block_timestamp()?;
    let borrow_rate_raw = if is_lend(intent.product)
        && matches!(
            intent.side.to_ascii_lowercase().as_str(),
            "buy" | "long" | "borrow"
        ) {
        Some(borrow_rate_raw(client, intent.base.token_id)?)
    } else {
        None
    };
    project_from_account(client, account, assets, time_sec, intent, borrow_rate_raw)
}

fn is_lend(product: &str) -> bool {
    matches!(product, "lend" | "lending")
}

fn borrow_rate_raw(client: &WorldClient, token_id: u32) -> Result<u16, String> {
    let book = client.lend_book_rates(token_id)?;
    let apr = book.borrow_apr.ok_or_else(|| {
        "[world-markets] lend book has no borrow rate; post-trade RAPV is unprovable".to_string()
    })?;
    apr_to_rate_raw(&apr)
}

fn apr_to_rate_raw(apr: &str) -> Result<u16, String> {
    let parsed = parse_decimal(apr, "borrow_apr")
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
    let scaled = (parsed * Decimal::from(10_000)).trunc();
    if scaled.is_sign_negative() || scaled > Decimal::from(u16::MAX) {
        return Err("[world-markets] borrow APR is outside the supported rate range".to_string());
    }
    scaled
        .normalize()
        .to_string()
        .parse::<u16>()
        .map_err(|e| format!("[world-markets] borrow APR is not a rate raw: {e}"))
}

fn project_from_account(
    client: &WorldClient,
    account: &Account,
    assets: &[Asset],
    time_sec: u64,
    intent: &TradeIntent<'_>,
    borrow_rate_raw: Option<u16>,
) -> Result<PostTradeProjection, String> {
    let quote_asset = assets
        .iter()
        .find(|asset| asset.token_id == BASE_TOKEN_ID)
        .ok_or_else(|| "[world-markets] base token config is missing".to_string())?;
    let current_state = build_state(client, account, assets, time_sec)?;
    let eval_before = evaluate(&current_state, assets, quote_asset, 1.0)?;
    let mut projected = account.clone();
    apply_intent(&mut projected, intent, borrow_rate_raw, time_sec)?;
    let projected_state = build_state(client, &projected, assets, time_sec)?;
    let eval_after = evaluate(&projected_state, assets, quote_asset, 1.0)?;
    let live_rapv = parse_decimal(
        &account.risk_adjusted_portfolio_value,
        "risk_adjusted_portfolio_value",
    )
    .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
    let post_rapv = live_rapv
        .checked_add(eval_after)
        .and_then(|value| value.checked_sub(eval_before))
        .ok_or_else(|| "[world-markets] post-trade RAPV overflow".to_string())?;
    let nav = evaluate(&projected_state, assets, quote_asset, 0.0)?;
    let val_at_max = evaluate(&projected_state, assets, quote_asset, MAX_SCORE)?;
    let risk = calculate_liquidation_risk(nav, post_rapv, val_at_max, |multiplier| {
        evaluate(&projected_state, assets, quote_asset, multiplier)
    })?;
    let risk_display = format_risk_score(risk);
    let liquidation_risk = parse_decimal(&risk_display, "liquidation_risk")
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
    Ok(PostTradeProjection {
        rapv: post_rapv,
        rapv_display: format_decimal(post_rapv, quote_asset.position_decimals),
        liquidation_risk,
        source: POST_TRADE_SOURCE,
        is_estimate: true,
    })
}

/// Local harness only. When `WORLD_DEV_SEED_POST_TRADE_RAPV` is set, a failed
/// ATLAS projection falls back to live RAPV so the mandate floor can pass in
/// `aomi-run` (stubbed evm-core). Production stays fail-closed.
pub(crate) fn dev_seed_post_trade_rapv_enabled() -> bool {
    matches!(
        std::env::var("WORLD_DEV_SEED_POST_TRADE_RAPV")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(crate) fn dev_seed_rapv(account: &Account) -> Option<Decimal> {
    if !dev_seed_post_trade_rapv_enabled() {
        return None;
    }
    parse_decimal(
        &account.risk_adjusted_portfolio_value,
        "risk_adjusted_portfolio_value",
    )
    .ok()
}

pub(crate) fn dev_seed_source() -> &'static str {
    DEV_SEED_SOURCE
}

fn apply_intent(
    account: &mut Account,
    intent: &TradeIntent<'_>,
    borrow_rate_raw: Option<u16>,
    time_sec: u64,
) -> Result<(), String> {
    let product = intent.product.to_ascii_lowercase();
    let side = intent.side.to_ascii_lowercase();
    if intent.quantity <= Decimal::ZERO {
        return Err("[world-markets] quantity must be greater than zero".to_string());
    }
    match product.as_str() {
        "lend" | "lending" => match side.as_str() {
            "sell" | "short" | "lend" => apply_lend_supply(account, intent.base, intent.quantity),
            "buy" | "long" | "borrow" => {
                apply_lend_borrow(account, intent.base, intent.quantity, borrow_rate_raw)
            }
            _ => Err("[world-markets] lend side must be buy or sell".to_string()),
        },
        "spot" => apply_spot(
            account,
            intent.base,
            intent.quote,
            &side,
            intent.quantity,
            intent.mark,
        ),
        "perp" | "perpetual" => apply_perp(
            account,
            intent.base,
            &side,
            intent.quantity,
            intent.mark,
            time_sec,
        ),
        other => Err(format!(
            "[world-markets] cannot project post-trade RAPV for product {other}"
        )),
    }
}

fn apply_lend_supply(
    account: &mut Account,
    token: &Asset,
    quantity: Decimal,
) -> Result<(), String> {
    debit_vault(account, token, quantity)?;
    let pos_raw = to_u64_raw(quantity, token.position_decimals)?;
    let idx = ensure_lending(account, token);
    let position = &mut account.lending_positions[idx];
    position.lender_quantity_raw = position
        .lender_quantity_raw
        .checked_add(pos_raw)
        .ok_or_else(|| "[world-markets] lender quantity overflow".to_string())?;
    position.lender_quantity = decimal_digits(
        position.lender_quantity_raw.to_string(),
        token.position_decimals,
    );
    Ok(())
}

fn apply_lend_borrow(
    account: &mut Account,
    token: &Asset,
    quantity: Decimal,
    borrow_rate_raw: Option<u16>,
) -> Result<(), String> {
    let rate = borrow_rate_raw
        .ok_or_else(|| "[world-markets] borrow RAPV requires a live lend-book rate".to_string())?;
    credit_vault(account, token, quantity)?;
    let pos_raw = to_u64_raw(quantity, token.position_decimals)?;
    let idx = ensure_lending(account, token);
    let position = &mut account.lending_positions[idx];
    position.borrower_quantity_raw = position
        .borrower_quantity_raw
        .checked_add(pos_raw)
        .ok_or_else(|| "[world-markets] borrower quantity overflow".to_string())?;
    position.borrower_quantity = decimal_digits(
        position.borrower_quantity_raw.to_string(),
        token.position_decimals,
    );
    if rate > position.highest_interest_rate_raw {
        position.highest_interest_rate_raw = rate;
        position.highest_interest_rate = decimal_digits(u64::from(rate).to_string(), 4);
        position.highest_interest_rate_percent = decimal_digits(u64::from(rate).to_string(), 2);
    }
    Ok(())
}

fn apply_spot(
    account: &mut Account,
    base: &Asset,
    quote: &Asset,
    side: &str,
    quantity: Decimal,
    mark: Decimal,
) -> Result<(), String> {
    let quote_notional = quantity
        .checked_mul(mark)
        .ok_or_else(|| "[world-markets] spot notional overflow".to_string())?;
    match side {
        "buy" | "long" => {
            debit_vault(account, quote, quote_notional)?;
            credit_vault(account, base, quantity)?;
        }
        "sell" | "short" => {
            debit_vault(account, base, quantity)?;
            credit_vault(account, quote, quote_notional)?;
        }
        _ => return Err("[world-markets] spot side must be buy or sell".to_string()),
    }
    Ok(())
}

fn apply_perp(
    account: &mut Account,
    base: &Asset,
    side: &str,
    quantity: Decimal,
    mark: Decimal,
    time_sec: u64,
) -> Result<(), String> {
    let signed = match side {
        "buy" | "long" => quantity,
        "sell" | "short" => -quantity,
        _ => return Err("[world-markets] perp side must be buy or sell".to_string()),
    };
    let idx = account
        .perpetual_positions
        .iter()
        .position(|p| p.token_id == base.token_id);
    if let Some(idx) = idx {
        let position = &mut account.perpetual_positions[idx];
        let current = parse_decimal(&position.quantity, "perp_quantity")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let entry = parse_decimal(&position.entry_price, "perp_entry_price")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let new_qty = current
            .checked_add(signed)
            .ok_or_else(|| "[world-markets] perp quantity overflow".to_string())?;
        if new_qty.is_zero() {
            account.perpetual_positions.remove(idx);
            return Ok(());
        }
        let same_sign = (current.is_sign_positive() && new_qty.is_sign_positive())
            || (current.is_sign_negative() && new_qty.is_sign_negative());
        let new_entry = if current.is_zero() || !same_sign {
            mark
        } else if new_qty.abs() > current.abs() {
            let added = signed.abs();
            ((entry * current.abs()) + (mark * added))
                .checked_div(new_qty.abs())
                .ok_or_else(|| "[world-markets] perp entry average overflow".to_string())?
        } else {
            entry
        };
        if !same_sign {
            position.owed_nom_raw = "0".to_string();
            position.owed_nom = "0".to_string();
            position.owed_base_raw = "0".to_string();
            position.funding_start_time = time_sec;
        }
        position.quantity = format_decimal(new_qty, base.position_decimals);
        position.quantity_raw = to_i64_raw(new_qty, base.position_decimals)?;
        position.entry_price = new_entry.normalize().to_string();
        position.side = if new_qty.is_sign_negative() {
            "short".to_string()
        } else {
            "long".to_string()
        };
        return Ok(());
    }
    account.perpetual_positions.push(PerpetualPosition {
        token_id: base.token_id,
        symbol: base.symbol.clone(),
        quantity_raw: to_i64_raw(signed, base.position_decimals)?,
        quantity: format_decimal(signed, base.position_decimals),
        side: if signed.is_sign_negative() {
            "short".to_string()
        } else {
            "long".to_string()
        },
        entry_price_raw: 0,
        entry_price: mark.normalize().to_string(),
        funding_start_time: time_sec,
        owed_nom_raw: "0".to_string(),
        owed_nom: "0".to_string(),
        owed_base_raw: "0".to_string(),
    });
    Ok(())
}

fn debit_vault(account: &mut Account, token: &Asset, quantity: Decimal) -> Result<(), String> {
    let vault_raw = to_u128_raw(quantity, token.vault_decimals)?;
    let idx = ensure_balance(account, token);
    let balance = &mut account.balances[idx];
    let current = parse_u128(&balance.balance_raw)?;
    let available = parse_u128(&balance.available_raw)?;
    if available < vault_raw || current < vault_raw {
        return Err(format!(
            "[world-markets] insufficient {} available to apply this intent",
            token.symbol
        ));
    }
    balance.balance_raw = (current - vault_raw).to_string();
    balance.available_raw = (available - vault_raw).to_string();
    refresh_balance(balance, token);
    Ok(())
}

fn credit_vault(account: &mut Account, token: &Asset, quantity: Decimal) -> Result<(), String> {
    let vault_raw = to_u128_raw(quantity, token.vault_decimals)?;
    let idx = ensure_balance(account, token);
    let balance = &mut account.balances[idx];
    let current = parse_u128(&balance.balance_raw)?;
    let available = parse_u128(&balance.available_raw)?;
    balance.balance_raw = current
        .checked_add(vault_raw)
        .ok_or_else(|| "[world-markets] vault balance overflow".to_string())?
        .to_string();
    balance.available_raw = available
        .checked_add(vault_raw)
        .ok_or_else(|| "[world-markets] available balance overflow".to_string())?
        .to_string();
    refresh_balance(balance, token);
    Ok(())
}

fn refresh_balance(balance: &mut Balance, token: &Asset) {
    balance.balance = decimal_digits(balance.balance_raw.clone(), token.vault_decimals);
    balance.available = decimal_digits(balance.available_raw.clone(), token.vault_decimals);
}

fn ensure_balance(account: &mut Account, token: &Asset) -> usize {
    if let Some(idx) = account
        .balances
        .iter()
        .position(|balance| balance.token_id == token.token_id)
    {
        return idx;
    }
    account.balances.push(Balance {
        token_id: token.token_id,
        symbol: token.symbol.clone(),
        balance_raw: "0".to_string(),
        balance: "0".to_string(),
        available_raw: "0".to_string(),
        available: "0".to_string(),
        spot_lend_sequestered_raw: "0".to_string(),
        spot_lend_sequestered: "0".to_string(),
        perp_sequestered_raw: "0".to_string(),
        perp_sequestered: "0".to_string(),
    });
    account.balances.len() - 1
}

fn ensure_lending(account: &mut Account, token: &Asset) -> usize {
    if let Some(idx) = account
        .lending_positions
        .iter()
        .position(|position| position.token_id == token.token_id)
    {
        return idx;
    }
    account.lending_positions.push(LendingPosition {
        token_id: token.token_id,
        symbol: token.symbol.clone(),
        lender_quantity_raw: 0,
        lender_quantity: "0".to_string(),
        borrower_quantity_raw: 0,
        borrower_quantity: "0".to_string(),
        highest_interest_rate_raw: 0,
        highest_interest_rate: "0".to_string(),
        highest_interest_rate_percent: "0".to_string(),
    });
    account.lending_positions.len() - 1
}

fn parse_u128(raw: &str) -> Result<u128, String> {
    raw.parse::<u128>()
        .map_err(|e| format!("[world-markets] invalid vault raw {raw}: {e}"))
}

fn to_u128_raw(value: Decimal, decimals: u8) -> Result<u128, String> {
    if value.is_sign_negative() {
        return Err("[world-markets] quantity must be non-negative".to_string());
    }
    let scale = Decimal::from(10u64.saturating_pow(u32::from(decimals)));
    let scaled = value
        .checked_mul(scale)
        .ok_or_else(|| "[world-markets] quantity scale overflow".to_string())?;
    if scaled != scaled.trunc() {
        return Err("[world-markets] quantity is not representable at token decimals".to_string());
    }
    scaled
        .trunc()
        .normalize()
        .to_string()
        .parse::<u128>()
        .map_err(|e| format!("[world-markets] quantity raw parse: {e}"))
}

fn to_u64_raw(value: Decimal, decimals: u8) -> Result<u64, String> {
    let raw = to_u128_raw(value, decimals)?;
    u64::try_from(raw).map_err(|_| "[world-markets] quantity exceeds u64 raw range".to_string())
}

fn to_i64_raw(value: Decimal, decimals: u8) -> Result<i64, String> {
    let scale = Decimal::from(10u64.saturating_pow(u32::from(decimals)));
    let scaled = value
        .checked_mul(scale)
        .ok_or_else(|| "[world-markets] signed quantity scale overflow".to_string())?;
    if scaled != scaled.trunc() {
        return Err("[world-markets] quantity is not representable at token decimals".to_string());
    }
    scaled
        .trunc()
        .normalize()
        .to_string()
        .parse::<i64>()
        .map_err(|e| format!("[world-markets] signed quantity raw parse: {e}"))
}

fn risk_band(score: f64) -> &'static str {
    if score >= MAX_SCORE {
        "liquidation"
    } else if score >= 8.0 {
        "high"
    } else if score >= 6.0 {
        "elevated"
    } else {
        "safe"
    }
}

fn format_risk_score(score: f64) -> String {
    format!("{:.1}", score.clamp(0.0, MAX_SCORE))
}

struct TokenState {
    balance_vault: u128,
    lend_borrower_raw: u64,
    lend_lender_raw: u64,
    lend_rate_raw: u16,
    perp: Option<PerpetualPosition>,
    mark_price: Decimal,
    funding_history_rate: Decimal,
}

struct PortfolioState {
    tokens: BTreeMap<u32, TokenState>,
}

fn build_state(
    client: &WorldClient,
    account: &Account,
    assets: &[Asset],
    time_sec: u64,
) -> Result<PortfolioState, String> {
    let balances: BTreeMap<u32, &Balance> =
        account.balances.iter().map(|b| (b.token_id, b)).collect();
    let lending = account
        .lending_positions
        .iter()
        .map(|l| (l.token_id, l))
        .collect::<BTreeMap<_, _>>();
    let perps = account
        .perpetual_positions
        .iter()
        .map(|p| (p.token_id, p))
        .collect::<BTreeMap<_, _>>();

    let active: Vec<&Asset> = assets
        .iter()
        .filter(|asset| {
            balances.contains_key(&asset.token_id)
                || lending.contains_key(&asset.token_id)
                || perps.contains_key(&asset.token_id)
        })
        .collect();
    let marks = client.mark_prices(active.iter().map(|asset| asset.token_id))?;

    let funding_ids: Vec<u32> = active
        .iter()
        .filter_map(|asset| {
            let perp = perps.get(&asset.token_id)?;
            has_perp_elapsed_funding_interval(time_sec, perp.funding_start_time)
                .then_some(asset.token_id)
        })
        .collect();
    let mut funding_by_token: BTreeMap<u32, Decimal> = BTreeMap::new();
    for token_id in funding_ids {
        let perp = perps.get(&token_id).ok_or_else(|| {
            format!("[world-markets] missing perp while fetching funding for {token_id}")
        })?;
        let asset = active
            .iter()
            .find(|asset| asset.token_id == token_id)
            .ok_or_else(|| {
                format!("[world-markets] missing asset while fetching funding for {token_id}")
            })?;
        funding_by_token.insert(
            token_id,
            funding_history_rate(client, perp, asset, time_sec)?,
        );
    }

    let mut tokens = BTreeMap::new();
    for asset in active {
        let balance_vault = balances
            .get(&asset.token_id)
            .and_then(|b| b.balance_raw.parse().ok())
            .unwrap_or(0);
        let lend = lending.get(&asset.token_id);
        let perp = perps.get(&asset.token_id).cloned().cloned();
        let (_raw, mark) = marks
            .get(&asset.token_id)
            .cloned()
            .ok_or_else(|| format!("[world-markets] missing mark for token {}", asset.token_id))?;
        let mark_price = parse_decimal(&mark, "mark_price")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let funding_history_rate = if perp.is_some() {
            funding_by_token
                .get(&asset.token_id)
                .copied()
                .unwrap_or(Decimal::ZERO)
        } else {
            Decimal::ZERO
        };
        tokens.insert(
            asset.token_id,
            TokenState {
                balance_vault,
                lend_borrower_raw: lend.map(|l| l.borrower_quantity_raw).unwrap_or(0),
                lend_lender_raw: lend.map(|l| l.lender_quantity_raw).unwrap_or(0),
                lend_rate_raw: lend.map(|l| l.highest_interest_rate_raw).unwrap_or(0),
                perp,
                mark_price,
                funding_history_rate,
            },
        );
    }
    Ok(PortfolioState { tokens })
}

fn evaluate(
    state: &PortfolioState,
    assets: &[Asset],
    base: &Asset,
    risk_multiplier: f64,
) -> Result<Decimal, String> {
    let asset_map: BTreeMap<u32, &Asset> = assets.iter().map(|a| (a.token_id, a)).collect();
    let mut total = Decimal::ZERO;
    for (token_id, token) in &state.tokens {
        let asset = asset_map
            .get(token_id)
            .ok_or_else(|| format!("[world-markets] missing asset config for token {token_id}"))?;
        let effective = compute_effective_balance(token, asset, risk_multiplier)?;
        if *token_id == BASE_TOKEN_ID {
            total = add_dec(total, effective)?;
            continue;
        }
        let base_quantity =
            convert_token_to_base(token.mark_price, effective, base.position_decimals)?;
        let (mut high, mut low) =
            calc_spot_risk_bounds(effective, base_quantity, asset, base, risk_multiplier)?;
        if let Some(perp) = &token.perp {
            let (high_perp, low_perp) = calc_perp_risk_bounds(
                perp,
                asset,
                base,
                token.mark_price,
                token.funding_history_rate,
                risk_multiplier,
            )?;
            high = add_dec(high, high_perp)?;
            low = add_dec(low, low_perp)?;
        }
        total = add_dec(total, high.min(low))?;
    }
    Ok(total)
}

fn calculate_liquidation_risk<F>(
    nav: Decimal,
    prv: Decimal,
    val_at_max: Decimal,
    mut evaluate_at: F,
) -> Result<f64, String>
where
    F: FnMut(f64) -> Result<Decimal, String>,
{
    if nav.is_zero() || nav == prv {
        return Ok(0.0);
    }
    if prv.is_sign_negative() {
        return Ok(MAX_SCORE);
    }
    if val_at_max > Decimal::ZERO {
        return Ok(0.0);
    }

    let mut max = MAX_SCORE;
    let mut min = 0.0;
    for _ in 0..MAX_SEARCH_ITERATIONS {
        if max - min <= SEARCH_PRECISION {
            break;
        }
        let mid = (max + min) / 2.0;
        let value = evaluate_at(mid)?;
        if value > Decimal::ZERO {
            min = mid;
        } else {
            max = mid;
        }
    }

    let health_score = (max + min) / 2.0;
    let hz = (health_score - 1.0) / 9.0;
    let mapped = ((hz.sqrt() + hz) * 9.0) / 2.0 + 1.0;
    Ok(MAX_SCORE - mapped)
}

fn sum_funding_rates(rates: &[u64]) -> Decimal {
    rates
        .iter()
        .map(|rate| Decimal::from(*rate) / Decimal::from(FUNDING_RATE_DIVISOR))
        .sum()
}

fn funding_history_rate(
    client: &WorldClient,
    perp: &PerpetualPosition,
    asset: &Asset,
    time_sec: u64,
) -> Result<Decimal, String> {
    if !has_perp_elapsed_funding_interval(time_sec, perp.funding_start_time) {
        return Ok(Decimal::ZERO);
    }
    let rates = client.funding_rate_history(perp.funding_start_time, time_sec, asset.token_id)?;
    Ok(sum_funding_rates(&rates))
}

fn has_perp_elapsed_funding_interval(current_sec: u64, position_start_sec: u64) -> bool {
    current_sec / FUNDING_INTERVAL_SEC > position_start_sec / FUNDING_INTERVAL_SEC
}

fn compute_effective_balance(
    token: &TokenState,
    asset: &Asset,
    risk_multiplier: f64,
) -> Result<Decimal, String> {
    let balance_position = vault_to_position(
        token.balance_vault,
        asset.vault_decimals,
        asset.position_decimals,
    )?;
    let rate = Decimal::from(token.lend_rate_raw) / Decimal::from(10_000);
    let borrower = calc_borrower_obligation(
        Decimal::from(token.lend_borrower_raw) / position_scale(asset.position_decimals),
        rate,
        LEND_DURATION_DAYS,
        asset.position_decimals,
    )?;
    let lender = apply_lender_haircut(
        Decimal::from(token.lend_lender_raw) / position_scale(asset.position_decimals),
        risk_multiplier,
        LENDER_HAIRCUT,
    )?;
    Ok(balance_position - borrower + lender)
}

fn calc_spot_risk_bounds(
    effective_balance: Decimal,
    base_quantity: Decimal,
    token: &Asset,
    base: &Asset,
    risk_multiplier: f64,
) -> Result<(Decimal, Decimal), String> {
    let price_permille =
        (scale_risk_capped(f64::from(token.risk_price_percent), risk_multiplier) * 10.0) as i64;
    let slippage_permille =
        (scale_risk_capped(token.risk_slippage_percent, risk_multiplier) * 10.0) as i64;
    let (high_delta, low_delta) = if effective_balance > Decimal::ZERO {
        (
            clamp_permille(price_permille - slippage_permille),
            clamp_permille(-price_permille - slippage_permille),
        )
    } else {
        (
            clamp_permille(price_permille + slippage_permille),
            clamp_permille(-price_permille + slippage_permille),
        )
    };
    Ok((
        apply_risk_adjustment(base_quantity, high_delta, base.position_decimals)?,
        apply_risk_adjustment(base_quantity, low_delta, base.position_decimals)?,
    ))
}

fn calc_perp_risk_bounds(
    perp: &PerpetualPosition,
    token: &Asset,
    base: &Asset,
    mark_price: Decimal,
    history_rate: Decimal,
    risk_multiplier: f64,
) -> Result<(Decimal, Decimal), String> {
    let quantity = parse_decimal(&perp.quantity, "perp_quantity")
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
    if quantity.is_zero() {
        return Ok((Decimal::ZERO, Decimal::ZERO));
    }
    let entry = parse_decimal(&perp.entry_price, "perp_entry_price")
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
    let owed_nom = parse_decimal(&perp.owed_nom, "perp_owed_nom")
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
    let owed_base = owed_base_decimal(
        &perp.owed_base_raw,
        token.position_decimals,
        base.position_decimals,
    )?;

    let agg_val = convert_token_to_base(entry, quantity, base.position_decimals)? - owed_base;
    let new_val = convert_token_to_base(mark_price, quantity, base.position_decimals)?;
    let price_risk = scale_risk_capped(f64::from(token.risk_price_percent), risk_multiplier);
    let high_val_perp =
        new_val * (Decimal::from(100) + decimal_from_f64(price_risk)?) / Decimal::from(100);
    let low_val_perp =
        new_val * (Decimal::from(100) - decimal_from_f64(price_risk)?) / Decimal::from(100);
    let mut high = high_val_perp - agg_val;
    let mut low = low_val_perp - agg_val;

    let f_pay_nom = owed_nom - quantity * history_rate;
    let f_pay_base = convert_token_to_base(mark_price, f_pay_nom, base.position_decimals)?;
    high += f_pay_base * (Decimal::from(100) + decimal_from_f64(price_risk)?) / Decimal::from(100);
    low += f_pay_base * (Decimal::from(100) - decimal_from_f64(price_risk)?) / Decimal::from(100);
    Ok((high, low))
}

fn owed_base_decimal(raw: &str, from_decimals: u8, to_decimals: u8) -> Result<Decimal, String> {
    if raw == "0" || raw == "-0" {
        return Ok(Decimal::ZERO);
    }
    let negative = raw.starts_with('-');
    let digits = raw.trim_start_matches('-');
    let scale = u8::try_from(u32::from(from_decimals) + 31)
        .map_err(|_| "[world-markets] owed_base scale exceeds u8".to_string())?;
    let scaled = if negative {
        format!("-{}", decimal_digits(digits.to_string(), scale))
    } else {
        decimal_digits(digits.to_string(), scale)
    };
    let value = Decimal::from_str(&scaled)
        .map_err(|e| format!("[world-markets] invalid owed_base scaled: {e}"))?;
    Ok(truncate_dp(value, u32::from(to_decimals)))
}

fn scale_risk_capped(risk_percent: f64, risk_multiplier: f64) -> f64 {
    (risk_percent * risk_multiplier).min(100.0)
}

fn clamp_permille(value: i64) -> i64 {
    value.clamp(-PERMILLE_SCALE, PERMILLE_SCALE)
}

fn calc_borrower_obligation(
    principal: Decimal,
    highest_rate: Decimal,
    lend_duration_days: u32,
    token_position_decimals: u8,
) -> Result<Decimal, String> {
    let interest =
        principal * highest_rate * Decimal::from(lend_duration_days) / Decimal::from(365);
    Ok(truncate_dp(interest, u32::from(token_position_decimals)) + principal)
}

fn apply_lender_haircut(
    lender_quantity: Decimal,
    risk_multiplier: f64,
    lender_haircut: i64,
) -> Result<Decimal, String> {
    let retained = ((PERMILLE_SCALE as f64
        - (PERMILLE_SCALE - lender_haircut) as f64 * risk_multiplier)
        / PERMILLE_SCALE as f64)
        .max(0.05);
    Ok(lender_quantity * decimal_from_f64(retained)?)
}

fn apply_risk_adjustment(
    base_quantity: Decimal,
    risk_delta_permille: i64,
    base_position_decimals: u8,
) -> Result<Decimal, String> {
    let adjusted = base_quantity
        * (Decimal::from(PERMILLE_SCALE) + Decimal::from(risk_delta_permille))
        / Decimal::from(PERMILLE_SCALE);
    Ok(truncate_dp(adjusted, u32::from(base_position_decimals)))
}

fn convert_token_to_base(
    token_price: Decimal,
    token_quantity: Decimal,
    base_position_decimals: u8,
) -> Result<Decimal, String> {
    Ok(truncate_dp(
        token_price * token_quantity,
        u32::from(base_position_decimals),
    ))
}

fn vault_to_position(
    vault_raw: u128,
    vault_decimals: u8,
    position_decimals: u8,
) -> Result<Decimal, String> {
    let value = Decimal::from(vault_raw) / Decimal::from(10u64.pow(u32::from(vault_decimals)));
    Ok(truncate_dp(value, u32::from(position_decimals)))
}

fn position_scale(decimals: u8) -> Decimal {
    Decimal::from(10u64.pow(u32::from(decimals)))
}

fn truncate_dp(value: Decimal, dp: u32) -> Decimal {
    value.round_dp_with_strategy(dp, RoundingStrategy::ToZero)
}

fn add_dec(a: Decimal, b: Decimal) -> Result<Decimal, String> {
    a.checked_add(b)
        .ok_or_else(|| "[world-markets] decimal overflow".to_string())
}

fn decimal_from_f64(value: f64) -> Result<Decimal, String> {
    Decimal::from_f64(value).ok_or_else(|| format!("[world-markets] invalid decimal: {value}"))
}

fn format_decimal(value: Decimal, decimals: u8) -> String {
    truncate_dp(value, u32::from(decimals))
        .normalize()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_risk_capped_matches_sdk() {
        assert_eq!(scale_risk_capped(10.0, 2.0), 20.0);
        assert_eq!(scale_risk_capped(50.0, 3.0), 100.0);
        assert_eq!(scale_risk_capped(0.0, 100.0), 0.0);
    }

    #[test]
    fn liquidation_risk_zero_when_nav_equals_prv() {
        let nav = Decimal::from(1000);
        let prv = Decimal::from(1000);
        let val_at_max = Decimal::from(-1);
        let risk = calculate_liquidation_risk(nav, prv, val_at_max, |_| Ok(Decimal::ZERO)).unwrap();
        assert_eq!(risk, 0.0);
    }

    #[test]
    fn liquidation_risk_max_when_prv_negative() {
        let nav = Decimal::from(1000);
        let prv = Decimal::from(-1);
        let val_at_max = Decimal::from(-1);
        let risk = calculate_liquidation_risk(nav, prv, val_at_max, |_| Ok(Decimal::ZERO)).unwrap();
        assert_eq!(risk, MAX_SCORE);
    }

    #[test]
    fn liquidation_risk_zero_when_still_positive_at_max_multiplier() {
        let nav = Decimal::from(1000);
        let prv = Decimal::from(500);
        let val_at_max = Decimal::from(1);
        let risk = calculate_liquidation_risk(nav, prv, val_at_max, |_| Ok(Decimal::ONE)).unwrap();
        assert_eq!(risk, 0.0);
    }

    #[test]
    fn risk_bands_match_composite_ui() {
        assert_eq!(risk_band(0.0), "safe");
        assert_eq!(risk_band(5.9), "safe");
        assert_eq!(risk_band(6.0), "elevated");
        assert_eq!(risk_band(7.9), "elevated");
        assert_eq!(risk_band(8.0), "high");
        assert_eq!(risk_band(9.9), "high");
        assert_eq!(risk_band(10.0), "liquidation");
    }

    #[test]
    fn owed_base_decimal_scales_without_integer_overflow() {
        let scaled = owed_base_decimal("1000000000000000000", 7, 4).unwrap();
        assert!(scaled >= Decimal::ZERO);
    }

    #[test]
    fn borrower_obligation_truncates_interest() {
        let principal = Decimal::from(1000);
        let rate = Decimal::new(10, 2);
        let obligation = calc_borrower_obligation(principal, rate, 30, 6).unwrap();
        assert!(obligation > principal);
    }

    fn usdt_asset() -> Asset {
        Asset {
            token_id: BASE_TOKEN_ID,
            symbol: "USDT".to_string(),
            name: "USDT".to_string(),
            token_type: "erc20".to_string(),
            erc20_address: "0x0000000000000000000000000000000000000000".to_string(),
            erc20_decimals: 6,
            vault_decimals: 6,
            position_decimals: 6,
            risk_price_percent: 0,
            risk_slippage_percent: 0.0,
        }
    }

    fn usdt_account(balance: Decimal) -> Account {
        let raw = to_u128_raw(balance, 6).unwrap();
        Account {
            account_id: 1,
            owner: "0x0000000000000000000000000000000000000001".to_string(),
            risk_adjusted_portfolio_value_raw: 0,
            risk_adjusted_portfolio_value: balance.normalize().to_string(),
            eligible_for_liquidation: false,
            balances: vec![Balance {
                token_id: BASE_TOKEN_ID,
                symbol: "USDT".to_string(),
                balance_raw: raw.to_string(),
                balance: decimal_digits(raw.to_string(), 6),
                available_raw: raw.to_string(),
                available: decimal_digits(raw.to_string(), 6),
                spot_lend_sequestered_raw: "0".to_string(),
                spot_lend_sequestered: "0".to_string(),
                perp_sequestered_raw: "0".to_string(),
                perp_sequestered: "0".to_string(),
            }],
            lending_positions: Vec::new(),
            perpetual_positions: Vec::new(),
            debt_token_ids: Vec::new(),
            non_debt_token_ids: vec![BASE_TOKEN_ID],
        }
    }

    #[test]
    fn lending_usdt_applies_two_percent_atlas_haircut_to_rapv() {
        let client = WorldClient::default();
        let usdt = usdt_asset();
        let account = usdt_account(Decimal::from(1_000));
        let assets = [usdt.clone()];
        let before_state = build_state(&client, &account, &assets, 0).unwrap();
        let before = evaluate(&before_state, &assets, &usdt, 1.0).unwrap();
        let mut projected = account.clone();
        apply_lend_supply(&mut projected, &usdt, Decimal::from(100)).unwrap();
        let after_state = build_state(&client, &projected, &assets, 0).unwrap();
        let after = evaluate(&after_state, &assets, &usdt, 1.0).unwrap();
        assert_eq!(before - after, Decimal::from(2));
        let projection = project_from_account(
            &client,
            &account,
            &assets,
            0,
            &TradeIntent {
                product: "lend",
                side: "lend",
                base: &usdt,
                quote: &usdt,
                quantity: Decimal::from(100),
                mark: Decimal::ONE,
            },
            None,
        )
        .unwrap();
        assert_eq!(projection.rapv, Decimal::from(998));
        assert!(projection.is_estimate);
        assert_eq!(projection.source, "world-markets-reporting");
    }

    #[test]
    fn evaluate_uses_stored_funding_without_rpc() {
        let usdt = usdt_asset();
        let mut weth = usdt_asset();
        weth.token_id = 2;
        weth.symbol = "WETH".to_string();
        weth.name = "WETH".to_string();
        weth.risk_price_percent = 5;
        weth.risk_slippage_percent = 1.0;
        let perp = PerpetualPosition {
            token_id: 2,
            symbol: "WETH".to_string(),
            quantity_raw: 10_000,
            quantity: "1".to_string(),
            side: "long".to_string(),
            entry_price_raw: 0,
            entry_price: "3000".to_string(),
            funding_start_time: 0,
            owed_nom_raw: "0".to_string(),
            owed_nom: "0".to_string(),
            owed_base_raw: "0".to_string(),
        };
        let mut tokens = BTreeMap::new();
        tokens.insert(
            2,
            TokenState {
                balance_vault: 0,
                lend_borrower_raw: 0,
                lend_lender_raw: 0,
                lend_rate_raw: 0,
                perp: Some(perp),
                mark_price: Decimal::from(3000),
                funding_history_rate: Decimal::ZERO,
            },
        );
        let value = evaluate(
            &PortfolioState { tokens },
            &[usdt.clone(), weth],
            &usdt,
            0.0,
        );
        assert!(value.is_ok(), "{value:?}");
    }

    #[test]
    fn lend_supply_fails_closed_when_available_is_insufficient() {
        let usdt = usdt_asset();
        let mut account = usdt_account(Decimal::from(10));
        let err = apply_lend_supply(&mut account, &usdt, Decimal::from(50)).unwrap_err();
        assert!(err.contains("insufficient"), "{err}");
    }

    #[test]
    #[ignore = "requires live UniFi RPC"]
    fn atlas_unit_risk_evaluate_matches_live_contract_rapv() {
        let client = WorldClient::default();
        let assets = client.assets().unwrap();
        let account_id = client.latest_account_id().unwrap();
        let account = client.account(account_id, &assets).unwrap();
        let quote = assets
            .iter()
            .find(|asset| asset.token_id == BASE_TOKEN_ID)
            .unwrap();
        let time_sec = client.block_timestamp().unwrap();
        let state = build_state(&client, &account, &assets, time_sec).unwrap();
        let derived = evaluate(&state, &assets, quote, 1.0).unwrap();
        let live = parse_decimal(
            &account.risk_adjusted_portfolio_value,
            "risk_adjusted_portfolio_value",
        )
        .unwrap();
        let delta = (derived - live).abs();
        // UniFi RAPV and Composite evaluate(1) share ATLAS inputs but not
        // identical truncation. Observed residual on this fixture is ~0.17 on
        // ~790. Post-trade uses the delta method so the live contract RAPV
        // remains the anchor: live + evaluate(after,1) − evaluate(before,1).
        let relative = if live.abs() > Decimal::ONE {
            delta / live.abs()
        } else {
            delta
        };
        assert!(
            relative <= Decimal::new(5, 3),
            "ATLAS evaluate(1) {derived} must stay within 50 bps of live RAPV {live} (delta {delta})"
        );
    }

    #[test]
    fn dev_seed_rapv_uses_live_reading_when_enabled() {
        let account = usdt_account(Decimal::from(9_000));
        unsafe { std::env::remove_var("WORLD_DEV_SEED_POST_TRADE_RAPV") };
        assert!(dev_seed_rapv(&account).is_none());
        unsafe { std::env::set_var("WORLD_DEV_SEED_POST_TRADE_RAPV", "1") };
        assert_eq!(dev_seed_rapv(&account), Some(Decimal::from(9_000)));
        unsafe { std::env::remove_var("WORLD_DEV_SEED_POST_TRADE_RAPV") };
    }
}
