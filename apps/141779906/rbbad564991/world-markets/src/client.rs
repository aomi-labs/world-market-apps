use std::{env, str::FromStr};

use alloy_primitives::{Address, I256, U256, hex};
use alloy_sol_types::{SolCall, sol};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_RPC_URL: &str = "https://mainnet.megaeth.com/rpc";
const DEFAULT_EXCHANGE: &str = "0x5e3Ae52EbA0F9740364Bd5dd39738e1336086A8b";
pub(crate) const CHAIN_ID: u64 = 4326;

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
    function bulkReadTokenConfigs_3423260018() external view returns (uint256[] configs);
    function bulkReadMaxUserId_5445644137() external view returns (uint64 maxUserId);
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

#[derive(Deserialize)]
struct RpcResponse {
    result: Option<String>,
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
    fn latest_account_id(&self) -> Result<u64, String> {
        Ok(self.call(&bulkReadMaxUserId_5445644137Call {})?.maxUserId)
    }

    pub(crate) fn owner_for(&self, account_id: u64) -> Result<Address, String> {
        Ok(self.call(&getUserAddressCall { userId: account_id })?._0)
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

    pub(crate) fn resolve_account(
        &self,
        account_id: Option<u64>,
        wallet: Option<&str>,
    ) -> Result<u64, String> {
        let wallet = wallet
            .map(Address::from_str)
            .transpose()
            .map_err(|e| format!("[world-markets] invalid wallet address: {e}"))?;

        match (account_id, wallet) {
            (Some(id), Some(address)) => {
                let owner = self.owner_for(id)?;
                if owner != address {
                    return Err(format!(
                        "[world-markets] account {id} belongs to {owner:#x}, not {address:#x}"
                    ));
                }
                Ok(id)
            }
            (Some(id), None) => Ok(id),
            (None, Some(address)) => {
                let id = self.account_id_for(address)?;
                if id == 0 {
                    Err(format!(
                        "[world-markets] wallet {address:#x} has no registered World account"
                    ))
                } else {
                    Ok(id)
                }
            }
            (None, None) => Err(
                "[world-markets] no World account context is available; provide account_id or connect an EVM wallet"
                    .to_string(),
            ),
        }
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
        let mark_price_raw = self
            .call(&getMarkPriceCall {
                tokenId: base.token_id,
            })?
            .price;

        Ok(Market {
            product: product.to_string(),
            book: format!("{book:#x}"),
            base_token: base,
            quote_token: quote,
            buy_token_id,
            pay_token_id,
            mark_price_raw,
            mark_price: decode_price(mark_price_raw),
        })
    }

    fn call<C: SolCall>(&self, call: &C) -> Result<C::Return, String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [{
                "to": format!("{:#x}", self.exchange),
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
            .map_err(|e| format!("[world-markets] MegaETH RPC request failed: {e}"))?
            .json()
            .map_err(|e| format!("[world-markets] MegaETH RPC response was invalid: {e}"))
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

fn decimal_digits(mut digits: String, decimals: u8) -> String {
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
    use super::{WorldClient, decimal_digits, decode_price, packed_string, signed_field};
    use alloy_primitives::U256;

    #[test]
    fn formats_decimal_values() {
        assert_eq!(decimal_digits("123456".to_string(), 3), "123.456");
        assert_eq!(decimal_digits("5".to_string(), 6), "0.000005");
        assert_eq!(decimal_digits("1000".to_string(), 3), "1.0");
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
    #[ignore = "requires live MegaETH RPC"]
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
    #[ignore = "requires live MegaETH RPC"]
    fn reads_live_world_market_and_account() {
        let client = WorldClient::default();
        let assets = client.assets().unwrap();
        let weth = super::asset_by_symbol(&assets, "WETH").unwrap();
        let usdm = super::asset_by_symbol(&assets, "USDm").unwrap();
        let market = client.market("spot", weth, Some(usdm)).unwrap();
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
}
