use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::*;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::{Account, AccountAccess, CHAIN_ID, WorldClient, asset_by_symbol};
use crate::mandate::{Mandate, TradeFacts, Verdict, parse_decimal};

#[derive(Clone, Default)]
pub(crate) struct WorldMarketsApp {
    client: WorldClient,
}

pub(crate) struct ListWorldAssets;

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(extend("properties" = {}))]
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

impl WorldMarketsApp {
    fn account_id(ctx: &DynToolCallCtx, explicit: Option<u64>) -> Option<u64> {
        explicit
            .or_else(|| ctx.attribute_u64(&["world", "account_id"]))
            .or_else(|| value_u64(ctx.attribute_path(&["handover_account_id"])))
            .or_else(|| value_u64(ctx.attribute_path(&["platform_account_ref"])))
            .or_else(|| value_u64(ctx.attribute_path(&["handover_account_ref"])))
            .or_else(|| ctx.attribute_u64(&["handover_mandate", "account", "id"]))
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
        let mandate = Mandate::parse(ctx.attribute_path(&["handover_mandate"]));
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
                post_trade_risk_adjusted_portfolio_value: None,
                eligible_for_liquidation: account.eligible_for_liquidation,
            }),
            Err(verdict) => verdict,
        };
        let estimated_notional = quantity.checked_mul(mark_price).ok_or_else(|| {
            "[world-markets] estimated notional exceeds numeric range".to_string()
        })?;
        let status = if verdict.is_allow() {
            "policy_allowed_preview_only"
        } else {
            "policy_denied"
        };
        let reason = if verdict.is_allow() {
            "The deterministic mandate permits this intent, but this release remains non-executable until the host transaction boundary cannot bypass app policy."
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
                "post_trade_risk_adjusted_portfolio_value": null,
                "pre_execution_eligible_for_liquidation": account.eligible_for_liquidation,
                "policy_result": verdict,
                "executable": false,
                "status": status,
                "reason": reason,
            }
        }))
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
        Ok(json!({
            "source": "world-markets-contract",
            "chain_id": CHAIN_ID,
            "exchange": app.client.exchange(),
            "block_number": app.client.block_number()?,
            "standing_brief": WorldMarketsApp::brief(&ctx),
            "access": access,
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
        _ => None,
    }
    .unwrap_or("0");
    parse_decimal(value, "current_position_quantity")
        .map_err(|verdict: Verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_and_prefixed_account_references() {
        assert_eq!(value_u64(Some(&json!(42))), Some(42));
        assert_eq!(value_u64(Some(&json!("42"))), Some(42));
        assert_eq!(value_u64(Some(&json!("world-42"))), Some(42));
        assert_eq!(value_u64(Some(&json!("other-42"))), None);
    }

    #[test]
    #[ignore = "requires live MegaETH RPC"]
    fn live_preview_uses_actor_account_and_mandate_context() {
        let app = WorldMarketsApp::default();
        let account_id = 1_577;
        let owner = app.client.owner_for(account_id).unwrap();
        let attributes = json!({
            "domain": { "evm": { "address": format!("{owner:#x}") } },
            "handover_mandate": {
                "version": 1,
                "markets": [{ "product": "perp", "base": "WETH", "quote": "USDm" }],
                "max_position_notional": { "amount": "25000", "quote": "USDm" },
                "max_leverage": "3",
                "min_risk_adjusted_portfolio_value": { "amount": "1", "quote": "USDm" },
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
                    quote_symbol: "USDm".to_string(),
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
}
