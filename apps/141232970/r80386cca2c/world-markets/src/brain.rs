//! HTTP client for the unsigned brain sidecar (`brain/`).
//!
//! News, mark history, watches, preferences, and the outbound queue live there.
//! This client has no signing, submit, or order-placement path.

use std::time::Duration;

use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::{Value, json};

const DEFAULT_URL: &str = "http://127.0.0.1:8788";

#[derive(Clone)]
pub(crate) struct BrainClient {
    http: Client,
    base_url: String,
}

impl Default for BrainClient {
    fn default() -> Self {
        Self::from_env()
    }
}

impl BrainClient {
    pub(crate) fn from_env() -> Self {
        Self::with_timeout(20)
    }

    pub(crate) fn with_timeout(secs: u64) -> Self {
        let base_url = std::env::var("WORLD_BRAIN_URL")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_URL.to_string());
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(secs))
                .build()
                .expect("brain HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub(crate) fn research(&self, symbol: &str, window_secs: u64) -> Result<Value, String> {
        self.get(&format!(
            "/v1/research?symbol={symbol}&window_secs={window_secs}"
        ))
    }

    pub(crate) fn history_move(&self, symbol: &str, window_secs: u64) -> Result<Value, String> {
        self.get(&format!(
            "/v1/history/move?symbol={symbol}&window_secs={window_secs}"
        ))
    }

    pub(crate) fn tasks(&self, account_id: u64) -> Result<Value, String> {
        self.get(&format!("/v1/tasks?account_id={account_id}"))
    }

    pub(crate) fn ingest(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/history/ingest", body)
    }

    pub(crate) fn portfolio_impact(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/portfolio-impact", body)
    }

    pub(crate) fn set_watch(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/watches", body)
    }

    pub(crate) fn cancel_watch(&self, account_id: u64, id: &str) -> Result<Value, String> {
        self.post(
            "/v1/watches/cancel",
            &json!({ "account_id": account_id, "id": id }),
        )
    }

    pub(crate) fn set_preference(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/preferences", body)
    }

    pub(crate) fn cancel_preference(&self, account_id: u64, id: &str) -> Result<Value, String> {
        self.post(
            "/v1/preferences/cancel",
            &json!({ "account_id": account_id, "id": id }),
        )
    }

    pub(crate) fn seed_brief(&self, account_id: u64, brief: &Value) -> Result<Value, String> {
        self.post(
            "/v1/preferences/seed",
            &json!({ "account_id": account_id, "brief": brief }),
        )
    }

    pub(crate) fn drain_outbound(&self, limit: u32) -> Result<Value, String> {
        self.post("/v1/outbound/drain", &json!({ "limit": limit }))
    }

    pub(crate) fn enqueue_outbound(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/outbound/enqueue", body)
    }

    pub(crate) fn ledger_summary(&self, account_id: u64) -> Result<Value, String> {
        self.get(&format!("/v1/ledger/summary?account_id={account_id}"))
    }

    pub(crate) fn ledger(&self, account_id: u64) -> Result<Value, String> {
        self.get(&format!("/v1/ledger?account_id={account_id}"))
    }

    pub(crate) fn ledger_one(&self, account_id: u64, id: &str) -> Result<Value, String> {
        let encoded = id
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect::<String>();
        self.get(&format!("/v1/ledger/{encoded}?account_id={account_id}"))
    }

    pub(crate) fn compose(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/compose", body)
    }

    pub(crate) fn stage_trade(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/trades/stage", body)
    }

    #[allow(dead_code)]
    pub(crate) fn begin_execute(
        &self,
        account_id: u64,
        instruction_id: &str,
    ) -> Result<Value, String> {
        self.post(
            "/v1/trades/begin",
            &json!({ "account_id": account_id, "instruction_id": instruction_id }),
        )
    }

    pub(crate) fn claim_slice(
        &self,
        account_id: u64,
        instruction_id: &str,
    ) -> Result<Value, String> {
        self.post(
            "/v1/trades/claim",
            &json!({ "account_id": account_id, "instruction_id": instruction_id }),
        )
    }

    pub(crate) fn record_slice(
        &self,
        account_id: u64,
        instruction_id: &str,
        body: &Value,
    ) -> Result<Value, String> {
        let mut payload = body.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("account_id".to_string(), json!(account_id));
            obj.insert("instruction_id".to_string(), json!(instruction_id));
        }
        self.post("/v1/trades/progress", &payload)
    }

    pub(crate) fn due_trades(&self, account_id: Option<u64>) -> Result<Value, String> {
        let path = match account_id {
            Some(id) => format!("/v1/trades/due?account_id={id}"),
            None => "/v1/trades/due".to_string(),
        };
        self.get(&path)
    }

    pub(crate) fn complete_execute(
        &self,
        account_id: u64,
        instruction_id: &str,
        body: &Value,
    ) -> Result<Value, String> {
        let mut payload = body.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("account_id".to_string(), json!(account_id));
            obj.insert("instruction_id".to_string(), json!(instruction_id));
        }
        self.post("/v1/trades/complete", &payload)
    }

    pub(crate) fn pause_watch(
        &self,
        account_id: u64,
        id: Option<&str>,
        instruction_id: Option<&str>,
    ) -> Result<Value, String> {
        self.post(
            "/v1/watches/pause",
            &json!({
                "account_id": account_id,
                "id": id,
                "instruction_id": instruction_id,
            }),
        )
    }

    pub(crate) fn resume_watch(
        &self,
        account_id: u64,
        id: Option<&str>,
        instruction_id: Option<&str>,
    ) -> Result<Value, String> {
        self.post(
            "/v1/watches/resume",
            &json!({
                "account_id": account_id,
                "id": id,
                "instruction_id": instruction_id,
            }),
        )
    }

    pub(crate) fn cancel_task(&self, account_id: u64, id: &str) -> Result<Value, String> {
        self.post(
            "/v1/tasks/cancel",
            &json!({ "account_id": account_id, "id": id }),
        )
    }

    pub(crate) fn voice_keyterms(
        &self,
        account_id: u64,
        extra: &[String],
    ) -> Result<Vec<String>, String> {
        let extra_q = extra.join(",");
        let path = if extra_q.is_empty() {
            format!("/v1/voice/keyterms?account_id={account_id}")
        } else {
            format!("/v1/voice/keyterms?account_id={account_id}&extra={extra_q}")
        };
        let value = self.get(&path)?;
        Ok(value
            .get("keyterms")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| row.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub(crate) fn voice_context(&self, account_id: u64) -> Result<Value, String> {
        self.get(&format!("/v1/voice/context?account_id={account_id}"))
    }

    pub(crate) fn ingest_utterance(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/voice/utterance", body)
    }

    pub(crate) fn ontology_summary(&self) -> Result<Value, String> {
        self.get("/v1/ontology/summary")
    }

    pub(crate) fn ontology_stats(
        &self,
        account_id: Option<u64>,
        from: Option<&str>,
        to: Option<&str>,
        all: bool,
    ) -> Result<Value, String> {
        let mut path = "/v1/ontology/stats?".to_string();
        let mut first = true;
        let mut push = |key: &str, value: &str| {
            if !first {
                path.push('&');
            }
            first = false;
            path.push_str(key);
            path.push('=');
            path.push_str(value);
        };
        if let Some(id) = account_id {
            push("account_id", &id.to_string());
        }
        if let Some(from) = from.filter(|s| !s.is_empty()) {
            push("from", from);
        }
        if let Some(to) = to.filter(|s| !s.is_empty()) {
            push("to", to);
        }
        if all {
            push("all", "1");
        }
        self.get(&path)
    }

    pub(crate) fn record_correction(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/voice/correction", body)
    }

    pub(crate) fn set_consent(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/voice/consent", body)
    }

    pub(crate) fn close_episode(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/voice/episode/close", body)
    }

    pub(crate) fn heard(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/heard", body)
    }

    pub(crate) fn share(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/share", body)
    }

    pub(crate) fn confirm_action_kind(&self, account_id: u64, kind: &str) -> Result<Value, String> {
        self.post(
            "/v1/action-kinds/confirm",
            &json!({ "account_id": account_id, "kind": kind }),
        )
    }

    pub(crate) fn action_kind_status(&self, account_id: u64, kind: &str) -> Result<Value, String> {
        self.get(&format!(
            "/v1/action-kinds?account_id={account_id}&kind={kind}"
        ))
    }

    pub(crate) fn supersede_watch(&self, body: &Value) -> Result<Value, String> {
        self.post("/v1/watches/supersede", body)
    }

    pub(crate) fn match_watches(&self, account_id: u64, symbol: &str) -> Result<Value, String> {
        self.get(&format!(
            "/v1/watches/match?account_id={account_id}&symbol={symbol}"
        ))
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}{path}", self.base_url);
        let response = self
            .http
            .get(&url)
            .send()
            .map_err(|error| unreachable_brain(&url, error))?;
        parse_response(response)
    }

    fn post<T: Serialize>(&self, path: &str, body: &T) -> Result<Value, String> {
        let url = format!("{}{path}", self.base_url);
        let response = self
            .http
            .post(&url)
            .json(body)
            .send()
            .map_err(|error| unreachable_brain(&url, error))?;
        parse_response(response)
    }
}

fn parse_response(response: reqwest::blocking::Response) -> Result<Value, String> {
    let status = response.status();
    let value: Value = response
        .json()
        .map_err(|error| format!("[world-markets] brain returned invalid JSON: {error}"))?;
    if !status.is_success() || value.get("ok") == Some(&Value::Bool(false)) {
        let detail = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("brain sidecar rejected the request");
        return Err(format!("[world-markets] {detail}"));
    }
    Ok(value)
}

fn unreachable_brain(url: &str, error: reqwest::Error) -> String {
    format!(
        "[world-markets] brain sidecar is not reachable at {url} ({error}). Start it with `npm start` in brain/ (scripts/dev-run.sh starts it)."
    )
}
