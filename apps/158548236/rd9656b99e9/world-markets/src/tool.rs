use alloy_primitives::Address;
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use std::str::FromStr;

use crate::book::Book;
use crate::client::{Account, AccountAccess, Asset, WorldClient, asset_by_symbol};
use crate::mandate::{Mandate, TradeFacts, Verdict, parse_decimal};
use crate::order_intent::OrderIntent;
use crate::order_word::{OrderType, OrderWord, quantity_raw};
use crate::pnl::PnlLedger;
use crate::size::{ResolvedSize, SizeInput};

#[derive(Clone, Default)]
pub(crate) struct WorldMarketsApp {
    client: WorldClient,
    pnl_ledger: PnlLedger,
    loan_origins: crate::loans::LoanOriginStore,
}

pub(crate) struct ListWorldAssets;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoArgs {}

impl JsonSchema for NoArgs {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "NoArgs".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        })
        .try_into()
        .expect("empty tool argument schema")
    }
}

pub(crate) struct GetWorldAccount;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldAccountArgs {
    /// World account ID. Optional when handover account context is available.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Expected owner wallet. This does not replace the acting wallet authorization check.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
    /// `full` includes the raw account dump. Default is the compact card.
    #[serde(default)]
    pub(crate) detail: Option<String>,
}

pub(crate) struct GetHealthSnapshot;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetHealthSnapshotArgs {
    /// World account ID. Optional when handover account context is available.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Expected owner wallet. This does not replace the acting wallet authorization check.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
    /// Optional position filter for the PnL section (symbol or `perp:SYMBOL`).
    #[serde(default)]
    pub(crate) position: Option<String>,
}

pub(crate) struct GetWorldMarket;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldMarketArgs {
    /// Product type: spot, perp, or lend.
    pub(crate) product: String,
    /// Base asset symbol, such as BTC.b or WETH.
    pub(crate) base_symbol: String,
    /// Quote asset symbol. Required for spot and perp, omitted for lend.
    #[serde(default)]
    pub(crate) quote_symbol: Option<String>,
}

pub(crate) struct GetWorldRates;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldRatesArgs {
    /// Base symbols to include (e.g. ["WETH","WBTC"]). Omit for every listed asset.
    #[serde(default)]
    pub(crate) assets: Option<Vec<String>>,
}

pub(crate) struct GetWorldLoans;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldLoansArgs {
    /// World account ID. Optional when handover account context is available.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Expected owner wallet. This does not replace the acting wallet authorization check.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

pub(crate) struct GetWorldOpenOrders;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldOpenOrdersArgs {
    /// Product type: spot or perp.
    pub(crate) product: String,
    /// Base asset symbol.
    pub(crate) base_symbol: String,
    /// Quote asset symbol.
    pub(crate) quote_symbol: String,
    /// World account ID. Handover account context is used when omitted.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Optional expected owner wallet.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

pub(crate) struct PreviewWorldTrade;
pub(crate) struct CheckWorldMandate;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct WorldTradeArgs {
    /// Product type: spot or perp.
    pub(crate) product: String,
    /// Trade side: buy or sell (long → buy, short → sell).
    pub(crate) side: String,
    /// Base asset symbol.
    pub(crate) base_symbol: String,
    /// Quote asset symbol.
    pub(crate) quote_symbol: String,
    /// Human-readable base quantity, such as "0.25". Alias for size_base.
    #[serde(default)]
    pub(crate) quantity: String,
    /// Dollar/notional size when the user named dollars. Converted at the preview mark.
    #[serde(default)]
    pub(crate) size_usd: Option<String>,
    /// Base-asset size when the user named the asset unit.
    #[serde(default)]
    pub(crate) size_base: Option<String>,
    /// Limit price. Omit for a market intent (a limit at the mark moved by `slippage`).
    #[serde(default)]
    pub(crate) price: Option<String>,
    /// `market` or `limit`. Inferred from `price` when omitted.
    #[serde(default)]
    pub(crate) order_type: Option<String>,
    /// Slippage decimal applied to a market intent, e.g. "0.005" for 0.5%. Default 0.005.
    #[serde(default)]
    pub(crate) slippage: Option<String>,
    /// Optional World account ID. Handover account context is used when omitted.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Optional expected owner wallet.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
    /// The user's whole sentence, so a dollar vs asset size can be classified server-side.
    #[serde(default)]
    pub(crate) text: Option<String>,
}

pub(crate) struct GetWorldPnl;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldPnlArgs {
    /// World account ID. Handover account context is used when omitted.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Optional expected owner wallet.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
    /// Optional position filter: symbol (e.g. "WETH") or id (e.g. "perp:WETH").
    /// Omit for the full account, including recently closed positions this app observed.
    #[serde(default)]
    pub(crate) position: Option<String>,
}

pub(crate) struct ComputeResize;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ComputeResizeArgs {
    /// The engine `rule` code that gated the intent, verbatim.
    pub(crate) rule: String,
}

pub(crate) struct WorldPackOrder;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct WorldPackOrderArgs {
    /// World account id the order is placed for. Must match the bound handover account.
    pub(crate) account_id: u64,
    /// Quantity in position base units, copied from `preview.quantity_raw`. Integer string.
    pub(crate) quantity_raw: String,
    /// Decimal limit price, copied from `preview.limit_price`.
    pub(crate) limit_price: String,
    /// `limit` rests on the book; `fill_all_or_revert` / `fill_partial_kill_rest` are immediate-or-cancel forms.
    pub(crate) order_type: OrderType,
    /// Optional book insertion hint (resting order id near the price). 0 when unknown.
    #[serde(default)]
    pub(crate) insertion_hint: Option<u64>,
}

pub(crate) struct WorldResolveBook;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct WorldResolveBookArgs {
    /// Product type: spot, perp, or lend.
    pub(crate) product: String,
    /// Base token id, from `preview.base.token_id`.
    pub(crate) base_token_id: u32,
    /// Quote token id, from `preview.quote.token_id`. Required for spot and perp.
    #[serde(default)]
    pub(crate) quote_token_id: Option<u32>,
}

/// Everything the execution procedure needs from one previewed intent, with
/// the deterministic mandate verdict. Never executable on its own.
pub(crate) struct TradePreview {
    pub(crate) chain_id: u64,
    pub(crate) exchange: String,
    pub(crate) block_number: u64,
    pub(crate) access: AccountAccess,
    pub(crate) product: &'static str,
    pub(crate) side: String,
    pub(crate) base: Asset,
    pub(crate) quote: Asset,
    pub(crate) order_book: String,
    pub(crate) quantity: Decimal,
    pub(crate) quantity_raw: u64,
    pub(crate) resolved_size: ResolvedSize,
    pub(crate) mark_price: String,
    pub(crate) mark_price_raw: u64,
    pub(crate) intent: OrderIntent,
    pub(crate) estimated_notional: Decimal,
    pub(crate) current_position_quantity: Decimal,
    pub(crate) verdict: Verdict,
}

impl TradePreview {
    pub(crate) fn to_json(&self) -> Value {
        let token = |asset: &Asset| {
            json!({
                "token_id": asset.token_id,
                "symbol": asset.symbol,
                "erc20_address": asset.erc20_address,
                "position_decimals": asset.position_decimals,
            })
        };
        json!({
            "source": "world-markets-contract",
            "chain_id": self.chain_id,
            "exchange": self.exchange,
            "block_number": self.block_number,
            "access": {
                "account_id": self.access.account_id,
                "actor": self.access.actor,
                "authorization": self.access.authorization,
            },
            "preview": {
                "account_id": self.access.account_id,
                "product": self.product,
                "side": self.side,
                "base": token(&self.base),
                "quote": token(&self.quote),
                "order_book": self.order_book,
                "quantity": self.quantity.normalize().to_string(),
                "quantity_raw": self.quantity_raw.to_string(),
                "resolved_size": self.resolved_size.to_json(),
                "mark_price": self.mark_price,
                "mark_price_raw": self.mark_price_raw,
                "order_type": self.intent.order_type,
                "limit_price": self.intent.limit_price.normalize().to_string(),
                "slippage": self.intent.slippage.normalize().to_string(),
                "estimated_notional": self.estimated_notional.normalize().to_string(),
                "current_position_quantity": self.current_position_quantity.normalize().to_string(),
                "verdict": {
                    "status": self.verdict.status,
                    "rule": self.verdict.rule,
                    "detail": self.verdict.detail,
                },
                "executable": false,
            },
        })
    }
}

impl WorldMarketsApp {
    /// The bound World account: the host's handover account reference, then
    /// the account the mandate was issued for. A local build may fall back to
    /// `WORLD_ACCOUNT_ID`; a hosted build never does.
    fn account_id(ctx: &DynToolCallCtx, explicit: Option<u64>) -> Option<u64> {
        let from_value = |value: Option<&Value>| {
            value.and_then(|value| {
                value.as_u64().or_else(|| {
                    let raw = value.as_str()?;
                    raw.parse::<u64>()
                        .ok()
                        .or_else(|| raw.strip_prefix("world-")?.parse::<u64>().ok())
                })
            })
        };
        from_value(ctx.attribute_path(&["handover", "account_ref"]))
            .or_else(|| from_value(ctx.attribute_path(&["handover", "mandate", "account", "id"])))
            .or(explicit)
            .or_else(Self::local_account_id)
    }

    /// `WORLD_ACCOUNT_ID` for aomi-run, where the runtime stubs every handover
    /// attribute. Compiled out of a hosted build.
    #[cfg(feature = "local-dev")]
    fn local_account_id() -> Option<u64> {
        let raw = std::env::var("WORLD_ACCOUNT_ID").ok()?;
        raw.parse::<u64>()
            .ok()
            .or_else(|| raw.strip_prefix("world-")?.parse::<u64>().ok())
    }

    #[cfg(not(feature = "local-dev"))]
    fn local_account_id() -> Option<u64> {
        None
    }

    /// The client for this call: the host's chain must be the World chain,
    /// and a handover may pin the exchange address it was issued against.
    fn client_for(&self, ctx: &DynToolCallCtx) -> Result<WorldClient, String> {
        if let Some(chain_id) = ctx
            .attribute_u64(&["handover", "chain_id"])
            .or_else(|| ctx.attribute_u64(&["domain", "evm", "chain_id"]))
            && chain_id != self.client.chain_id()
        {
            return Err(format!(
                "[world-markets] active chain {chain_id} is not the World chain {}",
                self.client.chain_id()
            ));
        }
        match ctx.attribute_string(&["handover", "context", "world", "exchange"]) {
            Some(raw) => {
                let exchange = Address::from_str(raw.trim()).map_err(|e| {
                    format!("[world-markets] invalid handover exchange address: {e}")
                })?;
                Ok(self.client.with_exchange(exchange))
            }
            None => Ok(self.client.clone()),
        }
    }

    fn access(
        &self,
        client: &WorldClient,
        account_id: Option<u64>,
        wallet_address: Option<&str>,
        ctx: &DynToolCallCtx,
    ) -> Result<AccountAccess, String> {
        let account_id = Self::account_id(ctx, account_id);
        // A host-verified handover fixes the account and owner. Model arguments
        // are lookup hints only when no such binding exists; they cannot replace
        // it with the managed signer or another user's account.
        let owner_wallet = ctx
            .attribute_string(&["handover", "owner_address"])
            .or_else(|| wallet_address.map(ToString::to_string));
        // World grants the OperatingAccount because that is the address which
        // calls the venue under AA. The Para agent in domain.evm is only the
        // OperatingAccount owner/signature authority and must never be treated
        // as the venue trader.
        let actor = ctx
            .attribute_string(&["handover", "operating_address"])
            .or_else(|| ctx.attribute_string(&["domain", "evm", "address"]));
        client.resolve_account(account_id, owner_wallet.as_deref(), actor.as_deref())
    }

    fn live_account(
        client: &WorldClient,
        access: &AccountAccess,
    ) -> Result<(Vec<Asset>, Account), String> {
        let assets = client.assets()?;
        let owner = Address::from_str(&access.owner)
            .map_err(|e| format!("[world-markets] invalid owner address: {e}"))?;
        let account = client.account_with_owner(access.account_id, owner, &assets)?;
        Ok((assets, account))
    }

    /// Account card: access, contract account facts, ATLAS metrics and the
    /// derived lookup figures.
    fn inspect_account(
        &self,
        account_id: Option<u64>,
        wallet_address: Option<&str>,
        ctx: &DynToolCallCtx,
    ) -> Result<(Value, Account, AccountAccess, WorldClient), String> {
        let client = self.client_for(ctx)?;
        let access = self.access(&client, account_id, wallet_address, ctx)?;
        let (assets, account) = Self::live_account(&client, &access)?;
        let block_number = client.block_number()?;
        let metrics =
            crate::liquidation_risk::compute_metrics(&client, &account, &assets, block_number)?;
        let lookups = crate::lookups::compute_lookups(
            &client,
            &account,
            &assets,
            &metrics.net_asset_value,
            block_number,
        )?;
        let payload = json!({
            "source": "world-markets-contract",
            "chain_id": client.chain_id(),
            "exchange": client.exchange(),
            "block_number": block_number,
            "access": {
                "account_id": access.account_id,
                "authorization": access.authorization,
            },
            "account": {
                "account_id": account.account_id,
                "eligible_for_liquidation": account.eligible_for_liquidation,
                "risk_adjusted_portfolio_value": account.risk_adjusted_portfolio_value,
            },
            "metrics": metrics,
            "lookups": lookups,
        });
        Ok((payload, account, access, client))
    }

    fn trade_preview(&self, args: WorldTradeArgs, ctx: &DynToolCallCtx) -> Result<Value, String> {
        let product = match args.product.to_ascii_lowercase().as_str() {
            "spot" => "spot",
            "perp" | "perpetual" => "perp",
            _ => return Err("[world-markets] trade tools support spot and perp only".to_string()),
        };
        let side = match args.side.trim().to_ascii_lowercase().as_str() {
            "buy" | "long" => "buy".to_string(),
            "sell" | "short" => "sell".to_string(),
            _ => {
                return Err(
                    "[world-markets] side must be buy or sell (short→sell, long→buy). Resend with side set."
                        .to_string(),
                );
            }
        };

        let client = self.client_for(ctx)?;
        let access = self.access(
            &client,
            args.account_id,
            args.wallet_address.as_deref(),
            ctx,
        )?;
        let (assets, account) = Self::live_account(&client, &access)?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = asset_by_symbol(&assets, &args.quote_symbol)?;
        let market = client.market(product, base.clone(), Some(quote.clone()))?;
        let mark_price = parse_decimal(&market.mark_price, "mark_price")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;

        let quantity_arg = args.quantity.trim();
        let resolved = match (SizeInput {
            sentence: args.text.as_deref(),
            size_usd: args.size_usd.as_deref(),
            size_base: args.size_base.as_deref(),
            quantity: (!quantity_arg.is_empty()).then_some(quantity_arg),
            instrument: Some(&base.symbol),
        })
        .resolve(mark_price)
        {
            Ok(resolved) => resolved,
            Err(err) => return Ok(err.to_json()),
        };
        let quantity_raw = quantity_raw(resolved.base_qty, base.position_decimals)?;
        if quantity_raw == 0 {
            return Err(format!(
                "[world-markets] quantity {} is below the {} position precision ({} decimals)",
                resolved.base_qty.normalize(),
                base.symbol,
                base.position_decimals
            ));
        }
        let quantity = Decimal::from(quantity_raw)
            / Decimal::from(10u64.pow(u32::from(base.position_decimals)));

        let intent = OrderIntent::resolve(
            args.order_type.as_deref(),
            args.price
                .as_deref()
                .map(|raw| parse_decimal(raw, "price"))
                .transpose()
                .map_err(|verdict| {
                    format!("[world-markets] {}: {}", verdict.rule, verdict.detail)
                })?,
            args.slippage
                .as_deref()
                .map(|raw| parse_decimal(raw, "slippage"))
                .transpose()
                .map_err(|verdict| {
                    format!("[world-markets] {}: {}", verdict.rule, verdict.detail)
                })?,
            &side,
            mark_price,
        )?;

        let current_position_quantity = account.position_quantity(product, &base.symbol)?;
        let rapv = parse_decimal(
            &account.risk_adjusted_portfolio_value,
            "risk_adjusted_portfolio_value",
        )
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        // A failed ATLAS projection leaves `post_trade_rapv` unproven and the
        // mandate denies with `post_trade_risk_unavailable`.
        let post_trade_rapv = crate::liquidation_risk::project_post_trade(
            &client,
            &account,
            &assets,
            &crate::liquidation_risk::TradeIntent {
                product,
                side: &side,
                base: &base,
                quote: &quote,
                quantity,
                mark: mark_price,
            },
        )
        .ok()
        .map(|projection| projection.rapv);
        let verdict = match Mandate::bound(ctx.attribute_path(&["handover", "mandate"])) {
            Ok(mandate) => mandate.evaluate(&TradeFacts {
                product,
                side: &side,
                base: &base.symbol,
                quote: &quote.symbol,
                quantity,
                mark_price,
                current_position_quantity,
                risk_adjusted_portfolio_value: rapv,
                post_trade_risk_adjusted_portfolio_value: post_trade_rapv,
                eligible_for_liquidation: account.eligible_for_liquidation,
            }),
            Err(verdict) => verdict,
        };
        let estimated_notional = quantity.checked_mul(mark_price).ok_or_else(|| {
            "[world-markets] estimated notional exceeds numeric range".to_string()
        })?;

        Ok(TradePreview {
            chain_id: client.chain_id(),
            exchange: client.exchange(),
            block_number: client.block_number()?,
            access,
            product,
            side,
            base,
            quote,
            order_book: market.book,
            quantity,
            quantity_raw,
            resolved_size: resolved,
            mark_price: market.mark_price,
            mark_price_raw: market.mark_price_raw,
            intent,
            estimated_notional,
            current_position_quantity,
            verdict,
        }
        .to_json())
    }
}

impl DynAomiTool for ListWorldAssets {
    type App = WorldMarketsApp;
    type Args = NoArgs;
    const NAME: &'static str = "list_world_assets";
    const DESCRIPTION: &'static str = "List live World Markets assets and their token IDs, symbols, addresses, decimals, and risk parameters.";

    fn run(app: &WorldMarketsApp, _args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let client = app.client_for(&ctx)?;
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": client.chain_id(),
            "exchange": client.exchange(),
            "block_number": client.block_number()?,
            "assets": client.assets()?,
        }))
    }
}

impl DynAomiTool for GetWorldAccount {
    type App = WorldMarketsApp;
    type Args = GetWorldAccountArgs;
    const NAME: &'static str = "get_world_account";
    const DESCRIPTION: &'static str = "Inspect a live World account after proving that the active actor is its owner or an on-chain permitted trader. Returns account facts, ATLAS metrics (net asset value, 0–10 liquidation risk) and per-class exposure lookups. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let before = app.client.rpc_stats();
        let (mut payload, account, access, client) =
            app.inspect_account(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        if args.detail.as_deref() == Some("full")
            && let Some(obj) = payload.as_object_mut()
        {
            obj.insert("account".into(), json!(account));
            obj.insert("access".into(), json!(access));
        }
        client.attach_rpc_trace(before, &mut payload);
        Ok(payload)
    }
}

impl DynAomiTool for GetHealthSnapshot {
    type App = WorldMarketsApp;
    type Args = GetHealthSnapshotArgs;
    const NAME: &'static str = "get_health_snapshot";
    const DESCRIPTION: &'static str =
        "Health card in one call: the account card plus perpetual PnL. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let before = app.client.rpc_stats();
        let (mut payload, account, _access, client) =
            app.inspect_account(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let pnl = crate::pnl::report(&client, &app.pnl_ledger, &account, args.position.as_deref())?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("pnl".to_string(), json!(pnl));
            obj.insert("executable".to_string(), json!(false));
        }
        client.attach_rpc_trace(before, &mut payload);
        Ok(payload)
    }
}

impl DynAomiTool for GetWorldMarket {
    type App = WorldMarketsApp;
    type Args = GetWorldMarketArgs;
    const NAME: &'static str = "get_world_market";
    const DESCRIPTION: &'static str = "Resolve a live World spot, perpetual, or lending order book and the current configured mark price.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let client = app.client_for(&ctx)?;
        let assets = client.assets()?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = args
            .quote_symbol
            .as_deref()
            .map(|symbol| asset_by_symbol(&assets, symbol))
            .transpose()?;
        let product = args.product.to_ascii_lowercase();
        let market = client.market(&product, base, quote)?;
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": client.chain_id(),
            "exchange": client.exchange(),
            "block_number": client.block_number()?,
            "market": market,
        }))
    }
}

impl DynAomiTool for GetWorldRates {
    type App = WorldMarketsApp;
    type Args = GetWorldRatesArgs;
    const NAME: &'static str = "get_world_rates";
    const DESCRIPTION: &'static str = crate::rates::RATES_DESCRIPTION;

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let client = app.client_for(&ctx)?;
        let snapshot = crate::rates::snapshot(&client, args.assets.as_deref())?;
        serde_json::to_value(&snapshot)
            .map_err(|e| format!("[world-markets] failed to encode rates snapshot: {e}"))
    }
}

impl DynAomiTool for GetWorldLoans {
    type App = WorldMarketsApp;
    type Args = GetWorldLoansArgs;
    const NAME: &'static str = "get_world_loans";
    const DESCRIPTION: &'static str = "Individual lend/borrow loans: rate_apr, matures_at, time_remaining_seconds, extensible, counterparty. Aggregates on get_world_account are not enough for roll timing. 10-day term; missing start → first-seen+10d, extensible true. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let client = app.client_for(&ctx)?;
        let access = app.access(
            &client,
            args.account_id,
            args.wallet_address.as_deref(),
            &ctx,
        )?;
        let (assets, account) = WorldMarketsApp::live_account(&client, &access)?;
        let snapshot = crate::loans::snapshot(&client, &app.loan_origins, &account, &assets)?;
        serde_json::to_value(&snapshot)
            .map_err(|e| format!("[world-markets] failed to encode loans snapshot: {e}"))
    }
}

impl DynAomiTool for GetWorldOpenOrders {
    type App = WorldMarketsApp;
    type Args = GetWorldOpenOrdersArgs;
    const NAME: &'static str = "get_world_open_orders";
    const DESCRIPTION: &'static str = "Read the authorized World account's resting buy and sell orders for one live spot or perpetual market. Order ids here are what a cancel targets.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let product = match args.product.to_ascii_lowercase().as_str() {
            "spot" => "spot",
            "perp" | "perpetual" => "perp",
            _ => return Err("[world-markets] open orders support spot and perp only".to_string()),
        };
        let client = app.client_for(&ctx)?;
        let access = app.access(
            &client,
            args.account_id,
            args.wallet_address.as_deref(),
            &ctx,
        )?;
        let assets = client.assets()?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = asset_by_symbol(&assets, &args.quote_symbol)?;
        let market = client.market(product, base, Some(quote))?;
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": client.chain_id(),
            "exchange": client.exchange(),
            "block_number": client.block_number()?,
            "access": access,
            "open_orders": client.open_orders(&market, access.account_id)?,
        }))
    }
}

impl DynAomiTool for PreviewWorldTrade {
    type App = WorldMarketsApp;
    type Args = WorldTradeArgs;
    const NAME: &'static str = "preview_world_trade";
    const DESCRIPTION: &'static str = "Preview one World spot or perpetual intent from live state: resolved size, book, mark, limit price, and the deterministic mandate verdict. It never stages or executes. A deny verdict is a hard stop. An allow verdict is the input to the world-markets/execution procedure (world_resolve_book → world_pack_order → evm_stage_tx). Pass text=the user's whole sentence so dollar vs asset sizes classify server-side.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        app.trade_preview(args, &ctx)
    }
}

impl DynAomiTool for CheckWorldMandate {
    type App = WorldMarketsApp;
    type Args = WorldTradeArgs;
    const NAME: &'static str = "check_world_mandate";
    const DESCRIPTION: &'static str = "Evaluate one structured World trade intent against the bound mandate and live account/market state. Same body as preview_world_trade: the exact allow or deny rule under `preview.verdict`; it does not execute.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        app.trade_preview(args, &ctx)
    }
}

impl DynAomiTool for GetWorldPnl {
    type App = WorldMarketsApp;
    type Args = GetWorldPnlArgs;
    const NAME: &'static str = "get_world_pnl";
    const DESCRIPTION: &'static str = "Compute account-level and per-position perpetual PnL. Open PnL is mark versus contract entry minus unpaid funding. Position PnL covers that position's lifetime (open to now, or open to close). Realized figures are captured when this app observes a true-up or close. Not a calendar-range report; never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let client = app.client_for(&ctx)?;
        let access = app.access(
            &client,
            args.account_id,
            args.wallet_address.as_deref(),
            &ctx,
        )?;
        let (_assets, account) = WorldMarketsApp::live_account(&client, &access)?;
        let pnl = crate::pnl::report(&client, &app.pnl_ledger, &account, args.position.as_deref())?;
        Ok(json!({
            "source": "world-markets-reporting",
            "chain_id": client.chain_id(),
            "exchange": client.exchange(),
            "block_number": client.block_number()?,
            "access": access,
            "executable": false,
            "pnl": pnl,
        }))
    }
}

impl DynAomiTool for ComputeResize {
    type App = WorldMarketsApp;
    type Args = ComputeResizeArgs;
    const NAME: &'static str = "compute_resize";
    const DESCRIPTION: &'static str = "For a blocked intent, return the user's RAPV floor from the signed mandate. A block cites exactly one number: the floor. Never executes.";

    fn run(_app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let resize = Mandate::bound(ctx.attribute_path(&["handover", "mandate"]))
            .and_then(|mandate| mandate.floor_solution(&args.rule))
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        Ok(json!({
            "source": "world-markets-reporting",
            "resize": resize,
            "executable": false,
        }))
    }
}

impl DynAomiTool for WorldPackOrder {
    type App = WorldMarketsApp;
    type Args = WorldPackOrderArgs;
    const NAME: &'static str = "world_pack_order";
    const DESCRIPTION: &'static str = "Pack one World order into the uint256 word a new*Order(address,uint256) call takes: [insertionHint 64 | orderType 4 | accountId 44 | quantity 64 | price 64]. Inputs come verbatim from an allow preview (quantity_raw, limit_price, account_id). Pure computation; never executes.";

    fn run(_app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        if let Some(bound) = WorldMarketsApp::account_id(&ctx, None)
            && bound != args.account_id
        {
            return Err(format!(
                "[world-markets] account_id {} is not the bound handover account {bound}",
                args.account_id
            ));
        }
        let quantity_raw = u64::from_str(args.quantity_raw.trim()).map_err(|_| {
            "[world-markets] quantity_raw must be an integer string in position units (copy preview.quantity_raw)"
                .to_string()
        })?;
        if quantity_raw == 0 {
            return Err("[world-markets] quantity_raw must be greater than zero".to_string());
        }
        let limit_price = parse_decimal(args.limit_price.trim(), "limit_price")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        if limit_price <= Decimal::ZERO {
            return Err("[world-markets] limit_price must be greater than zero".to_string());
        }
        let mut value = OrderWord {
            account_id: args.account_id,
            quantity_raw,
            limit_price,
            order_type: args.order_type,
            insertion_hint: args.insertion_hint.unwrap_or(0),
        }
        .to_json()?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert("source".to_string(), json!("world-markets-packer"));
            obj.insert("executable".to_string(), json!(false));
        }
        Ok(value)
    }
}

impl DynAomiTool for WorldResolveBook {
    type App = WorldMarketsApp;
    type Args = WorldResolveBookArgs;
    const NAME: &'static str = "world_resolve_book";
    const DESCRIPTION: &'static str = "Resolve the World order-book contract for a product and token pair: the `book` address a new*Order/cancel*Order call takes as its first argument, plus the book's buy and sell token ids. Returns error=no_book when the exchange lists no such book. Read-only.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let client = app.client_for(&ctx)?;
        let mut value = match Book::resolve(
            &client,
            &args.product,
            args.base_token_id,
            args.quote_token_id,
        )? {
            Some(book) => book.to_json(),
            None => json!({
                "error": "no_book",
                "detail": format!(
                    "the exchange lists no {} book for token {} / {:?}",
                    args.product.to_ascii_lowercase(),
                    args.base_token_id,
                    args.quote_token_id
                ),
            }),
        };
        if let Some(obj) = value.as_object_mut() {
            obj.insert("source".to_string(), json!("world-markets-contract"));
            obj.insert("chain_id".to_string(), json!(client.chain_id()));
            obj.insert("exchange".to_string(), json!(client.exchange()));
            obj.insert("executable".to_string(), json!(false));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(attributes: Value) -> DynToolCallCtx {
        DynToolCallCtx {
            session_id: "test".to_string(),
            tool_name: "get_world_account".to_string(),
            call_id: "test-1".to_string(),
            state_attributes: attributes.as_object().unwrap().clone(),
            secrets: Default::default(),
        }
    }

    fn mandate_json(account: Option<u64>) -> Value {
        let mut mandate = json!({
            "version": 1,
            "markets": [{ "product": "perp", "base": "WETH", "quote": "USDT" }],
            "max_position_notional": { "amount": "25000", "quote": "USDT" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "6000", "quote": "USDT" },
            "halt_if_eligible_for_liquidation": true,
            "can_withdraw": false
        });
        if let Some(id) = account {
            mandate["account"] = json!({ "id": id });
        }
        mandate
    }

    #[test]
    fn handover_identity_overrides_model_wallet_and_account_arguments() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Venue(AtomicUsize, bool);
        impl crate::rpc::RpcExecutor for Venue {
            fn post_json(&self, body: &Value) -> Result<Value, String> {
                assert_eq!(body["method"], "eth_call");
                assert!(
                    body["params"][0]["data"]
                        .as_str()
                        .unwrap()
                        .ends_with(&format!("{:064x}", 21))
                );
                let result = match self.0.fetch_add(1, Ordering::SeqCst) {
                    0 => format!("0x{:0>64}", "1111111111111111111111111111111111111111"),
                    1 if self.1 => format!(
                        "0x{:064x}{:064x}{:0>64}",
                        32, 1, "2222222222222222222222222222222222222222"
                    ),
                    1 => format!("0x{:064x}{:064x}", 32, 0),
                    _ => panic!("unexpected venue read"),
                };
                Ok(json!({"jsonrpc":"2.0", "id":body["id"], "result":result}))
            }
        }
        let client = WorldClient::with_rpc(crate::rpc::RpcTransport::with_executor(Arc::new(
            Venue(AtomicUsize::new(0), true),
        )));
        let app = WorldMarketsApp::default();
        let ctx = ctx_with(json!({
            "domain": { "evm": { "address": "0x3333333333333333333333333333333333333333", "chain_id": 1 } },
            "handover": {
                "account_ref": "21", "chain_id": 2092151908u64,
                "owner_address": "0x1111111111111111111111111111111111111111",
                "operating_address": "0x2222222222222222222222222222222222222222"
            }
        }));
        assert!(app.client_for(&ctx).is_ok());
        let access = app
            .access(
                &client,
                Some(99),
                Some("0x3333333333333333333333333333333333333333"),
                &ctx,
            )
            .unwrap();
        assert_eq!(access.account_id, 21);
        assert_eq!(access.owner, "0x1111111111111111111111111111111111111111");
        assert_eq!(access.actor, "0x2222222222222222222222222222222222222222");
        assert_eq!(access.authorization, "delegated_trader");
        let revoked = WorldClient::with_rpc(crate::rpc::RpcTransport::with_executor(Arc::new(
            Venue(AtomicUsize::new(0), false),
        )));
        let error = app.access(&revoked, None, None, &ctx).unwrap_err();
        assert!(error.contains("neither the owner"), "{error}");
    }

    #[test]
    fn account_id_prefers_handover_account_ref_then_mandate_account() {
        let both = ctx_with(json!({
            "handover": { "account_ref": "world-1234", "mandate": mandate_json(Some(99)) }
        }));
        assert_eq!(WorldMarketsApp::account_id(&both, None), Some(1234));
        assert_eq!(WorldMarketsApp::account_id(&both, Some(7)), Some(1234));
        assert_eq!(
            WorldMarketsApp::account_id(&ctx_with(json!({})), Some(7)),
            Some(7)
        );

        let numeric = ctx_with(json!({ "handover": { "account_ref": 55 } }));
        assert_eq!(WorldMarketsApp::account_id(&numeric, None), Some(55));

        let mandate_only = ctx_with(json!({ "handover": { "mandate": mandate_json(Some(99)) } }));
        assert_eq!(WorldMarketsApp::account_id(&mandate_only, None), Some(99));

        let foreign = ctx_with(json!({ "handover": { "account_ref": "other-55" } }));
        assert_eq!(WorldMarketsApp::account_id(&foreign, None), None);

        // Attribute names the host never publishes must not resolve.
        let legacy = ctx_with(json!({
            "world": { "account_id": 1 },
            "handover_account_id": 2,
            "platform_account_ref": 3,
            "telegram": { "user_id": 4 },
        }));
        assert_eq!(WorldMarketsApp::account_id(&legacy, None), None);
    }

    #[cfg(not(feature = "local-dev"))]
    #[test]
    fn hosted_build_ignores_world_account_id_env() {
        unsafe { std::env::set_var("WORLD_ACCOUNT_ID", "world-777") };
        assert_eq!(
            WorldMarketsApp::account_id(&ctx_with(json!({})), None),
            None
        );
        unsafe { std::env::remove_var("WORLD_ACCOUNT_ID") };
    }

    #[test]
    fn client_for_rejects_a_foreign_chain_and_pins_the_handover_exchange() {
        let app = WorldMarketsApp::default();
        let foreign = ctx_with(json!({ "domain": { "evm": { "chain_id": 1 } } }));
        assert!(app.client_for(&foreign).is_err());

        let pinned = ctx_with(json!({
            "domain": { "evm": { "chain_id": 2092151908u64 } },
            "handover": { "context": { "world": {
                "exchange": "0x1111111111111111111111111111111111111111"
            } } }
        }));
        let client = app.client_for(&pinned).unwrap();
        assert_eq!(
            client.exchange(),
            "0x1111111111111111111111111111111111111111"
        );
        assert_eq!(client.chain_id(), 2092151908);

        let bad =
            ctx_with(json!({ "handover": { "context": { "world": { "exchange": "nope" } } } }));
        assert!(app.client_for(&bad).is_err());
    }

    #[test]
    fn preview_contract_snapshot() {
        let weth = Asset {
            token_id: 4,
            symbol: "WETH".to_string(),
            name: "Wrapped Ether".to_string(),
            token_type: "erc20".to_string(),
            erc20_address: "0x2222222222222222222222222222222222222222".to_string(),
            erc20_decimals: 18,
            vault_decimals: 8,
            position_decimals: 4,
            risk_price_percent: 10,
            risk_slippage_percent: 1.0,
        };
        let usdt = Asset {
            token_id: 1,
            symbol: "USDT".to_string(),
            name: "Tether".to_string(),
            token_type: "erc20".to_string(),
            erc20_address: "0x3333333333333333333333333333333333333333".to_string(),
            erc20_decimals: 6,
            vault_decimals: 6,
            position_decimals: 2,
            risk_price_percent: 0,
            risk_slippage_percent: 0.0,
        };
        let mark = Decimal::from_str("2465.71").unwrap();
        let resolved = SizeInput {
            sentence: Some("sell 0.01 WETH spot at market"),
            instrument: Some("WETH"),
            ..SizeInput::default()
        }
        .resolve(mark)
        .unwrap();
        let quantity = Decimal::from_str("0.01").unwrap();
        let preview = TradePreview {
            chain_id: 2092151908,
            exchange: "0xf6b54e033bb45a583aa642924bcef78b804588ae".to_string(),
            block_number: 12345,
            access: AccountAccess {
                account_id: 1577,
                owner: "0x4444444444444444444444444444444444444444".to_string(),
                actor: "0x5555555555555555555555555555555555555555".to_string(),
                authorization: "delegated_trader".to_string(),
            },
            product: "spot",
            side: "sell".to_string(),
            base: weth,
            quote: usdt,
            order_book: "0x6666666666666666666666666666666666666666".to_string(),
            quantity,
            quantity_raw: 100,
            resolved_size: resolved,
            mark_price: "2465.71".to_string(),
            mark_price_raw: 7890274,
            intent: OrderIntent::resolve(None, None, None, "sell", mark).unwrap(),
            estimated_notional: quantity * mark,
            current_position_quantity: Decimal::from_str("0.05").unwrap(),
            verdict: Verdict {
                status: "allow",
                rule: "mandate_v1",
                detail: "Mandate v1 permits spot sell 0.01 WETH.".to_string(),
            },
        }
        .to_json();

        assert_eq!(
            preview,
            json!({
                "source": "world-markets-contract",
                "chain_id": 2092151908u64,
                "exchange": "0xf6b54e033bb45a583aa642924bcef78b804588ae",
                "block_number": 12345,
                "access": {
                    "account_id": 1577,
                    "actor": "0x5555555555555555555555555555555555555555",
                    "authorization": "delegated_trader",
                },
                "preview": {
                    "account_id": 1577,
                    "product": "spot",
                    "side": "sell",
                    "base": {
                        "token_id": 4,
                        "symbol": "WETH",
                        "erc20_address": "0x2222222222222222222222222222222222222222",
                        "position_decimals": 4,
                    },
                    "quote": {
                        "token_id": 1,
                        "symbol": "USDT",
                        "erc20_address": "0x3333333333333333333333333333333333333333",
                        "position_decimals": 2,
                    },
                    "order_book": "0x6666666666666666666666666666666666666666",
                    "quantity": "0.01",
                    "quantity_raw": "100",
                    "resolved_size": {
                        "input": "0.01",
                        "denomination": "base",
                        "mark": "2465.71",
                        "base_qty": "0.01",
                        "notional": "24.6571",
                        "notional_rendered": "`$24.66`",
                    },
                    "mark_price": "2465.71",
                    "mark_price_raw": 7890274,
                    "order_type": "market",
                    "limit_price": "2453.38145",
                    "slippage": "0.005",
                    "estimated_notional": "24.6571",
                    "current_position_quantity": "0.05",
                    "verdict": {
                        "status": "allow",
                        "rule": "mandate_v1",
                        "detail": "Mandate v1 permits spot sell 0.01 WETH.",
                    },
                    "executable": false,
                },
            })
        );
    }

    #[test]
    fn preview_fails_closed_without_account_context() {
        let app = WorldMarketsApp::default();
        let err = app
            .trade_preview(
                WorldTradeArgs {
                    product: "perp".to_string(),
                    side: "buy".to_string(),
                    base_symbol: "WETH".to_string(),
                    quote_symbol: "USDT".to_string(),
                    quantity: "0.1".to_string(),
                    ..WorldTradeArgs::default()
                },
                &ctx_with(json!({ "domain": { "evm": { "chain_id": 1 } } })),
            )
            .unwrap_err();
        assert!(err.contains("not the World chain"), "{err}");
    }

    #[test]
    fn compute_resize_carries_floor_and_rule() {
        let app = WorldMarketsApp::default();
        let value = ComputeResize::run(
            &app,
            ComputeResizeArgs {
                rule: "portfolio_floor".to_string(),
            },
            ctx_with(json!({ "handover": { "mandate": mandate_json(None) } })),
        )
        .unwrap();
        assert_eq!(value["resize"]["rule"], "portfolio_floor");
        assert_eq!(value["resize"]["floor"]["value"], "6000");
        assert_eq!(value["executable"], false);

        let missing = ComputeResize::run(
            &app,
            ComputeResizeArgs {
                rule: "portfolio_floor".to_string(),
            },
            ctx_with(json!({})),
        )
        .unwrap_err();
        assert!(missing.contains("missing_mandate"), "{missing}");
    }

    #[test]
    fn pack_order_checks_the_bound_account_and_inputs() {
        let app = WorldMarketsApp::default();
        let ctx = || ctx_with(json!({ "handover": { "account_ref": 1577 } }));
        let ok = WorldPackOrder::run(
            &app,
            WorldPackOrderArgs {
                account_id: 1577,
                quantity_raw: "100".to_string(),
                limit_price: "2465.71".to_string(),
                order_type: OrderType::Limit,
                insertion_hint: None,
            },
            ctx(),
        )
        .unwrap();
        assert_eq!(
            ok["word"],
            "0x0000000000000000000000000000062900000000000000640000000000786562"
        );
        assert_eq!(ok["executable"], false);

        let foreign = WorldPackOrder::run(
            &app,
            WorldPackOrderArgs {
                account_id: 19,
                quantity_raw: "100".to_string(),
                limit_price: "2465.71".to_string(),
                order_type: OrderType::Limit,
                insertion_hint: None,
            },
            ctx(),
        )
        .unwrap_err();
        assert!(foreign.contains("bound handover account"), "{foreign}");

        let bad_qty = WorldPackOrder::run(
            &app,
            WorldPackOrderArgs {
                account_id: 1577,
                quantity_raw: "0.01".to_string(),
                limit_price: "2465.71".to_string(),
                order_type: OrderType::Limit,
                insertion_hint: None,
            },
            ctx(),
        )
        .unwrap_err();
        assert!(bad_qty.contains("quantity_raw"), "{bad_qty}");
    }

    #[test]
    #[ignore = "requires live UniFi RPC"]
    fn live_preview_uses_actor_account_and_mandate_context() {
        let app = WorldMarketsApp::default();
        let account_id = 1_577;
        let owner = app.client.owner_for(account_id).unwrap();
        let ctx = ctx_with(json!({
            "domain": { "evm": { "address": format!("{owner:#x}"), "chain_id": 2092151908u64 } },
            "handover": { "account_ref": account_id, "mandate": mandate_json(Some(account_id)) }
        }));
        let value = app
            .trade_preview(
                WorldTradeArgs {
                    product: "perp".to_string(),
                    side: "buy".to_string(),
                    base_symbol: "WETH".to_string(),
                    quote_symbol: "USDT".to_string(),
                    quantity: "0.01".to_string(),
                    ..WorldTradeArgs::default()
                },
                &ctx,
            )
            .unwrap();
        assert_eq!(value["access"]["authorization"], "owner");
        assert_eq!(value["preview"]["verdict"]["status"], "deny");
        assert_eq!(value["preview"]["verdict"]["rule"], "portfolio_floor");
        assert_eq!(value["preview"]["executable"], false);
    }
}
