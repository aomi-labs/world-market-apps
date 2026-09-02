//! Public Mini App snapshot: live contract + reporting figures only.
//!
//! 24h change and isolated leverage are omitted (`null`) until a reporting
//! function produces them. Dollarpower is computed from this account's NAV and
//! gross notionals — the fixture `get_dollarpower` book is not this user's book.

use std::str::FromStr;
use std::sync::OnceLock;

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;
use rust_decimal::prelude::ToPrimitive;
use serde::Serialize;
use serde_json::{Value, json};

use crate::brain::BrainClient;
use crate::client::{Account, Asset, BASE_TOKEN_ID, Market, WorldClient};
use crate::liquidation_risk::{self, PortfolioMetrics};
use crate::lookups::notional_usdt;
use crate::mandate::{Mandate, parse_decimal};
use crate::pnl::PnlLedger;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PortfolioResponse {
    pub positions: Vec<PositionRow>,
    pub dollarpower: DollarpowerSnapshot,
    pub risk: RiskSnapshot,
    pub total_usd_value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_change_24h_pct: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<String>,
    pub flags: MiniFlags,
    pub block_number: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PositionRow {
    pub symbol: String,
    pub quantity: String,
    pub usd_value: String,
    pub change_24h_pct: Option<String>,
    pub leverage: Option<String>,
    pub change_direction: Option<String>,
    pub asset_type: String,
    pub group: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    pub can_exit: bool,
    pub watch_count: u32,
    pub keywords: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProductsResponse {
    pub products: Vec<ProductRow>,
    pub block_number: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProductRow {
    pub id: String,
    pub symbol: String,
    pub name: String,
    pub product: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_symbol: Option<String>,
    pub mark_price: String,
    pub keywords: String,
    pub base_token_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_token_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DollarpowerSnapshot {
    pub ratio: String,
    pub equivalent_usd: String,
    pub committed_usd: String,
    pub fill_pct: String,
    pub is_estimate: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RiskSnapshot {
    #[serde(rename = "liquidation_score")]
    pub score: u8,
    pub band: String,
    pub distance_from_floor_pct: Option<String>,
    pub is_estimate: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MiniFlags {
    pub primary_view: String,
    pub jobline_negative: bool,
    pub family: String,
    pub voice_home: bool,
    pub voice_mode: String,
    pub live_words: bool,
}

struct PositionDraft {
    symbol: String,
    quantity: Decimal,
    usd: Decimal,
    asset_type: &'static str,
    side: Option<String>,
}

fn shared_client() -> &'static WorldClient {
    static CLIENT: OnceLock<WorldClient> = OnceLock::new();
    CLIENT.get_or_init(WorldClient::default)
}

/// Parse `WORLD_ACCOUNT_ID` (`17` or `world-17`).
pub fn account_id_from_env() -> Option<u64> {
    parse_account_id(&std::env::var("WORLD_ACCOUNT_ID").ok()?)
}

pub fn parse_account_id(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(id) = trimmed.parse::<u64>() {
        return Some(id);
    }
    let rest = trimmed
        .strip_prefix("world-")
        .or_else(|| trimmed.strip_prefix("WORLD-"))?;
    rest.parse().ok()
}

pub fn load_portfolio(account_id: u64) -> Result<PortfolioResponse, String> {
    let client = shared_client();
    let assets = client.assets()?;
    let account = client.account(account_id, &assets)?;
    let block_number = client.block_number()?;
    let metrics = liquidation_risk::compute_metrics(client, &account, &assets, block_number)?;
    let floor = mandate_floor();
    assemble(client, &account, &assets, &metrics, floor, block_number)
}

/// Full tradeable catalog: every live spot, perp, and lend book.
pub fn load_products() -> Result<ProductsResponse, String> {
    let client = shared_client();
    let markets = client.list_markets()?;
    let block_number = client.block_number()?;
    let mut products: Vec<ProductRow> = markets.into_iter().map(product_from_market).collect();
    products.sort_by(|a, b| {
        a.symbol
            .cmp(&b.symbol)
            .then_with(|| product_rank(&a.product).cmp(&product_rank(&b.product)))
    });
    Ok(ProductsResponse {
        products,
        block_number,
    })
}

fn product_from_market(market: Market) -> ProductRow {
    let quote = market
        .quote_token
        .as_ref()
        .map(|asset| asset.symbol.clone());
    let product = canonical_product(&market.product);
    ProductRow {
        id: format!("{product}:{}", market.base_token.symbol),
        symbol: market.base_token.symbol.clone(),
        name: market.base_token.name.clone(),
        product: product.clone(),
        quote_symbol: quote.clone(),
        mark_price: market.mark_price,
        keywords: product_keywords(
            &market.base_token.symbol,
            &market.base_token.name,
            &product,
            quote.as_deref(),
        ),
        base_token_id: market.base_token.token_id,
        quote_token_id: market.quote_token.as_ref().map(|asset| asset.token_id),
    }
}

fn canonical_product(product: &str) -> String {
    match product {
        "perpetual" => "perp".to_string(),
        "lending" => "lend".to_string(),
        other => other.to_string(),
    }
}

fn product_rank(product: &str) -> u8 {
    match product {
        "spot" => 0,
        "perp" => 1,
        "lend" => 2,
        _ => 3,
    }
}

fn product_keywords(symbol: &str, name: &str, product: &str, quote: Option<&str>) -> String {
    let kind = match product {
        "spot" => "spot",
        "perp" | "perpetual" => "perp perpetual",
        "lend" | "lending" => "lend lending",
        other => other,
    };
    format!("{} {} {} {}", symbol, name, kind, quote.unwrap_or(""))
}

#[derive(Debug, Clone, Serialize)]
pub struct ChartSnapshot {
    pub symbol: String,
    pub feed_symbol: String,
    pub period: String,
    pub period_label: String,
    pub bar_label: String,
    pub source: String,
    pub candles: Vec<ChartBar>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChartBar {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
}

#[derive(Debug)]
pub enum ChartError {
    BadRequest(String),
    NotFound(String),
    Upstream(String),
}

impl std::fmt::Display for ChartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest(m) | Self::NotFound(m) | Self::Upstream(m) => write!(f, "{m}"),
        }
    }
}

/// Live OHLC for the Mini App chart (same Yahoo feed as the Telegram PNG).
pub fn load_chart(ticker: &str, period: &str) -> Result<ChartSnapshot, ChartError> {
    use crate::marketdata::{
        ChartRange, FeedError, feed_from_env, load_or_refresh, normalize_ticker, resolve_ticker,
    };

    let range = ChartRange::parse(period).ok_or_else(|| {
        ChartError::BadRequest("[world-markets] period must be d, w, or m".into())
    })?;
    let requested = normalize_ticker(ticker)
        .ok_or_else(|| ChartError::BadRequest("[world-markets] ticker is empty".into()))?;
    let feed = feed_from_env().map_err(ChartError::Upstream)?;
    let universe = load_or_refresh(feed.as_ref()).map_err(ChartError::Upstream)?;
    let resolved = resolve_ticker(&requested, &universe);
    let series = match feed.candles(&resolved.feed_symbol, range) {
        Ok(series) => series,
        Err(FeedError::NotFound { symbol }) => {
            return Err(ChartError::NotFound(format!(
                "[world-markets] no chart for {symbol}"
            )));
        }
        Err(err) => return Err(ChartError::Upstream(err.to_string())),
    };
    Ok(ChartSnapshot {
        symbol: requested,
        feed_symbol: series.feed_symbol,
        period: range.as_token().to_string(),
        period_label: range.label().to_string(),
        bar_label: range.bar_label().to_string(),
        source: series.source,
        candles: series
            .candles
            .into_iter()
            .map(|c| ChartBar {
                t: c.ts,
                o: c.open,
                h: c.high,
                l: c.low,
                c: c.close,
            })
            .collect(),
    })
}

fn mandate_floor() -> Option<Decimal> {
    let path = std::env::var("WORLD_MANDATE_PATH").unwrap_or_default();
    let path_lc = path.trim().to_ascii_lowercase();
    let has_json = std::env::var("WORLD_MANDATE_JSON")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let real_file = !path_lc.is_empty()
        && !matches!(
            path_lc.as_str(),
            "placeholder" | "dev" | "-" | "none" | "off" | "missing"
        );
    if !has_json && !real_file {
        return None;
    }
    let mandate = Mandate::bound(None).ok()?;
    parse_decimal(
        &mandate.min_risk_adjusted_portfolio_value.amount,
        "min_risk_adjusted_portfolio_value",
    )
    .ok()
}

fn assemble(
    client: &WorldClient,
    account: &Account,
    assets: &[Asset],
    metrics: &PortfolioMetrics,
    floor: Option<Decimal>,
    block_number: u64,
) -> Result<PortfolioResponse, String> {
    let base = assets
        .iter()
        .find(|asset| asset.token_id == BASE_TOKEN_ID)
        .ok_or_else(|| "[world-markets] base token config is missing".to_string())?;
    let by_id: std::collections::BTreeMap<u32, &Asset> =
        assets.iter().map(|asset| (asset.token_id, asset)).collect();

    let mut drafts = Vec::new();
    let mut had_unpriced = false;
    let mut had_any = false;

    for perp in &account.perpetual_positions {
        let qty = parse_qty(&perp.quantity, "perp_quantity")?;
        if qty.is_zero() {
            continue;
        }
        had_any = true;
        let Some(asset) = by_id.get(&perp.token_id) else {
            had_unpriced = true;
            continue;
        };
        match notional_usdt(client, asset, base, qty.abs()) {
            Ok(usd) => drafts.push(PositionDraft {
                symbol: perp.symbol.clone(),
                quantity: qty.abs(),
                usd,
                asset_type: "perp",
                side: Some(perp.side.clone()),
            }),
            Err(_) => had_unpriced = true,
        }
    }

    for lend in &account.lending_positions {
        let Some(asset) = by_id.get(&lend.token_id) else {
            continue;
        };
        let lender = parse_qty(&lend.lender_quantity, "lender_quantity")?;
        let borrower = parse_qty(&lend.borrower_quantity, "borrower_quantity")?;
        if !lender.is_zero() {
            had_any = true;
            match notional_usdt(client, asset, base, lender.abs()) {
                Ok(usd) => drafts.push(PositionDraft {
                    symbol: lend.symbol.clone(),
                    quantity: lender.abs(),
                    usd,
                    asset_type: "lend",
                    side: None,
                }),
                Err(_) => had_unpriced = true,
            }
        }
        if !borrower.is_zero() {
            had_any = true;
            match notional_usdt(client, asset, base, borrower.abs()) {
                Ok(usd) => drafts.push(PositionDraft {
                    symbol: lend.symbol.clone(),
                    quantity: borrower.abs(),
                    usd,
                    asset_type: "borrow",
                    side: None,
                }),
                Err(_) => had_unpriced = true,
            }
        }
    }

    for balance in &account.balances {
        let qty = parse_qty(&balance.balance, "balance")?;
        if qty.is_zero() {
            continue;
        }
        had_any = true;
        let Some(asset) = by_id.get(&balance.token_id) else {
            had_unpriced = true;
            continue;
        };
        match notional_usdt(client, asset, base, qty.abs()) {
            Ok(usd) => drafts.push(PositionDraft {
                symbol: balance.symbol.clone(),
                quantity: qty.abs(),
                usd,
                asset_type: "spot",
                side: None,
            }),
            Err(_) => had_unpriced = true,
        }
    }

    if had_any && drafts.is_empty() {
        return Err("[world-markets] position marks unavailable".to_string());
    }

    drafts.sort_by(|a, b| b.usd.cmp(&a.usd).then_with(|| a.symbol.cmp(&b.symbol)));

    let mut total = Decimal::ZERO;
    let mut positions = Vec::with_capacity(drafts.len());
    for row in &drafts {
        total += row.usd;
        positions.push(PositionRow {
            symbol: row.symbol.clone(),
            quantity: format_qty(row.quantity),
            usd_value: two_dp(row.usd),
            change_24h_pct: None,
            leverage: None,
            change_direction: None,
            asset_type: row.asset_type.to_string(),
            group: group_for(row.asset_type).to_string(),
            extra: extra_for(&row.symbol, row.asset_type, floor),
            side: row.side.clone(),
            can_exit: row.asset_type != "lend",
            watch_count: 0,
            keywords: format!(
                "{} {} {}",
                row.symbol,
                row.asset_type,
                row.side.as_deref().unwrap_or("")
            ),
        });
    }

    let committed = parse_decimal(&metrics.net_asset_value, "net_asset_value")
        .map_err(|v| format!("[world-markets] {}: {}", v.rule, v.detail))?;
    let equivalent = if total > Decimal::ZERO {
        total
    } else {
        committed
    };
    let is_estimate = had_unpriced;
    let ratio = if committed > Decimal::ZERO {
        (equivalent / committed).round_dp_with_strategy(1, RoundingStrategy::MidpointAwayFromZero)
    } else {
        Decimal::ZERO
    };

    let rapv = parse_decimal(
        &account.risk_adjusted_portfolio_value,
        "risk_adjusted_portfolio_value",
    )
    .map_err(|v| format!("[world-markets] {}: {}", v.rule, v.detail))?;
    let score = risk_score(metrics, account.eligible_for_liquidation);
    let band = spec_band(score).to_string();
    let distance = if score < 9 {
        distance_pct(rapv, floor)
    } else {
        None
    };

    Ok(PortfolioResponse {
        positions,
        dollarpower: DollarpowerSnapshot {
            ratio: ratio.normalize().to_string(),
            equivalent_usd: two_dp(equivalent),
            committed_usd: two_dp(committed),
            fill_pct: fill_pct(committed, equivalent),
            is_estimate,
        },
        risk: RiskSnapshot {
            score,
            band,
            distance_from_floor_pct: distance,
            is_estimate: false,
        },
        total_usd_value: two_dp(total),
        total_change_24h_pct: None,
        floor: floor.map(two_dp),
        flags: mini_flags(),
        block_number,
    })
}

fn parse_qty(raw: &str, field: &'static str) -> Result<Decimal, String> {
    parse_decimal(raw, field).map_err(|v| format!("[world-markets] {}: {}", v.rule, v.detail))
}

pub fn spec_band(score: u8) -> &'static str {
    match score {
        0..=5 => "safe",
        6..=8 => "elevated",
        _ => "high",
    }
}

fn env_flag_on(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("on")
        }
        Err(_) => default,
    }
}

fn env_flag_off(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => {
            let t = v.trim();
            t == "0" || t.eq_ignore_ascii_case("false") || t.eq_ignore_ascii_case("off")
        }
        Err(_) => false,
    }
}

pub fn mini_flags() -> MiniFlags {
    MiniFlags {
        primary_view: std::env::var("WORLD_MINI_PRIMARY_VIEW")
            .ok()
            .filter(|v| v == "portfolio" || v == "ledger")
            .unwrap_or_else(|| "ledger".to_string()),
        jobline_negative: env_flag_on("WORLD_MINI_JOBLINE_NEGATIVE", false),
        family: std::env::var("WORLD_MINI_FAMILY")
            .ok()
            .filter(|v| v == "violet" || v == "blue")
            .unwrap_or_else(|| "blue".to_string()),
        voice_home: !env_flag_off("WORLD_MINI_VOICE_HOME"),
        voice_mode: std::env::var("WORLD_MINI_VOICE_MODE")
            .ok()
            .filter(|v| v == "tap" || v == "hold")
            .unwrap_or_else(|| "hold".to_string()),
        live_words: !env_flag_off("WORLD_MINI_LIVE_WORDS"),
    }
}

fn group_for(asset_type: &str) -> &'static str {
    match asset_type {
        "perp" => "positions",
        "lend" | "borrow" => "lending",
        _ => "holdings",
    }
}

fn extra_for(symbol: &str, asset_type: &str, floor: Option<Decimal>) -> Option<String> {
    let upper = symbol.to_ascii_uppercase();
    if upper.contains("STETH") {
        return Some("accrues in price".to_string());
    }
    if upper.contains("XETH") {
        return Some("borrow leg backs it".to_string());
    }
    match asset_type {
        "perp" => floor.map(|value| format!("floor ${}", two_dp(value))),
        "lend" => Some("fixed term".to_string()),
        "spot" => Some("free collateral".to_string()),
        _ => None,
    }
}

pub fn apply_watch_counts(
    portfolio: &mut PortfolioResponse,
    counts: &serde_json::Map<String, Value>,
) {
    for row in &mut portfolio.positions {
        let key = row.symbol.to_ascii_uppercase();
        let n = counts
            .get(&key)
            .and_then(Value::as_u64)
            .or_else(|| {
                counts.iter().find_map(|(k, v)| {
                    if key.contains(&k.to_ascii_uppercase())
                        || k.to_ascii_uppercase().contains(&key)
                    {
                        v.as_u64()
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(0) as u32;
        row.watch_count = n;
    }
}

pub fn load_ledger_summary(account_id: u64) -> Result<Value, String> {
    BrainClient::from_env().ledger_summary(account_id)
}

pub fn load_ledger(account_id: u64) -> Result<Value, String> {
    BrainClient::from_env().ledger(account_id)
}

pub fn load_instruction(account_id: u64, id: &str) -> Result<Value, String> {
    BrainClient::from_env().ledger_one(account_id, id)
}

pub fn load_pnl(account_id: u64) -> Result<Value, String> {
    let client = shared_client();
    let assets = client.assets()?;
    let account = client.account(account_id, &assets)?;
    let ledger = PnlLedger::default();
    let report = crate::pnl::report(client, &ledger, &account, None)?;
    serde_json::to_value(report).map_err(|err| err.to_string())
}

/// Live account + catalog + brain ledger + PnL for The Desk (same rails as the plugin).
pub fn load_desk_context(account_id: u64) -> Result<Value, String> {
    let portfolio = load_portfolio(account_id)?;
    let products = load_products()?;
    let ledger = load_ledger(account_id).ok();
    let ledger_summary = load_ledger_summary(account_id).ok();
    let pnl = load_pnl(account_id).ok();
    Ok(json!({
        "ok": true,
        "account_id": account_id,
        "portfolio": portfolio,
        "products": products,
        "ledger": ledger,
        "ledger_summary": ledger_summary,
        "pnl": pnl,
    }))
}

pub fn submit_compose(body: &Value) -> Result<Value, String> {
    BrainClient::from_env().compose(body)
}

/// Unfulfillable / near-match / unclear. None means fall through to the agent.
pub fn submit_heard(account_id: u64, text: &str, extra: Option<&Value>) -> Option<Value> {
    crate::cant::try_heard(account_id, text, extra)
}

pub fn flush_staged_trade(account_id: u64, instruction_id: &str) -> Result<Value, String> {
    crate::staged::flush_staged_trade(account_id, instruction_id)
}

pub fn flush_due_trades(account_id: u64) -> Result<Value, String> {
    crate::staged::flush_due_trades(account_id)
}

/// Best-effort Bot API send. Failures are logged by the caller; they must not
/// undo a completed ledger cancel.
pub fn post_chat_lines(bot_token: &str, chat_id: u64, lines: &[String]) -> Result<(), String> {
    if bot_token.is_empty() || chat_id == 0 || lines.is_empty() {
        return Ok(());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|err| err.to_string())?;
    let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");
    for text in lines {
        if text.trim().is_empty() {
            continue;
        }
        let response = client
            .post(&url)
            .json(&json!({ "chat_id": chat_id, "text": text, "disable_notification": true }))
            .send()
            .map_err(|err| err.to_string())?;
        if !response.status().is_success() {
            return Err(format!("telegram sendMessage {}", response.status()));
        }
    }
    Ok(())
}

/// Prepare M10 for Telegram.WebApp.shareMessage. Mutates nothing the Mini App
/// displays. Falls back to a bot-chat deep link when savePreparedInlineMessage
/// is unavailable.
pub fn prepare_introduction(
    account_id: u64,
    telegram_user_id: Option<u64>,
    first_name: Option<&str>,
    bot_token: &str,
) -> Result<Value, String> {
    let telegram_bot = std::env::var("WORLD_TELEGRAM_BOT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "WorldMarketsBot".to_string());
    let intro = BrainClient::from_env().share(&json!({
        "action": "introduce",
        "user_id": account_id,
        "account_id": account_id,
        "first_name": first_name,
        "telegram_bot": telegram_bot,
    }))?;
    let message = intro
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let link = intro
        .get("link")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let fallback_url = format!("https://t.me/{telegram_bot}?start=share");
    let prepared = telegram_user_id
        .and_then(|user_id| save_prepared_inline_message(bot_token, user_id, &message, &link).ok());
    Ok(json!({
        "ok": true,
        "prepared_inline_message_id": prepared,
        "fallback_url": fallback_url,
        "intent": crate::share::INTENT,
    }))
}

fn save_prepared_inline_message(
    bot_token: &str,
    user_id: u64,
    message: &str,
    link: &str,
) -> Result<String, String> {
    if bot_token.is_empty() || user_id == 0 || message.is_empty() {
        return Err("unprepared".into());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|err| err.to_string())?;
    let url = format!("https://api.telegram.org/bot{bot_token}/savePreparedInlineMessage");
    let mut keyboard = Vec::new();
    if !link.is_empty() {
        keyboard.push(json!([
            { "text": crate::share::PAPER_BTN, "url": link },
            { "text": crate::share::CANT_BTN, "url": link }
        ]));
    }
    let response = client
        .post(&url)
        .json(&json!({
            "user_id": user_id,
            "result": {
                "type": "article",
                "id": "m10",
                "title": "aomi",
                "input_message_content": {
                    "message_text": message,
                    "link_preview_options": { "is_disabled": true }
                },
                "reply_markup": { "inline_keyboard": keyboard }
            },
            "allow_user_chats": true,
            "allow_bot_chats": false,
            "allow_group_chats": true,
            "allow_channel_chats": false
        }))
        .send()
        .map_err(|err| err.to_string())?;
    let value: Value = response.json().map_err(|err| err.to_string())?;
    if value.get("ok") != Some(&Value::Bool(true)) {
        return Err(value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("savePreparedInlineMessage failed")
            .to_string());
    }
    value
        .pointer("/result/id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "savePreparedInlineMessage missing id".to_string())
}

/// Mini App hold-to-talk: STT + brain utterance + compose into the agent.
/// Does not call The Desk. Does not place an order.
pub fn ingest_voice_note(account_id: u64, body: &Value) -> Result<Value, String> {
    crate::voice::ingest_voice(account_id, body)
}

pub fn load_answer(account_id: u64, correlation_id: &str) -> Result<Value, String> {
    BrainClient::from_env().get_answer(account_id, correlation_id)
}

pub fn upsert_answer(account_id: u64, body: &Value) -> Result<Value, String> {
    let mut payload = body.clone();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("account_id".to_string(), json!(account_id));
    }
    BrainClient::from_env().upsert_answer(&payload)
}

/// Open a working (or clarify) projection for a question. Never a ledger row.
pub fn open_question_answer(account_id: u64, body: &Value) -> Result<Value, String> {
    let mut payload = body.clone();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("account_id".to_string(), json!(account_id));
        if obj.get("status").and_then(Value::as_str).is_none() {
            obj.insert("status".to_string(), json!("working"));
        }
    }
    BrainClient::from_env().upsert_answer(&payload)
}

pub fn position_referent_label(symbol: &str, side: Option<&str>, asset_type: &str) -> String {
    match (side.filter(|s| !s.is_empty()), asset_type) {
        (Some(side), _) => format!("{symbol} {side}"),
        (None, "perp") => format!("{symbol} perp"),
        _ => symbol.to_string(),
    }
}

pub fn clarify_position_chips(account_id: u64) -> Vec<String> {
    load_portfolio(account_id)
        .map(|portfolio| {
            portfolio
                .positions
                .into_iter()
                .map(|row| {
                    position_referent_label(&row.symbol, row.side.as_deref(), &row.asset_type)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Live captions while holding Record. Transcribes only; does not ingest.
pub fn transcribe_live(account_id: u64, body: &Value) -> Result<Value, String> {
    crate::voice::transcribe_live(account_id, body)
}

pub fn deepgram_ready() -> bool {
    crate::stt::deepgram_configured()
}

/// Same keyterm seed as ingest, for live HTTP fallback and the streaming proxy.
pub fn voice_stream_keyterms(account_id: u64) -> Vec<String> {
    crate::voice::voice_keyterms_for(account_id)
}

/// Instant keyterms for the live WebSocket: never blocks on brain or chain.
pub fn voice_stream_keyterms_fast(account_id: u64) -> Vec<String> {
    crate::voice::voice_stream_keyterms_fast(account_id)
}

pub fn voice_keyterm_boosts(keyterms: &[String]) -> Vec<String> {
    crate::stt::keyterm_params(keyterms)
}

pub fn deepgram_stream_query(sample_rate: u32) -> Vec<(&'static str, String)> {
    crate::stt::deepgram_stream_query(sample_rate)
}

pub fn deepgram_stream_sample_rate_ok(sample_rate: u32) -> bool {
    crate::stt::stream_sample_rate_ok(sample_rate)
}

pub fn deepgram_replace_pairs() -> &'static [(&'static str, &'static str)] {
    crate::stt::deepgram_replace_pairs()
}

pub fn stream_transcript_text(value: &Value) -> Option<(String, bool)> {
    crate::stt::stream_transcript(value).map(|(text, is_final, _)| (text, is_final))
}

pub fn stream_transcript_caption(value: &Value) -> Option<(String, bool, f64)> {
    crate::stt::stream_transcript(value)
}

pub fn ontology_summary() -> Result<Value, String> {
    BrainClient::from_env().ontology_summary()
}

pub fn ontology_stats(
    account_id: Option<u64>,
    from: Option<&str>,
    to: Option<&str>,
    all: bool,
) -> Result<Value, String> {
    BrainClient::from_env().ontology_stats(account_id, from, to, all)
}

fn risk_score(metrics: &PortfolioMetrics, eligible: bool) -> u8 {
    if eligible {
        return 10;
    }
    let parsed = Decimal::from_str(&metrics.liquidation_risk).unwrap_or(Decimal::ZERO);
    let rounded = parsed.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
    rounded.to_u8().unwrap_or(0).min(10)
}

pub fn fill_pct(committed: Decimal, equivalent: Decimal) -> String {
    if equivalent <= Decimal::ZERO {
        return "0".to_string();
    }
    let pct = (committed / equivalent * Decimal::from(100))
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .max(Decimal::ZERO)
        .min(Decimal::from(100));
    pct.normalize().to_string()
}

fn distance_pct(rapv: Decimal, floor: Option<Decimal>) -> Option<String> {
    let floor = floor?;
    if rapv <= Decimal::ZERO {
        return None;
    }
    let pct = ((rapv - floor) / rapv * Decimal::from(100))
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .max(Decimal::ZERO);
    Some(pct.normalize().to_string())
}

pub fn format_qty(value: Decimal) -> String {
    let v = value.normalize();
    let abs = v.abs();
    if abs.is_zero() {
        return "0".to_string();
    }
    if abs < Decimal::new(1, 2) {
        return v.normalize().to_string();
    }
    if abs >= Decimal::from(100) {
        return v
            .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
            .normalize()
            .to_string();
    }
    v.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        .normalize()
        .to_string()
}

pub fn two_dp(value: Decimal) -> String {
    let rounded = value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    let s = rounded.to_string();
    if let Some(i) = s.find('.') {
        match s.len() - i - 1 {
            0 => format!("{s}00"),
            1 => format!("{s}0"),
            _ => s,
        }
    } else {
        format!("{s}.00")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_id_parses_bare_and_prefixed() {
        assert_eq!(parse_account_id("17"), Some(17));
        assert_eq!(parse_account_id("world-17"), Some(17));
        assert_eq!(parse_account_id("WORLD-99"), Some(99));
        assert_eq!(parse_account_id(""), None);
        assert_eq!(parse_account_id("wallet-17"), None);
    }

    #[test]
    fn bands_follow_spec_not_engine_eight() {
        assert_eq!(spec_band(0), "safe");
        assert_eq!(spec_band(5), "safe");
        assert_eq!(spec_band(6), "elevated");
        assert_eq!(spec_band(8), "elevated");
        assert_eq!(spec_band(9), "high");
        assert_eq!(spec_band(10), "high");
    }

    #[test]
    fn fill_is_committed_over_equivalent() {
        assert_eq!(fill_pct(Decimal::from(25), Decimal::from(100)), "25");
        assert_eq!(fill_pct(Decimal::from(10), Decimal::ZERO), "0");
        assert_eq!(fill_pct(Decimal::from(200), Decimal::from(100)), "100");
    }

    #[test]
    fn quantity_keeps_small_precision() {
        assert_eq!(format_qty(Decimal::new(5, 4)), "0.0005");
        assert_eq!(format_qty(Decimal::new(235, 2)), "2.35");
        assert_eq!(format_qty(Decimal::from(12800)), "12800");
    }

    #[test]
    fn money_always_two_dp() {
        assert_eq!(two_dp(Decimal::from(8432)), "8432.00");
        assert_eq!(two_dp(Decimal::new(84325, 1)), "8432.50");
    }

    #[test]
    fn mini_flags_voice_home_defaults_on() {
        let flags = mini_flags();
        assert!(flags.voice_home);
        assert_eq!(flags.voice_mode, "hold");
        assert!(flags.live_words);
        assert_eq!(flags.primary_view, "ledger");
        assert!(!flags.jobline_negative);
        assert_eq!(flags.family, "blue");
    }

    #[test]
    fn copy_module_strings_have_no_exclamation() {
        let src = include_str!("../mini-app/static/copy.js");
        let mut quoted = String::new();
        let mut in_str = false;
        let mut chars = src.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if in_str && let Some(next) = chars.next() {
                    quoted.push(next);
                }
                continue;
            }
            if ch == '"' {
                if in_str {
                    quoted.push('\n');
                }
                in_str = !in_str;
                continue;
            }
            if in_str {
                quoted.push(ch);
            }
        }
        for line in quoted.lines() {
            assert!(
                !line.contains('!'),
                "copy strings must not use exclamation marks: {line}"
            );
        }
    }

    #[test]
    fn risk_json_uses_spec_liquidation_score_key() {
        let json = serde_json::to_value(&RiskSnapshot {
            score: 3,
            band: "safe".into(),
            distance_from_floor_pct: Some("47".into()),
            is_estimate: false,
        })
        .unwrap();
        assert_eq!(json["liquidation_score"], 3);
        assert!(json.get("score").is_none());
    }

    #[test]
    fn load_chart_rejects_bad_period() {
        match load_chart("AAPL", "year") {
            Err(ChartError::BadRequest(_)) => {}
            other => panic!("expected bad request, got {other:?}"),
        }
        match load_chart("   ", "d") {
            Err(ChartError::BadRequest(_)) => {}
            other => panic!("expected empty ticker, got {other:?}"),
        }
    }

    #[test]
    fn distance_omits_without_floor_or_nonpositive_rapv() {
        assert_eq!(distance_pct(Decimal::from(100), None), None);
        assert_eq!(distance_pct(Decimal::ZERO, Some(Decimal::from(10))), None);
        assert_eq!(
            distance_pct(Decimal::from(100), Some(Decimal::from(53))),
            Some("47".to_string())
        );
    }

    #[test]
    fn product_keywords_cover_type_and_quote() {
        assert!(
            product_keywords("WETH", "Wrapped Ether", "perp", Some("USDT")).contains("perpetual")
        );
        assert!(product_keywords("WETH", "Wrapped Ether", "lend", None).contains("lending"));
        let row = product_from_market(Market {
            product: "perpetual".into(),
            book: "0x1".into(),
            base_token: Asset {
                token_id: 2,
                symbol: "WETH".into(),
                name: "Wrapped Ether".into(),
                token_type: "crypto".into(),
                erc20_address: "0x0".into(),
                erc20_decimals: 18,
                vault_decimals: 8,
                position_decimals: 8,
                risk_price_percent: 5,
                risk_slippage_percent: 0.5,
            },
            quote_token: None,
            buy_token_id: None,
            pay_token_id: None,
            mark_price_raw: 0,
            mark_price: "3500".into(),
        });
        assert_eq!(row.id, "perp:WETH");
        assert_eq!(row.product, "perp");
        assert_eq!(row.base_token_id, 2);
        assert_eq!(product_rank("spot"), 0);
        assert!(product_rank("spot") < product_rank("lend"));
    }
}
