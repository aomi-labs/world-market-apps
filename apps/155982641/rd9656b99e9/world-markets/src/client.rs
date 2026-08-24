use std::{env, str::FromStr};

use alloy_primitives::{Address, I256, U256, hex};
use alloy_sol_types::{SolCall, sol};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_RPC_URL: &str = "https://testnet-unifi-rpc.puffer.fi/";
const DEFAULT_EXCHANGE: &str = "0xf6b54e033bb45a583aa642924bcef78b804588ae";
pub(crate) const CHAIN_ID: u64 = 2092151908;
/// Quote token (USDT). On-chain `getMarkPrice` reverts for this id — treat as 1.0.
pub(crate) const BASE_TOKEN_ID: u32 = 1;

sol! {
    function getUserId(address userAddress) external view returns (uint64);
    function getUserAddress(uint64 userId) external view returns (address);
    function getBalance(uint64 user, uint32 tokenId)
        external view returns (uint128 balance, uint64 spotLendSequestered, uint64 perpSequestered);
    function getHighestTokenId() external view returns (uint32);
    function riskAdjustedPortfolioValue(uint64 userId) external view returns (int64);
    function userBits(uint64 userId) external view returns (uint256 debt, uint256 noDebt);
    function getSpotOrderBook(uint32 token1, uint32 token2)
        external view returns (address book, uint32 buyToken, uint32 payToken);
    function getPerpOrderBook(uint32 token1, uint32 token2)
        external view returns (address book, uint32 buyToken, uint32 payToken);
    function getLendOrderBook(uint32 tokenId) external view returns (address book);
    function getMarkPrice(uint32 tokenId) external view returns (uint64 price);
    function readLendingAggregation(uint64 userId, uint32 tokenId)
        external view returns (uint256 position);
    function readPerpAggPosition(uint64 userId, uint32 tokenId)
        external view returns (uint256 position, int256 owedBase);
    function bulkTraders_5523718714(uint64 userId) external view returns (address[] traders);
    function bulkReadTokenConfigs_3423260018() external view returns (uint256[] configs);
    function bulkReadMaxUserId_5445644137() external view returns (uint64 maxUserId);
    function searchBuyOrders(uint64 userId, uint32 maxDepth, uint32 maxOrders, uint64 restartPosition)
        external view returns (uint256[] orders);
    function searchSellOrders(uint64 userId, uint32 maxDepth, uint32 maxOrders, uint64 restartPosition)
        external view returns (uint256[] orders);
    function readFundingRateHistory_4648699482(uint64 startTime, uint64 endTime, uint32 tokenId)
        external view returns (uint64[] rates);
    function bestBidOffer() external view returns (uint256 packed);
    function retrieveBuyDepthChart(uint32 maxDepth) external view returns (uint256[] levels);
    function retrieveSellDepthChart(uint32 maxDepth) external view returns (uint256[] levels);
    function readLenderPositions(uint64 userId, uint32 tokenId) external view returns (uint256[] positions);
    function readBorrowerPositions(uint64 userId, uint32 tokenId) external view returns (uint256[] positions);
    function readLendingPosition(uint64 positionId) external view returns (uint256 position);
}

#[derive(Clone)]
pub(crate) struct WorldClient {
    http: Client,
    rpc_url: String,
    exchange: Address,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Asset {
    pub(crate) token_id: u32,
    pub(crate) symbol: String,
    pub(crate) name: String,
    pub(crate) token_type: String,
    pub(crate) erc20_address: String,
    pub(crate) erc20_decimals: u8,
    pub(crate) vault_decimals: u8,
    pub(crate) position_decimals: u8,
    pub(crate) risk_price_percent: u8,
    pub(crate) risk_slippage_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Balance {
    pub(crate) token_id: u32,
    pub(crate) symbol: String,
    pub(crate) balance_raw: String,
    pub(crate) balance: String,
    pub(crate) available_raw: String,
    pub(crate) available: String,
    pub(crate) spot_lend_sequestered_raw: String,
    pub(crate) spot_lend_sequestered: String,
    pub(crate) perp_sequestered_raw: String,
    pub(crate) perp_sequestered: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Account {
    pub(crate) account_id: u64,
    pub(crate) owner: String,
    pub(crate) risk_adjusted_portfolio_value_raw: i64,
    pub(crate) risk_adjusted_portfolio_value: String,
    pub(crate) eligible_for_liquidation: bool,
    pub(crate) balances: Vec<Balance>,
    pub(crate) lending_positions: Vec<LendingPosition>,
    pub(crate) perpetual_positions: Vec<PerpetualPosition>,
    pub(crate) debt_token_ids: Vec<u32>,
    pub(crate) non_debt_token_ids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AccountAccess {
    pub(crate) account_id: u64,
    pub(crate) owner: String,
    pub(crate) actor: String,
    pub(crate) authorization: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AgentPermission {
    pub(crate) account_id: u64,
    pub(crate) owner: String,
    pub(crate) actor: String,
    pub(crate) authorized: bool,
    pub(crate) authorization: String,
    pub(crate) permitted_traders: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LendingPosition {
    pub(crate) token_id: u32,
    pub(crate) symbol: String,
    pub(crate) lender_quantity_raw: u64,
    pub(crate) lender_quantity: String,
    pub(crate) borrower_quantity_raw: u64,
    pub(crate) borrower_quantity: String,
    pub(crate) highest_interest_rate_raw: u16,
    pub(crate) highest_interest_rate: String,
    pub(crate) highest_interest_rate_percent: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PerpetualPosition {
    pub(crate) token_id: u32,
    pub(crate) symbol: String,
    pub(crate) quantity_raw: i64,
    pub(crate) quantity: String,
    pub(crate) side: String,
    pub(crate) entry_price_raw: u64,
    pub(crate) entry_price: String,
    pub(crate) funding_start_time: u64,
    pub(crate) owed_nom_raw: String,
    pub(crate) owed_nom: String,
    pub(crate) owed_base_raw: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Market {
    pub(crate) product: String,
    pub(crate) book: String,
    pub(crate) base_token: Asset,
    pub(crate) quote_token: Option<Asset>,
    pub(crate) buy_token_id: Option<u32>,
    pub(crate) pay_token_id: Option<u32>,
    pub(crate) mark_price_raw: u64,
    pub(crate) mark_price: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct OpenOrder {
    pub(crate) product: String,
    pub(crate) side: String,
    pub(crate) order_id: u64,
    pub(crate) order_type: String,
    pub(crate) quantity_raw: u64,
    pub(crate) quantity: String,
    pub(crate) price_raw: u64,
    pub(crate) price: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct OpenOrders {
    pub(crate) account_id: u64,
    pub(crate) order_book: String,
    pub(crate) product: String,
    pub(crate) orders: Vec<OpenOrder>,
    pub(crate) buy_cursor: String,
    pub(crate) sell_cursor: String,
}

#[derive(Deserialize)]
struct RpcResponse {
    result: Option<String>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcValueResponse {
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

impl Default for WorldClient {
    fn default() -> Self {
        let rpc_url = env::var("WORLD_RPC_URL").unwrap_or_else(|_| DEFAULT_RPC_URL.to_string());
        let exchange = env::var("WORLD_EXCHANGE_ADDRESS")
            .unwrap_or_else(|_| DEFAULT_EXCHANGE.to_string())
            .parse()
            .expect("WORLD_EXCHANGE_ADDRESS must be a valid EVM address");
        Self {
            http: Client::new(),
            rpc_url,
            exchange,
        }
    }
}

impl WorldClient {
    pub(crate) fn exchange(&self) -> String {
        format!("{:#x}", self.exchange)
    }

    /// Mark price for an asset, independent of a specific order book.
    pub(crate) fn mark_price(&self, token_id: u32) -> Result<(u64, String), String> {
        if token_id == BASE_TOKEN_ID {
            // Quote token has no on-chain mark; USDT notionals are already in quote units.
            const ONE_RAW: u64 = 1 << 5;
            return Ok((ONE_RAW, decode_price(ONE_RAW)));
        }
        let raw = self.call(&getMarkPriceCall { tokenId: token_id })?.price;
        Ok((raw, decode_price(raw)))
    }

    pub(crate) fn block_timestamp(&self) -> Result<u64, String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getBlockByNumber",
            "params": ["latest", false],
        });
        let response = self.rpc_value(body)?;
        let block = response
            .result
            .ok_or_else(|| self.rpc_error("eth_getBlockByNumber", response.error))?;
        let timestamp = block
            .get("timestamp")
            .and_then(Value::as_str)
            .ok_or_else(|| "[world-markets] block payload missing timestamp".to_string())?;
        u64::from_str_radix(timestamp.trim_start_matches("0x"), 16)
            .map_err(|e| format!("[world-markets] invalid block timestamp: {e}"))
    }

    pub(crate) fn funding_rate_history(
        &self,
        from_sec: u64,
        to_sec: u64,
        token_id: u32,
    ) -> Result<Vec<u64>, String> {
        Ok(self
            .call(&readFundingRateHistory_4648699482Call {
                startTime: from_sec,
                endTime: to_sec,
                tokenId: token_id,
            })?
            .rates)
    }

    pub(crate) fn current_funding_rate_8h(&self, token_id: u32) -> Result<Option<String>, String> {
        if token_id == BASE_TOKEN_ID {
            return Ok(None);
        }
        let now = self.block_timestamp()?;
        let from = now.saturating_sub(8 * 3600);
        let rates = self.funding_rate_history(from, now, token_id)?;
        Ok(rates.last().copied().map(decode_funding_rate))
    }

    pub(crate) fn lend_book_rates(&self, token_id: u32) -> Result<LendBookRates, String> {
        let book = self.call(&getLendOrderBookCall { tokenId: token_id })?.book;
        if book.is_zero() {
            return Ok(LendBookRates {
                lend_apr: None,
                borrow_apr: None,
            });
        }
        if let Ok(ret) = self.call_at(book, &bestBidOfferCall {}) {
            let packed = ret.packed;
            // Lend BBO packing (live UniFi): each 128-bit side is
            // (64-bit 4-dp rate | 64-bit quantity). Buy/bid = low 128,
            // sell/ask = high 128.
            let bid = decode_book_rate(field(packed, 0, 64));
            let ask = decode_book_rate(field(packed, 128, 64));
            if bid.is_some() || ask.is_some() {
                return Ok(LendBookRates {
                    lend_apr: bid,
                    borrow_apr: ask,
                });
            }
        }
        let buys = self
            .call_at(book, &retrieveBuyDepthChartCall { maxDepth: 8 })
            .ok()
            .and_then(|ret| first_depth_rate(&ret.levels));
        let sells = self
            .call_at(book, &retrieveSellDepthChartCall { maxDepth: 8 })
            .ok()
            .and_then(|ret| first_depth_rate(&ret.levels));
        Ok(LendBookRates {
            lend_apr: buys,
            borrow_apr: sells,
        })
    }

    pub(crate) fn lender_position_ids(
        &self,
        user_id: u64,
        token_id: u32,
    ) -> Result<Vec<u64>, String> {
        let words = self
            .call(&readLenderPositionsCall {
                userId: user_id,
                tokenId: token_id,
            })?
            .positions;
        Ok(decode_position_ids(&words))
    }

    pub(crate) fn borrower_position_ids(
        &self,
        user_id: u64,
        token_id: u32,
    ) -> Result<Vec<u64>, String> {
        let words = self
            .call(&readBorrowerPositionsCall {
                userId: user_id,
                tokenId: token_id,
            })?
            .positions;
        Ok(decode_position_ids(&words))
    }

    pub(crate) fn lending_position(&self, position_id: u64) -> Result<PackedLoan, String> {
        let packed = self
            .call(&readLendingPositionCall {
                positionId: position_id,
            })?
            .position;
        Ok(PackedLoan::decode(position_id, packed))
    }

    pub(crate) fn block_number(&self) -> Result<u64, String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_blockNumber",
            "params": [],
        });
        let response = self.rpc(body)?;
        let raw = response
            .result
            .ok_or_else(|| self.rpc_error("eth_blockNumber", response.error))?;
        u64::from_str_radix(raw.trim_start_matches("0x"), 16)
            .map_err(|e| format!("[world-markets] invalid block number: {e}"))
    }

    pub(crate) fn assets(&self) -> Result<Vec<Asset>, String> {
        let returns = self.call(&bulkReadTokenConfigs_3423260018Call {})?;
        let mut assets = Vec::new();
        for chunk in returns.configs.chunks_exact(3) {
            if chunk[0].is_zero() {
                break;
            }
            assets.push(Asset::decode(chunk[0], chunk[1], chunk[2])?);
        }
        Ok(assets)
    }

    pub(crate) fn account_id_for(&self, wallet: Address) -> Result<u64, String> {
        Ok(self
            .call(&getUserIdCall {
                userAddress: wallet,
            })?
            ._0)
    }

    #[cfg(test)]
    pub(crate) fn latest_account_id(&self) -> Result<u64, String> {
        Ok(self.call(&bulkReadMaxUserId_5445644137Call {})?.maxUserId)
    }

    pub(crate) fn owner_for(&self, account_id: u64) -> Result<Address, String> {
        Ok(self.call(&getUserAddressCall { userId: account_id })?._0)
    }

    pub(crate) fn traders_for(&self, account_id: u64) -> Result<Vec<Address>, String> {
        Ok(self
            .call(&bulkTraders_5523718714Call { userId: account_id })?
            .traders)
    }

    pub(crate) fn account(&self, account_id: u64, assets: &[Asset]) -> Result<Account, String> {
        let owner = self.owner_for(account_id)?;
        if owner.is_zero() {
            return Err(format!(
                "[world-markets] World account {account_id} does not exist"
            ));
        }

        let risk_raw = self
            .call(&riskAdjustedPortfolioValueCall { userId: account_id })?
            ._0;
        let bits = self.call(&userBitsCall { userId: account_id })?;
        let debt_token_ids = token_ids_from_bits(bits.debt);
        let non_debt_token_ids = token_ids_from_bits(bits.noDebt);
        let mut balances = Vec::new();
        let mut lending_positions = Vec::new();
        let mut perpetual_positions = Vec::new();

        for asset in assets {
            let raw = self.call(&getBalanceCall {
                user: account_id,
                tokenId: asset.token_id,
            })?;
            if raw.balance != 0 || raw.spotLendSequestered != 0 || raw.perpSequestered != 0 {
                let reserved_position_raw = u128::from(raw.spotLendSequestered)
                    .saturating_add(u128::from(raw.perpSequestered));
                let decimal_delta =
                    u32::from(asset.vault_decimals.saturating_sub(asset.position_decimals));
                let reserved_vault_raw =
                    reserved_position_raw.saturating_mul(10u128.saturating_pow(decimal_delta));
                let available = raw.balance.saturating_sub(reserved_vault_raw);
                balances.push(Balance {
                    token_id: asset.token_id,
                    symbol: asset.symbol.clone(),
                    balance_raw: raw.balance.to_string(),
                    balance: decimal(raw.balance, asset.vault_decimals),
                    available_raw: available.to_string(),
                    available: decimal(available, asset.vault_decimals),
                    spot_lend_sequestered_raw: raw.spotLendSequestered.to_string(),
                    spot_lend_sequestered: decimal(
                        raw.spotLendSequestered,
                        asset.position_decimals,
                    ),
                    perp_sequestered_raw: raw.perpSequestered.to_string(),
                    perp_sequestered: decimal(raw.perpSequestered, asset.position_decimals),
                });
            }

            let lending = self.call(&readLendingAggregationCall {
                userId: account_id,
                tokenId: asset.token_id,
            })?;
            if field(lending.position, 0, 64) != 0 || field(lending.position, 64, 64) != 0 {
                lending_positions.push(LendingPosition::decode(asset, lending.position));
            }

            let perpetual = self.call(&readPerpAggPositionCall {
                userId: account_id,
                tokenId: asset.token_id,
            })?;
            let perp_quantity = signed_field(perpetual.position, 64, 64)?;
            let perp_owed_nom = signed_field(perpetual.position, 128, 96)?;
            if perp_quantity != 0 || perp_owed_nom != 0 || perpetual.owedBase != I256::ZERO {
                perpetual_positions.push(PerpetualPosition::decode(
                    asset,
                    perpetual.position,
                    perpetual.owedBase,
                )?);
            }
        }

        let base_decimals = assets
            .iter()
            .find(|asset| asset.token_id == 1)
            .map(|asset| asset.position_decimals)
            .unwrap_or(6);

        Ok(Account {
            account_id,
            owner: format!("{owner:#x}"),
            risk_adjusted_portfolio_value_raw: risk_raw,
            risk_adjusted_portfolio_value: signed_decimal(risk_raw, base_decimals),
            eligible_for_liquidation: risk_raw < 0,
            balances,
            lending_positions,
            perpetual_positions,
            debt_token_ids,
            non_debt_token_ids,
        })
    }

    fn dev_owner_read_enabled() -> bool {
        std::env::var("WORLD_ACCOUNT_ID").is_ok()
    }

    pub(crate) fn resolve_account(
        &self,
        account_id: Option<u64>,
        owner_wallet: Option<&str>,
        actor: Option<&str>,
    ) -> Result<AccountAccess, String> {
        let owner_wallet = owner_wallet
            .map(Address::from_str)
            .transpose()
            .map_err(|e| format!("[world-markets] invalid wallet address: {e}"))?;
        let actor = actor
            .map(Address::from_str)
            .transpose()
            .map_err(|e| format!("[world-markets] invalid actor address: {e}"))?;

        let id = match (account_id, owner_wallet, actor) {
            (Some(id), _, _) => id,
            (None, Some(owner), _) => self.account_id_for(owner)?,
            (None, None, Some(actor)) => self.account_id_for(actor)?,
            (None, None, None) => {
                return Err(
                    "[world-markets] no World account context is available; provide account_id or bind a handover account reference"
                        .to_string(),
                );
            }
        };
        if id == 0 {
            return Err(
                "[world-markets] the supplied wallet has no registered World account".to_string(),
            );
        }
        let owner = self.owner_for(id)?;
        if owner.is_zero() {
            return Err(format!("[world-markets] World account {id} does not exist"));
        }
        if let Some(expected_owner) = owner_wallet
            && owner != expected_owner
        {
            return Err(format!(
                "[world-markets] account {id} owner is {owner:#x}, not {expected_owner:#x}"
            ));
        }

        let actor = actor.or(owner_wallet);
        let (actor, authorization) = if let Some(actor) = actor {
            let (authorized, authorization) = if actor == owner {
                (true, "owner")
            } else {
                let traders = self.traders_for(id)?;
                (traders.contains(&actor), "delegated_trader")
            };
            if !authorized {
                return Err(format!(
                    "[world-markets] actor {actor:#x} is neither the owner {owner:#x} nor a permitted trader for account {id}; the grant may be missing or revoked"
                ));
            }
            (actor, authorization.to_string())
        } else if account_id.is_some() && Self::dev_owner_read_enabled() {
            // aomi-run stubs evm-core; WORLD_ACCOUNT_ID scopes read-only dev lookups.
            (owner, "dev_owner_read".to_string())
        } else {
            return Err(
                "[world-markets] no acting wallet is bound; connect the owner wallet or use an active handover"
                    .to_string(),
            );
        };

        Ok(AccountAccess {
            account_id: id,
            owner: format!("{owner:#x}"),
            actor: format!("{actor:#x}"),
            authorization,
        })
    }

    pub(crate) fn agent_permission(
        &self,
        account_id: u64,
        actor: &str,
    ) -> Result<AgentPermission, String> {
        let actor = Address::from_str(actor)
            .map_err(|e| format!("[world-markets] invalid actor address: {e}"))?;
        let owner = self.owner_for(account_id)?;
        if owner.is_zero() {
            return Err(format!(
                "[world-markets] World account {account_id} does not exist"
            ));
        }
        let traders = self.traders_for(account_id)?;
        let authorization = if actor == owner {
            "owner"
        } else if traders.contains(&actor) {
            "delegated_trader"
        } else {
            "none"
        };
        Ok(AgentPermission {
            account_id,
            owner: format!("{owner:#x}"),
            actor: format!("{actor:#x}"),
            authorized: authorization != "none",
            authorization: authorization.to_string(),
            permitted_traders: traders
                .into_iter()
                .map(|address| format!("{address:#x}"))
                .collect(),
        })
    }

    pub(crate) fn market(
        &self,
        product: &str,
        base: Asset,
        quote: Option<Asset>,
    ) -> Result<Market, String> {
        let (book, buy_token_id, pay_token_id) = match product {
            "spot" => {
                let quote = quote.as_ref().ok_or_else(|| {
                    "[world-markets] quote_symbol is required for a spot market".to_string()
                })?;
                let result = self.call(&getSpotOrderBookCall {
                    token1: base.token_id,
                    token2: quote.token_id,
                })?;
                (result.book, Some(result.buyToken), Some(result.payToken))
            }
            "perp" | "perpetual" => {
                let quote = quote.as_ref().ok_or_else(|| {
                    "[world-markets] quote_symbol is required for a perpetual market".to_string()
                })?;
                let result = self.call(&getPerpOrderBookCall {
                    token1: base.token_id,
                    token2: quote.token_id,
                })?;
                (result.book, Some(result.buyToken), Some(result.payToken))
            }
            "lend" | "lending" => {
                let result = self.call(&getLendOrderBookCall {
                    tokenId: base.token_id,
                })?;
                (result.book, None, None)
            }
            other => {
                return Err(format!(
                    "[world-markets] unsupported product {other:?}; use spot, perp, or lend"
                ));
            }
        };
        if book.is_zero() {
            let pair = quote
                .as_ref()
                .map(|asset| format!("/{}", asset.symbol))
                .unwrap_or_default();
            return Err(format!(
                "[world-markets] no {product} market exists for {}{pair}",
                base.symbol
            ));
        }
        let (mark_price_raw, mark_price) = self.mark_price(base.token_id)?;

        Ok(Market {
            product: product.to_string(),
            book: format!("{book:#x}"),
            base_token: base,
            quote_token: quote,
            buy_token_id,
            pay_token_id,
            mark_price_raw,
            mark_price,
        })
    }

    pub(crate) fn open_orders(
        &self,
        market: &Market,
        account_id: u64,
    ) -> Result<OpenOrders, String> {
        const MAX_DEPTH: u32 = 1_000;
        const MAX_ORDERS: u32 = 200;
        let book = Address::from_str(&market.book)
            .map_err(|e| format!("[world-markets] invalid order-book address: {e}"))?;
        let buys = self.call_at(
            book,
            &searchBuyOrdersCall {
                userId: account_id,
                maxDepth: MAX_DEPTH,
                maxOrders: MAX_ORDERS,
                restartPosition: 0,
            },
        )?;
        let sells = self.call_at(
            book,
            &searchSellOrdersCall {
                userId: account_id,
                maxDepth: MAX_DEPTH,
                maxOrders: MAX_ORDERS,
                restartPosition: 0,
            },
        )?;
        let (mut orders, buy_cursor) = decode_open_orders(
            &buys.orders,
            &market.product,
            "buy",
            market.base_token.position_decimals,
        );
        let (sell_orders, sell_cursor) = decode_open_orders(
            &sells.orders,
            &market.product,
            "sell",
            market.base_token.position_decimals,
        );
        orders.extend(sell_orders);
        Ok(OpenOrders {
            account_id,
            order_book: market.book.clone(),
            product: market.product.clone(),
            orders,
            buy_cursor,
            sell_cursor,
        })
    }

    fn call<C: SolCall>(&self, call: &C) -> Result<C::Return, String> {
        self.call_at(self.exchange, call)
    }

    fn call_at<C: SolCall>(&self, target: Address, call: &C) -> Result<C::Return, String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [{
                "to": format!("{target:#x}"),
                "data": format!("0x{}", hex::encode(call.abi_encode())),
            }, "latest"],
        });
        let response = self.rpc(body)?;
        let raw = response
            .result
            .ok_or_else(|| self.rpc_error(C::SIGNATURE, response.error))?;
        let bytes = hex::decode(raw.trim_start_matches("0x"))
            .map_err(|e| format!("[world-markets] invalid RPC hex for {}: {e}", C::SIGNATURE))?;
        C::abi_decode_returns(&bytes, true)
            .map_err(|e| format!("[world-markets] ABI decode for {}: {e}", C::SIGNATURE))
    }

    fn rpc(&self, body: Value) -> Result<RpcResponse, String> {
        self.http
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|e| format!("[world-markets] World RPC request failed: {e}"))?
            .json()
            .map_err(|e| format!("[world-markets] World RPC response was invalid: {e}"))
    }

    fn rpc_value(&self, body: Value) -> Result<RpcValueResponse, String> {
        self.http
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|e| format!("[world-markets] World RPC request failed: {e}"))?
            .json()
            .map_err(|e| format!("[world-markets] World RPC response was invalid: {e}"))
    }

    fn rpc_error(&self, method: &str, error: Option<RpcError>) -> String {
        match error {
            Some(error) => format!(
                "[world-markets] RPC {method} failed ({}): {}",
                error.code, error.message
            ),
            None => format!("[world-markets] RPC {method} returned no result"),
        }
    }
}

fn decode_open_orders(
    words: &[U256],
    product: &str,
    side: &str,
    quantity_decimals: u8,
) -> (Vec<OpenOrder>, String) {
    let Some(cursor) = words.first().copied() else {
        return (Vec::new(), "0".to_string());
    };
    let returned = field(cursor, 0, 32) as usize;
    let restart = field(cursor, 32, 64);
    let orders = words
        .iter()
        .skip(1)
        .take(returned)
        .filter(|word| !word.is_zero())
        .map(|word| {
            let price_raw = field(*word, 0, 64);
            let quantity_raw = field(*word, 64, 64);
            let order_id = field(*word, 128, 64);
            let order_type_raw = field(*word, 192, 4) as u8;
            let order_type = match order_type_raw {
                0 => "limit".to_string(),
                2 => "fill_all_or_revert".to_string(),
                3 => "fill_partial_kill_rest".to_string(),
                other => format!("unknown_{other}"),
            };
            OpenOrder {
                product: product.to_string(),
                side: side.to_string(),
                order_id,
                order_type,
                quantity_raw,
                quantity: decimal(quantity_raw, quantity_decimals),
                price_raw,
                price: decode_price(price_raw),
            }
        })
        .collect();
    (orders, restart.to_string())
}

impl Asset {
    fn decode(config: U256, symbol: U256, name: U256) -> Result<Self, String> {
        let token_address = Address::from_word(config.into());
        let sequestration_multiplier = field(config, 160, 8) as u8;
        let position_decimals = field(config, 168, 8) as u8;
        let vault_decimals = field(config, 176, 8) as u8;
        let erc20_decimals = field(config, 184, 8) as u8;
        let token_type = field(config, 192, 8) as u8;
        let token_id = field(config, 200, 32) as u32;
        let risk_price_percent = field(config, 232, 8) as u8;
        let risk_slippage_percent = field(config, 240, 8) as f64 / 10.0;
        let token_type = match token_type {
            10 => "erc20",
            20 => "vault",
            _ => "unknown",
        };

        let _ = sequestration_multiplier;
        Ok(Self {
            token_id,
            symbol: packed_string(symbol)?,
            name: packed_string(name)?,
            token_type: token_type.to_string(),
            erc20_address: format!("{token_address:#x}"),
            erc20_decimals,
            vault_decimals,
            position_decimals,
            risk_price_percent,
            risk_slippage_percent,
        })
    }
}

impl LendingPosition {
    fn decode(asset: &Asset, packed: U256) -> Self {
        let borrower_quantity_raw = field(packed, 0, 64);
        let lender_quantity_raw = field(packed, 64, 64);
        let highest_interest_rate_raw = field(packed, 128, 16) as u16;
        Self {
            token_id: asset.token_id,
            symbol: asset.symbol.clone(),
            lender_quantity_raw,
            lender_quantity: decimal(lender_quantity_raw, asset.position_decimals),
            borrower_quantity_raw,
            borrower_quantity: decimal(borrower_quantity_raw, asset.position_decimals),
            highest_interest_rate_raw,
            highest_interest_rate: decimal(highest_interest_rate_raw, 4),
            highest_interest_rate_percent: decimal(u64::from(highest_interest_rate_raw), 2),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LendBookRates {
    /// Best bid on this asset's lend book (taker-lend).
    pub(crate) lend_apr: Option<String>,
    /// Best ask on this asset's lend book (taker-borrow).
    pub(crate) borrow_apr: Option<String>,
}

/// Individual lend/borrow position unpacked from `readLendingPosition`.
/// Layout: bits 0–64 quantity, 64–64 counterparty user id, 128–16 interest
/// (4 dp fraction), 144–32 start unix seconds, 176–1 do-not-return.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PackedLoan {
    pub(crate) position_id: u64,
    pub(crate) quantity_raw: u64,
    pub(crate) counterparty_id: u64,
    pub(crate) interest_rate: String,
    pub(crate) started_at_unix: Option<u64>,
    pub(crate) do_not_return: bool,
}

impl PackedLoan {
    pub(crate) fn decode(position_id: u64, packed: U256) -> Self {
        let quantity_raw = field(packed, 0, 64);
        let counterparty_id = field(packed, 64, 64);
        let rate_raw = field(packed, 128, 16) as u16;
        let started = field(packed, 144, 32);
        let started_at_unix = if (1_600_000_000..2_200_000_000).contains(&started) {
            Some(started)
        } else {
            None
        };
        let do_not_return = field(packed, 176, 1) == 1;
        Self {
            position_id,
            quantity_raw,
            counterparty_id,
            interest_rate: decimal(rate_raw, 4),
            started_at_unix,
            do_not_return,
        }
    }
}

fn decode_funding_rate(raw: u64) -> String {
    decimal(raw, 7)
}

fn decode_book_rate(raw: u64) -> Option<String> {
    if raw == 0 {
        return None;
    }
    // Lend-book "price" is an APR. On UniFi it matches lending aggregation:
    // 16-bit 4-decimal fraction (600 → 0.0600 = 6%). Packed-price encoding is
    // the fallback for books that quote like spot.
    if raw <= u64::from(u16::MAX) {
        return Some(decimal(raw, 4));
    }
    let as_price = decode_price(raw);
    if as_price != "0" {
        return Some(as_price);
    }
    None
}

fn first_depth_rate(levels: &[U256]) -> Option<String> {
    let word = levels.iter().find(|w| !w.is_zero())?;
    decode_book_rate(field(*word, 0, 64)).or_else(|| decode_book_rate(field(*word, 64, 64)))
}

fn decode_position_ids(words: &[U256]) -> Vec<u64> {
    if words.is_empty() {
        return Vec::new();
    }
    let returned = field(words[0], 0, 32) as usize;
    if returned > 0 && returned < words.len() {
        return words
            .iter()
            .skip(1)
            .take(returned)
            .filter(|w| !w.is_zero())
            .map(|w| field(*w, 0, 64))
            .filter(|id| *id != 0)
            .collect();
    }
    words
        .iter()
        .filter(|w| !w.is_zero())
        .filter(|w| w.bit_len() <= 64)
        .map(|w| w.to::<u64>())
        .collect()
}

impl PerpetualPosition {
    fn decode(asset: &Asset, packed: U256, owed_base: I256) -> Result<Self, String> {
        let entry_price_raw = field(packed, 0, 64);
        let quantity_raw = signed_field(packed, 64, 64)?;
        let owed_nom_raw = signed_field(packed, 128, 96)?;
        let funding_period = field(packed, 224, 32);
        Ok(Self {
            token_id: asset.token_id,
            symbol: asset.symbol.clone(),
            quantity_raw: i64::try_from(quantity_raw)
                .map_err(|e| format!("[world-markets] perp quantity does not fit into i64: {e}"))?,
            quantity: signed_decimal_i128(quantity_raw, asset.position_decimals),
            side: if quantity_raw < 0 { "short" } else { "long" }.to_string(),
            entry_price_raw,
            entry_price: decode_price(entry_price_raw),
            funding_start_time: 1_704_067_200 + funding_period.saturating_mul(28_800),
            owed_nom_raw: owed_nom_raw.to_string(),
            owed_nom: signed_decimal_i128(owed_nom_raw, asset.position_decimals.saturating_add(7)),
            owed_base_raw: owed_base.to_string(),
        })
    }
}

pub(crate) fn asset_by_symbol(assets: &[Asset], symbol: &str) -> Result<Asset, String> {
    assets
        .iter()
        .find(|asset| asset.symbol.eq_ignore_ascii_case(symbol))
        .cloned()
        .ok_or_else(|| format!("[world-markets] unknown World asset symbol {symbol:?}"))
}

fn field(value: U256, offset: usize, bits: usize) -> u64 {
    let mask = (U256::from(1u8) << bits) - U256::from(1u8);
    ((value >> offset) & mask).to::<u64>()
}

fn signed_field(value: U256, offset: usize, bits: usize) -> Result<i128, String> {
    if bits == 0 || bits > 127 {
        return Err(format!(
            "[world-markets] unsupported signed field width {bits}"
        ));
    }
    let mask = (U256::from(1u8) << bits) - U256::from(1u8);
    let raw = ((value >> offset) & mask).to::<u128>();
    let sign_bit = 1u128 << (bits - 1);
    if raw & sign_bit == 0 {
        Ok(raw as i128)
    } else {
        Ok((raw as i128) - (1i128 << bits))
    }
}

fn packed_string(value: U256) -> Result<String, String> {
    let raw_len = (value & U256::from(0xffu16)).to::<u8>();
    let len = usize::from(raw_len.min(31));
    if len == 0 {
        return Ok(String::new());
    }
    let data = value >> 8;
    let shift = u32::try_from((31 - len) * 8)
        .map_err(|e| format!("[world-markets] invalid token metadata shift: {e}"))?;
    let shifted: U256 = data >> shift;
    let bytes = shifted.to_be_bytes::<32>();
    String::from_utf8(bytes[32 - len..].to_vec())
        .map_err(|e| format!("[world-markets] invalid token metadata: {e}"))
}

fn token_ids_from_bits(bits: U256) -> Vec<u32> {
    let max = (bits & U256::from(u32::MAX)).to::<u32>();
    (1..=max.min(224))
        .filter(|token_id| bits.bit(32 + usize::try_from(*token_id).unwrap_or(0)))
        .collect()
}

fn decimal<T: ToString>(value: T, decimals: u8) -> String {
    decimal_digits(value.to_string(), decimals)
}

fn signed_decimal(value: i64, decimals: u8) -> String {
    signed_decimal_i128(i128::from(value), decimals)
}

fn signed_decimal_i128(value: i128, decimals: u8) -> String {
    if value.is_negative() {
        format!(
            "-{}",
            decimal_digits(value.unsigned_abs().to_string(), decimals)
        )
    } else {
        decimal_digits(value.to_string(), decimals)
    }
}

pub(crate) fn decimal_digits(mut digits: String, decimals: u8) -> String {
    let decimals = usize::from(decimals);
    if decimals == 0 {
        return digits;
    }
    if digits.len() <= decimals {
        digits.insert_str(0, &"0".repeat(decimals + 1 - digits.len()));
    }
    let split = digits.len() - decimals;
    digits.insert(split, '.');
    while digits.ends_with('0') {
        digits.pop();
    }
    if digits.ends_with('.') {
        digits.push('0');
    }
    digits
}

fn decode_price(raw: u64) -> String {
    let exponent = (raw & 0x1f) as u8;
    decimal_digits((raw >> 5).to_string(), exponent)
}

#[cfg(test)]
mod tests {
    use super::{
        BASE_TOKEN_ID, WorldClient, decimal_digits, decode_book_rate, decode_open_orders,
        decode_price, packed_string, signed_field,
    };
    use alloy_primitives::U256;

    #[test]
    fn base_token_mark_price_is_one_without_rpc() {
        let client = WorldClient::default();
        let (raw, mark) = client.mark_price(BASE_TOKEN_ID).unwrap();
        assert_eq!(raw, 1 << 5);
        assert_eq!(mark, "1");
    }

    #[test]
    fn formats_decimal_values() {
        assert_eq!(decimal_digits("123456".to_string(), 3), "123.456");
        assert_eq!(decimal_digits("5".to_string(), 6), "0.000005");
        assert_eq!(decimal_digits("1000".to_string(), 3), "1.0");
    }

    #[test]
    fn decodes_lend_book_rate_as_four_decimal_fraction() {
        assert_eq!(decode_book_rate(600).as_deref(), Some("0.06"));
        assert_eq!(decode_book_rate(550).as_deref(), Some("0.055"));
        assert_eq!(decode_book_rate(0), None);
    }

    #[test]
    fn decodes_price_mantissa_and_exponent() {
        assert_eq!(decode_price((12345 << 5) | 2), "123.45");
    }

    #[test]
    fn decodes_bulk_metadata_string() {
        let mut bytes = [0u8; 32];
        bytes[..4].copy_from_slice(b"USDC");
        bytes[31] = 4;
        let value = U256::from_be_bytes(bytes);
        assert_eq!(packed_string(value).unwrap(), "USDC");
    }

    #[test]
    fn decodes_signed_packed_fields() {
        let positive = U256::from(42u8) << 64;
        assert_eq!(signed_field(positive, 64, 64).unwrap(), 42);

        let negative_two = (U256::from(u64::MAX - 1) << 64) | U256::from(1u8);
        assert_eq!(signed_field(negative_two, 64, 64).unwrap(), -2);
    }

    #[test]
    fn decodes_open_order_search_words() {
        let cursor = U256::from(1u8);
        let price = (12_345u64 << 5) | 2;
        let word = U256::from(price) | (U256::from(250_000u64) << 64) | (U256::from(77u64) << 128);
        let (orders, restart) = decode_open_orders(&[cursor, word], "perp", "buy", 4);
        assert_eq!(restart, "0");
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].order_id, 77);
        assert_eq!(orders[0].price, "123.45");
        assert_eq!(orders[0].quantity, "25.0");
        assert_eq!(orders[0].order_type, "limit");
    }

    #[test]
    #[ignore = "requires live UniFi RPC"]
    fn reads_live_world_assets() {
        let client = WorldClient::default();
        let block = client.block_number().unwrap();
        let assets = client.assets().unwrap();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "block": block,
                "assets": assets,
            }))
            .unwrap()
        );
        assert!(block > 0);
        assert!(!assets.is_empty());
        assert!(assets.iter().any(|asset| asset.token_id == 1));
    }

    #[test]
    #[ignore = "requires live UniFi RPC"]
    fn reads_live_world_market_and_account() {
        let client = WorldClient::default();
        let assets = client.assets().unwrap();
        let weth = super::asset_by_symbol(&assets, "WETH").unwrap();
        let usdt = super::asset_by_symbol(&assets, "USDT").unwrap();
        let market = client.market("spot", weth, Some(usdt)).unwrap();
        let account_id = client.latest_account_id().unwrap();
        let account = client.account(account_id, &assets).unwrap();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "market": market,
                "account": account,
            }))
            .unwrap()
        );
        assert_ne!(market.book, "0x0000000000000000000000000000000000000000");
        assert!(account.account_id > 0);
    }

    #[test]
    #[ignore = "requires live UniFi RPC"]
    fn reads_live_world_permissions_and_open_orders() {
        let client = WorldClient::default();
        let assets = client.assets().unwrap();
        let weth = super::asset_by_symbol(&assets, "WETH").unwrap();
        let usdt = super::asset_by_symbol(&assets, "USDT").unwrap();
        let market = client.market("perp", weth, Some(usdt)).unwrap();
        let account_id = client.latest_account_id().unwrap();
        let owner = client.owner_for(account_id).unwrap();
        let permission = client
            .agent_permission(account_id, &format!("{owner:#x}"))
            .unwrap();
        let orders = client.open_orders(&market, account_id).unwrap();
        assert!(permission.authorized);
        assert_eq!(permission.authorization, "owner");
        assert_eq!(orders.account_id, account_id);
    }

    #[test]
    #[ignore = "requires live UniFi RPC"]
    fn reads_live_world_rates_and_loans() {
        let client = WorldClient::default();
        let assets = client.assets().unwrap();
        let weth = super::asset_by_symbol(&assets, "WETH").unwrap();
        let usdt = super::asset_by_symbol(&assets, "USDT").unwrap();
        let funding = client.current_funding_rate_8h(weth.token_id);
        let weth_book = client.lend_book_rates(weth.token_id);
        let usdt_book = client.lend_book_rates(usdt.token_id);
        funding.expect("funding read");
        let weth_book = weth_book.expect("weth lend book");
        let usdt_book = usdt_book.expect("usdt lend book");
        assert!(
            weth_book
                .lend_apr
                .as_deref()
                .is_some_and(|r| r.starts_with("0.")),
            "weth lend apr should be a decimal fraction, got {weth_book:?}"
        );
        assert!(
            usdt_book
                .borrow_apr
                .as_deref()
                .is_some_and(|r| r.starts_with("0.")),
            "usdt borrow apr should be a decimal fraction, got {usdt_book:?}"
        );

        let rates = crate::rates::snapshot(&client, Some(&["WETH".to_string()])).unwrap();
        assert_eq!(rates.source, "world-markets-contract");
        assert!(!rates.executable);
        let row = rates
            .rates
            .iter()
            .find(|row| row.base_symbol == "WETH")
            .expect("WETH rates row");
        assert!(row.funding_rate_8h.is_some());
        assert!(row.funding_annualized.is_some());
        assert_eq!(row.lend_apr.as_deref(), weth_book.lend_apr.as_deref());
        assert_eq!(row.borrow_apr.as_deref(), usdt_book.borrow_apr.as_deref());
        assert_eq!(row.native_yield_source, "none");
        assert!(row.yield_basis_spread_apr.is_none());
        assert!(row.basis_spread_apr.is_some());
    }
}
