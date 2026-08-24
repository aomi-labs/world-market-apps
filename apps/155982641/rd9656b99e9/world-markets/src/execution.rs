//! HTTP client for the local execution sidecar (`sidecar/`).
//!
//! The plugin never holds a private key. Locally the sidecar signs with
//! `WORLD_PRIVATE_KEY`. Later this module is the swap point for an Aomi-hosted
//! signer: keep the request types, replace [`ExecutionClient`] internals.

use std::time::Duration;

use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::Value;

const DEFAULT_URL: &str = "http://127.0.0.1:8787";

#[derive(Clone)]
pub(crate) struct ExecutionClient {
    http: Client,
    base_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PlaceOrderRequest {
    pub(crate) account_id: u64,
    pub(crate) product: String,
    pub(crate) side: String,
    pub(crate) base_token_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) quote_token_id: Option<u32>,
    pub(crate) quantity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) price: Option<String>,
    pub(crate) order_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) slippage: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CancelOrderRequest {
    pub(crate) account_id: u64,
    pub(crate) product: String,
    pub(crate) side: String,
    pub(crate) base_token_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) quote_token_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) interest_rate: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SwapRequest {
    pub(crate) account_id: u64,
    pub(crate) token_in: String,
    pub(crate) token_out: String,
    pub(crate) amount_in: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) slippage: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RenewLoansRequest {
    pub(crate) account_id: u64,
    pub(crate) token_ids: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_hours_remaining: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PayInterestRequest {
    pub(crate) account_id: u64,
    pub(crate) token_ids: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) position_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) extend_period: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CloseLoanRequest {
    pub(crate) account_id: u64,
    pub(crate) token_ids: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) position_id: Option<String>,
}

impl Default for ExecutionClient {
    fn default() -> Self {
        Self::from_env()
    }
}

impl ExecutionClient {
    pub(crate) fn from_env() -> Self {
        let base_url = std::env::var("WORLD_EXECUTION_URL")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_URL.to_string());
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("execution HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub(crate) fn place_order(&self, request: &PlaceOrderRequest) -> Result<Value, String> {
        self.post("/v1/orders", request)
    }

    pub(crate) fn cancel_order(&self, request: &CancelOrderRequest) -> Result<Value, String> {
        self.post("/v1/orders/cancel", request)
    }

    pub(crate) fn swap(&self, request: &SwapRequest) -> Result<Value, String> {
        self.post("/v1/swaps", request)
    }

    pub(crate) fn renew_loans(&self, request: &RenewLoansRequest) -> Result<Value, String> {
        self.post("/v1/loans/renew", request)
    }

    pub(crate) fn pay_interest(&self, request: &PayInterestRequest) -> Result<Value, String> {
        self.post("/v1/loans/pay-interest", request)
    }

    pub(crate) fn close_loan(&self, request: &CloseLoanRequest) -> Result<Value, String> {
        self.post("/v1/loans/close", request)
    }

    fn post<T: Serialize>(&self, path: &str, body: &T) -> Result<Value, String> {
        let url = format!("{}{path}", self.base_url);
        let response = self
            .http
            .post(&url)
            .json(body)
            .send()
            .map_err(|error| sidecar_unreachable(&url, error))?;
        let status = response.status();
        let value: Value = response.json().map_err(|error| {
            format!("[world-markets] execution sidecar returned invalid JSON: {error}")
        })?;
        if !status.is_success() || value.get("ok") == Some(&Value::Bool(false)) {
            let detail = value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("execution sidecar rejected the request");
            return Err(format!("[world-markets] {detail}"));
        }
        Ok(value)
    }
}

fn sidecar_unreachable(url: &str, error: reqwest::Error) -> String {
    format!(
        "[world-markets] execution sidecar is not reachable at {url} ({error}). Start it with scripts/dev-run.sh or `npm start` in sidecar/"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_order_json_omits_optional_fields() {
        let request = PlaceOrderRequest {
            account_id: 17,
            product: "perp".to_string(),
            side: "buy".to_string(),
            base_token_id: 2,
            quote_token_id: Some(1),
            quantity: "0.1".to_string(),
            price: None,
            order_type: "market".to_string(),
            slippage: None,
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["account_id"], 17);
        assert!(value.get("price").is_none());
        assert!(value.get("slippage").is_none());
        assert_eq!(value["quote_token_id"], 1);
    }
}
