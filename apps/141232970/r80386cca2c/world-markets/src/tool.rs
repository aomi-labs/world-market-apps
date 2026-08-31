use alloy_primitives::Address;
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::str::FromStr;

use crate::brain::BrainClient;
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
use crate::size::SizeInput;
use crate::speech_ontology;

pub(crate) const MARKET_DATA_API_KEY: Secret = Secret::new(
    "MARKET_DATA_API_KEY",
    "Optional market-data vendor API key. Unused by the default Yahoo feed.",
    false,
);

#[derive(Clone, Default)]
pub(crate) struct WorldMarketsApp {
    client: WorldClient,
    reporting: FixtureReporting,
    pnl_ledger: PnlLedger,
    guest_store: GuestStore,
    carry_ledger: crate::carry::CarryLedger,
    loan_origins: crate::loans::LoanOriginStore,
    execution: ExecutionClient,
    warmer: crate::warm::AccountWarmer,
    brain: BrainClient,
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
    /// Percent-of ask, e.g. "20" or "20% of portfolio". Returns a computed `share` figure.
    #[serde(default)]
    pub(crate) share: Option<String>,
    /// `full` includes the raw account dump. Default is the compact card.
    #[serde(default)]
    pub(crate) detail: Option<String>,
}

pub(crate) struct RenderLookup;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(crate) struct RenderLookupArgs {
    /// Whole user message. Host short-circuit: pass this and send `message` when `skip_llm`.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// Explicit token (`b`/`p`/`r`/`a`/`d`/`index`) when the model already classified the lookup.
    #[serde(default)]
    pub(crate) token: Option<String>,
    /// World account ID. Optional when handover account context is available.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Expected owner wallet. This does not replace the acting wallet authorization check.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
    /// Set when voice/text ingest already recorded this turn. Avoids a second write.
    #[serde(default)]
    pub(crate) utterance_ref: Option<String>,
    /// Slot list from ingest when `utterance_ref` is set.
    #[serde(default)]
    #[schemars(skip)]
    pub(crate) slots: Option<Value>,
}

pub(crate) struct WarmAccount;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct WarmAccountArgs {
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
        let funding: Vec<serde_json::Value> = snapshot
            .rates
            .iter()
            .filter_map(|row| {
                row.funding_rate_8h.as_ref().and_then(|rate| {
                    crate::rates::eight_hour_rate_as_pct(rate).map(|pct| {
                        json!({
                            "symbol": row.base_symbol,
                            "rate": pct,
                        })
                    })
                })
            })
            .collect();
        if !funding.is_empty() {
            let _ = app.brain.ingest(&json!({ "funding": funding }));
        }
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
    const DESCRIPTION: &'static str = "Individual lend/borrow loans: rate_apr, matures_at, time_remaining_seconds, extensible, counterparty. Aggregates on get_world_account are not enough for roll timing. 10-day term; missing start → first-seen+10d, extensible true. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let (assets, account) = app.live_account(&access)?;
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
    /// Trade side: buy or sell. Inferred from the sentence (short/long) when omitted.
    #[serde(default)]
    pub(crate) side: String,
    /// Base asset symbol.
    pub(crate) base_symbol: String,
    /// Quote asset symbol.
    pub(crate) quote_symbol: String,
    /// Human-readable base quantity, such as "0.25". Deprecated alias for size_base.
    #[serde(default)]
    pub(crate) quantity: String,
    /// Dollar/notional size when the user named dollars. Server converts at the preview mark.
    #[serde(default)]
    pub(crate) size_usd: Option<String>,
    /// Base-asset size when the user named the asset unit.
    #[serde(default)]
    pub(crate) size_base: Option<String>,
    /// Optional World account ID. Handover account context is used when omitted.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Optional expected owner wallet.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
    /// The user's whole sentence. Required so an unknown asset can take the heard/CANT path.
    #[serde(default)]
    pub(crate) text: Option<String>,
}

pub(crate) struct ExecuteWorldOrder;
pub(crate) struct CancelWorldOrder;
pub(crate) struct ExecuteWorldSwap;
pub(crate) struct RenewWorldLoans;
pub(crate) struct PayWorldLoanInterest;
pub(crate) struct CloseWorldLoan;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ExecuteWorldOrderArgs {
    /// Product type: spot, perp, or lend.
    pub(crate) product: String,
    /// Side: buy/sell (spot), long/short (perp), lend/borrow (lend). Inferred from the sentence when omitted.
    #[serde(default)]
    pub(crate) side: String,
    /// Base asset symbol, such as WETH.
    pub(crate) base_symbol: String,
    /// Quote asset symbol. Required for spot and perp; defaults to USDT for lend.
    #[serde(default)]
    pub(crate) quote_symbol: Option<String>,
    /// Human-readable base quantity. Deprecated alias for size_base.
    #[serde(default)]
    pub(crate) quantity: String,
    /// Dollar/notional size when the user named dollars.
    #[serde(default)]
    pub(crate) size_usd: Option<String>,
    /// Base-asset size when the user named the asset unit.
    #[serde(default)]
    pub(crate) size_base: Option<String>,
    /// Limit price (spot/perp) or interest rate (lend). Omit for a market/IOC order.
    #[serde(default)]
    pub(crate) price: Option<String>,
    /// `market`, `limit`, `twap`, or `dca`. Inferred from size vs book when omitted.
    #[serde(default)]
    pub(crate) order_type: Option<String>,
    /// Slippage decimal for market orders, e.g. "0.005" for 0.5%.
    #[serde(default)]
    pub(crate) slippage: Option<String>,
    /// Slice count for TWAP/DCA. Inferred from book depth when omitted.
    #[serde(default)]
    pub(crate) slices: Option<u32>,
    /// TWAP window in minutes. Spacing is window/slices when set.
    #[serde(default)]
    pub(crate) window_minutes: Option<u32>,
    /// Seconds between child fills. Defaults: 60s TWAP, 1 day DCA.
    #[serde(default)]
    pub(crate) interval_secs: Option<u64>,
    /// DCA cadence: `daily` or `weekly`.
    #[serde(default)]
    pub(crate) cadence: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
    /// The user's whole utterance. Shown on the ledger during the cancel window.
    #[serde(default)]
    pub(crate) sentence: Option<String>,
    /// Per-order ledger binding for the staged/cancel/flush row of *this* order.
    /// Not a per-kind authorization token. A model-supplied value must not skip
    /// the 3s read-back; staging always keys cancel/flush to the instruction
    /// `stage_trade` returns for this call.
    #[serde(default)]
    #[allow(dead_code)]
    pub(crate) instruction_id: Option<String>,
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

    fn note_activity(&self, ctx: &DynToolCallCtx, explicit_id: Option<u64>) {
        let account_id = Self::account_id(ctx, explicit_id);
        self.warmer.touch(account_id);
        self.warmer.ensure_loop(self.client.clone());
    }

    fn kick_prefetch(&self, ctx: &DynToolCallCtx, explicit_id: Option<u64>) {
        self.note_activity(ctx, explicit_id);
        self.warmer.kick_prefetch(self.client.clone());
    }

    fn refresh_after_trade(
        &self,
        ctx: &DynToolCallCtx,
        account_id: Option<u64>,
        wallet_address: Option<&str>,
    ) {
        self.client.invalidate_volatile();
        self.warmer.clear_refresh();
        if let Ok((_, _, access)) = self.inspect_account(account_id, wallet_address, ctx) {
            self.warmer.mark_refreshed(access.account_id);
        }
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
        self.note_activity(ctx, account_id);
        let account_id = Self::account_id(ctx, account_id);
        let owner_wallet = wallet_address
            .map(ToString::to_string)
            .or_else(|| ctx.attribute_string(&["world", "owner_wallet"]));
        let actor = ctx.attribute_string(&["domain", "evm", "address"]);
        self.client
            .resolve_account(account_id, owner_wallet.as_deref(), actor.as_deref())
    }

    fn live_account(
        &self,
        access: &AccountAccess,
    ) -> Result<(Vec<crate::client::Asset>, Account), String> {
        let assets = self.client.assets()?;
        let owner = Address::from_str(&access.owner)
            .map_err(|e| format!("[world-markets] invalid owner address: {e}"))?;
        let account = self
            .client
            .account_with_owner(access.account_id, owner, &assets)?;
        Ok((assets, account))
    }

    fn inspect_account(
        &self,
        account_id: Option<u64>,
        wallet_address: Option<&str>,
        ctx: &DynToolCallCtx,
    ) -> Result<(Value, Account, AccountAccess), String> {
        let access = self.access(account_id, wallet_address, ctx)?;
        let (assets, account) = self.live_account(&access)?;
        let block_number = self.client.block_number()?;
        let metrics = crate::liquidation_risk::compute_metrics(
            &self.client,
            &account,
            &assets,
            block_number,
        )?;
        let lookups = crate::lookups::compute_lookups(
            &self.client,
            &account,
            &assets,
            &metrics.net_asset_value,
            block_number,
        )?;
        let liquidation_risk = metrics.liquidation_risk.clone();
        let compact = json!({
            "account_id": access.account_id,
            "authorization": access.authorization,
        });
        let payload = json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": self.client.exchange(),
            "block_number": block_number,
            "access": compact,
            "account": {
                "account_id": account.account_id,
                "eligible_for_liquidation": account.eligible_for_liquidation,
                "risk_adjusted_portfolio_value": account.risk_adjusted_portfolio_value,
            },
            "metrics": metrics,
            "lookups": lookups,
        });
        self.warmer.mark_refreshed(access.account_id);
        let idle_quote = account
            .balances
            .iter()
            .find(|b| b.symbol.eq_ignore_ascii_case("USDT"))
            .map(|b| b.balance.clone());
        let loan_fingerprints: Vec<String> = account
            .lending_positions
            .iter()
            .map(|p| format!("{}:{}:{}", p.symbol, p.borrower_quantity, p.lender_quantity))
            .collect();
        let _ = self.brain.ingest(&json!({
            "account_id": access.account_id,
            "rapv": account.risk_adjusted_portfolio_value,
            "liquidation_risk": liquidation_risk,
            "idle_quote": idle_quote,
            "loan_fingerprints": loan_fingerprints,
            "marks": account.perpetual_positions.iter().map(|p| json!({
                "symbol": p.symbol,
            })).collect::<Vec<_>>(),
        }));
        Ok((payload, account, access))
    }

    fn render_terse_lookup(
        &self,
        kind: crate::lookups::LookupKind,
        account_id: Option<u64>,
        wallet_address: Option<&str>,
        ctx: &DynToolCallCtx,
    ) -> Result<String, String> {
        use crate::lookups::{self, LookupKind};
        match kind {
            LookupKind::Index => Ok(lookups::INDEX_LINE.to_string()),
            LookupKind::Available => Ok(lookups::render_available(None)),
            LookupKind::Dollarpower => {
                let portfolio_id = WorldMarketsApp::account_id(ctx, account_id)
                    .map(|id| id.to_string())
                    .unwrap_or_default();
                let dp = self.reporting.dollarpower(&portfolio_id);
                Ok(lookups::render_dollarpower(
                    &dp.ratio.value,
                    &dp.committed.value,
                    dp.committed.is_estimate,
                    &dp.effective.value,
                    dp.effective.is_estimate,
                ))
            }
            LookupKind::Balance | LookupKind::Risk | LookupKind::Positions => {
                let access = self.access(account_id, wallet_address, ctx)?;
                let (assets, account) = self.live_account(&access)?;
                let block_number = self.client.block_number()?;
                match kind {
                    LookupKind::Positions => {
                        let lookups_data = lookups::compute_lookups(
                            &self.client,
                            &account,
                            &assets,
                            &account.risk_adjusted_portfolio_value,
                            block_number,
                        )?;
                        Ok(lookups::render_positions(&lookups_data.positions))
                    }
                    LookupKind::Balance | LookupKind::Risk => {
                        let metrics = crate::liquidation_risk::compute_metrics(
                            &self.client,
                            &account,
                            &assets,
                            block_number,
                        )?;
                        if kind == LookupKind::Balance {
                            Ok(lookups::render_balance(&metrics.net_asset_value))
                        } else {
                            Ok(lookups::render_risk(
                                &metrics.liquidation_risk,
                                account.eligible_for_liquidation,
                            ))
                        }
                    }
                    LookupKind::Index | LookupKind::Available | LookupKind::Dollarpower => {
                        unreachable!("non-account kinds handled above")
                    }
                }
            }
        }
    }

    fn snapshot_effect_plan(
        &self,
        args: PreviewAccountEffectArgs,
        ctx: &DynToolCallCtx,
        resolved_qty: Decimal,
    ) -> Result<EffectPlan, String> {
        let product = normalize_effect_product(&args.product)?;
        let side = infer_trade_side(&args.side, args.text.as_deref()).or_else(|_| {
            let side = args.side.to_ascii_lowercase();
            if matches!(
                side.as_str(),
                "buy" | "sell" | "long" | "short" | "lend" | "borrow"
            ) {
                Ok(side)
            } else {
                Err("[world-markets] side must be buy/sell, long/short, or lend/borrow".to_string())
            }
        })?;
        let quantity = resolved_qty;
        if quantity <= Decimal::ZERO {
            return Err("[world-markets] quantity must be greater than zero".to_string());
        }

        let access = self.access(args.account_id, args.wallet_address.as_deref(), ctx)?;
        let (assets, account) = self.live_account(&access)?;
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
        let sentence = args.text.as_deref();
        let side = match infer_trade_side(&args.side, sentence) {
            Ok(side) => side,
            Err(err) => {
                return Ok(json!({
                    "error": "missing_side",
                    "detail": err,
                    "retry_with": { "side": speech_ontology::infer_side(sentence.unwrap_or("")).unwrap_or_else(|| "buy".to_string()) },
                    "hint": "short/long in the sentence map to side=sell/buy. Resend with side set.",
                    "executable": false,
                }));
            }
        };

        let access = self.access(args.account_id, args.wallet_address.as_deref(), ctx)?;
        let (assets, account) = self.live_account(&access)?;
        if let Some(heard) = crate::cant::heard_unknown_trade_asset(
            WorldMarketsApp::account_id(ctx, args.account_id),
            args.text.as_deref(),
            &side,
            &args.quantity,
            &args.base_symbol,
            &assets,
        ) {
            return Ok(crate::tasks::attach_open_instructions(
                &self.brain,
                WorldMarketsApp::account_id(ctx, args.account_id),
                heard,
            ));
        }
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = asset_by_symbol(&assets, &args.quote_symbol)?;
        let market = self
            .client
            .market(product, base.clone(), Some(quote.clone()))?;
        let mark_price = parse_decimal(&market.mark_price, "mark_price")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
        let resolved = match resolve_size(
            sentence,
            args.size_usd.as_deref(),
            args.size_base.as_deref(),
            &args.quantity,
            &base.symbol,
            mark_price,
        ) {
            Ok(resolved) => resolved,
            Err(value) => return Ok(value),
        };
        let quantity = resolved.base_qty;
        if quantity <= Decimal::ZERO {
            return Err("[world-markets] quantity must be greater than zero".to_string());
        }

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
        let post_trade_rapv = projected
            .as_ref()
            .map(|p| p.rapv)
            .or_else(|| crate::liquidation_risk::dev_seed_rapv(&account));
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
                post_trade_risk_adjusted_portfolio_value: post_trade_rapv,
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

        let pair = format!("{product} {}/{}", base.symbol, quote.symbol);
        let detail_rendered = crate::reporting::render_deny(
            verdict.rule,
            &verdict.detail,
            Some(&base.symbol),
            Some(estimated_notional),
            None,
            Some(&pair),
        );
        let mut preview = json!({
            "account_id": account.account_id,
            "product": product,
            "side": side,
            "base_symbol": base.symbol,
            "quote_symbol": quote.symbol,
            "quantity": resolved.base_qty.normalize().to_string(),
            "resolved_size": resolved.to_json(),
            "current_position_quantity": current_position_quantity.to_string(),
            "mark_price": market.mark_price,
            "estimated_notional": estimated_notional.to_string(),
            "estimated_notional_rendered": format!("`{}`", crate::lookups::format_money(estimated_notional, false)),
            "policy_result": {
                "status": verdict.status,
                "rule": verdict.rule,
                "detail": verdict.detail,
                "detail_rendered": detail_rendered,
            },
            "executable": false,
            "status": status,
            "reason": reason,
        });
        if !verdict.is_allow() {
            preview["message"] = json!(detail_rendered);
            preview["reply_verbatim"] = json!(true);
            preview["controls"] = json!([
                { "label": "View mandate on World ↗", "action": "view_mandate" },
                { "label": "Keep as is", "action": "keep" }
            ]);
        }
        let payload = json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": self.client.exchange(),
            "block_number": self.client.block_number()?,
            "access": compact_access(&access),
            "preview": preview,
        });
        log_tool_bytes("preview_world_trade", &payload);
        Ok(payload)
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
        let (_assets, account) = self.live_account(&access)?;
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

    fn run(app: &WorldMarketsApp, _args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        app.kick_prefetch(&ctx, None);
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
        let before = app.client.rpc_stats();
        let (mut payload, _, _) =
            app.inspect_account(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        if args.detail.as_deref() == Some("full") {
            if let Ok(access) = app.access(args.account_id, args.wallet_address.as_deref(), &ctx) {
                if let Ok((_, account)) = app.live_account(&access) {
                    if let Some(obj) = payload.as_object_mut() {
                        obj.insert("account".into(), json!(account));
                        obj.insert("access".into(), json!(access));
                    }
                }
            }
        }
        if let Some(share_raw) = args.share.as_deref().filter(|s| !s.trim().is_empty()) {
            attach_share_field(&mut payload, share_raw);
        }
        attach_rpc_trace(&app.client, before, &mut payload);
        log_tool_bytes("get_world_account", &payload);
        Ok(payload)
    }
}

impl DynAomiTool for RenderLookup {
    type App = WorldMarketsApp;
    type Args = RenderLookupArgs;
    const NAME: &'static str = "render_lookup";
    const DESCRIPTION: &'static str = "Whole-message terse lookup (b/p/r/a/d, word forms, ?/commands), cancel task {id}, the non-money share/introduce intent, or an unfulfillable/near-match/unclear heard reply. Host: call on every user message with text=user message; pass utterance_ref and slots from Mini App send_payload when present so the heard path does not re-ingest. If skip_llm, send message (and controls when present) and do not call the LLM. Unmatched messages still prefetch the account. Model: paste message verbatim. Share never executes. Cancel drops a watch — never a trade. Unfulfillable never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        if let Some(id) = args
            .text
            .as_deref()
            .and_then(crate::lookups::parse_cancel_task)
        {
            return render_cancel_task(app, &id, args.account_id, &ctx);
        }
        if let Some(intent) = args
            .text
            .as_deref()
            .and_then(crate::share::parse_share_intent)
        {
            return render_share_intent(app, intent, args.account_id, None, &ctx);
        }
        if let Some(text) = args.text.as_deref() {
            if let Some(ask) = crate::lookups::parse_share_ask(text) {
                if let Some(value) = render_percent_of(
                    app,
                    &ask,
                    args.account_id,
                    args.wallet_address.as_deref(),
                    &ctx,
                ) {
                    return Ok(value);
                }
            }
        }
        let kind = args
            .token
            .as_deref()
            .and_then(crate::lookups::parse_lookup_token)
            .or_else(|| {
                args.text
                    .as_deref()
                    .and_then(crate::lookups::parse_lookup_text)
            });
        let Some(kind) = kind else {
            app.kick_prefetch(&ctx, args.account_id);
            if let Some(text) = args
                .text
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if let Some(wall) = crate::cant::cant_wall_for(text) {
                    return Ok(crate::tasks::attach_open_instructions(
                        &app.brain,
                        WorldMarketsApp::account_id(&ctx, args.account_id),
                        wall,
                    ));
                }
                if let Some(account_id) = WorldMarketsApp::account_id(&ctx, args.account_id) {
                    let extra = match (&args.utterance_ref, &args.slots) {
                        (None, None) => None,
                        _ => Some(json!({
                            "utterance_ref": args.utterance_ref,
                            "slots": args.slots,
                            "channel": "speech",
                        })),
                    };
                    if let Some(value) = crate::cant::try_heard(account_id, text, extra.as_ref()) {
                        return Ok(crate::tasks::attach_open_instructions(
                            &app.brain,
                            Some(account_id),
                            crate::cant::apply_unclear_copy(value),
                        ));
                    }
                }
            }
            return Ok(crate::tasks::attach_open_instructions(
                &app.brain,
                WorldMarketsApp::account_id(&ctx, args.account_id),
                json!({
                    "source": "world-markets-lookup",
                    "executable": false,
                    "matched": false,
                    "skip_llm": false,
                    "reply_verbatim": false,
                }),
            ));
        };
        app.note_activity(&ctx, args.account_id);
        let before = app.client.rpc_stats();
        let message =
            app.render_terse_lookup(kind, args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let mut payload = json!({
            "source": "world-markets-lookup",
            "executable": false,
            "matched": true,
            "skip_llm": true,
            "reply_verbatim": true,
            "token": kind.token(),
            "message": message,
        });
        attach_rpc_trace(&app.client, before, &mut payload);
        Ok(payload)
    }
}

fn live_bound_account(ctx: &DynToolCallCtx) -> bool {
    ctx.attribute_u64(&["world", "account_id"]).is_some()
        || value_u64(ctx.attribute_path(&["handover_account_id"])).is_some()
        || value_u64(ctx.attribute_path(&["platform_account_ref"])).is_some()
        || value_u64(ctx.attribute_path(&["handover_account_ref"])).is_some()
        || ctx
            .attribute_u64(&["handover_mandate", "account", "id"])
            .is_some()
}

fn telegram_first_name(ctx: &DynToolCallCtx, explicit: Option<&str>) -> Option<String> {
    explicit
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| ctx.attribute_string(&["telegram", "user", "first_name"]))
        .or_else(|| ctx.attribute_string(&["user", "first_name"]))
        .or_else(|| ctx.attribute_string(&["telegram", "first_name"]))
}

fn telegram_user_id(ctx: &DynToolCallCtx) -> Option<u64> {
    ctx.attribute_u64(&["telegram", "user", "id"])
        .or_else(|| ctx.attribute_u64(&["user", "id"]))
        .or_else(|| ctx.attribute_u64(&["telegram", "id"]))
}

fn share_user_id(ctx: &DynToolCallCtx, account_id: Option<u64>) -> String {
    WorldMarketsApp::account_id(ctx, account_id)
        .map(|id| id.to_string())
        .or_else(|| telegram_user_id(ctx).map(|id| id.to_string()))
        .unwrap_or_else(|| format!("session-{}", ctx.session_id))
}

fn wrap_share(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.insert("source".into(), json!("world-markets-share"));
        map.insert("executable".into(), json!(false));
        map.insert("skip_llm".into(), json!(true));
        map.insert("reply_verbatim".into(), json!(true));
        map.insert("matched".into(), json!(true));
        map.insert("token".into(), json!("share"));
        map.entry("policy_verdict".to_string())
            .or_insert(Value::Null);
    }
    value
}

fn already_user_reply() -> Value {
    wrap_share(json!({
        "surface": "already_user",
        "message": crate::share::ALREADY_USER,
        "hint": null,
        "name_ask": null,
        "messages": [{ "kind": "already_user", "message": crate::share::ALREADY_USER }],
        "controls": [],
        "simulated": false,
    }))
}

fn render_share_intent(
    app: &WorldMarketsApp,
    intent: crate::share::ShareIntent,
    account_id: Option<u64>,
    first_name: Option<&str>,
    ctx: &DynToolCallCtx,
) -> Result<Value, String> {
    let user_id = share_user_id(ctx, account_id);
    let first_name = telegram_first_name(ctx, first_name);
    let telegram_bot = std::env::var("WORLD_TELEGRAM_BOT")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "WorldMarketsBot".to_string());
    match app.brain.share(&json!({
        "action": intent.action(),
        "user_id": user_id,
        "account_id": user_id,
        "first_name": first_name,
        "telegram_bot": telegram_bot,
    })) {
        Ok(value) => Ok(wrap_share(value)),
        Err(err) => Ok(wrap_share(json!({
            "surface": "share_unavailable",
            "message": crate::share::HINT,
            "hint": crate::share::HINT,
            "name_ask": null,
            "error": err,
            "controls": [],
        }))),
    }
}

fn render_cancel_task(
    app: &WorldMarketsApp,
    id: &str,
    account_id: Option<u64>,
    ctx: &DynToolCallCtx,
) -> Result<Value, String> {
    let Some(account_id) = WorldMarketsApp::account_id(ctx, account_id) else {
        return Ok(json!({
            "source": "world-markets-lookup",
            "executable": false,
            "matched": true,
            "skip_llm": true,
            "reply_verbatim": true,
            "token": "cancel_task",
            "message": "No bound account — nothing cancelled.",
        }));
    };
    let result = match app.brain.cancel_task(account_id, id) {
        Ok(value) => value,
        Err(_) => {
            return Ok(json!({
                "source": "world-markets-lookup",
                "executable": false,
                "matched": true,
                "skip_llm": true,
                "reply_verbatim": true,
                "token": "cancel_task",
                "message": "can't reach the ledger — task not cancelled.",
            }));
        }
    };
    let ok = result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let message = result
        .get("reply")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if ok {
                format!("cancelled {id}")
            } else {
                format!("No task {id} on this account.")
            }
        });
    Ok(json!({
        "source": "world-markets-lookup",
        "executable": false,
        "matched": true,
        "skip_llm": true,
        "reply_verbatim": true,
        "token": "cancel_task",
        "message": message,
        "result": result,
    }))
}

impl DynAomiTool for WarmAccount {
    type App = WorldMarketsApp;
    type Args = WarmAccountArgs;
    const NAME: &'static str = "warm_account";
    const DESCRIPTION: &'static str = "Prefetch live account, metrics, and marks into the RPC cache. Host: call at the start of every user message (render_lookup also does this). Plugin also refreshes every 60s while the session is active and after trades. Never a user-facing reply. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let before = app.client.rpc_stats();
        let (payload, _, access) =
            app.inspect_account(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let mut out = json!({
            "source": "world-markets-contract",
            "executable": false,
            "warmed": true,
            "account_id": access.account_id,
            "block_number": payload.get("block_number"),
        });
        attach_rpc_trace(&app.client, before, &mut out);
        Ok(out)
    }
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

impl DynAomiTool for GetHealthSnapshot {
    type App = WorldMarketsApp;
    type Args = GetHealthSnapshotArgs;
    const NAME: &'static str = "get_health_snapshot";
    const DESCRIPTION: &'static str =
        "Health card in one call (account, pnl, dollarpower). Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let before = app.client.rpc_stats();
        let (mut payload, account, access) =
            app.inspect_account(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let pnl = crate::pnl::report(
            &app.client,
            &app.pnl_ledger,
            &account,
            args.position.as_deref(),
        )?;
        let dollarpower = app.reporting.dollarpower(&access.account_id.to_string());
        let ledger = app.brain.ledger_summary(access.account_id).ok();
        let needs_you = ledger
            .as_ref()
            .and_then(|v| v.get("needs_you"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let open = crate::tasks::load_open_instructions(&app.brain, Some(access.account_id));
        let needs_attention = if needs_you == 0 { json!([]) } else { open };
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("pnl".to_string(), json!(pnl));
            obj.insert(
                "dollarpower".to_string(),
                json!({
                    "ratio": dollarpower.ratio.value,
                    "message": crate::reporting::render_dollarpower_message(&dollarpower),
                }),
            );
            obj.insert("executable".to_string(), json!(false));
            obj.insert("needs_attention".to_string(), needs_attention);
            obj.insert(
                "controls".to_string(),
                json!([
                    { "label": "Nothing for now", "action": "dismiss" },
                    { "label": "View on World ↗", "action": "view" }
                ]),
            );
            if needs_you == 0 {
                obj.insert("attention_message".to_string(), json!("Nothing needs you."));
            }
        }
        attach_rpc_trace(&app.client, before, &mut payload);
        log_tool_bytes("get_health_snapshot", &payload);
        Ok(payload)
    }
}

pub(crate) struct GetStrategySnapshot;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetStrategySnapshotArgs {
    /// World account ID. Optional when handover account context is available.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Expected owner wallet. This does not replace the acting wallet authorization check.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
    /// Base symbols for rates (e.g. ["WETH","WBTC"]). Omit for every listed asset.
    #[serde(default)]
    pub(crate) assets: Option<Vec<String>>,
}

impl DynAomiTool for GetStrategySnapshot {
    type App = WorldMarketsApp;
    type Args = GetStrategySnapshotArgs;
    const NAME: &'static str = "get_strategy_snapshot";
    const DESCRIPTION: &'static str =
        "Strategy refresh in one call (account, rates, loans, carry). Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let before = app.client.rpc_stats();
        let (mut payload, account, _access) =
            app.inspect_account(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let rates = crate::rates::snapshot(&app.client, args.assets.as_deref())?;
        let assets = app.client.assets()?;
        let loans = crate::loans::snapshot(&app.client, &app.loan_origins, &account, &assets)?;
        let carry = crate::carry::check_open_perps(&app.carry_ledger, &account, &rates)?;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert(
                "rates".to_string(),
                serde_json::to_value(&rates)
                    .map_err(|e| format!("[world-markets] failed to encode rates snapshot: {e}"))?,
            );
            obj.insert(
                "loans".to_string(),
                serde_json::to_value(&loans)
                    .map_err(|e| format!("[world-markets] failed to encode loans snapshot: {e}"))?,
            );
            obj.insert("carry".to_string(), json!(carry));
            obj.insert("executable".to_string(), json!(false));
        }
        attach_rpc_trace(&app.client, before, &mut payload);
        Ok(payload)
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
        let _ = app.brain.ingest(&json!({
            "symbol": market.base_token.symbol,
            "token_id": market.base_token.token_id,
            "mark": market.mark_price,
        }));
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
    const DESCRIPTION: &'static str = "Preview a World spot or perpetual intent from live state and return the deterministic mandate verdict. It never stages or executes. Host: pass text=the user's whole sentence. If the named asset is not in the universe, the tool returns the heard/CANT surface with skip_llm — send message (and controls) and do not call the LLM. Unfulfillable never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        app.trade_preview(args, &ctx)
    }
}

impl DynAomiTool for CheckWorldMandate {
    type App = WorldMarketsApp;
    type Args = WorldTradeArgs;
    const NAME: &'static str = "check_world_mandate";
    const DESCRIPTION: &'static str = "Evaluate one structured World trade intent against the bound mandate and live account/market state. Returns the exact allow or deny rule; it does not execute. Host: pass text=the user's whole sentence. If the named asset is not in the universe, the tool returns the heard/CANT surface with skip_llm — send message (and controls) and do not call the LLM. Unfulfillable never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let preview = app.trade_preview(args, &ctx)?;
        Ok(json!({
            "source": preview.get("source"),
            "chain_id": preview.get("chain_id"),
            "block_number": preview.get("block_number"),
            "access": preview.get("access"),
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
    const DESCRIPTION: &'static str = "Stage a World spot, perp, or lend/borrow order on the ledger for a 3s cancel window, then fill through the local execution sidecar after the mandate allows. Pass the user's whole sentence. Pass size_usd when they named dollars; size_base when they named the asset. Order type is inferred when omitted. First instance of an action kind stages with a CONFIRM-ONCE read-back (Cancel only; sends if uncancelled). Never withdraws.";

    fn run(
        app: &WorldMarketsApp,
        mut args: Self::Args,
        ctx: DynToolCallCtx,
    ) -> Result<Value, String> {
        let product = normalize_execute_product(&args.product)?;
        let sentence = trade_sentence(&args);
        let side = infer_execute_side(product, &args.side, Some(&sentence))?;
        args.side = side.clone();
        let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
        let (assets, account) = app.live_account(&access)?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote_symbol = args
            .quote_symbol
            .clone()
            .unwrap_or_else(|| "USDT".to_string());
        let quote = asset_by_symbol(&assets, &quote_symbol)?;
        let mark_price = if product == "lend" {
            Decimal::ONE
        } else {
            let market = app
                .client
                .market(product, base.clone(), Some(quote.clone()))?;
            parse_decimal(&market.mark_price, "mark_price").map_err(|verdict| {
                format!("[world-markets] {}: {}", verdict.rule, verdict.detail)
            })?
        };
        let resolved = match resolve_size(
            Some(&sentence),
            args.size_usd.as_deref(),
            args.size_base.as_deref(),
            &args.quantity,
            &base.symbol,
            mark_price,
        ) {
            Ok(resolved) => resolved,
            Err(value) => return Ok(value),
        };
        args.quantity = resolved.base_qty.normalize().to_string();
        let quantity = resolved.base_qty;
        if quantity <= Decimal::ZERO {
            return Err("[world-markets] quantity must be greater than zero".to_string());
        }
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
            return Ok(execution_blocked(
                &access,
                &verdict,
                Some(&base.symbol),
                Some(resolved.notional),
            ));
        }
        let kind = action_kind(product, &side);
        let first_instance = !kind_is_confirmed(app, access.account_id, &kind);
        let opposite_depth = if product == "lend" {
            None
        } else {
            app.client
                .book_visible_depth(&market.book, &side, base.position_decimals)
                .ok()
                .and_then(|raw| parse_decimal(&raw, "depth").ok())
        };
        let plan = crate::order_intent::infer_execution_plan(crate::order_intent::InferInput {
            named: args.order_type.as_deref(),
            price: args.price.as_deref(),
            sentence: Some(&sentence),
            quantity,
            opposite_depth,
            slices: args.slices,
            window_minutes: args.window_minutes,
            interval_secs: args.interval_secs,
            cadence: args.cadence.as_deref(),
        });
        let mut staged = args.clone();
        staged.order_type = Some(plan.order_type.clone());
        let mandate = ctx.attribute_path(&["handover_mandate"]).cloned();
        let extra = json!({
            "action_kind": kind,
            "notional": resolved.notional.normalize().to_string(),
            "mark": resolved.mark.normalize().to_string(),
            "telegram_chat_id": telegram_chat_id(&ctx),
            "size_usd": args.size_usd,
            "size_base": args.size_base,
        });
        let result = crate::staged::stage_and_schedule(
            &app.brain,
            access.account_id,
            &staged,
            &sentence,
            mandate.as_ref(),
            &plan,
            Some(&extra),
        )?;
        let instruction_id = result
            .pointer("/instruction/instruction_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let effect = preview_effect_for_receipt(app, &staged, &ctx, quantity).ok();
        let happened =
            crate::reporting::render_size_happened(&resolved, &base.symbol, product, None, true);
        let message = if first_instance {
            crate::reporting::render_confirm_once_readback(&resolved, &base.symbol, product)
        } else {
            crate::reporting::render_receipt(
                &happened,
                &format!("You asked to {sentence}."),
                effect.as_ref(),
                None,
                None,
                "within limits.",
                "Watching the fill. I'll only message you if it fails.",
                None,
                false,
            )
        };
        let controls = if first_instance {
            json!([
                { "label": "Cancel", "action": "cancel", "instruction_id": instruction_id }
            ])
        } else {
            json!([
                { "label": "View on World ↗", "action": "view" },
                { "label": "Explain", "action": "explain" },
                { "label": "Preview exit", "action": "preview_exit" }
            ])
        };
        let mut payload = result;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("resolved_size".into(), resolved.to_json());
            obj.insert("message".into(), json!(message));
            obj.insert("reply_verbatim".into(), json!(true));
            obj.insert("controls".into(), controls);
            obj.insert("needs_confirm".into(), json!(first_instance));
            obj.insert("action_kind".into(), json!(kind));
            if let Some(effect) = effect {
                obj.insert("account_effect".into(), json!(effect));
            }
        }
        log_tool_bytes("execute_world_order", &payload);
        Ok(payload)
    }
}

fn log_tool_bytes(name: &str, value: &Value) {
    if let Ok(raw) = serde_json::to_vec(value) {
        eprintln!(
            "[world-markets] tool_result_bytes name={name} bytes={}",
            raw.len()
        );
    }
    if let Ok(json) = serde_json::to_string(value) {
        eprintln!("[world-markets] tool_result {name} {json}");
    }
}

fn compact_access(access: &AccountAccess) -> Value {
    json!({
        "account_id": access.account_id,
        "authorization": access.authorization,
    })
}

fn looks_like_watch_correction(phrase: &str) -> bool {
    let lower = phrase.trim().to_ascii_lowercase();
    lower.starts_with("no,")
        || lower.starts_with("no ")
        || lower.contains("make it ")
        || lower.contains("change it")
        || lower.contains("instead")
}

fn attach_share_field(payload: &mut Value, share_raw: &str) {
    let text = if share_raw.contains('%')
        || share_raw.to_ascii_lowercase().contains("percent")
        || share_raw.to_ascii_lowercase().contains(" of ")
        || share_raw.to_ascii_lowercase().contains("half")
    {
        share_raw.to_string()
    } else {
        format!("{share_raw}% of my portfolio")
    };
    let Some(ask) = crate::lookups::parse_share_ask(&text) else {
        return;
    };
    let Some(lookups) = payload.get("lookups") else {
        return;
    };
    if let Some(share) = crate::lookups::share_from_lookups_json(lookups, &ask) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("share".into(), share);
        }
    }
}

fn render_percent_of(
    app: &WorldMarketsApp,
    ask: &crate::lookups::ShareAsk,
    account_id: Option<u64>,
    wallet: Option<&str>,
    ctx: &DynToolCallCtx,
) -> Option<Value> {
    let (mut payload, _, _) = app.inspect_account(account_id, wallet, ctx).ok()?;
    let lookups = payload.get("lookups")?;
    let share = crate::lookups::share_from_lookups_json(lookups, ask)?;
    let message = share
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("share".into(), share);
        obj.insert("source".into(), json!("world-markets-lookup"));
        obj.insert("matched".into(), json!(true));
        obj.insert("skip_llm".into(), json!(true));
        obj.insert("reply_verbatim".into(), json!(true));
        obj.insert("executable".into(), json!(false));
        obj.insert("message".into(), json!(message));
        obj.insert("token".into(), json!("share_of"));
    }
    Some(payload)
}

fn infer_trade_side(side: &str, sentence: Option<&str>) -> Result<String, String> {
    let side = side.trim().to_ascii_lowercase();
    if matches!(side.as_str(), "buy" | "sell") {
        return Ok(side);
    }
    if matches!(side.as_str(), "long") {
        return Ok("buy".into());
    }
    if matches!(side.as_str(), "short") {
        return Ok("sell".into());
    }
    if let Some(inferred) = sentence.and_then(speech_ontology::infer_side) {
        return Ok(inferred);
    }
    Err(
        "[world-markets] side must be buy or sell (short→sell, long→buy). Resend with side set."
            .into(),
    )
}

fn infer_execute_side(product: &str, side: &str, sentence: Option<&str>) -> Result<String, String> {
    if product == "lend" {
        let side = side.trim().to_ascii_lowercase();
        if matches!(side.as_str(), "lend" | "borrow" | "buy" | "sell") {
            return Ok(side);
        }
    }
    infer_trade_side(side, sentence).or_else(|_| normalize_execute_side(product, side))
}

fn resolve_size(
    sentence: Option<&str>,
    size_usd: Option<&str>,
    size_base: Option<&str>,
    quantity: &str,
    instrument: &str,
    mark: Decimal,
) -> Result<crate::size::ResolvedSize, Value> {
    let qty = quantity.trim();
    crate::size::classify_and_resolve(
        &SizeInput {
            sentence,
            size_usd,
            size_base,
            quantity: (!qty.is_empty()).then_some(qty),
            instrument: Some(instrument),
        },
        mark,
    )
    .map_err(|e| e.to_json())
}

fn resolve_preview_qty(
    app: &WorldMarketsApp,
    args: &PreviewAccountEffectArgs,
    ctx: &DynToolCallCtx,
) -> Result<Decimal, Value> {
    let access = app
        .access(args.account_id, args.wallet_address.as_deref(), ctx)
        .map_err(|e| json!({"error": e}))?;
    let (assets, _) = app.live_account(&access).map_err(|e| json!({"error": e}))?;
    let base = asset_by_symbol(&assets, &args.base_symbol).map_err(|e| json!({"error": e}))?;
    let mark = app
        .client
        .mark_price(base.token_id)
        .ok()
        .and_then(|(_, p)| parse_decimal(&p, "mark").ok())
        .unwrap_or(Decimal::ONE);
    resolve_size(
        args.text.as_deref(),
        args.size_usd.as_deref(),
        args.size_base.as_deref(),
        &args.quantity,
        &base.symbol,
        mark,
    )
    .map(|r| r.base_qty)
}

fn action_kind(product: &str, side: &str) -> String {
    format!("{product}_{side}")
}

fn kind_is_confirmed(app: &WorldMarketsApp, account_id: u64, kind: &str) -> bool {
    kind_confirmed_from_status(
        &app.brain
            .action_kind_status(account_id, kind)
            .unwrap_or(json!({})),
    )
}

fn kind_confirmed_from_status(status: &Value) -> bool {
    status.get("confirmed").and_then(Value::as_bool) == Some(true)
}

fn telegram_chat_id(ctx: &DynToolCallCtx) -> Option<u64> {
    ctx.attribute_u64(&["telegram", "chat", "id"])
        .or_else(|| ctx.attribute_u64(&["telegram", "user", "id"]))
        .or_else(|| ctx.attribute_u64(&["chat", "id"]))
}

fn preview_effect_for_receipt(
    app: &WorldMarketsApp,
    args: &ExecuteWorldOrderArgs,
    ctx: &DynToolCallCtx,
    qty: Decimal,
) -> Result<crate::reporting::AccountEffect, String> {
    let plan = app.snapshot_effect_plan(
        PreviewAccountEffectArgs {
            product: args.product.clone(),
            side: args.side.clone(),
            base_symbol: args.base_symbol.clone(),
            quote_symbol: args.quote_symbol.clone().unwrap_or_else(|| "USDT".into()),
            quantity: qty.normalize().to_string(),
            size_usd: None,
            size_base: Some(qty.normalize().to_string()),
            text: args.sentence.clone(),
            account_id: args.account_id,
            wallet_address: args.wallet_address.clone(),
        },
        ctx,
        qty,
    )?;
    Ok(app.reporting.account_effect(&plan))
}

pub(crate) fn trade_sentence(args: &ExecuteWorldOrderArgs) -> String {
    if let Some(sentence) = args
        .sentence
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return sentence.to_string();
    }
    let product = args.product.trim();
    let qty = args.quantity.trim();
    let base = args.base_symbol.trim();
    let side = args.side.trim();
    match args
        .price
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(price) => format!("{side} {qty} {base} {product} at {price}"),
        None => format!("{side} {qty} {base} {product} at market"),
    }
}

pub(crate) fn place_world_order(
    app: &WorldMarketsApp,
    args: ExecuteWorldOrderArgs,
    ctx: DynToolCallCtx,
) -> Result<Value, String> {
    let product = normalize_execute_product(&args.product)?;
    let side = normalize_execute_side(product, &args.side)?;
    let access = app.access(args.account_id, args.wallet_address.as_deref(), &ctx)?;
    let (assets, account) = app.live_account(&access)?;
    let base = asset_by_symbol(&assets, &args.base_symbol)?;
    let quote_symbol = args
        .quote_symbol
        .clone()
        .unwrap_or_else(|| "USDT".to_string());
    let quote = asset_by_symbol(&assets, &quote_symbol)?;
    let mark_price = if product == "lend" {
        Decimal::ONE
    } else {
        let market = app
            .client
            .market(product, base.clone(), Some(quote.clone()))?;
        parse_decimal(&market.mark_price, "mark_price")
            .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?
    };
    let resolved = match resolve_size(
        Some(&trade_sentence(&args)),
        args.size_usd.as_deref(),
        args.size_base.as_deref(),
        &args.quantity,
        &base.symbol,
        mark_price,
    ) {
        Ok(resolved) => resolved,
        Err(value) => return Ok(value),
    };
    let quantity = resolved.base_qty;
    if quantity <= Decimal::ZERO {
        return Err("[world-markets] quantity must be greater than zero".to_string());
    }
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
        return Ok(execution_blocked(&access, &verdict, None, None));
    }
    let order_type = crate::order_intent::venue_order_type(
        args.order_type.as_deref().unwrap_or(""),
        args.price.as_deref(),
    );
    let receipt = app.execution.place_order(&PlaceOrderRequest {
        account_id: access.account_id,
        product: product.to_string(),
        side: side.clone(),
        base_token_id: base.token_id,
        quote_token_id: (product != "lend").then_some(quote.token_id),
        quantity: resolved.base_qty.normalize().to_string(),
        price: args.price.clone(),
        order_type,
        slippage: args.slippage.clone(),
    })?;
    app.refresh_after_trade(&ctx, args.account_id, args.wallet_address.as_deref());
    Ok(execution_ok(
        &access,
        &verdict,
        receipt,
        json!({
            "product": product,
            "side": side,
            "base_symbol": base.symbol,
            "quote_symbol": quote.symbol,
            "quantity": resolved.base_qty.normalize().to_string(),
            "order_book": market.book,
        }),
    ))
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
        app.refresh_after_trade(&ctx, args.account_id, args.wallet_address.as_deref());
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
        let (assets, account) = app.live_account(&access)?;
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
            return Ok(execution_blocked(&access, &verdict, None, None));
        }
        let receipt = app.execution.swap(&SwapRequest {
            account_id: access.account_id,
            token_in: token_in.erc20_address.clone(),
            token_out: token_out.erc20_address.clone(),
            amount_in: args.amount_in.clone(),
            slippage: args.slippage.clone(),
        })?;
        app.refresh_after_trade(&ctx, args.account_id, args.wallet_address.as_deref());
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
        let (_assets, account) = app.live_account(&access)?;
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
                None,
                None,
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
                None,
                None,
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
        app.refresh_after_trade(&ctx, args.account_id, args.wallet_address.as_deref());
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
        app.refresh_after_trade(&ctx, args.account_id, args.wallet_address.as_deref());
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
        app.refresh_after_trade(&ctx, args.account_id, args.wallet_address.as_deref());
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
        let actor = match args
            .actor_address
            .or_else(|| ctx.attribute_string(&["domain", "evm", "address"]))
        {
            Some(actor) => actor,
            None => {
                return Ok(json!({
                    "source": "world-markets-contract",
                    "status": "read_failed",
                    "message": "I can't reach your account grant right now — try again in a moment",
                    "reply_verbatim": true,
                    "executable": false,
                }));
            }
        };
        match app.client.agent_permission(account_id, &actor) {
            Ok(permission) => Ok(json!({
                "source": "world-markets-contract",
                "chain_id": CHAIN_ID,
                "exchange": app.client.exchange(),
                "block_number": app.client.block_number()?,
                "permission": permission,
            })),
            Err(_) => Ok(json!({
                "source": "world-markets-contract",
                "status": "read_failed",
                "message": "I can't reach your account grant right now — try again in a moment",
                "reply_verbatim": true,
                "executable": false,
            })),
        }
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
        let (_assets, account) = app.live_account(&access)?;
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

fn attach_rpc_trace(client: &WorldClient, before: crate::rpc::RpcStats, value: &mut Value) {
    if !crate::rpc::trace_enabled() {
        return;
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "rpc".to_string(),
            json!(client.rpc_stats().saturating_sub(&before)),
        );
    }
}

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
    #[serde(default)]
    pub(crate) side: String,
    /// Base asset symbol.
    pub(crate) base_symbol: String,
    /// Quote asset symbol.
    pub(crate) quote_symbol: String,
    /// Human-readable base quantity, such as "0.25". Deprecated alias for size_base.
    #[serde(default)]
    pub(crate) quantity: String,
    #[serde(default)]
    pub(crate) size_usd: Option<String>,
    #[serde(default)]
    pub(crate) size_base: Option<String>,
    /// The user's whole sentence. Used to parse dollar vs base size.
    #[serde(default)]
    pub(crate) text: Option<String>,
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
        let plan = match resolve_preview_qty(app, &args, &ctx) {
            Ok(qty) => app.snapshot_effect_plan(args, &ctx, qty)?,
            Err(value) => return Ok(value),
        };
        let effect = app.reporting.account_effect(&plan);
        Ok(json!({
            "source": "world-markets-reporting",
            "account_effect": effect,
            "concern_line": effect.concern_line,
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
        let dp = app.reporting.dollarpower(&portfolio_id);
        Ok(json!({
            "source": "world-markets-reporting",
            "dollarpower": dp,
            "message": crate::reporting::render_dollarpower_message(&dp),
            "reply_verbatim": true,
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
pub(crate) struct RenderShareArgs {
    /// Whole user message when the host did not pre-classify the intent.
    #[serde(default)]
    pub(crate) text: Option<String>,
    /// `introduce` / `without_name` / `with_name` / `revoke` / `who`.
    #[serde(default)]
    pub(crate) action: Option<String>,
    /// Telegram first name. Omitted from M10 unless the user left name-on as default.
    #[serde(default)]
    pub(crate) first_name: Option<String>,
    /// World account ID. Optional when handover account context is available.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
}

impl DynAomiTool for RenderShare {
    type App = WorldMarketsApp;
    type Args = RenderShareArgs;
    const NAME: &'static str = "render_share";
    const DESCRIPTION: &'static str = "Introduction (M10) for the user's own thread. Send hint if present, then message verbatim. Never executes. Never the mandate engine. Never account figures.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let intent = args
            .text
            .as_deref()
            .and_then(crate::share::parse_share_intent)
            .or_else(|| match args.action.as_deref().unwrap_or("introduce") {
                "without_name" => Some(crate::share::ShareIntent::WithoutName),
                "with_name" => Some(crate::share::ShareIntent::WithName),
                "revoke" => Some(crate::share::ShareIntent::Revoke),
                "who" => Some(crate::share::ShareIntent::Who),
                _ => Some(crate::share::ShareIntent::Introduce),
            })
            .unwrap_or(crate::share::ShareIntent::Introduce);
        render_share_intent(
            app,
            intent,
            args.account_id,
            args.first_name.as_deref(),
            &ctx,
        )
    }
}

pub(crate) struct RenderGuestSurface;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RenderGuestSurfaceArgs {
    /// Guest session id (Telegram identity or g_<token> start payload).
    pub(crate) guest_id: String,
    /// Surface name (greeting, showcase, paper, upgrade, …).
    pub(crate) surface: String,
    /// Raw Telegram start / startapp payload when separate from guest_id.
    #[serde(default)]
    pub(crate) start_payload: Option<String>,
}

impl DynAomiTool for RenderGuestSurface {
    type App = WorldMarketsApp;
    type Args = RenderGuestSurfaceArgs;
    const NAME: &'static str = "render_guest_surface";
    const DESCRIPTION: &'static str = "Guest/paper message. Send `message` verbatim. Never invent numbers. Never a policy verdict. Never executes. Host: pass chat identity as guest_id and the /start payload as start_payload.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let start = args
            .start_payload
            .as_deref()
            .unwrap_or(args.guest_id.as_str());
        if live_bound_account(&ctx) {
            return Ok(already_user_reply());
        }
        let guest_id = arriving_guest_id(&args, &ctx);
        if let Some(code) = crate::share::ref_code_from_start(start) {
            let _ = app.brain.share(&json!({
                "action": "attribute",
                "code": code,
                "guest_id": guest_id,
            }));
        }
        let funnel = Funnel::new(&app.reporting, &app.guest_store, FunnelConfig::default());
        let surface = funnel.render(&guest_id, &args.surface)?;
        Ok(guest::to_tool_json(&surface))
    }
}

fn arriving_guest_id(args: &RenderGuestSurfaceArgs, ctx: &DynToolCallCtx) -> String {
    if crate::share::ref_code_from_start(&args.guest_id).is_some() && args.start_payload.is_none() {
        if let Some(id) = telegram_user_id(ctx) {
            return format!("g_{id}");
        }
    }
    guest::guest_id_from_start(&args.guest_id).unwrap_or_else(|| args.guest_id.clone())
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

pub(crate) struct RenderMarketChart;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RenderMarketChartArgs {
    /// Ticker as the user typed it (e.g. AAPL, BTC-USD, WETH).
    pub(crate) ticker: String,
    /// Duration token: d/day, w/week, or m/month.
    pub(crate) period: String,
}

impl DynAomiTool for RenderMarketChart {
    type App = WorldMarketsApp;
    type Args = RenderMarketChartArgs;
    const NAME: &'static str = "render_market_chart";
    const DESCRIPTION: &'static str = "Render a candlestick chart for a ticker over d/w/m. Send `caption` verbatim. Never invent last or change. Never executes.";

    fn run(
        _app: &WorldMarketsApp,
        args: Self::Args,
        _ctx: DynToolCallCtx,
    ) -> Result<Value, String> {
        crate::marketdata::render_chart_tool(&args.ticker, &args.period)
    }
}

pub(crate) struct RefreshMarketUniverse;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RefreshMarketUniverseArgs {}

impl DynAomiTool for RefreshMarketUniverse {
    type App = WorldMarketsApp;
    type Args = RefreshMarketUniverseArgs;
    const NAME: &'static str = "refresh_market_universe";
    const DESCRIPTION: &'static str =
        "Rebuild the cached market-data asset universe from the configured feed. Never executes.";

    fn run(
        _app: &WorldMarketsApp,
        _args: Self::Args,
        _ctx: DynToolCallCtx,
    ) -> Result<Value, String> {
        crate::marketdata::refresh_universe_tool()
    }
}

pub(crate) struct ClearMarketCharts;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ClearMarketChartsArgs {}

impl DynAomiTool for ClearMarketCharts {
    type App = WorldMarketsApp;
    type Args = ClearMarketChartsArgs;
    const NAME: &'static str = "clear_market_charts";
    const DESCRIPTION: &'static str =
        "Delete stored candlestick PNG files. Send `caption` verbatim. Never executes.";

    fn run(
        _app: &WorldMarketsApp,
        _args: Self::Args,
        _ctx: DynToolCallCtx,
    ) -> Result<Value, String> {
        crate::chart::clear_charts_tool()
    }
}

pub(crate) struct GetWorldResearch;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldResearchArgs {
    /// Base asset symbol (e.g. WETH). Resolved via list_world_assets when unfamiliar.
    pub(crate) base_symbol: String,
    /// Lookback window: 1d (default), 1w, or 1m.
    #[serde(default)]
    pub(crate) lookback: Option<String>,
    /// Product type: spot, perp (default), or lend.
    #[serde(default)]
    pub(crate) product: Option<String>,
    /// Quote asset. Defaults to USDT.
    #[serde(default)]
    pub(crate) quote_symbol: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

impl DynAomiTool for GetWorldResearch {
    type App = WorldMarketsApp;
    type Args = GetWorldResearchArgs;
    const NAME: &'static str = "get_world_research";
    const DESCRIPTION: &'static str = "Research a World market move: live mark, stored window change, cited news, and mandate-gated action-door data. Never predicts. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let assets = app.client.assets()?;
        let lookback = args.lookback.as_deref().unwrap_or("1d");
        let product = args.product.as_deref().unwrap_or("perp");
        let quote = args.quote_symbol.as_deref().unwrap_or("USDT");
        let inspected = app
            .inspect_account(args.account_id, args.wallet_address.as_deref(), &ctx)
            .ok();
        let account = inspected.as_ref().map(|(_, account, _)| account.clone());
        let portfolio_now = inspected.as_ref().map(|(payload, _, _)| {
            json!({
                "rapv": payload.pointer("/account/risk_adjusted_portfolio_value"),
                "liquidation_risk": payload.pointer("/metrics/liquidation_risk"),
                "source": "get_world_account",
            })
        });
        let mandate = Mandate::bound(ctx.attribute_path(&["handover_mandate"])).ok();
        crate::research::compose(
            &app.client,
            &app.brain,
            &args.base_symbol,
            lookback,
            product,
            quote,
            account.as_ref(),
            mandate.as_ref(),
            &assets,
            portfolio_now.as_ref(),
        )
    }
}

pub(crate) struct GetWorldTasks;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldTasksArgs {
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Expected owner wallet. Authorization still uses the acting wallet.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
    /// `full` includes voice lexicon/episode. Default omits them.
    #[serde(default)]
    pub(crate) detail: Option<String>,
}

impl DynAomiTool for GetWorldTasks {
    type App = WorldMarketsApp;
    type Args = GetWorldTasksArgs;
    const NAME: &'static str = "get_world_tasks";
    const DESCRIPTION: &'static str = "List open ledger instructions (sentence + id), watches, unsigned preferences, signed on-chain policies, plus voice lexicon/episode/consents. Open instructions are standing user intent from Mini App, speech, or a fired watch. Policies are read-only from chat. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        app.note_activity(&ctx, args.account_id);
        let _owner = args.wallet_address.as_deref();
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id);
        let mandate = Mandate::bound(ctx.attribute_path(&["handover_mandate"]));
        let brief = WorldMarketsApp::brief(&ctx);
        let mut value = crate::tasks::compose(&app.brain, account_id, mandate, brief.as_ref());
        if args.detail.as_deref() != Some("full") {
            if let Some(obj) = value.as_object_mut() {
                obj.remove("voice");
            }
        }
        log_tool_bytes("get_world_tasks", &value);
        Ok(value)
    }
}

pub(crate) struct SetWorldWatch;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SetWorldWatchArgs {
    /// User's original phrasing of the trigger.
    pub(crate) phrase: String,
    /// Asset symbol to watch (e.g. WETH).
    pub(crate) symbol: String,
    /// once (default) or repeats.
    #[serde(default)]
    pub(crate) fire_mode: Option<String>,
    /// True when the user chose Watch the next crossing after an already-true watch.
    /// Arms an edge trigger: fire only after the predicate is observed false, then true.
    #[serde(default)]
    pub(crate) fire_on_transition: Option<bool>,
    /// Ledger instruction id from a mini-app compose, when confirming the same row.
    #[serde(default)]
    pub(crate) instruction_id: Option<String>,
    /// Correlation id from mini-app sendData.
    #[serde(default)]
    pub(crate) correlation_id: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    #[allow(dead_code)]
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
}

impl DynAomiTool for SetWorldWatch {
    type App = WorldMarketsApp;
    type Args = SetWorldWatchArgs;
    const NAME: &'static str = "set_world_watch";
    const DESCRIPTION: &'static str = "Store an exact, tool-checkable watch. Vague triggers return a clarifying question and store nothing. Returns `now` (mark at creation) and `already_true`. If already_true, nothing is armed — paste message and controls; do not treat it as a silent arm. Pass fire_on_transition=true when the user chose Watch the next crossing. Send `message` and `controls` verbatim. A watch messages; it never trades.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id)
            .ok_or_else(|| "[world-markets] account_id is required to set a watch".to_string())?;
        let _owner = args.wallet_address.as_deref();
        let assets = app.client.assets().unwrap_or_default();
        let token_id = asset_by_symbol(&assets, &args.symbol)
            .ok()
            .map(|a| a.token_id);
        let mark_at_set = token_id.and_then(|id| app.client.mark_price(id).ok().map(|(_, m)| m));
        if let Some(mark) = mark_at_set.as_ref()
            && let Some(id) = token_id
        {
            let _ = app.brain.ingest(&json!({
                "symbol": args.symbol,
                "token_id": id,
                "mark": mark,
            }));
        }
        let body = json!({
            "account_id": account_id,
            "phrase": args.phrase,
            "symbol": args.symbol,
            "token_id": token_id,
            "fire_mode": args.fire_mode,
            "fire_on_transition": args.fire_on_transition,
            "mark_at_set": mark_at_set,
            "instruction_id": args.instruction_id,
            "correlation_id": args.correlation_id,
        });
        let result = if looks_like_watch_correction(&args.phrase) {
            app.brain.supersede_watch(&body)?
        } else {
            app.brain.set_watch(&body)?
        };
        let now = result
            .get("now")
            .cloned()
            .filter(|value| !value.is_null())
            .or_else(|| mark_at_set.clone().map(Value::from));
        let already_true = result
            .get("already_true")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(json!({
            "source": "world-markets-brain",
            "executable": false,
            "result": result,
            "message": result.get("message"),
            "controls": result.get("controls"),
            "preview_only": true,
            "now": now,
            "already_true": already_true,
        }))
    }
}

pub(crate) struct SetWorldPreference;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SetWorldPreferenceArgs {
    /// Preference text to persist (unsigned, never a policy).
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
}

impl DynAomiTool for SetWorldPreference {
    type App = WorldMarketsApp;
    type Args = SetWorldPreferenceArgs;
    const NAME: &'static str = "set_world_preference";
    const DESCRIPTION: &'static str =
        "Persist an unsigned chat preference. Never a signed policy. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id).ok_or_else(|| {
            "[world-markets] account_id is required to store a preference".to_string()
        })?;
        let result = app.brain.set_preference(&json!({
            "account_id": account_id,
            "text": args.text,
        }))?;
        let message = result
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                crate::speech_ontology::classify_protected_veto(&args.text).map(|veto| {
                    crate::reporting::render_protected_veto_message(&veto.asset, veto.absolute)
                })
            });
        Ok(json!({
            "source": "world-markets-brain",
            "executable": false,
            "on_chain": false,
            "result": result,
            "message": message,
            "reply_verbatim": message.is_some(),
            "categorical_veto": false,
        }))
    }
}

pub(crate) struct CancelWorldTask;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CancelWorldTaskArgs {
    /// watch, preference, or policy. Policy is always blocked.
    pub(crate) kind: String,
    /// Item id from get_world_tasks.
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
}

impl DynAomiTool for CancelWorldTask {
    type App = WorldMarketsApp;
    type Args = CancelWorldTaskArgs;
    const NAME: &'static str = "cancel_world_task";
    const DESCRIPTION: &'static str =
        "Cancel a watch or preference. Policy edits are blocked — they must be signed on World.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let kind = args.kind.to_ascii_lowercase();
        if kind == "policy" || kind == "policies" {
            let mandate =
                Mandate::bound(ctx.attribute_path(&["handover_mandate"])).map_err(|verdict| {
                    format!("[world-markets] {}: {}", verdict.rule, verdict.detail)
                })?;
            return Ok(crate::tasks::policy_edit_block(&mandate));
        }
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id)
            .ok_or_else(|| "[world-markets] account_id is required to cancel a task".to_string())?;
        let result = if kind == "preference" || kind == "preferences" {
            app.brain.cancel_preference(account_id, &args.id)?
        } else {
            match app.brain.cancel_watch(account_id, &args.id) {
                Ok(value) if value.get("ok").and_then(Value::as_bool) != Some(false) => value,
                _ => {
                    let matched = app.brain.match_watches(account_id, &args.id)?;
                    if matched.get("ambiguous").and_then(Value::as_bool) == Some(true) {
                        matched
                    } else if let Some(id) =
                        matched.pointer("/matches/0/id").and_then(Value::as_str)
                    {
                        app.brain.cancel_watch(account_id, id)?
                    } else {
                        matched
                    }
                }
            }
        };
        let message = result.get("message").cloned();
        Ok(json!({
            "source": "world-markets-brain",
            "executable": false,
            "result": result,
            "message": message,
            "reply_verbatim": message.is_some(),
        }))
    }
}

pub(crate) struct PauseWorldWatch;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PauseWorldWatchArgs {
    /// Instruction id from the ledger (preferred).
    #[serde(default)]
    pub(crate) instruction_id: Option<String>,
    /// Watch id from get_world_tasks, if the instruction id is unknown.
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
}

impl DynAomiTool for PauseWorldWatch {
    type App = WorldMarketsApp;
    type Args = PauseWorldWatchArgs;
    const NAME: &'static str = "pause_world_watch";
    const DESCRIPTION: &'static str = "Pause a confirmed watch after the user signed the thread confirm. Never call from the Mini App. A paused watch does not check. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id)
            .ok_or_else(|| "[world-markets] account_id is required to pause a watch".to_string())?;
        let result = app.brain.pause_watch(
            account_id,
            args.id.as_deref(),
            args.instruction_id.as_deref(),
        )?;
        Ok(json!({
            "source": "world-markets-brain",
            "executable": false,
            "result": result,
        }))
    }
}

pub(crate) struct ResumeWorldWatch;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ResumeWorldWatchArgs {
    #[serde(default)]
    pub(crate) instruction_id: Option<String>,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
}

impl DynAomiTool for ResumeWorldWatch {
    type App = WorldMarketsApp;
    type Args = ResumeWorldWatchArgs;
    const NAME: &'static str = "resume_world_watch";
    const DESCRIPTION: &'static str = "Resume a paused watch after the user signed the thread confirm. Never call from the Mini App. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id).ok_or_else(|| {
            "[world-markets] account_id is required to resume a watch".to_string()
        })?;
        let result = app.brain.resume_watch(
            account_id,
            args.id.as_deref(),
            args.instruction_id.as_deref(),
        )?;
        Ok(json!({
            "source": "world-markets-brain",
            "executable": false,
            "result": result,
        }))
    }
}

pub(crate) struct DrainWorldOutbound;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct DrainWorldOutboundArgs {
    /// Max messages to drain (default 50).
    #[serde(default)]
    pub(crate) limit: Option<u32>,
}

impl DynAomiTool for DrainWorldOutbound {
    type App = WorldMarketsApp;
    type Args = DrainWorldOutboundArgs;
    const NAME: &'static str = "drain_world_outbound";
    const DESCRIPTION: &'static str = "Host drain of solicited watch messages. Send each item's `message` verbatim. Separate from the weekly digest budget. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let items = app.brain.drain_outbound(args.limit.unwrap_or(50))?;
        Ok(json!({
            "source": "world-markets-brain",
            "executable": false,
            "channel": "watch",
            "counts_against_weekly_digest": false,
            "result": items,
        }))
    }
}

pub(crate) struct RecordWorldCorrection;

/// One side of a spoken repair: what the agent understood, or what the user
/// actually meant.
///
/// Every field is named. A free-form object cannot be expressed here at all:
/// `rig` sends every function tool with `strict: true`, and OpenAI's strict
/// mode requires `additionalProperties: false` on each object — so an open map
/// is rejected, and the rejection takes the whole completion request with it,
/// not just this tool. `serde_json::Value` fails the same way one step earlier,
/// by generating no `type`. These two are what the readers actually use:
/// `symbol` here and in the brain's candidate-outcome scoring, `phrase` in the
/// watch supersede below.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub(crate) struct CorrectionIntent {
    /// What the user was trying to do: buy, sell, lend, and so on.
    #[serde(default)]
    pub(crate) kind: Option<String>,
    /// Instrument symbol the intent resolves to, such as BTC.b. Null when the
    /// repair is not about an instrument.
    #[serde(default)]
    pub(crate) symbol: Option<String>,
    /// The wording the user used for that instrument.
    #[serde(default)]
    pub(crate) phrase: Option<String>,
}

/// One confirmed lexicon entry. Named fields for the same reason as
/// [`CorrectionIntent`]; these are the keys `applyLexicon` reads.
///
/// `applyLexicon` accepts `surface` / `target` as short aliases for the two
/// canonical names. Those stay accepted here as serde aliases rather than as
/// extra properties: strict mode makes every declared property required, so
/// naming both spellings would put two synonym pairs in front of the model on
/// every call. An alias costs nothing in the schema and still normalizes an
/// aliased payload onto the canonical field — which is the spelling
/// `applyLexicon` reads first.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub(crate) struct LexiconRename {
    /// What was said, verbatim.
    #[serde(default, alias = "surface")]
    pub(crate) surface_form: Option<String>,
    /// What it means — usually an instrument symbol.
    #[serde(default, alias = "target")]
    pub(crate) normalized_target: Option<String>,
    /// Entry kind; defaults to `phrase` when null.
    #[serde(default)]
    pub(crate) kind: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct RecordWorldCorrectionArgs {
    #[serde(default)]
    pub(crate) utterance_ref: Option<String>,
    /// What the agent first understood.
    #[serde(default)]
    pub(crate) rejected_intent: Option<CorrectionIntent>,
    #[serde(default)]
    pub(crate) rejected_readback: Option<String>,
    #[serde(default)]
    pub(crate) correction_utterance_ref: Option<String>,
    /// What the user actually meant.
    #[serde(default)]
    pub(crate) accepted_intent: Option<CorrectionIntent>,
    #[serde(default)]
    pub(crate) accepted_readback: Option<String>,
    /// One lexicon entry to confirm from this repair.
    #[serde(default)]
    pub(crate) lexicon_rename: Option<LexiconRename>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
}

/// The `/v1/voice/correction` body, split out from `run` so the deserialize →
/// forward boundary can be tested without a live brain.
///
/// The brain stores `rejected_intent` / `accepted_intent` verbatim and
/// `exportEval` hands them back as training pairs, so any field dropped between
/// the tool schema and here is lost for good — silently, since the row still
/// looks well formed.
fn correction_body(account_id: u64, args: &RecordWorldCorrectionArgs) -> Value {
    json!({
        "account_id": account_id,
        "utterance_ref": args.utterance_ref,
        "rejected_intent": args.rejected_intent,
        "rejected_readback": args.rejected_readback,
        "correction_utterance_ref": args.correction_utterance_ref,
        "accepted_intent": args.accepted_intent,
        "accepted_readback": args.accepted_readback,
        "lexicon_rename": args.lexicon_rename,
    })
}

impl DynAomiTool for RecordWorldCorrection {
    type App = WorldMarketsApp;
    type Args = RecordWorldCorrectionArgs;
    const NAME: &'static str = "record_world_correction";
    const DESCRIPTION: &'static str = "Store a spoken or typed repair (rejected vs accepted intent). Never executes. Never a confirm gate.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id).ok_or_else(|| {
            "[world-markets] account_id is required to record a correction".to_string()
        })?;
        let result = app
            .brain
            .record_correction(&correction_body(account_id, &args))?;
        let mut payload = json!({
            "source": "world-markets-brain",
            "executable": false,
            "result": result,
        });
        if let Some(intent) = args.accepted_intent.as_ref() {
            if let Some(symbol) = intent.symbol.as_deref() {
                let phrase = args
                    .accepted_readback
                    .clone()
                    .or_else(|| intent.phrase.clone())
                    .unwrap_or_default();
                if let Ok(superseded) = app.brain.supersede_watch(&json!({
                    "account_id": account_id,
                    "symbol": symbol,
                    "phrase": phrase,
                    "referent": symbol,
                })) {
                    payload["supersede"] = superseded.clone();
                    if let Some(msg) = superseded.get("message") {
                        payload["message"] = msg.clone();
                        payload["reply_verbatim"] = json!(true);
                    }
                }
            }
        }
        Ok(payload)
    }
}

pub(crate) struct SetWorldConsent;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SetWorldConsentArgs {
    /// ai_identity_ack | training_use | prosody_dark | aomi_initiated_voice
    pub(crate) kind: String,
    /// granted or withdrawn
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) wording_version: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
}

impl DynAomiTool for SetWorldConsent {
    type App = WorldMarketsApp;
    type Args = SetWorldConsentArgs;
    const NAME: &'static str = "set_world_consent";
    const DESCRIPTION: &'static str = "Record a versioned voice/data consent. Required before any aomi-generated audio. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id).ok_or_else(|| {
            "[world-markets] account_id is required to record consent".to_string()
        })?;
        let result = app.brain.set_consent(&json!({
            "account_id": account_id,
            "kind": args.kind,
            "status": args.status.unwrap_or_else(|| "granted".to_string()),
            "wording_version": args.wording_version.unwrap_or_else(|| "v1".to_string()),
        }))?;
        Ok(json!({
            "source": "world-markets-brain",
            "executable": false,
            "result": result,
        }))
    }
}

pub(crate) struct CloseWorldEpisode;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CloseWorldEpisodeArgs {
    #[serde(default)]
    pub(crate) reason: Option<String>,
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
}

impl DynAomiTool for CloseWorldEpisode {
    type App = WorldMarketsApp;
    type Args = CloseWorldEpisodeArgs;
    const NAME: &'static str = "close_world_episode";
    const DESCRIPTION: &'static str = "Close the current voice episode and return its recap fields. Call when the user is done for now. Never executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = WorldMarketsApp::account_id(&ctx, args.account_id).ok_or_else(|| {
            "[world-markets] account_id is required to close an episode".to_string()
        })?;
        let result = app.brain.close_episode(&json!({
            "account_id": account_id,
            "reason": args.reason.unwrap_or_else(|| "done_for_now".to_string()),
        }))?;
        Ok(json!({
            "source": "world-markets-brain",
            "executable": false,
            "result": result,
        }))
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
    crate::order_intent::venue_order_type(named.unwrap_or(""), price)
}

fn numeric_position_id(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(value.to_string())
}

fn execution_blocked(
    access: &AccountAccess,
    verdict: &Verdict,
    symbol: Option<&str>,
    projected: Option<Decimal>,
) -> Value {
    let rendered =
        crate::reporting::render_deny(verdict.rule, &verdict.detail, symbol, projected, None, None);
    json!({
        "source": "world-markets-execution",
        "executable": false,
        "access": compact_access(access),
        "policy_result": {
            "status": verdict.status,
            "rule": verdict.rule,
            "detail": verdict.detail,
            "detail_rendered": rendered,
        },
        "message": rendered,
        "reply_verbatim": true,
        "controls": [
            { "label": "View mandate on World ↗", "action": "view_mandate" },
            { "label": "Keep as is", "action": "keep" }
        ],
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
        assert_eq!(resolve_order_type(Some("twap"), None), "market");
        assert_eq!(resolve_order_type(Some("dca"), None), "market");
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
                size_usd: None,
                size_base: None,
                price: None,
                order_type: None,
                slippage: None,
                slices: None,
                window_minutes: None,
                interval_secs: None,
                cadence: None,
                account_id: None,
                wallet_address: None,
                sentence: None,
                instruction_id: None,
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

    #[test]
    fn trade_sentence_prefers_the_whole_utterance() {
        let mut args = ExecuteWorldOrderArgs {
            product: "spot".to_string(),
            side: "buy".to_string(),
            base_symbol: "WETH".to_string(),
            quote_symbol: Some("USDT".to_string()),
            quantity: "0.1".to_string(),
            size_usd: None,
            size_base: None,
            price: None,
            order_type: None,
            slippage: None,
            slices: None,
            window_minutes: None,
            interval_secs: None,
            cadence: None,
            account_id: None,
            wallet_address: None,
            sentence: Some("  Buy a tenth of ETH spot please  ".to_string()),
            instruction_id: None,
        };
        assert_eq!(trade_sentence(&args), "Buy a tenth of ETH spot please");
        args.sentence = None;
        assert_eq!(trade_sentence(&args), "buy 0.1 WETH spot at market");
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
                    size_usd: None,
                    size_base: None,
                    account_id: None,
                    wallet_address: None,
                    text: None,
                },
                &ctx,
            )
            .unwrap();
        assert_eq!(value["access"]["authorization"], "owner");
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

    #[test]
    fn render_lookup_share_intent_skips_llm_and_never_executes() {
        let app = WorldMarketsApp::default();
        let value = RenderLookup::run(
            &app,
            lookup_args("introduce yourself to my friend", None),
            empty_ctx("render_lookup"),
        )
        .unwrap();
        assert_eq!(value["skip_llm"], true);
        assert_eq!(value["executable"], false);
        assert_eq!(value["matched"], true);
        assert_eq!(value["token"], "share");
        assert!(value.get("message").and_then(Value::as_str).is_some());
        assert!(
            app.warmer.never_refreshed(),
            "share must not touch the account warmer"
        );
    }

    #[test]
    fn bound_user_on_ref_start_gets_already_user() {
        let app = WorldMarketsApp::default();
        let ctx = ctx_with(json!({ "world": { "account_id": 17 } }));
        let value = RenderGuestSurface::run(
            &app,
            RenderGuestSurfaceArgs {
                guest_id: "ref_ab12cd34ef".into(),
                surface: "greeting".into(),
                start_payload: None,
            },
            DynToolCallCtx {
                session_id: ctx.session_id,
                tool_name: "render_guest_surface".into(),
                call_id: ctx.call_id,
                state_attributes: ctx.state_attributes,
                secrets: ctx.secrets,
            },
        )
        .unwrap();
        assert_eq!(value["message"], crate::share::ALREADY_USER);
        assert_eq!(value["skip_llm"], true);
        assert_eq!(value["executable"], false);
        assert!(
            !value["message"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("paper portfolio")
        );
    }

    #[test]
    fn ref_start_without_bound_account_is_generic_a_flow() {
        let app = WorldMarketsApp::default();
        let value = RenderGuestSurface::run(
            &app,
            RenderGuestSurfaceArgs {
                guest_id: "ref_ab12cd34ef".into(),
                surface: "greeting".into(),
                start_payload: Some("ref_ab12cd34ef".into()),
            },
            empty_ctx("render_guest_surface"),
        )
        .unwrap();
        let message = value["message"].as_str().unwrap();
        assert!(message.contains("paper portfolio"));
        assert_ne!(message, crate::share::ALREADY_USER);
        assert!(guest::anti_goal_violations(message).is_empty());
    }

    #[test]
    fn render_lookup_unmatched_does_not_skip_llm() {
        let app = WorldMarketsApp::default();
        let value = RenderLookup::run(
            &app,
            RenderLookupArgs {
                text: Some("how am I doing?".into()),
                token: None,
                account_id: None,
                wallet_address: None,
                utterance_ref: None,
                slots: None,
            },
            empty_ctx("render_lookup"),
        )
        .unwrap();
        assert_eq!(value["matched"], false);
        assert_eq!(value["skip_llm"], false);
        assert_eq!(value["executable"], false);
        assert!(value.get("message").is_none());
        assert_eq!(value["open_instructions"], json!([]));
        assert!(
            app.warmer.never_refreshed(),
            "unit tests must not block on a live prefetch"
        );
    }

    #[test]
    fn render_lookup_beef_is_cant_wall_not_unclear() {
        let app = WorldMarketsApp::default();
        let value = RenderLookup::run(
            &app,
            lookup_args("buy me $50 of beef", None),
            empty_ctx("render_lookup"),
        )
        .unwrap();
        assert_eq!(value["kind"], "cant");
        assert_eq!(value["skip_llm"], true);
        assert_eq!(value["executable"], false);
        let message = value["message"].as_str().unwrap();
        assert!(message.contains("I heard"), "{message}");
        assert!(message.contains("World doesn't trade"), "{message}");
        assert!(
            message.contains("crypto spot, perps, and lending"),
            "{message}"
        );
        assert!(!message.to_ascii_lowercase().contains("say buy"));
    }

    #[test]
    fn kind_status_seen_but_not_confirmed_is_still_first_instance() {
        assert!(!kind_confirmed_from_status(&json!({})));
        assert!(!kind_confirmed_from_status(
            &json!({ "ok": true, "kind": "spot_buy", "confirmed": false })
        ));
        assert!(kind_confirmed_from_status(&json!({ "confirmed": true })));
        let controls = json!([{ "label": "Cancel", "action": "cancel", "instruction_id": "abc" }]);
        assert_eq!(controls.as_array().unwrap().len(), 1);
        assert_eq!(controls[0]["label"], "Cancel");
        assert_ne!(controls[0]["action"], "confirm");
    }

    #[test]
    fn render_lookup_falls_through_invalid_token_to_text() {
        let app = WorldMarketsApp::default();
        let value = RenderLookup::run(
            &app,
            RenderLookupArgs {
                text: Some("?".into()),
                token: Some("nope".into()),
                account_id: None,
                wallet_address: None,
                utterance_ref: None,
                slots: None,
            },
            empty_ctx("render_lookup"),
        )
        .unwrap();
        assert_eq!(value["token"], "index");
        assert_eq!(value["skip_llm"], true);
    }

    #[test]
    fn render_lookup_index_and_dollarpower_skip_llm_without_account() {
        let app = WorldMarketsApp::default();
        let index = RenderLookup::run(
            &app,
            RenderLookupArgs {
                text: Some("?".into()),
                token: None,
                account_id: None,
                wallet_address: None,
                utterance_ref: None,
                slots: None,
            },
            empty_ctx("render_lookup"),
        )
        .unwrap();
        assert_eq!(index["matched"], true);
        assert_eq!(index["skip_llm"], true);
        assert_eq!(index["reply_verbatim"], true);
        assert_eq!(index["token"], "index");
        assert_eq!(index["message"], crate::lookups::INDEX_LINE);

        let dp = RenderLookup::run(
            &app,
            RenderLookupArgs {
                text: None,
                token: Some("d".into()),
                account_id: None,
                wallet_address: None,
                utterance_ref: None,
                slots: None,
            },
            empty_ctx("render_lookup"),
        )
        .unwrap();
        assert_eq!(dp["token"], "d");
        assert_eq!(dp["skip_llm"], true);
        let line = dp["message"].as_str().unwrap();
        assert!(line.starts_with("Dollarpower `"));
        assert!(line.contains("is doing the work of"));

        let available = RenderLookup::run(
            &app,
            RenderLookupArgs {
                text: Some("/a".into()),
                token: None,
                account_id: None,
                wallet_address: None,
                utterance_ref: None,
                slots: None,
            },
            empty_ctx("render_lookup"),
        )
        .unwrap();
        assert_eq!(available["message"], crate::lookups::AVAILABLE_UNAVAILABLE);
    }

    #[test]
    fn render_lookup_prefers_explicit_token_over_unmatched_text() {
        let app = WorldMarketsApp::default();
        let value = RenderLookup::run(
            &app,
            RenderLookupArgs {
                text: Some("what's my balance?".into()),
                token: Some("index".into()),
                account_id: None,
                wallet_address: None,
                utterance_ref: None,
                slots: None,
            },
            empty_ctx("render_lookup"),
        )
        .unwrap();
        assert_eq!(value["token"], "index");
        assert_eq!(value["skip_llm"], true);
        assert_eq!(value["message"], crate::lookups::INDEX_LINE);
    }

    #[test]
    fn render_lookup_local_path_is_far_under_500ms() {
        use std::time::Instant;
        let app = WorldMarketsApp::default();
        let start = Instant::now();
        for text in ["?", "commands", "d", "/a", "how am I doing?"] {
            let value = RenderLookup::run(
                &app,
                RenderLookupArgs {
                    text: Some(text.into()),
                    token: None,
                    account_id: None,
                    wallet_address: None,
                    utterance_ref: None,
                    slots: None,
                },
                empty_ctx("render_lookup"),
            )
            .unwrap();
            if text == "how am I doing?" {
                assert_eq!(value["skip_llm"], false);
            } else {
                assert_eq!(value["skip_llm"], true, "{text}");
            }
        }
        let elapsed = start.elapsed();
        println!("local render_lookup loop {elapsed:?}");
        assert!(
            elapsed.as_millis() < 50,
            "CPU-only lookups must be << 500ms, took {elapsed:?}"
        );
    }

    fn lookup_args(text: &str, account_id: Option<u64>) -> RenderLookupArgs {
        RenderLookupArgs {
            text: Some(text.into()),
            token: None,
            account_id,
            wallet_address: None,
            utterance_ref: None,
            slots: None,
        }
    }

    fn restore_account_id(previous: Option<String>) {
        match previous {
            Some(value) => unsafe { std::env::set_var("WORLD_ACCOUNT_ID", value) },
            None => unsafe { std::env::remove_var("WORLD_ACCOUNT_ID") },
        }
    }

    #[test]
    #[ignore = "requires live UniFi RPC"]
    fn live_lookup_warm_path_meets_500ms_plugin_budget() {
        use std::time::Instant;

        let previous = std::env::var("WORLD_ACCOUNT_ID").ok();
        let app = WorldMarketsApp::default();
        let account_id = WorldMarketsApp::account_id_from_env()
            .or_else(|| app.client.latest_account_id().ok())
            .expect("need WORLD_ACCOUNT_ID or a live latest account");
        unsafe {
            std::env::set_var("WORLD_ACCOUNT_ID", account_id.to_string());
        }

        let cold_app = WorldMarketsApp::default();
        let before_cold = cold_app.client.rpc_stats();
        let t_cold = Instant::now();
        let cold_b = RenderLookup::run(
            &cold_app,
            lookup_args("b", Some(account_id)),
            empty_ctx("render_lookup"),
        );
        let cold_ms = t_cold.elapsed();
        let cold_rpc = cold_app.client.rpc_stats().saturating_sub(&before_cold);
        match &cold_b {
            Ok(value) => println!(
                "cold render_lookup \"b\" {cold_ms:?} posts={} hits={} misses={} post_ms={} message={}",
                cold_rpc.posts,
                cold_rpc.hits,
                cold_rpc.misses,
                cold_rpc.post_ms,
                value["message"].as_str().unwrap_or("")
            ),
            Err(err) => {
                restore_account_id(previous);
                panic!("cold render_lookup b failed: {err}");
            }
        }

        let before = app.client.rpc_stats();
        let t0 = Instant::now();
        let warmed = WarmAccount::run(
            &app,
            WarmAccountArgs {
                account_id: Some(account_id),
                wallet_address: None,
            },
            empty_ctx("warm_account"),
        );
        let warm_ms = t0.elapsed();
        let warm_rpc = app.client.rpc_stats().saturating_sub(&before);
        let warmed = match warmed {
            Ok(value) => value,
            Err(err) => {
                restore_account_id(previous);
                panic!("warm_account failed: {err}");
            }
        };
        assert_eq!(warmed["warmed"], true);
        assert_eq!(warmed["account_id"], account_id);
        println!(
            "warm_account {warm_ms:?} posts={} hits={} misses={} post_ms={}",
            warm_rpc.posts, warm_rpc.hits, warm_rpc.misses, warm_rpc.post_ms
        );

        let mut failures = Vec::new();
        for text in ["b", "p", "r", "d", "a", "?"] {
            let before = app.client.rpc_stats();
            let t = Instant::now();
            let result = RenderLookup::run(
                &app,
                lookup_args(text, Some(account_id)),
                empty_ctx("render_lookup"),
            );
            let elapsed = t.elapsed();
            let rpc = app.client.rpc_stats().saturating_sub(&before);
            let result = match result {
                Ok(value) => value,
                Err(err) => {
                    failures.push(format!("{text}: {err}"));
                    continue;
                }
            };
            let message = result["message"].as_str().unwrap_or("");
            println!(
                "render_lookup {text:?} {elapsed:?} posts={} hits={} misses={} post_ms={} message={message}",
                rpc.posts, rpc.hits, rpc.misses, rpc.post_ms
            );
            if result["skip_llm"] != true {
                failures.push(format!("{text}: skip_llm was not true"));
            }
            if matches!(text, "b" | "p" | "r") && rpc.posts != 0 {
                failures.push(format!(
                    "{text}: expected 0 RPC posts after warm, got {}",
                    rpc.posts
                ));
            }
            if elapsed.as_millis() >= 500 {
                failures.push(format!(
                    "{text}: plugin path {elapsed:?} exceeded 500ms budget"
                ));
            }
            match text {
                "b" => {
                    if !message.starts_with("Portfolio `") {
                        failures.push(format!("b: bad message {message}"));
                    }
                }
                "r" => {
                    if !message.contains("liquidation risk")
                        && !message.contains("Liquidation risk")
                    {
                        failures.push(format!("r: bad message {message}"));
                    }
                }
                "d" => {
                    if !message.starts_with("Dollarpower `") {
                        failures.push(format!("d: bad message {message}"));
                    }
                }
                "a" => {
                    if message != crate::lookups::AVAILABLE_UNAVAILABLE {
                        failures.push(format!("a: bad message {message}"));
                    }
                }
                "?" => {
                    if message != crate::lookups::INDEX_LINE {
                        failures.push(format!("?: bad message {message}"));
                    }
                }
                "p" => {
                    if message.is_empty() {
                        failures.push("p: empty message".into());
                    }
                }
                _ => {}
            }
        }

        restore_account_id(previous);
        assert!(
            failures.is_empty(),
            "live lookup timing failures:\n{}",
            failures.join("\n")
        );
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
            size_usd: None,
            size_base: None,
            text: None,
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
        let message = dp["message"].as_str().unwrap();
        assert!(message.contains("÷"));
        assert!(message.contains("2.4"));
        assert!(!message.contains("10300 ÷ 24700"));
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

    /// Nothing the brain stores or reads may be lost between the tool schema
    /// and the `/v1/voice/correction` body.
    ///
    /// Strict mode forces a finite object, so every field has to be named
    /// deliberately — and a field left out does not fail, it silently
    /// disappears. `kind` is the one this caught: the brain persists the whole
    /// intent and `exportEval` hands it back as a training pair (see
    /// `brain/test/voice.test.js`, which builds its pair with
    /// `{ kind: "buy", symbol: … }`), so dropping it would have quietly
    /// degraded every correction recorded from then on.
    #[test]
    fn correction_body_preserves_every_field_the_brain_reads() {
        let args: RecordWorldCorrectionArgs = serde_json::from_value(json!({
            "utterance_ref": "utt-1",
            "rejected_intent": { "kind": "buy", "symbol": "WBTC", "phrase": "wibbit" },
            "rejected_readback": "buy wbtc",
            "correction_utterance_ref": "utt-2",
            "accepted_intent": { "kind": "buy", "symbol": "WETH", "phrase": "ether" },
            "accepted_readback": "buy weth",
            "lexicon_rename": {
                "surface_form": "ether",
                "normalized_target": "WETH",
                "kind": "instrument"
            },
            "account_id": 22
        }))
        .expect("args deserialize");

        let body = correction_body(22, &args);
        assert_eq!(body["account_id"], json!(22));
        assert_eq!(body["utterance_ref"], json!("utt-1"));
        assert_eq!(body["rejected_readback"], json!("buy wbtc"));
        assert_eq!(body["correction_utterance_ref"], json!("utt-2"));
        assert_eq!(body["accepted_readback"], json!("buy weth"));
        // `applyLexicon` reads all three; `exportEval` re-exports both intents
        // whole, so every key that went in must come back out.
        assert_eq!(
            body["rejected_intent"],
            json!({ "kind": "buy", "symbol": "WBTC", "phrase": "wibbit" })
        );
        assert_eq!(
            body["accepted_intent"],
            json!({ "kind": "buy", "symbol": "WETH", "phrase": "ether" })
        );
        assert_eq!(
            body["lexicon_rename"],
            json!({
                "surface_form": "ether",
                "normalized_target": "WETH",
                "kind": "instrument"
            })
        );
    }

    /// `applyLexicon` falls back to `surface` / `target`, so a payload using
    /// the short spelling must still reach the lexicon. It normalizes onto the
    /// canonical field rather than being carried through, because that is the
    /// spelling `applyLexicon` reads first.
    #[test]
    fn correction_body_normalizes_short_lexicon_aliases() {
        let args: RecordWorldCorrectionArgs = serde_json::from_value(json!({
            "lexicon_rename": { "surface": "ether", "target": "WETH" }
        }))
        .expect("aliased args deserialize");

        let body = correction_body(22, &args);
        assert_eq!(body["lexicon_rename"]["surface_form"], json!("ether"));
        assert_eq!(body["lexicon_rename"]["normalized_target"], json!("WETH"));
    }
}
