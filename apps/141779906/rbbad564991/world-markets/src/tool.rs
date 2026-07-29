use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::*;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{CHAIN_ID, WorldClient, asset_by_symbol};

#[derive(Clone, Default)]
pub(crate) struct WorldMarketsApp {
    client: WorldClient,
}

pub(crate) struct ListWorldAssets;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ListWorldAssetsArgs {}

pub(crate) struct GetWorldAccount;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct GetWorldAccountArgs {
    /// World account ID. Optional when a wallet is supplied or connected.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// EVM owner wallet. Optional when account context or a connected wallet exists.
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

pub(crate) struct PreviewWorldTrade;

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct PreviewWorldTradeArgs {
    /// Product type: spot or perp.
    pub(crate) product: String,
    /// buy or sell.
    pub(crate) side: String,
    /// Base asset symbol.
    pub(crate) base_symbol: String,
    /// Quote asset symbol.
    pub(crate) quote_symbol: String,
    /// Human-readable base quantity, such as "0.25".
    pub(crate) quantity: String,
    /// Optional World account ID.
    #[serde(default)]
    pub(crate) account_id: Option<u64>,
    /// Optional owner wallet. The connected wallet is used when omitted.
    #[serde(default)]
    pub(crate) wallet_address: Option<String>,
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
    const DESCRIPTION: &'static str = "Inspect a live World account: owner, balances, available and reserved amounts, lending and borrowing aggregates, perpetual positions, and risk-adjusted portfolio value.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let account_id = args
            .account_id
            .or_else(|| ctx.attribute_u64(&["world", "account_id"]));
        let wallet = args
            .wallet_address
            .or_else(|| ctx.attribute_string(&["world", "owner_wallet"]))
            .or_else(|| ctx.attribute_string(&["domain", "evm", "address"]));
        let account_id = app.client.resolve_account(account_id, wallet.as_deref())?;
        let assets = app.client.assets()?;
        let account = app.client.account(account_id, &assets)?;
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": app.client.block_number()?,
            "account": account,
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
    type Args = PreviewWorldTradeArgs;
    const NAME: &'static str = "preview_world_trade";
    const DESCRIPTION: &'static str = "Build a deterministic read-only World trade preview from live market and account data. It never approves or executes.";

    fn run(app: &WorldMarketsApp, args: Self::Args, ctx: DynToolCallCtx) -> Result<Value, String> {
        let product = args.product.to_ascii_lowercase();
        if !matches!(product.as_str(), "spot" | "perp" | "perpetual") {
            return Err(
                "[world-markets] trade previews currently support spot and perp only".to_string(),
            );
        }
        let side = args.side.to_ascii_lowercase();
        if !matches!(side.as_str(), "buy" | "sell") {
            return Err("[world-markets] side must be buy or sell".to_string());
        }
        let quantity = args
            .quantity
            .parse::<f64>()
            .map_err(|e| format!("[world-markets] invalid quantity: {e}"))?;
        if !quantity.is_finite() || quantity <= 0.0 {
            return Err("[world-markets] quantity must be a positive number".to_string());
        }

        let account_id = args
            .account_id
            .or_else(|| ctx.attribute_u64(&["world", "account_id"]));
        let wallet = args
            .wallet_address
            .or_else(|| ctx.attribute_string(&["world", "owner_wallet"]))
            .or_else(|| ctx.attribute_string(&["domain", "evm", "address"]));
        let account_id = app.client.resolve_account(account_id, wallet.as_deref())?;
        let assets = app.client.assets()?;
        let account = app.client.account(account_id, &assets)?;
        let base = asset_by_symbol(&assets, &args.base_symbol)?;
        let quote = asset_by_symbol(&assets, &args.quote_symbol)?;
        let market = app
            .client
            .market(&product, base.clone(), Some(quote.clone()))?;
        let mark_price = market
            .mark_price
            .parse::<f64>()
            .map_err(|e| format!("[world-markets] invalid mark price: {e}"))?;
        let estimated_notional = quantity * mark_price;

        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": app.client.block_number()?,
            "preview": {
                "account_id": account.account_id,
                "owner": account.owner,
                "product": product,
                "side": side,
                "base_symbol": base.symbol,
                "quote_symbol": quote.symbol,
                "quantity": args.quantity,
                "mark_price": market.mark_price,
                "estimated_notional": estimated_notional.to_string(),
                "order_book": market.book,
                "pre_execution_risk_adjusted_portfolio_value": account.risk_adjusted_portfolio_value,
                "pre_execution_eligible_for_liquidation": account.eligible_for_liquidation,
                "policy_result": "not_evaluated",
                "executable": false,
                "status": "preview_only",
                "reason": "Account-level World mandate and deterministic policy approval are not connected in this version."
            }
        }))
    }
}
