//! Order-book resolution for the execution procedure: which book contract a
//! `new*Order` / `cancel*Order` call targets, and which token ids that book
//! buys and pays in.

use alloy_primitives::Address;
use serde_json::{Value, json};

use crate::client::WorldClient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Book {
    pub(crate) product: &'static str,
    pub(crate) address: Address,
    pub(crate) base_token_id: u32,
    pub(crate) quote_token_id: Option<u32>,
    /// Token the book's buy side receives (spot/perp only).
    pub(crate) buy_token: Option<u32>,
    /// Token the book's buy side pays with (spot/perp only).
    pub(crate) sell_token: Option<u32>,
}

impl Book {
    /// `Ok(None)` when the exchange has no book for the pair (zero address).
    pub(crate) fn resolve(
        client: &WorldClient,
        product: &str,
        base_token_id: u32,
        quote_token_id: Option<u32>,
    ) -> Result<Option<Self>, String> {
        let product = match product.to_ascii_lowercase().as_str() {
            "spot" => "spot",
            "perp" | "perpetual" => "perp",
            "lend" | "lending" => "lend",
            other => {
                return Err(format!(
                    "[world-markets] unsupported product {other:?}; use spot, perp, or lend"
                ));
            }
        };
        let quote_token_id = match (product, quote_token_id) {
            ("lend", _) => None,
            (_, Some(quote)) => Some(quote),
            (_, None) => {
                return Err(format!(
                    "[world-markets] quote_token_id is required for a {product} book"
                ));
            }
        };
        let (address, buy_token, sell_token) =
            client.book(product, base_token_id, quote_token_id)?;
        if address.is_zero() {
            return Ok(None);
        }
        Ok(Some(Self {
            product,
            address,
            base_token_id,
            quote_token_id,
            buy_token,
            sell_token,
        }))
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "product": self.product,
            "book": format!("{:#x}", self.address),
            "base_token_id": self.base_token_id,
            "quote_token_id": self.quote_token_id,
            "buy_token": self.buy_token,
            "sell_token": self.sell_token,
        })
    }
}
