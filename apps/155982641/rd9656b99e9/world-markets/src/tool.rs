use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{Account, AccountAccess, CHAIN_ID, WorldClient, asset_by_symbol};
use crate::execution::{
    CancelOrderRequest, CloseLoanRequest, ExecutionClient, PayInterestRequest, PlaceOrderRequest,
    RenewLoansRequest, SwapRequest,
};
use crate::guest::{self, Funnel, FunnelConfig, GuestStore};
use crate::mandate::{Mandate, TradeFacts, Verdict, parse_decimal};
use crate::pnl::PnlLedger;
use crate::reporting::{
    EffectPlan, FixtureReporting, GuardianPreference, Reporting, ResizeInput, SliceInput,
    UnwindCandidate,
};

#[derive(Clone, Default)]
pub(crate) struct WorldMarketsApp {
    client: WorldClient,
    reporting: FixtureReporting,
    pnl_ledger: PnlLedger,
    guest_store: GuestStore,
    carry_ledger: crate::carry::CarryLedger,
    loan_origins: crate::loans::LoanOriginStore,
    execution: ExecutionClient,
}

struct LiveVerdictInput<'a> {
    product: &'a str,
    side: &'a str,
    base: &'a crate::client::Asset,
    quote: &'a crate::client::Asset,
    quantity: Decimal,
    account: &'a Account,
}

pub(crate) struct ListWorldAssets;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ListWorldAssetsArgs {}

pub(crate) struct GetWorldAccount;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldAccountArgs {
    /// World account ID. Optional when handover account context is available.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Expected owner wallet. This does not replace the acting wallet authorization check.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
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

impl DynAomiTool for GetWorldRates {
    type App = WorldMarketsApp;
    type Args = GetWorldRatesArgs;
    const NAME: &'static str = "get_world_rates";
    const DESCRIPTION: &'static str = crate::rates::RATES_DESCRIPTION;

    fn run(app: &WorldMarketsApp, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let snapshot = crate::rates::snapshot(&app.client, args.assets.as_deref())?;
        serde_json::to_value(&snapshot)
            .map_err(|e| format!("[world-markets] failed to encode rates snapshot: {e}"))
    }
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

impl DynAomiTool for GetWorldLoans {
    type App = WorldMarketsApp;
    type Args = GetWorldLoansArgs;
    const NAME: &'static str = "get_world_loans";
    const DESCRIPTION: &'static str = "List this account's individual lend and borrow loans with fixed rate_apr, matures_at (unix seconds), time_remaining_seconds, extensible, and counterparty. get_world_account only exposes aggregated lend/borrow quantities, so this tool is required for roll timing. World loans are a 10-day term. When the contract does not expose start time, maturity is first-seen plus 10 days and extensible defaults true. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let assets = app.client.assets()?;
        let account = app.client.account(access.account_id, &assets)?;
        let snapshot = crate::loans::snapshot(&app.client, &app.loan_origins, &account, &assets)?;
        serde_json::to_value(&snapshot)
            .map_err(|e| format!("[world-markets] failed to encode loans snapshot: {e}"))
    }
}

pub(crate) struct PreviewWorldTrade;
pub(crate) struct CheckWorldMandate;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct WorldTradeArgs {
    /// Product type: spot or perp.
    pub(crate) product: String,
    /// Trade side: buy or sell.
    pub(crate) side: String,
    /// Base asset symbol.
    pub(crate) base_symbol: String,
    /// Quote asset symbol.
    pub(crate) quote_symbol: String,
    /// Human-readable base quantity, such as "0.25".
    pub(crate) quantity: String,
    /// Optional World account ID. Handover account context is used when omitted.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Optional expected owner wallet.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

pub(crate) struct ExecuteWorldOrder;
pub(crate) struct CancelWorldOrder;
pub(crate) struct ExecuteWorldSwap;
pub(crate) struct RenewWorldLoans;
pub(crate) struct PayWorldLoanInterest;
pub(crate) struct CloseWorldLoan;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ExecuteWorldOrderArgs {
    /// Product type: spot, perp, or lend.
    pub(crate) product: String,
    /// Side: buy/sell (spot), long/short (perp), lend/borrow (lend).
    pub(crate) side: String,
    /// Base asset symbol, such as WETH.
    pub(crate) base_symbol: String,
    /// Quote asset symbol. Required for spot and perp; defaults to USDT for lend.
    #[serde(default)]
    pub(crate) quote_symbol: Option<String>,
    /// Human-readable base quantity.
    pub(crate) quantity: String,
    /// Limit price (spot/perp) or interest rate (lend). Omit for a market/IOC order.
    #[serde(default)]
    pub(crate) price: Option<String>,
    /// `limit` or `market`. Inferred from price when omitted.
    #[serde(default)]
    pub(crate) order_type: Option<String>,
    /// Slippage decimal for market orders, e.g. "0.005" for 0.5%.
    #[serde(default)]
    pub(crate) slippage: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CancelWorldOrderArgs {
    /// Product type: spot, perp, or lend.
    pub(crate) product: String,
    /// Side of the resting order to cancel.
    pub(crate) side: String,
    pub(crate) base_symbol: String,
    #[serde(default)]
    pub(crate) quote_symbol: Option<String>,
    /// Resting order id (spot/perp).
    #[serde(default)]
    pub(crate) order_id: Option<String>,
    /// Interest rate of the resting lend/borrow order. Required for lend cancels.
    #[serde(default)]
    pub(crate) interest_rate: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ExecuteWorldSwapArgs {
    /// Symbol to sell, such as USDT.
    pub(crate) token_in_symbol: String,
    /// Symbol to buy, such as WETH.
    pub(crate) token_out_symbol: String,
    /// Human-readable input amount.
    pub(crate) amount_in: String,
    #[serde(default)]
    pub(crate) slippage: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RenewWorldLoansArgs {
    /// Extend borrower loans due within this many hours. Defaults to 24.
    #[serde(default)]
    pub(crate) within_hours: Option<u64>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PayWorldLoanInterestArgs {
    /// Optional base symbol to restrict which borrower loans are paid.
    #[serde(default)]
    pub(crate) base_symbol: Option<String>,
    /// When true, extend the period (same as a renewal). Defaults to false (pay dues only).
    #[serde(default)]
    pub(crate) extend_period: Option<bool>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CloseWorldLoanArgs {
    /// Optional base symbol to restrict which borrower loans are closed.
    #[serde(default)]
    pub(crate) base_symbol: Option<String>,
    /// Optional on-chain position id. When omitted, close matching borrower loans.
    #[serde(default)]
    pub(crate) position_id: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

pub(crate) struct GetWorldAgentPermission;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldAgentPermissionArgs {
    /// World account ID. Handover account context is used when omitted.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Actor address to inspect. The active EVM actor is used when omitted.
    #[serde(default)]
    pub(crate) actor_address: Option<String>,
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

impl WorldMarketsApp {
    fn account_id(ctx: &DynToolCallCtx, explicit: Option<u64>) -> Option<u64> {
        explicit
            .or_else(|| ctx.attribute_u64(&["world", "account_id"]))
            .or_else(|| value_u64(ctx.attribute_path(&["handover_account_id"])))
            .or_else(|| value_u64(ctx.attribute_path(&["platform_account_ref"])))
            .or_else(|| value_u64(ctx.attribute_path(&["handover_account_ref"])))
            .or_else(|| ctx.attribute_u64(&["handover_mandate", "account", "id"]))
            // Final fallback: a session-persistent account id supplied via the
            // environment (WORLD_ACCOUNT_ID). Every runtime handover path above
            // wins over it, so a live handover is never overridden; this only
            // fills the gap in dev/CLI mode, where the runtime stubs all state
            // attributes to None and the account id would otherwise have to be
            // re-supplied by the model on every single tool call.
            .or_else(Self::account_id_from_env)
    }

    /// Parse a session-persistent account id from the `WORLD_ACCOUNT_ID`
    /// environment variable. Mirrors the `WORLD_RPC_URL` / `WORLD_EXCHANGE_ADDRESS`
    /// override pattern in `client.rs`. Accepts a bare integer or the same
    /// `world-<id>` prefixed form the handover reference paths accept.
    fn account_id_from_env() -> Option<u64> {
        let raw = std::env::var("WORLD_ACCOUNT_ID").ok()?;
        value_u64(Some(&Value::String(raw)))
    }

    fn brief(ctx: &DynToolCallCtx) -> Option<Value> {
        ctx.attribute_path(&["handover_brief"])
            .or_else(|| ctx.attribute_path(&["brief"]))
            .or_else(|| ctx.attribute_path(&["handover_mandate", "brief"]))
            .cloned()
    }

    fn access(
        &self,
        account_id: Option<u64>,
        wallet_address: Option<&str>,
        ctx: &DynToolCallCtx,
    ) -> Result<AccountAccess, String> {
        let account_id = Self::account_id(ctx, account_id);
        let owner_wallet = wallet_address
            .map(ToString::to_string)
            .or_else(|| ctx.attribute_string(&["world", "owner_wallet"]));
        let actor = ctx.attribute_string(&["domain", "evm", "address"]);
        self.client
            .resolve_account(account_id, owner_wallet.as_deref(), actor.as_deref())
    }

    fn snapshot_effect_plan(
        &self,
        args: PreviewAccountEffectArgs,
        ctx: &DynToolCallCtx,
    ) -> Result<EffectPlan, String> {
        let product = normalize_effect_product(&args.product)?;
        let side = args.side.to_ascii_lowercase();
        if !matches!(
            side.as_str(),
            "buy" | "sell" | "long" | "short" | "lend" | "borrow"
        ) {
            return Err(
                "[world-markets] side must be buy/sell, long/short, or lend/borrow".to_string(),
            );
        }
        let quantity = parse_decimal(&args.quantity, "quantity")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        if quantity <= Decimal::ZERO {
            return Err("[world-markets] quantity must be greater than zero".to_string());
        }

        let access = self.access(args.account_id, args.wallet_address.as_deref(), ctx)?;
        let assets = self.client.assets()?;
        let account = self.client.account(access.account_id, &assets)?;
        let block = self.client.block_number()?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = asset_by_symbol(&assets, &args.quote_symbol)?;

        let mut missing_mark_symbols = Vec::new();
        let mark = match self.client.mark_price(base.token_id) {
            Ok((_, price)) => parse_decimal(&price, "mark_price").ok(),
            Err(_) => None,
        };
        if mark.is_none() {
            missing_mark_symbols.push(base.symbol.clone());
        }

        let current_qty =
            current_position(&account, product, &base.symbol).unwrap_or(Decimal::ZERO);
        let signed_delta = if matches!(side.as_str(), "buy" | "long" | "borrow") {
            quantity
        } else {
            -quantity
        };
        let after_qty = current_qty + signed_delta;

        let (exposure_before, exposure_after) = match mark {
            Some(price) => (current_qty.abs() * price, after_qty.abs() * price),
            None => (Decimal::ZERO, Decimal::ZERO),
        };

        let available_before = quote_available(&account, &quote.symbol)?;
        let available_after = available_before + (exposure_before - exposure_after);

        let metrics =
            crate::liquidation_risk::compute_metrics(&self.client, &account, &assets, block).ok();
        let liquidation_risk_before = metrics
            .as_ref()
            .and_then(|m| parse_decimal(&m.liquidation_risk, "liquidation_risk").ok());

        let projection_mark =
            mark.or_else(|| matches!(product, "lend" | "lending").then_some(Decimal::ONE));
        let projected = projection_mark.and_then(|price| {
            crate::liquidation_risk::project_post_trade(
                &self.client,
                &account,
                &assets,
                &crate::liquidation_risk::TradeIntent {
                    product,
                    side: &side,
                    base: &base,
                    quote: &quote,
                    quantity,
                    mark: price,
                },
            )
            .ok()
        });
        let others = other_directional_legs(&account, &base.symbol);
        let concern_clause = concern_clause(&base.symbol, current_qty, after_qty, &others);

        Ok(EffectPlan {
            exposure_symbol: base.symbol,
            exposure_before,
            exposure_after,
            available_before,
            available_after,
            quote: quote.symbol,
            liquidation_risk_before,
            liquidation_risk_after: projected.as_ref().map(|p| p.liquidation_risk),
            estimated_cost: None,
            missing_mark_symbols,
            post_trade_risk_unavailable: projected.is_none(),
            concern_clause,
            baseline: format!(
                "live account snapshot at block {block} versus this intent — derived, not model-typed"
            ),
        })
    }

    fn trade_preview(&self, args: WorldTradeArgs, ctx: &DynToolCallCtx) -> Result<Value, String> {
        let product = normalize_product(&args.product)?;
        let side = args.side.to_ascii_lowercase();
        if !matches!(side.as_str(), "buy" | "sell") {
            return Err("[world-markets] side must be buy or sell".to_string());
        }
        let quantity = parse_decimal(&args.quantity, "quantity")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        if quantity <= Decimal::ZERO {
            return Err("[world-markets] quantity must be greater than zero".to_string());
        }

        let access = self.access(args.account_id, args.wallet_address.as_deref(), ctx)?;
        let assets = self.client.assets()?;
        let account = self.client.account(access.account_id, &assets)?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = asset_by_symbol(&assets, &args.quote_symbol)?;
        let market = self
            .client
            .market(product, base.clone(), Some(quote.clone()))?;
        let mark_price = parse_decimal(&market.mark_price, "mark_price")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let current_position_quantity = current_position(&account, product, &base.symbol)?;
        let rapv = parse_decimal(
            &account.risk_adjusted_portfolio_value,
            "risk_adjusted_portfolio_value",
        )
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let projected = crate::liquidation_risk::project_post_trade(
            &self.client,
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
        .ok();
        let mandate = Mandate::bound(ctx.attribute_path(&["handover_mandate"]));
        let verdict = match mandate {
            Ok(mandate) => mandate.evaluate(&TradeFacts {
                product,
                side: &side,
                base: &base.symbol,
                quote: &quote.symbol,
                quantity,
                mark_price,
                current_position_quantity,
                risk_adjusted_portfolio_value: rapv,
                post_trade_risk_adjusted_portfolio_value: projected.as_ref().map(|p| p.rapv),
                eligible_for_liquidation: account.eligible_for_liquidation,
            }),
            Err(verdict) => verdict,
        };
        let estimated_notional = quantity.checked_mul(mark_price).ok_or_else(|| {
            "[world-markets] estimated notional exceeds numeric range".to_string()
        })?;
        let status = if verdict.is_allow() {
            "policy_allowed"
        } else {
            "policy_denied"
        };
        let reason = if verdict.is_allow() {
            "The deterministic mandate permits this intent. Submit with execute_world_order to send it through the local execution sidecar."
        } else {
            "The deterministic World mandate denied this intent; do not construct or stage a transaction."
        };

        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": self.client.exchange(),
            "block_number": self.client.block_number()?,
            "standing_brief": Self::brief(ctx),
            "access": access,
            "preview": {
                "account_id": account.account_id,
                "owner": account.owner,
                "product": product,
                "side": side,
                "base_symbol": base.symbol,
                "quote_symbol": quote.symbol,
                "quantity": args.quantity,
                "current_position_quantity": current_position_quantity.to_string(),
                "mark_price": market.mark_price,
                "estimated_notional": estimated_notional.to_string(),
                "order_book": market.book,
                "pre_execution_risk_adjusted_portfolio_value": account.risk_adjusted_portfolio_value,
                "post_trade_risk_adjusted_portfolio_value": projected.as_ref().map(|p| p.rapv_display.clone()),
                "post_trade_risk_is_estimate": projected.as_ref().map(|p| p.is_estimate),
                "post_trade_risk_source": projected.as_ref().map(|p| p.source),
                "pre_execution_eligible_for_liquidation": account.eligible_for_liquidation,
                "policy_result": verdict,
                "executable": false,
                "status": status,
                "reason": reason,
            }
        }))
    }

    fn live_verdict(
        &self,
        input: LiveVerdictInput<'_>,
        ctx: &DynToolCallCtx,
    ) -> Result<(crate::client::Market, Decimal, Verdict), String> {
        let quote_for_book = (input.product != "lend").then(|| input.quote.clone());
        let market = self
            .client
            .market(input.product, input.base.clone(), quote_for_book)?;
        let mark_price = parse_decimal(&market.mark_price, "mark_price")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let current_position_quantity =
            current_position(input.account, input.product, &input.base.symbol)?;
        let rapv = parse_decimal(
            &input.account.risk_adjusted_portfolio_value,
            "risk_adjusted_portfolio_value",
        )
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let mandate = Mandate::bound(ctx.attribute_path(&["handover_mandate"]));
        let verdict = match mandate {
            Ok(mandate) => mandate.evaluate(&TradeFacts {
                product: input.product,
                side: input.side,
                base: &input.base.symbol,
                quote: &input.quote.symbol,
                quantity: input.quantity,
                mark_price,
                current_position_quantity,
                risk_adjusted_portfolio_value: rapv,
                post_trade_risk_adjusted_portfolio_value: Some(rapv),
                eligible_for_liquidation: input.account.eligible_for_liquidation,
            }),
            Err(verdict) => verdict,
        };
        Ok((market, mark_price, verdict))
    }

    fn loan_execution_prep(
        &self,
        account_id: Option<u64>,
        wallet_address: Option<&str>,
        base_symbol: Option<&str>,
        ctx: &DynToolCallCtx,
    ) -> Result<(AccountAccess, Vec<u32>), String> {
        let access = self.access(account_id, wallet_address, ctx)?;
        let mandate = Mandate::bound(ctx.attribute_path(&["handover_mandate"]))
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let assets = self.client.assets()?;
        let account = self.client.account(access.account_id, &assets)?;
        let rapv = parse_decimal(
            &account.risk_adjusted_portfolio_value,
            "risk_adjusted_portfolio_value",
        )
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        if account.eligible_for_liquidation && mandate.halt_if_eligible_for_liquidation {
            return Err(
                "[world-markets] liquidatable: The live World account is eligible for liquidation and this mandate requires a halt."
                    .to_string(),
            );
        }
        let floor = parse_decimal(&mandate.min_risk_adjusted_portfolio_value.amount, "floor")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        if rapv < floor {
            return Err(format!(
                "[world-markets] portfolio_floor: Live risk-adjusted portfolio value {rapv} is below the mandate floor {floor}."
            ));
        }
        let wanted = base_symbol.map(|symbol| symbol.to_ascii_uppercase());
        let token_ids = account
            .lending_positions
            .iter()
            .filter(|position| position.borrower_quantity_raw > 0)
            .filter(|position| {
                wanted
                    .as_ref()
                    .is_none_or(|symbol| position.symbol.eq_ignore_ascii_case(symbol))
            })
            .map(|position| position.token_id)
            .collect::<Vec<_>>();
        Ok((access, token_ids))
    }
}

impl DynAomiTool for ListWorldAssets {
    type App = WorldMarketsApp;
    type Args = ListWorldAssetsArgs;
    const NAME: &'static str = "list_world_assets";
    const DESCRIPTION: &'static str = "List live World Markets assets and their token IDs, symbols, addresses, decimals, and risk parameters.";

    fn run(
        app: &WorldMarketsApp,
        _args: Self::Args,
        _ctx: DynToolCallCtx,
    ) -> Result<Value, String> {
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": app.client.block_number()?,
            "assets": app.client.assets()?,
        }))
    }
}

impl DynAomiTool for GetWorldAccount {
    type App = WorldMarketsApp;
    type Args = GetWorldAccountArgs;
    const NAME: &'static str = "get_world_account";
    const DESCRIPTION: &'static str = "Inspect a live World account after proving that the active actor is its owner or an on-chain permitted trader.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let assets = app.client.assets()?;
        let account = app.client.account(access.account_id, &assets)?;
        let block_number = app.client.block_number()?;
        let metrics =
            crate::liquidation_risk::compute_metrics(&app.client, &account, &assets, block_number)?;
        let lookups = crate::lookups::compute_lookups(
            &app.client,
            &account,
            &assets,
            &metrics.net_asset_value,
            block_number,
        )?;
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": block_number,
            "standing_brief": WorldMarketsApp::brief(&ctx),
            "access": access,
            "account": account,
            "metrics": metrics,
            "lookups": lookups,
        }))
    }
}

impl DynAomiTool for GetWorldMarket {
    type App = WorldMarketsApp;
    type Args = GetWorldMarketArgs;
    const NAME: &'static str = "get_world_market";
    const DESCRIPTION: &'static str = "Resolve a live World spot, perpetual, or lending order book and the current configured mark price.";

    fn run(app: &WorldMarketsApp, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let assets = app.client.assets()?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = args
            .quote_symbol
            .as_deref()
            .map(|symbol| asset_by_symbol(&assets, symbol))
            .transpose()?;
        let product = args.product.to_ascii_lowercase();
        let market = app.client.market(&product, base, quote)?;
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": app.client.block_number()?,
            "market": market,
        }))
    }
}

impl DynAomiTool for PreviewWorldTrade {
    type App = WorldMarketsApp;
    type Args = WorldTradeArgs;
    const NAME: &'static str = "preview_world_trade";
    const DESCRIPTION: &'static str = "Preview a World spot or perpetual intent from live state and return the deterministic mandate verdict. It never stages or executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        app.trade_preview(args, &ctx)
    }
}

impl DynAomiTool for CheckWorldMandate {
    type App = WorldMarketsApp;
    type Args = WorldTradeArgs;
    const NAME: &'static str = "check_world_mandate";
    const DESCRIPTION: &'static str = "Evaluate one structured World trade intent against the bound mandate and live account/market state. Returns the exact allow or deny rule; it does not execute.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let preview = app.trade_preview(args, &ctx)?;
        Ok(json!({
            "source": preview.get("source"),
            "chain_id": preview.get("chain_id"),
            "block_number": preview.get("block_number"),
            "access": preview.get("access"),
            "standing_brief": preview.get("standing_brief"),
            "intent": preview.pointer("/preview").map(|value| json!({
                "account_id": value.get("account_id"),
                "product": value.get("product"),
                "side": value.get("side"),
                "base_symbol": value.get("base_symbol"),
                "quote_symbol": value.get("quote_symbol"),
                "quantity": value.get("quantity"),
                "mark_price": value.get("mark_price"),
                "estimated_notional": value.get("estimated_notional"),
            })),
            "policy_result": preview.pointer("/preview/policy_result"),
            "executable": false,
        }))
    }
}

impl DynAomiTool for ExecuteWorldOrder {
    type App = WorldMarketsApp;
    type Args = ExecuteWorldOrderArgs;
    const NAME: &'static str = "execute_world_order";
    const DESCRIPTION: &'static str = "Place a World spot, perp, or lend/borrow order through the local execution sidecar after the mandate allows. Limit if price is set, otherwise market/IOC. Never withdraws.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let product = normalize_execute_product(&args.product)?;
        let side = normalize_execute_side(product, &args.side)?;
        let quantity = parse_decimal(&args.quantity, "quantity")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        if quantity <= Decimal::ZERO {
            return Err("[world-markets] quantity must be greater than zero".to_string());
        }
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let assets = app.client.assets()?;
        let account = app.client.account(access.account_id, &assets)?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote_symbol = args
            .quote_symbol
            .clone()
            .unwrap_or_else(|| "USDT".to_string());
        let quote = asset_by_symbol(&assets, &quote_symbol)?;
        let (market, _, verdict) = app.live_verdict(
            LiveVerdictInput {
                product,
                side: &side,
                base: &base,
                quote: &quote,
                quantity,
                account: &account,
            },
            &ctx,
        )?;
        if !verdict.is_allow() {
            return Ok(execution_blocked(&access, &verdict));
        }
        let order_type = resolve_order_type(args.order_type.as_deref(), args.price.as_deref());
        let receipt = app.execution.place_order(&PlaceOrderRequest {
            account_id: access.account_id,
            product: product.to_string(),
            side: side.clone(),
            base_token_id: base.token_id,
            quote_token_id: (product != "lend").then_some(quote.token_id),
            quantity: args.quantity.clone(),
            price: args.price.clone(),
            order_type,
            slippage: args.slippage.clone(),
        })?;
        Ok(execution_ok(
            &access,
            &verdict,
            receipt,
            json!({
                "product": product,
                "side": side,
                "base_symbol": base.symbol,
                "quote_symbol": quote.symbol,
                "quantity": args.quantity,
                "order_book": market.book,
            }),
        ))
    }
}

impl DynAomiTool for CancelWorldOrder {
    type App = WorldMarketsApp;
    type Args = CancelWorldOrderArgs;
    const NAME: &'static str = "cancel_world_order";
    const DESCRIPTION: &'static str = "Cancel a resting World order through the local execution sidecar. Requires a bound mandate and a live trader grant. Spot/perp need order_id; lend/borrow need interest_rate.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let product = normalize_execute_product(&args.product)?;
        let side = normalize_execute_side(product, &args.side)?;
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        Mandate::bound(ctx.attribute_path(&["handover_mandate"]))
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let assets = app.client.assets()?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = match args.quote_symbol.as_deref() {
            Some(symbol) => Some(asset_by_symbol(&assets, symbol)?),
            None if product == "lend" => None,
            None => {
                return Err(
                    "[world-markets] quote_symbol is required to cancel a spot or perp order"
                        .to_string(),
                );
            }
        };
        if product != "lend" && args.order_id.is_none() {
            return Err(
                "[world-markets] order_id is required to cancel a spot or perp order".to_string(),
            );
        }
        if product == "lend" && args.interest_rate.is_none() {
            return Err(
                "[world-markets] interest_rate is required to cancel a lend or borrow order"
                    .to_string(),
            );
        }
        let receipt = app.execution.cancel_order(&CancelOrderRequest {
            account_id: access.account_id,
            product: product.to_string(),
            side,
            base_token_id: base.token_id,
            quote_token_id: quote.as_ref().map(|asset| asset.token_id),
            order_id: args.order_id.clone(),
            price: args.interest_rate.clone(),
            interest_rate: args.interest_rate.clone(),
        })?;
        Ok(json!({
            "source": "world-markets-execution",
            "executable": true,
            "access": access,
            "receipt": receipt,
        }))
    }
}

impl DynAomiTool for ExecuteWorldSwap {
    type App = WorldMarketsApp;
    type Args = ExecuteWorldSwapArgs;
    const NAME: &'static str = "execute_world_swap";
    const DESCRIPTION: &'static str = "Swap two World assets through the local execution sidecar (SwapAggregator) after the mandate allows the equivalent spot intent.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let amount_in = parse_decimal(&args.amount_in, "amount_in")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        if amount_in <= Decimal::ZERO {
            return Err("[world-markets] amount_in must be greater than zero".to_string());
        }
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let assets = app.client.assets()?;
        let account = app.client.account(access.account_id, &assets)?;
        let token_in = asset_by_symbol(&assets, &args.token_in_symbol)?;
        let token_out = asset_by_symbol(&assets, &args.token_out_symbol)?;
        let usdt_in = token_in.symbol.eq_ignore_ascii_case("USDT");
        let usdt_out = token_out.symbol.eq_ignore_ascii_case("USDT");
        if !usdt_in && !usdt_out {
            return Err(
                "[world-markets] local swaps must include USDT so the mandate quote matches"
                    .to_string(),
            );
        }
        let (side, base, quote) = if usdt_in {
            ("buy".to_string(), token_out.clone(), token_in.clone())
        } else {
            ("sell".to_string(), token_in.clone(), token_out.clone())
        };
        let market = app
            .client
            .market("spot", base.clone(), Some(quote.clone()))?;
        let mark = parse_decimal(&market.mark_price, "mark_price")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let quantity = if usdt_in && !mark.is_zero() {
            amount_in
                .checked_div(mark)
                .ok_or_else(|| "[world-markets] swap quantity exceeds numeric range".to_string())?
        } else {
            amount_in
        };
        let (_, _, verdict) = app.live_verdict(
            LiveVerdictInput {
                product: "spot",
                side: &side,
                base: &base,
                quote: &quote,
                quantity,
                account: &account,
            },
            &ctx,
        )?;
        if !verdict.is_allow() {
            return Ok(execution_blocked(&access, &verdict));
        }
        let receipt = app.execution.swap(&SwapRequest {
            account_id: access.account_id,
            token_in: token_in.erc20_address.clone(),
            token_out: token_out.erc20_address.clone(),
            amount_in: args.amount_in.clone(),
            slippage: args.slippage.clone(),
        })?;
        Ok(execution_ok(
            &access,
            &verdict,
            receipt,
            json!({
                "token_in": token_in.symbol,
                "token_out": token_out.symbol,
                "amount_in": args.amount_in,
            }),
        ))
    }
}

impl DynAomiTool for RenewWorldLoans {
    type App = WorldMarketsApp;
    type Args = RenewWorldLoansArgs;
    const NAME: &'static str = "renew_world_loans";
    const DESCRIPTION: &'static str = "Extend borrower loans that are due or within the given hour window, via the local execution sidecar. Requires a bound mandate and a live trader grant. Routine renewals are silent in chat.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let mandate = Mandate::bound(ctx.attribute_path(&["handover_mandate"]))
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let assets = app.client.assets()?;
        let account = app.client.account(access.account_id, &assets)?;
        let rapv = parse_decimal(
            &account.risk_adjusted_portfolio_value,
            "risk_adjusted_portfolio_value",
        )
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        if account.eligible_for_liquidation && mandate.halt_if_eligible_for_liquidation {
            return Ok(execution_blocked(
                &access,
                &Verdict {
                    status: "deny",
                    rule: "liquidatable",
                    detail: "The live World account is eligible for liquidation and this mandate requires a halt.".to_string(),
                },
            ));
        }
        let floor = parse_decimal(&mandate.min_risk_adjusted_portfolio_value.amount, "floor")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        if rapv < floor {
            return Ok(execution_blocked(
                &access,
                &Verdict {
                    status: "deny",
                    rule: "portfolio_floor",
                    detail: format!(
                        "Live risk-adjusted portfolio value {rapv} is below the mandate floor {floor}."
                    ),
                },
            ));
        }
        let token_ids = account
            .lending_positions
            .iter()
            .filter(|position| position.borrower_quantity_raw > 0)
            .map(|position| position.token_id)
            .collect::<Vec<_>>();
        let receipt = app.execution.renew_loans(&RenewLoansRequest {
            account_id: access.account_id,
            token_ids,
            max_hours_remaining: args.within_hours,
        })?;
        Ok(json!({
            "source": "world-markets-execution",
            "executable": true,
            "access": access,
            "receipt": receipt,
        }))
    }
}

impl DynAomiTool for PayWorldLoanInterest {
    type App = WorldMarketsApp;
    type Args = PayWorldLoanInterestArgs;
    const NAME: &'static str = "pay_world_loan_interest";
    const DESCRIPTION: &'static str = "Pay interest and fees on live borrower loans through the local execution sidecar. Does not extend the term unless extend_period is true. Requires a bound mandate.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let (access, token_ids) = app.loan_execution_prep(
            args.account_id,
            args.wallet_address.as_deref(),
            args.base_symbol.as_deref(),
            &ctx,
        )?;
        let receipt = app.execution.pay_interest(&PayInterestRequest {
            account_id: access.account_id,
            token_ids,
            position_id: None,
            extend_period: args.extend_period,
        })?;
        Ok(json!({
            "source": "world-markets-execution",
            "executable": true,
            "access": access,
            "receipt": receipt,
        }))
    }
}

impl DynAomiTool for CloseWorldLoan {
    type App = WorldMarketsApp;
    type Args = CloseWorldLoanArgs;
    const NAME: &'static str = "close_world_loan";
    const DESCRIPTION: &'static str = "Close borrower loans and pay remaining interest through the local execution sidecar. Requires a bound mandate. Pass position_id to close one loan, or a base_symbol to close matching borrows.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let (access, token_ids) = app.loan_execution_prep(
            args.account_id,
            args.wallet_address.as_deref(),
            args.base_symbol.as_deref(),
            &ctx,
        )?;
        let receipt = app.execution.close_loan(&CloseLoanRequest {
            account_id: access.account_id,
            token_ids,
            position_id: numeric_position_id(args.position_id.as_deref()),
        })?;
        Ok(json!({
            "source": "world-markets-execution",
            "executable": true,
            "access": access,
            "receipt": receipt,
        }))
    }
}

impl DynAomiTool for GetWorldAgentPermission {
    type App = WorldMarketsApp;
    type Args = GetWorldAgentPermissionArgs;
    const NAME: &'static str = "get_world_agent_permission";
    const DESCRIPTION: &'static str = "Read the World account owner and permitted-trader list to determine whether the active agent grant is live or revoked.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id).ok_or_else(|| {
            "[world-markets] no World account id is available for the permission check".to_string()
        })?;
        let actor = args
            .actor_address
            .or_else(|| ctx.attribute_string(&["domain", "evm", "address"]))
            .ok_or_else(|| {
                "[world-markets] no active EVM actor is available for the permission check"
                    .to_string()
            })?;
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": app.client.block_number()?,
            "permission": app.client.agent_permission(account_id, &actor)?,
        }))
    }
}

impl DynAomiTool for GetWorldOpenOrders {
    type App = WorldMarketsApp;
    type Args = GetWorldOpenOrdersArgs;
    const NAME: &'static str = "get_world_open_orders";
    const DESCRIPTION: &'static str = "Read the authorized World account's resting buy and sell orders for one live spot or perpetual market.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let product = normalize_product(&args.product)?;
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let assets = app.client.assets()?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = asset_by_symbol(&assets, &args.quote_symbol)?;
        let market = app.client.market(product, base, Some(quote))?;
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": app.client.block_number()?,
            "access": access,
            "open_orders": app.client.open_orders(&market, access.account_id)?,
        }))
    }
}

impl DynAomiTool for GetWorldPnl {
    type App = WorldMarketsApp;
    type Args = GetWorldPnlArgs;
    const NAME: &'static str = "get_world_pnl";
    const DESCRIPTION: &'static str = "Compute account-level and per-position perpetual PnL. Open PnL is mark versus contract entry minus unpaid funding. Position PnL covers that position's lifetime (open to now, or open to close). Realized figures are captured when this app observes a true-up or close. Not a calendar-range report; never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let assets = app.client.assets()?;
        let account = app.client.account(access.account_id, &assets)?;
        let pnl = crate::pnl::report(
            &app.client,
            &app.pnl_ledger,
            &account,
            args.position.as_deref(),
        )?;
        Ok(json!({
            "source": "world-markets-reporting",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": app.client.block_number()?,
            "access": access,
            "executable": false,
            "pnl": pnl,
        }))
    }
}

// ============================================================================
// Reporting-service tools (the honest-numbers layer).
//
// Each tool returns DERIVED figures computed deterministically in Rust. The
// message layer may state a number only if it appears in one of these results
// (or in a live contract read). See TELEGRAM-MESSAGING-UX-SPEC §4.1 and §11.
// All numeric arguments are decimal strings so no f64 rounding enters a receipt.
// Every result carries `source` and `executable: false`.
// ============================================================================

/// Parse a decimal string tool argument, surfacing a clear error to the model.
fn report_decimal(value: &str, field: &'static str) -> Result<Decimal, String> {
    parse_decimal(value, field)
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))
}

pub(crate) struct PreviewAccountEffect;

/// Intent-only. Same shape as `WorldTradeArgs` plus lend. No figure fields —
/// before/after numbers are derived from live state in Rust.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PreviewAccountEffectArgs {
    /// Product type: spot, perp, or lend.
    pub(crate) product: String,
    /// Trade side: buy or sell.
    pub(crate) side: String,
    /// Base asset symbol.
    pub(crate) base_symbol: String,
    /// Quote asset symbol.
    pub(crate) quote_symbol: String,
    /// Human-readable base quantity, such as "0.25".
    pub(crate) quantity: String,
    /// Optional World account ID. Handover account context is used when omitted.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Optional expected owner wallet.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

impl DynAomiTool for PreviewAccountEffect {
    type App = WorldMarketsApp;
    type Args = PreviewAccountEffectArgs;
    const NAME: &'static str = "preview_account_effect";
    const DESCRIPTION: &'static str = "Snapshot live account state and apply this intent through the same post-trade path the mandate uses. Returns before/after exposure, available-to-deploy, 0–10 liquidation risk (omitted when unprovable), and cost. Pass only the intent — never figures. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let plan = app.snapshot_effect_plan(args, &ctx)?;
        Ok(json!({
            "source": "world-markets-reporting",
            "account_effect": app.reporting.account_effect(&plan),
            "executable": false,
        }))
    }
}

pub(crate) struct ComputeResize;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ComputeResizeArgs {
    /// The engine `rule` code that gated the intent, verbatim.
    pub(crate) rule: String,
}

impl DynAomiTool for ComputeResize {
    type App = WorldMarketsApp;
    type Args = ComputeResizeArgs;
    const NAME: &'static str = "compute_resize";
    const DESCRIPTION: &'static str = "For a blocked intent, return the user's RAPV floor from the signed mandate. A block cites exactly one number: the floor. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let mandate = Mandate::bound(ctx.attribute_path(&["handover_mandate"]))
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let floor = parse_decimal(&mandate.min_risk_adjusted_portfolio_value.amount, "floor")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let input = ResizeInput {
            floor,
            largest_compliant_size: None,
            quote: mandate.min_risk_adjusted_portfolio_value.quote.clone(),
            rule: args.rule,
        };
        Ok(json!({
            "source": "world-markets-reporting",
            "resize": app.reporting.resize_solution(&input),
            "executable": false,
        }))
    }
}

pub(crate) struct PreviewExit;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PreviewExitArgs {
    /// Position identifier to price an exit for.
    pub(crate) position_id: String,
}

impl DynAomiTool for PreviewExit {
    type App = WorldMarketsApp;
    type Args = PreviewExitArgs;
    const NAME: &'static str = "preview_exit";
    const DESCRIPTION: &'static str = "Price closing a position before entry is possible: price impact, p90 time-to-flat, and the net-of-everything result. Estimate against the live book; never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        Ok(json!({
            "source": "world-markets-reporting",
            "exit_cost": app.reporting.exit_cost(&args.position_id),
            "executable": false,
        }))
    }
}

pub(crate) struct PlanLargeOrder;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PlanLargeOrderArgs {
    /// Estimated cost of one market order, in quote units.
    pub(crate) market_order_cost: String,
    /// Estimated cost of the sliced plan, in quote units.
    pub(crate) sliced_cost: String,
    /// Number of slices in the plan.
    pub(crate) slices: u32,
    /// Total window for the plan, minutes.
    pub(crate) window_minutes: u32,
    /// Quote symbol.
    pub(crate) quote_symbol: String,
    /// The baseline the saving is measured against, one sentence.
    pub(crate) baseline: String,
}

impl DynAomiTool for PlanLargeOrder {
    type App = WorldMarketsApp;
    type Args = PlanLargeOrderArgs;
    const NAME: &'static str = "plan_large_order";
    const DESCRIPTION: &'static str = "Compare a single market order against a sliced plan and return the money saved, or a plain $0 when slicing wouldn't help at this size. Estimate; never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let input = SliceInput {
            market_order_cost: report_decimal(&args.market_order_cost, "market_order_cost")?,
            sliced_cost: report_decimal(&args.sliced_cost, "sliced_cost")?,
            slices: args.slices,
            window_minutes: args.window_minutes,
            quote: args.quote_symbol,
            baseline: args.baseline,
        };
        Ok(json!({
            "source": "world-markets-reporting",
            "slice_plan": app.reporting.slice_plan(&input),
            "executable": false,
        }))
    }
}

pub(crate) struct GetDollarpower;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetDollarpowerArgs {
    /// Portfolio identifier. Handover account context is used when omitted.
    #[serde(default)]
    pub(crate) portfolio_id: Option<String>,
}

impl DynAomiTool for GetDollarpower {
    type App = WorldMarketsApp;
    type Args = GetDollarpowerArgs;
    const NAME: &'static str = "get_dollarpower";
    const DESCRIPTION: &'static str = "Return capital efficiency (dollarpower) as a ratio plus its dollar translation (committed vs effective). A status figure, never a headline; never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let portfolio_id = args
            .portfolio_id
            .or_else(|| WorldMarketsApp::account_id(&ctx, None).map(|id| id.to_string()))
            .unwrap_or_default();
        Ok(json!({
            "source": "world-markets-reporting",
            "dollarpower": app.reporting.dollarpower(&portfolio_id),
            "executable": false,
        }))
    }
}

pub(crate) struct SimulateGuardianUnwind;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GuardianCandidateArg {
    /// Human label for the leg, e.g. "close 0.4 ETH short".
    pub(crate) label: String,
    /// Risk-score points recovered if this leg fully closes (engine units).
    pub(crate) delta_score: String,
    /// Slippage + fees (+ accrued interest for a loan leg) to fully close.
    pub(crate) exit_cost: String,
    /// True when closing this leg leaves a worse residual (e.g. breaks a hedge).
    #[serde(default)]
    pub(crate) breaks_structure_into_worse_residual: bool,
    /// True when this leg reduces directional exposure.
    #[serde(default)]
    pub(crate) reduces_directional_exposure: bool,
    /// True when this holding is protected by policy (a veto, never a candidate).
    #[serde(default)]
    pub(crate) protected: bool,
    /// True when closing this leg touches ETH.
    #[serde(default)]
    pub(crate) is_eth: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SimulateGuardianUnwindArgs {
    /// Candidate legs the guardian may close.
    pub(crate) candidates: Vec<GuardianCandidateArg>,
    /// Current risk score (engine units).
    pub(crate) current_score: String,
    /// Recovery target to reach (engine units).
    pub(crate) recovery_target: String,
    /// Standing preference: "cheapest_safe" (default) or "protect_eth".
    #[serde(default)]
    pub(crate) preference: Option<String>,
    /// Whether the emergency slippage limit can be met at required size.
    #[serde(default = "default_true")]
    pub(crate) emergency_slippage_reachable: bool,
}

fn default_true() -> bool {
    true
}

impl DynAomiTool for SimulateGuardianUnwind {
    type App = WorldMarketsApp;
    type Args = SimulateGuardianUnwindArgs;
    const NAME: &'static str = "simulate_guardian_unwind";
    const DESCRIPTION: &'static str = "Run the cheapest-safe unwind algorithm over candidate legs and return the chosen order, per-step recovery and cost, total cost, and what a protection preference kept. For fire drills and guardian reports; never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let preference = match args.preference.as_deref() {
            None | Some("cheapest_safe") => GuardianPreference::CheapestSafe,
            Some("protect_eth") => GuardianPreference::ProtectEth,
            Some(other) => {
                return Err(format!(
                    "[world-markets] unknown guardian preference {other:?}; use cheapest_safe or protect_eth"
                ));
            }
        };
        let mut candidates = Vec::with_capacity(args.candidates.len());
        for candidate in &args.candidates {
            candidates.push(UnwindCandidate {
                label: candidate.label.clone(),
                delta_score: report_decimal(&candidate.delta_score, "delta_score")?,
                exit_cost: report_decimal(&candidate.exit_cost, "exit_cost")?,
                breaks_structure_into_worse_residual: candidate
                    .breaks_structure_into_worse_residual,
                reduces_directional_exposure: candidate.reduces_directional_exposure,
                protected: candidate.protected,
                is_eth: candidate.is_eth,
            });
        }
        let plan = app.reporting.guardian_unwind(
            &candidates,
            report_decimal(&args.current_score, "current_score")?,
            report_decimal(&args.recovery_target, "recovery_target")?,
            preference,
            args.emergency_slippage_reachable,
        );
        Ok(json!({
            "source": "world-markets-reporting",
            "unwind_plan": plan,
            "executable": false,
        }))
    }
}

pub(crate) struct CheckNegativeCarry;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CheckNegativeCarryArgs {
    /// Position identifier to inspect.
    pub(crate) position_id: String,
}

impl DynAomiTool for CheckNegativeCarry {
    type App = WorldMarketsApp;
    type Args = CheckNegativeCarryArgs;
    const NAME: &'static str = "check_negative_carry";
    const DESCRIPTION: &'static str = "Return the negative-carry regime state for a basis position: days negative, the pre-authorized trigger window, average daily carry, and whether the plan has fired. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = WorldMarketsApp::account_id(&ctx, None);
        let carry_state = crate::carry::check(
            &app.client,
            &app.carry_ledger,
            &args.position_id,
            account_id,
        )?;
        Ok(json!({
            "source": "world-markets-reporting",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": app.client.block_number()?,
            "carry_state": carry_state,
            "executable": false,
            "cadence_note": "This plugin persists carry state and returns it. The host runtime owns the daily cadence that invokes this check and the push when fired flips.",
        }))
    }
}

pub(crate) struct RenderShare;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RenderShareArgs {}

impl DynAomiTool for RenderShare {
    type App = WorldMarketsApp;
    type Args = RenderShareArgs;
    const NAME: &'static str = "render_share";
    const DESCRIPTION: &'static str = "Share card caption + guest deep link. Send `message` verbatim. Never invent a deposit amount. Never executes.";

    fn run(
        app: &WorldMarketsApp,
        _args: Self::Args,
        _ctx: DynToolCallCtx,
    ) -> Result<Value, String> {
        let funnel = Funnel::new(&app.reporting, &app.guest_store, FunnelConfig::default());
        // PNG renderer is a host dependency (see docs/FUTURE-WORK.md).
        let image_available = std::env::var("WORLD_SHARE_CARD_RENDERER")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let surface = funnel.share(image_available);
        Ok(guest::to_tool_json(&surface))
    }
}

pub(crate) struct RenderGuestSurface;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RenderGuestSurfaceArgs {
    /// Guest session id (Telegram identity or g_<token> start payload).
    pub(crate) guest_id: String,
    /// Surface name (greeting, showcase, paper, upgrade, …).
    pub(crate) surface: String,
}

impl DynAomiTool for RenderGuestSurface {
    type App = WorldMarketsApp;
    type Args = RenderGuestSurfaceArgs;
    const NAME: &'static str = "render_guest_surface";
    const DESCRIPTION: &'static str = "Guest/paper message. Send `message` verbatim. Never invent numbers. Never a policy verdict. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let guest_id =
            guest::guest_id_from_start(&args.guest_id).unwrap_or_else(|| args.guest_id.clone());
        let funnel = Funnel::new(&app.reporting, &app.guest_store, FunnelConfig::default());
        let surface = funnel.render(&guest_id, &args.surface)?;
        Ok(guest::to_tool_json(&surface))
    }
}

pub(crate) struct ApplyGuestUpgrade;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ApplyGuestUpgradeArgs {
    /// Guest session id after grant-key on world.inc.
    pub(crate) guest_id: String,
}

impl DynAomiTool for ApplyGuestUpgrade {
    type App = WorldMarketsApp;
    type Args = ApplyGuestUpgradeArgs;
    const NAME: &'static str = "apply_guest_upgrade";
    const DESCRIPTION: &'static str =
        "In-place upgrade after grant-key; freeze paper read-only. Once. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let guest_id =
            guest::guest_id_from_start(&args.guest_id).unwrap_or_else(|| args.guest_id.clone());
        let funnel = Funnel::new(&app.reporting, &app.guest_store, FunnelConfig::default());
        let surface = funnel.render(&guest_id, "upgrade")?;
        Ok(guest::to_tool_json(&surface))
    }
}

fn value_u64(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value.as_u64().or_else(|| {
            let raw = value.as_str()?;
            raw.parse::<u64>()
                .ok()
                .or_else(|| raw.strip_prefix("world-")?.parse::<u64>().ok())
        })
    })
}

fn normalize_product(product: &str) -> Result<&'static str, String> {
    match product.to_ascii_lowercase().as_str() {
        "spot" => Ok("spot"),
        "perp" | "perpetual" => Ok("perp"),
        _ => Err("[world-markets] trade tools support spot and perp only".to_string()),
    }
}

fn normalize_execute_product(product: &str) -> Result<&'static str, String> {
    match product.to_ascii_lowercase().as_str() {
        "spot" => Ok("spot"),
        "perp" | "perpetual" => Ok("perp"),
        "lend" | "lending" => Ok("lend"),
        _ => Err("[world-markets] execute tools support spot, perp, or lend".to_string()),
    }
}

fn normalize_execute_side(product: &str, side: &str) -> Result<String, String> {
    let side = side.to_ascii_lowercase();
    let ok = match product {
        "lend" => matches!(side.as_str(), "lend" | "borrow" | "buy" | "sell"),
        "perp" => matches!(side.as_str(), "buy" | "sell" | "long" | "short"),
        _ => matches!(side.as_str(), "buy" | "sell"),
    };
    if !ok {
        return Err(format!(
            "[world-markets] unsupported side {side:?} for product {product}"
        ));
    }
    Ok(match side.as_str() {
        "long" => "buy".to_string(),
        "short" => "sell".to_string(),
        other => other.to_string(),
    })
}

fn resolve_order_type(named: Option<&str>, price: Option<&str>) -> String {
    match named.map(|value| value.to_ascii_lowercase()).as_deref() {
        Some("market") | Some("ioc") => "market".to_string(),
        Some("limit") => "limit".to_string(),
        _ if price.is_some() => "limit".to_string(),
        _ => "market".to_string(),
    }
}

fn numeric_position_id(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(value.to_string())
}

fn execution_blocked(access: &AccountAccess, verdict: &Verdict) -> Value {
    json!({
        "source": "world-markets-execution",
        "executable": false,
        "access": access,
        "policy_result": verdict,
    })
}

fn execution_ok(access: &AccountAccess, verdict: &Verdict, receipt: Value, intent: Value) -> Value {
    json!({
        "source": "world-markets-execution",
        "executable": true,
        "access": access,
        "intent": intent,
        "policy_result": verdict,
        "receipt": receipt,
    })
}

fn normalize_effect_product(product: &str) -> Result<&'static str, String> {
    match product.to_ascii_lowercase().as_str() {
        "spot" => Ok("spot"),
        "perp" | "perpetual" => Ok("perp"),
        "lend" | "lending" => Ok("lend"),
        _ => Err("[world-markets] preview_account_effect supports spot, perp, or lend".to_string()),
    }
}

fn quote_available(account: &Account, quote_symbol: &str) -> Result<Decimal, String> {
    let raw = account
        .balances
        .iter()
        .find(|balance| balance.symbol.eq_ignore_ascii_case(quote_symbol))
        .map(|balance| balance.available.as_str())
        .unwrap_or("0");
    parse_decimal(raw, "available")
        .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))
}

fn other_directional_legs(account: &Account, except: &str) -> Vec<(String, String)> {
    account
        .perpetual_positions
        .iter()
        .filter(|position| !position.symbol.eq_ignore_ascii_case(except))
        .filter_map(|position| {
            let qty = parse_decimal(&position.quantity, "quantity").ok()?;
            if qty.is_zero() {
                return None;
            }
            Some((position.symbol.clone(), position.side.clone()))
        })
        .collect()
}

fn concern_clause(
    base: &str,
    current: Decimal,
    after: Decimal,
    others: &[(String, String)],
) -> String {
    let side = if current.is_sign_negative() {
        "short"
    } else {
        "long"
    };
    if after.is_zero() && !current.is_zero() {
        if others.is_empty() {
            format!("the open {base} {side} was your main directional exposure")
        } else {
            let carried = others
                .iter()
                .map(|(symbol, side)| format!("{symbol} {side}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("you'd be flat {base} while still carrying {carried}")
        }
    } else if after.abs() < current.abs() {
        format!("this reduces your {base} directional exposure")
    } else {
        format!("this adds {base} directional exposure")
    }
}

fn current_position(
    account: &Account,
    product: &str,
    base_symbol: &str,
) -> Result<Decimal, String> {
    let value = match product {
        "perp" => account
            .perpetual_positions
            .iter()
            .find(|position| position.symbol.eq_ignore_ascii_case(base_symbol))
            .map(|position| position.quantity.as_str()),
        "spot" => account
            .balances
            .iter()
            .find(|balance| balance.symbol.eq_ignore_ascii_case(base_symbol))
            .map(|balance| balance.balance.as_str()),
        "lend" => account
            .lending_positions
            .iter()
            .find(|position| position.symbol.eq_ignore_ascii_case(base_symbol))
            .map(|position| position.lender_quantity.as_str()),
        _ => None,
    }
    .unwrap_or("0");
    parse_decimal(value, "current_position_quantity")
        .map_err(|verdict: Verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reporting::derive_account_effect;

    #[test]
    fn parses_numeric_and_prefixed_account_references() {
        assert_eq!(value_u64(Some(&json!(42))), Some(42));
        assert_eq!(value_u64(Some(&json!("42"))), Some(42));
        assert_eq!(value_u64(Some(&json!("world-42"))), Some(42));
        assert_eq!(value_u64(Some(&json!("other-42"))), None);
    }

    #[test]
    fn resolve_order_type_infers_limit_from_price() {
        assert_eq!(resolve_order_type(None, Some("2000")), "limit");
        assert_eq!(resolve_order_type(None, None), "market");
        assert_eq!(resolve_order_type(Some("market"), Some("2000")), "market");
    }

    #[test]
    fn numeric_position_id_ignores_aggregated_loan_ids() {
        assert_eq!(
            numeric_position_id(Some("436080915855955")),
            Some("436080915855955".to_string())
        );
        assert_eq!(numeric_position_id(Some("agg:WETH:borrower")), None);
        assert_eq!(numeric_position_id(None), None);
    }

    #[test]
    fn execute_order_fails_closed_without_account_context() {
        let app = WorldMarketsApp::default();
        let err = ExecuteWorldOrder::run(
            &app,
            ExecuteWorldOrderArgs {
                product: "perp".to_string(),
                side: "buy".to_string(),
                base_symbol: "WETH".to_string(),
                quote_symbol: Some("USDT".to_string()),
                quantity: "0.1".to_string(),
                price: None,
                order_type: None,
                slippage: None,
                account_id: None,
                wallet_address: None,
            },
            empty_ctx("execute_world_order"),
        )
        .unwrap_err();
        assert!(
            err.contains("no World account")
                || err.contains("no acting wallet")
                || err.contains("execution sidecar"),
            "{err}"
        );
    }

    fn ctx_with(attributes: Value) -> DynToolCallCtx {
        DynToolCallCtx {
            session_id: "account-id-resolution".to_string(),
            tool_name: "get_world_account".to_string(),
            call_id: "account-id-resolution-1".to_string(),
            state_attributes: attributes.as_object().unwrap().clone(),
            secrets: Default::default(),
        }
    }

    /// (3) A session-persistent `WORLD_ACCOUNT_ID` resolves when the runtime
    /// stubs all state attributes to None (dev/CLI mode), yet never overrides an
    /// explicit arg or a live handover attribute. Env-var mutation is process
    /// global, so the whole precedence ladder is asserted inside one test to keep
    /// it serial and leak-free.
    #[test]
    fn env_account_id_is_last_resort_and_never_overrides_context() {
        // SAFETY: single-threaded within this test; restored before returning.
        unsafe { std::env::set_var("WORLD_ACCOUNT_ID", "world-777") };

        // Empty context + no explicit arg → the env fallback fills the gap.
        let empty = ctx_with(json!({}));
        assert_eq!(
            WorldMarketsApp::account_id(&empty, None),
            Some(777),
            "env var should resolve when no handover/context account is present"
        );

        // The prefixed `world-<id>` form parses like the handover paths do.
        assert_eq!(WorldMarketsApp::account_id_from_env(), Some(777));

        // An explicit tool arg always wins over the env var.
        assert_eq!(WorldMarketsApp::account_id(&empty, Some(42)), Some(42));

        // A live handover attribute always wins over the env var.
        let handover = ctx_with(json!({ "world": { "account_id": 1234 } }));
        assert_eq!(
            WorldMarketsApp::account_id(&handover, None),
            Some(1234),
            "a real handover account must never be overridden by the env fallback"
        );

        // Unset → no phantom account; resolution fails closed as before.
        unsafe { std::env::remove_var("WORLD_ACCOUNT_ID") };
        assert_eq!(WorldMarketsApp::account_id(&empty, None), None);
    }

    #[test]
    #[ignore = "requires live UniFi RPC"]
    fn live_preview_uses_actor_account_and_mandate_context() {
        let app = WorldMarketsApp::default();
        let account_id = 1_577;
        let owner = app.client.owner_for(account_id).unwrap();
        let attributes = json!({
            "domain": { "evm": { "address": format!("{owner:#x}") } },
            "handover_mandate": {
                "version": 1,
                "markets": [{ "product": "perp", "base": "WETH", "quote": "USDT" }],
                "max_position_notional": { "amount": "25000", "quote": "USDT" },
                "max_leverage": "3",
                "min_risk_adjusted_portfolio_value": { "amount": "1", "quote": "USDT" },
                "halt_if_eligible_for_liquidation": true,
                "can_withdraw": false,
                "account": { "id": account_id },
                "brief": { "objective": "watch risk" }
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let ctx = DynToolCallCtx {
            session_id: "live-world-preview".to_string(),
            tool_name: "preview_world_trade".to_string(),
            call_id: "live-world-preview-1".to_string(),
            state_attributes: attributes,
            secrets: Default::default(),
        };
        let value = app
            .trade_preview(
                WorldTradeArgs {
                    product: "perp".to_string(),
                    side: "buy".to_string(),
                    base_symbol: "WETH".to_string(),
                    quote_symbol: "USDT".to_string(),
                    quantity: "0.01".to_string(),
                    account_id: None,
                    wallet_address: None,
                },
                &ctx,
            )
            .unwrap();
        assert_eq!(value["access"]["authorization"], "owner");
        assert_eq!(value["standing_brief"]["objective"], "watch risk");
        assert_eq!(value["preview"]["policy_result"]["status"], "deny");
        assert_eq!(value["preview"]["policy_result"]["rule"], "portfolio_floor");
        assert_eq!(value["preview"]["executable"], false);
    }

    fn empty_ctx(tool: &str) -> DynToolCallCtx {
        DynToolCallCtx {
            session_id: "test".to_string(),
            tool_name: tool.to_string(),
            call_id: "test-1".to_string(),
            state_attributes: Default::default(),
            secrets: Default::default(),
        }
    }

    // Task 2.1: a zero-edge slice through the TOOL returns the $0 null case,
    // never a fabricated saving (§4.1 "null results are results").
    #[test]
    fn plan_large_order_tool_reports_zero_edge() {
        let app = WorldMarketsApp::default();
        let value = PlanLargeOrder::run(
            &app,
            PlanLargeOrderArgs {
                market_order_cost: "0.05".to_string(),
                sliced_cost: "0.05".to_string(),
                slices: 1,
                window_minutes: 0,
                quote_symbol: "USDT".to_string(),
                baseline: "book at quote time".to_string(),
            },
            empty_ctx("plan_large_order"),
        )
        .unwrap();
        assert_eq!(value["source"], "world-markets-reporting");
        assert_eq!(value["executable"], false);
        assert_eq!(value["slice_plan"]["null_case"], true);
        assert_eq!(value["slice_plan"]["saved"]["value"], "0");
    }

    // Every reporting tool tags its source and marks itself non-executable.
    #[test]
    fn reporting_tools_are_sourced_and_non_executable() {
        let app = WorldMarketsApp::default();

        let _intent_only = PreviewAccountEffectArgs {
            product: "perp".to_string(),
            side: "buy".to_string(),
            base_symbol: "WBTC".to_string(),
            quote_symbol: "USDT".to_string(),
            quantity: "0.035".to_string(),
            account_id: None,
            wallet_address: None,
        };
        let effect = json!({
            "source": "world-markets-reporting",
            "account_effect": derive_account_effect(&EffectPlan {
                exposure_symbol: "WBTC".to_string(),
                exposure_before: Decimal::new(270_785, 2),
                exposure_after: Decimal::ZERO,
                available_before: Decimal::new(49_115, 2),
                available_after: Decimal::new(330_891, 2),
                quote: "USDT".to_string(),
                liquidation_risk_before: Some(Decimal::new(38, 1)),
                liquidation_risk_after: Some(Decimal::new(21, 1)),
                estimated_cost: Some(Decimal::new(840, 2)),
                missing_mark_symbols: Vec::new(),
                post_trade_risk_unavailable: false,
                concern_clause: "the open WBTC short was your main directional exposure".to_string(),
                baseline: "live snapshot".to_string(),
            }),
            "executable": false,
        });
        assert_eq!(effect["source"], "world-markets-reporting");
        assert_eq!(effect["executable"], false);
        assert_eq!(
            effect["account_effect"]["liquidation_risk"]["direction"],
            "safer"
        );
        assert!(effect["account_effect"]["expected_net_yield"].is_null());

        let dp = GetDollarpower::run(
            &app,
            GetDollarpowerArgs { portfolio_id: None },
            empty_ctx("get_dollarpower"),
        )
        .unwrap();
        assert_eq!(dp["source"], "world-markets-reporting");
        assert_eq!(dp["executable"], false);
    }

    // The guardian tool runs the algorithm end-to-end and surfaces the plan.
    #[test]
    fn guardian_tool_runs_and_rejects_bad_preference() {
        let app = WorldMarketsApp::default();
        let ok = SimulateGuardianUnwind::run(
            &app,
            SimulateGuardianUnwindArgs {
                candidates: vec![GuardianCandidateArg {
                    label: "close A".to_string(),
                    delta_score: "1.5".to_string(),
                    exit_cost: "50".to_string(),
                    breaks_structure_into_worse_residual: false,
                    reduces_directional_exposure: true,
                    protected: false,
                    is_eth: false,
                }],
                current_score: "6.0".to_string(),
                recovery_target: "7.0".to_string(),
                preference: Some("cheapest_safe".to_string()),
                emergency_slippage_reachable: true,
            },
            empty_ctx("simulate_guardian_unwind"),
        )
        .unwrap();
        assert_eq!(ok["source"], "world-markets-reporting");
        assert_eq!(ok["unwind_plan"]["reached_target"], true);
        assert_eq!(ok["unwind_plan"]["steps"].as_array().unwrap().len(), 1);

        let bad = SimulateGuardianUnwind::run(
            &app,
            SimulateGuardianUnwindArgs {
                candidates: vec![],
                current_score: "6.0".to_string(),
                recovery_target: "7.0".to_string(),
                preference: Some("do_whatever".to_string()),
                emergency_slippage_reachable: true,
            },
            empty_ctx("simulate_guardian_unwind"),
        );
        assert!(bad.is_err());
    }

    // A block's resize surfaces the floor and the engine rule verbatim.
    #[test]
    fn compute_resize_carries_floor_and_rule() {
        let app = WorldMarketsApp::default();
        let ctx = ctx_with(json!({
            "handover_mandate": {
                "version": 1,
                "markets": [{ "product": "perp", "base": "WETH", "quote": "USDT" }],
                "max_position_notional": { "amount": "25000", "quote": "USDT" },
                "max_leverage": "3",
                "min_risk_adjusted_portfolio_value": { "amount": "6000", "quote": "USDT" },
                "halt_if_eligible_for_liquidation": true,
                "can_withdraw": false
            }
        }));
        let value = ComputeResize::run(
            &app,
            ComputeResizeArgs {
                rule: "portfolio_floor".to_string(),
            },
            ctx,
        )
        .unwrap();
        assert_eq!(value["resize"]["rule"], "portfolio_floor");
        assert_eq!(value["resize"]["floor"]["value"], "6000");
        assert!(value["resize"]["largest_compliant_size"].is_null());
    }
}
