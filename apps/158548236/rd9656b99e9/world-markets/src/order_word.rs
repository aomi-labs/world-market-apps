//! The packed `uint256` order word every `new*Order(address,uint256)` call on
//! the World exchange takes. Mirrors the venue SDK packer bit for bit:
//! MSB→LSB `[insertionHint 64 | orderType 4 | accountId 44 | quantity 64 | price 64]`
//! with `packed = (packed << n) | (value & mask)` per field, and the price as
//! `(mantissa << 5) | decimals`. Spot and perp books share the layout.

use alloy_primitives::U256;
use aomi_sdk::schemars::JsonSchema;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OrderType {
    /// Rests on the book; may fill partially or fully, now or later.
    Limit,
    /// Reverts unless the whole quantity fills immediately.
    FillAllOrRevert,
    /// Fills what it can immediately and cancels the rest.
    FillPartialKillRest,
}

impl OrderType {
    fn code(self) -> u64 {
        match self {
            OrderType::Limit => 0,
            OrderType::FillAllOrRevert => 2,
            OrderType::FillPartialKillRest => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderWord {
    pub(crate) account_id: u64,
    /// Quantity in position base units (already scaled by the base asset's
    /// `position_decimals`).
    pub(crate) quantity_raw: u64,
    /// Decimal limit price, e.g. `2465.71`.
    pub(crate) limit_price: Decimal,
    pub(crate) order_type: OrderType,
    pub(crate) insertion_hint: u64,
}

impl OrderWord {
    const LAYOUT: [usize; 5] = [64, 4, 44, 64, 64];

    /// Venue price encoding: the price's significant digits as a mantissa
    /// (≤ 2^59) with the decimal-place count (≤ 31) in the low five bits.
    pub(crate) fn encode_price(price: Decimal) -> Result<u64, String> {
        if price.is_sign_negative() {
            return Err(format!(
                "[world-markets] price {price} must not be negative"
            ));
        }
        let normalized = price.normalize();
        let decimals = normalized.scale();
        if decimals > 31 {
            return Err(format!(
                "[world-markets] price {price} has {decimals} decimal places; the venue allows 31"
            ));
        }
        let mantissa = normalized.mantissa().unsigned_abs();
        if mantissa > 1u128 << 59 {
            return Err(format!(
                "[world-markets] price {price} mantissa exceeds the venue's 59-bit limit"
            ));
        }
        Ok(((mantissa as u64) << 5) | u64::from(decimals))
    }

    pub(crate) fn pack(&self) -> Result<U256, String> {
        if self.account_id >= 1 << 44 {
            return Err(format!(
                "[world-markets] account id {} does not fit the 44-bit order field",
                self.account_id
            ));
        }
        let price = Self::encode_price(self.limit_price)?;
        let fields = [
            self.insertion_hint,
            self.order_type.code(),
            self.account_id,
            self.quantity_raw,
            price,
        ];
        let mut packed = U256::ZERO;
        for (value, bits) in fields.iter().zip(Self::LAYOUT) {
            let mask = (U256::from(1u8) << bits) - U256::from(1u8);
            packed = (packed << bits) | (U256::from(*value) & mask);
        }
        Ok(packed)
    }

    pub(crate) fn to_json(&self) -> Result<Value, String> {
        let word = self.pack()?;
        Ok(json!({
            "word": format!("0x{:064x}", word),
            "word_decimal": word.to_string(),
            "price_encoded": Self::encode_price(self.limit_price)?.to_string(),
            "quantity_scaled": self.quantity_raw.to_string(),
            "order_type": self.order_type,
            "account_id": self.account_id,
            "insertion_hint": self.insertion_hint,
        }))
    }
}

/// Scale a decimal base quantity to venue position units, truncating any
/// precision the book cannot represent.
pub(crate) fn quantity_raw(quantity: Decimal, position_decimals: u8) -> Result<u64, String> {
    let scaled = quantity
        .checked_mul(Decimal::from(10u64.pow(u32::from(position_decimals))))
        .ok_or_else(|| "[world-markets] quantity exceeds numeric range".to_string())?
        .trunc();
    if scaled.is_sign_negative() {
        return Err("[world-markets] quantity must not be negative".to_string());
    }
    u64::from_str(&scaled.normalize().to_string()).map_err(|_| {
        format!("[world-markets] quantity {quantity} does not fit the 64-bit order field")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(raw: &str) -> Decimal {
        Decimal::from_str(raw).unwrap()
    }

    struct Vector {
        account: u64,
        quantity: &'static str,
        price: &'static str,
        order_type: OrderType,
        hint: u64,
        position_decimals: u8,
        quantity_scaled: &'static str,
        price_encoded: u64,
        word: &'static str,
    }

    /// Vectors generated once from the venue SDK packer
    /// (`@concord/tools` `pack`/`scale` + `@concord/domain` `encodePrice`
    /// with the `SpotOrder` layout `[64, 4, 44, 64, 64]`).
    const VECTORS: &[Vector] = &[
        Vector {
            account: 1577,
            quantity: "0.01",
            price: "2465.71",
            order_type: OrderType::Limit,
            hint: 0,
            position_decimals: 4,
            quantity_scaled: "100",
            price_encoded: 7890274,
            word: "0x0000000000000000000000000000062900000000000000640000000000786562",
        },
        Vector {
            account: 19,
            quantity: "1",
            price: "2000",
            order_type: OrderType::FillAllOrRevert,
            hint: 0,
            position_decimals: 4,
            quantity_scaled: "10000",
            price_encoded: 64000,
            word: "0x000000000000000000002000000000130000000000002710000000000000fa00",
        },
        Vector {
            account: 42,
            quantity: "250.5",
            price: "0.05",
            order_type: OrderType::FillPartialKillRest,
            hint: 77,
            position_decimals: 6,
            quantity_scaled: "250500000",
            price_encoded: 162,
            word: "0x0000000000000000004d30000000002a000000000eee53a000000000000000a2",
        },
        Vector {
            account: 1,
            quantity: "0.0001",
            price: "123456.789",
            order_type: OrderType::Limit,
            hint: 123456789,
            position_decimals: 4,
            quantity_scaled: "1",
            price_encoded: 3950617251,
            word: "0x000000000000075bcd15000000000001000000000000000100000000eb79a2a3",
        },
        Vector {
            account: 17592186044415,
            quantity: "12.3456",
            price: "0.00001234",
            order_type: OrderType::Limit,
            hint: 0,
            position_decimals: 4,
            quantity_scaled: "123456",
            price_encoded: 39496,
            word: "0x000000000000000000000fffffffffff000000000001e2400000000000009a48",
        },
        Vector {
            account: 1577,
            quantity: "3",
            price: "1.10",
            order_type: OrderType::FillAllOrRevert,
            hint: 5,
            position_decimals: 2,
            quantity_scaled: "300",
            price_encoded: 353,
            word: "0x00000000000000000005200000000629000000000000012c0000000000000161",
        },
    ];

    #[test]
    fn order_word_matches_ts_vectors() {
        for v in VECTORS {
            let raw = quantity_raw(d(v.quantity), v.position_decimals).unwrap();
            assert_eq!(
                raw.to_string(),
                v.quantity_scaled,
                "quantity {}",
                v.quantity
            );
            assert_eq!(
                OrderWord::encode_price(d(v.price)).unwrap(),
                v.price_encoded,
                "price {}",
                v.price
            );
            let value = OrderWord {
                account_id: v.account,
                quantity_raw: raw,
                limit_price: d(v.price),
                order_type: v.order_type,
                insertion_hint: v.hint,
            }
            .to_json()
            .unwrap();
            assert_eq!(
                value["word"], v.word,
                "word for {} {}@{}",
                v.account, v.quantity, v.price
            );
            assert_eq!(value["price_encoded"], v.price_encoded.to_string());
            assert_eq!(value["quantity_scaled"], v.quantity_scaled);
        }
    }

    #[test]
    fn price_encoding_round_trips_the_client_decoder() {
        for raw in ["2465.71", "0.05", "1.10", "2000", "123456.789"] {
            let encoded = OrderWord::encode_price(d(raw)).unwrap();
            assert_eq!(
                d(&crate::client::decode_price(encoded)),
                d(raw).normalize(),
                "{raw}"
            );
        }
    }

    #[test]
    fn fields_are_bounded() {
        assert!(OrderWord::encode_price(d("-1")).is_err());
        assert!(OrderWord::encode_price(d("0.0000000000000000000000000001")).is_ok());
        assert!(
            OrderWord {
                account_id: 1 << 44,
                quantity_raw: 1,
                limit_price: d("1"),
                order_type: OrderType::Limit,
                insertion_hint: 0,
            }
            .pack()
            .is_err()
        );
        assert!(quantity_raw(d("-1"), 4).is_err());
        assert_eq!(quantity_raw(d("0.123456789"), 4).unwrap(), 1234);
        assert!(quantity_raw(d("99999999999999999999"), 4).is_err());
    }

    #[test]
    fn order_type_wire_names_are_snake_case() {
        assert_eq!(
            serde_json::to_value(OrderType::FillPartialKillRest).unwrap(),
            "fill_partial_kill_rest"
        );
        assert_eq!(
            serde_json::from_value::<OrderType>(json!("fill_all_or_revert")).unwrap(),
            OrderType::FillAllOrRevert
        );
    }
}
