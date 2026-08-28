//! JSON-RPC transport: timeouts, in-process cache, and request batching.
//!
//! Live tools issue dozens of `eth_call`s. Without this layer each one is its
//! own HTTP POST, and `evaluate()` used to refetch the same funding history on
//! every binary-search step.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Live account/mark reads stay warm for the active-session refresh cadence.
pub(crate) const DEFAULT_TTL: Duration = Duration::from_secs(60);
pub(crate) const TOKEN_CONFIG_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct RpcStats {
    pub(crate) posts: u64,
    pub(crate) hits: u64,
    pub(crate) misses: u64,
    pub(crate) post_ms: u64,
    pub(crate) methods: BTreeMap<String, MethodStats>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct MethodStats {
    pub(crate) posts: u64,
    pub(crate) hits: u64,
    pub(crate) ms: u64,
}

impl RpcStats {
    pub(crate) fn saturating_sub(&self, earlier: &Self) -> Self {
        let mut methods = BTreeMap::new();
        for (name, later) in &self.methods {
            let before = earlier.methods.get(name).cloned().unwrap_or_default();
            methods.insert(
                name.clone(),
                MethodStats {
                    posts: later.posts.saturating_sub(before.posts),
                    hits: later.hits.saturating_sub(before.hits),
                    ms: later.ms.saturating_sub(before.ms),
                },
            );
        }
        Self {
            posts: self.posts.saturating_sub(earlier.posts),
            hits: self.hits.saturating_sub(earlier.hits),
            misses: self.misses.saturating_sub(earlier.misses),
            post_ms: self.post_ms.saturating_sub(earlier.post_ms),
            methods,
        }
    }
}

struct CacheEntry {
    value: Value,
    expires: Instant,
}

#[derive(Default)]
struct RpcCache {
    entries: BTreeMap<String, CacheEntry>,
    stats: RpcStats,
}

impl RpcCache {
    fn get(&mut self, key: &str, method: &str) -> Option<Value> {
        let hit = self
            .entries
            .get(key)
            .filter(|entry| entry.expires > Instant::now())
            .map(|entry| entry.value.clone());
        if hit.is_some() {
            self.stats.hits += 1;
            self.stats
                .methods
                .entry(method.to_string())
                .or_default()
                .hits += 1;
        } else {
            self.entries.remove(key);
            self.stats.misses += 1;
        }
        hit
    }

    fn store(&mut self, key: String, value: Value, ttl: Duration) {
        self.entries.insert(
            key,
            CacheEntry {
                value,
                expires: Instant::now() + ttl,
            },
        );
    }

    fn invalidate_volatile(&mut self) {
        self.entries
            .retain(|key, _| key.contains("bulkReadTokenConfigs"));
    }

    fn record_post(&mut self, method: &str, ms: u128) {
        self.stats.posts += 1;
        self.stats.post_ms = self.stats.post_ms.saturating_add(ms as u64);
        let slot = self.stats.methods.entry(method.to_string()).or_default();
        slot.posts += 1;
        slot.ms = slot.ms.saturating_add(ms as u64);
    }
}

pub(crate) trait RpcExecutor: Send + Sync {
    fn post_json(&self, body: &Value) -> Result<Value, String>;
}

pub(crate) struct HttpExecutor {
    http: Client,
    url: String,
}

impl HttpExecutor {
    fn new(url: String) -> Self {
        let http = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .pool_max_idle_per_host(8)
            .build()
            .expect("World RPC HTTP client");
        Self { http, url }
    }
}

impl RpcExecutor for HttpExecutor {
    fn post_json(&self, body: &Value) -> Result<Value, String> {
        self.http
            .post(&self.url)
            .json(body)
            .send()
            .and_then(|response| response.error_for_status())
            .map_err(|e| format!("[world-markets] World RPC request failed: {e}"))?
            .json()
            .map_err(|e| format!("[world-markets] World RPC response was invalid: {e}"))
    }
}

#[derive(Clone)]
pub(crate) struct RpcTransport {
    executor: Arc<dyn RpcExecutor>,
    cache: Arc<Mutex<RpcCache>>,
}

impl RpcTransport {
    pub(crate) fn http(url: String) -> Self {
        Self {
            executor: Arc::new(HttpExecutor::new(url)),
            cache: Arc::new(Mutex::new(RpcCache::default())),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_executor(executor: Arc<dyn RpcExecutor>) -> Self {
        Self {
            executor,
            cache: Arc::new(Mutex::new(RpcCache::default())),
        }
    }

    pub(crate) fn stats(&self) -> RpcStats {
        self.lock().stats.clone()
    }

    /// Drop cached account/mark/block reads. Token configs stay.
    pub(crate) fn invalidate_volatile(&self) {
        self.lock().invalidate_volatile();
    }

    pub(crate) fn cached_value(
        &self,
        key: &str,
        method: &str,
        ttl: Duration,
        body: Value,
    ) -> Result<Value, String> {
        if let Some(hit) = self.lock().get(key, method) {
            return Ok(hit);
        }
        let value = self.post_one(method, &body)?;
        let result = extract_result(method, value)?;
        self.lock().store(key.to_string(), result.clone(), ttl);
        Ok(result)
    }

    /// Fetch many JSON-RPC results, using cache per key and one HTTP batch for misses.
    /// Outer error is transport failure; inner error is a per-call RPC/revert.
    pub(crate) fn cached_many(
        &self,
        items: &[(String, String, Duration, Value)],
    ) -> Result<Vec<Result<Value, String>>, String> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let mut out: Vec<Option<Result<Value, String>>> = vec![None; items.len()];
        let mut pending: Vec<(usize, Value)> = Vec::new();
        {
            let mut cache = self.lock();
            for (i, (key, method, _, _)) in items.iter().enumerate() {
                if let Some(hit) = cache.get(key, method) {
                    out[i] = Some(Ok(hit));
                } else {
                    pending.push((i, items[i].3.clone()));
                }
            }
        }
        if pending.is_empty() {
            return Ok(out.into_iter().map(|v| v.expect("filled")).collect());
        }
        let fetched = self.post_batch(&pending)?;
        {
            let mut cache = self.lock();
            for ((i, _), value) in pending.iter().zip(fetched.into_iter()) {
                let (key, method, ttl, _) = &items[*i];
                match extract_result(method, value) {
                    Ok(result) => {
                        cache.store(key.clone(), result.clone(), *ttl);
                        out[*i] = Some(Ok(result));
                    }
                    Err(err) => out[*i] = Some(Err(err)),
                }
            }
        }
        Ok(out.into_iter().map(|v| v.expect("filled")).collect())
    }

    fn post_one(&self, method: &str, body: &Value) -> Result<Value, String> {
        let started = Instant::now();
        let value = self.executor.post_json(body)?;
        let ms = started.elapsed().as_millis();
        self.lock().record_post(method, ms);
        if trace_enabled() {
            eprintln!("[world-markets rpc] post method={method} ms={ms}");
        }
        Ok(value)
    }

    fn post_batch(&self, pending: &[(usize, Value)]) -> Result<Vec<Value>, String> {
        if pending.len() == 1 {
            let method = pending[0]
                .1
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("rpc");
            return Ok(vec![self.post_one(method, &pending[0].1)?]);
        }
        let batch: Vec<Value> = pending.iter().map(|(_, body)| body.clone()).collect();
        let started = Instant::now();
        match self.executor.post_json(&Value::Array(batch.clone())) {
            Ok(Value::Array(items)) if items.len() == pending.len() || !items.is_empty() => {
                let ms = started.elapsed().as_millis();
                self.lock().record_post("batch", ms);
                if trace_enabled() {
                    eprintln!(
                        "[world-markets rpc] post method=batch n={} ms={ms}",
                        pending.len()
                    );
                }
                Ok(align_batch(&batch, items))
            }
            Ok(_) | Err(_) => {
                // Node rejected the batch envelope; fall back per item.
                pending
                    .iter()
                    .map(|(_, body)| {
                        let method = body.get("method").and_then(Value::as_str).unwrap_or("rpc");
                        self.post_one(method, body)
                    })
                    .collect()
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RpcCache> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Deserialize)]
struct RpcEnvelope {
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

fn extract_result(method: &str, value: Value) -> Result<Value, String> {
    let envelope: RpcEnvelope = serde_json::from_value(value)
        .map_err(|e| format!("[world-markets] World RPC response was invalid: {e}"))?;
    match (envelope.result, envelope.error) {
        (Some(result), _) => Ok(result),
        (None, Some(error)) => Err(format!(
            "[world-markets] RPC {method} failed ({}): {}",
            error.code, error.message
        )),
        (None, None) => Err(format!("[world-markets] RPC {method} returned no result")),
    }
}

fn align_batch(requests: &[Value], responses: Vec<Value>) -> Vec<Value> {
    if responses.len() == requests.len() && responses.iter().all(|item| item.get("id").is_none()) {
        return responses;
    }
    let mut by_id: BTreeMap<String, Value> = BTreeMap::new();
    for item in responses {
        if let Some(id) = item.get("id") {
            by_id.insert(id_key(id), item);
        }
    }
    requests
        .iter()
        .enumerate()
        .map(|(i, req)| {
            let key = req.get("id").map(id_key).unwrap_or_else(|| i.to_string());
            by_id.remove(&key).unwrap_or_else(|| {
                json!({
                    "jsonrpc": "2.0",
                    "id": req.get("id").cloned().unwrap_or(json!(i)),
                    "error": { "code": -32000, "message": "missing batch item" }
                })
            })
        })
        .collect()
}

fn id_key(id: &Value) -> String {
    match id {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub(crate) fn trace_enabled() -> bool {
    std::env::var("WORLD_RPC_TRACE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub(crate) fn ttl_for_signature(signature: &str) -> Duration {
    if signature.starts_with("bulkReadTokenConfigs") {
        TOKEN_CONFIG_TTL
    } else {
        DEFAULT_TTL
    }
}

pub(crate) fn jsonrpc(id: u64, method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct CountingExecutor {
        posts: AtomicU64,
        result: Value,
    }

    impl RpcExecutor for CountingExecutor {
        fn post_json(&self, body: &Value) -> Result<Value, String> {
            self.posts.fetch_add(1, Ordering::SeqCst);
            if let Value::Array(items) = body {
                let mut out = Vec::new();
                for item in items {
                    out.push(json!({
                        "jsonrpc": "2.0",
                        "id": item.get("id").cloned().unwrap_or(json!(1)),
                        "result": self.result,
                    }));
                }
                return Ok(Value::Array(out));
            }
            Ok(json!({
                "jsonrpc": "2.0",
                "id": body.get("id").cloned().unwrap_or(json!(1)),
                "result": self.result,
            }))
        }
    }

    #[test]
    fn cache_hit_skips_second_post() {
        let executor = Arc::new(CountingExecutor {
            posts: AtomicU64::new(0),
            result: json!("0x1"),
        });
        let transport = RpcTransport::with_executor(executor.clone());
        let body = jsonrpc(1, "eth_blockNumber", json!([]));
        let a = transport
            .cached_value(
                "eth_blockNumber",
                "eth_blockNumber",
                DEFAULT_TTL,
                body.clone(),
            )
            .unwrap();
        let b = transport
            .cached_value("eth_blockNumber", "eth_blockNumber", DEFAULT_TTL, body)
            .unwrap();
        assert_eq!(a, json!("0x1"));
        assert_eq!(b, json!("0x1"));
        assert_eq!(executor.posts.load(Ordering::SeqCst), 1);
        assert_eq!(transport.stats().hits, 1);
        assert_eq!(transport.stats().posts, 1);
    }

    #[test]
    fn batch_posts_once_for_misses() {
        let executor = Arc::new(CountingExecutor {
            posts: AtomicU64::new(0),
            result: json!("0xabc"),
        });
        let transport = RpcTransport::with_executor(executor.clone());
        let items = vec![
            (
                "k1".to_string(),
                "eth_call".to_string(),
                DEFAULT_TTL,
                jsonrpc(0, "eth_call", json!([])),
            ),
            (
                "k2".to_string(),
                "eth_call".to_string(),
                DEFAULT_TTL,
                jsonrpc(1, "eth_call", json!([])),
            ),
        ];
        let values = transport.cached_many(&items).unwrap();
        assert_eq!(values.len(), 2);
        assert!(
            values
                .iter()
                .all(|v| v.as_ref().unwrap() == &json!("0xabc"))
        );
        assert_eq!(executor.posts.load(Ordering::SeqCst), 1);
        let again = transport.cached_many(&items).unwrap();
        assert_eq!(again.len(), 2);
        assert_eq!(executor.posts.load(Ordering::SeqCst), 1);
        assert_eq!(transport.stats().hits, 2);
    }

    #[test]
    fn token_config_ttl_is_longer() {
        assert_eq!(
            ttl_for_signature("bulkReadTokenConfigs_3423260018()"),
            TOKEN_CONFIG_TTL
        );
        assert_eq!(ttl_for_signature("getBalance(uint64,uint32)"), DEFAULT_TTL);
        assert_eq!(DEFAULT_TTL, Duration::from_secs(60));
    }

    #[test]
    fn invalidate_volatile_keeps_token_configs() {
        let executor = Arc::new(CountingExecutor {
            posts: AtomicU64::new(0),
            result: json!("0x1"),
        });
        let transport = RpcTransport::with_executor(executor.clone());
        let block = jsonrpc(1, "eth_blockNumber", json!([]));
        let configs = jsonrpc(2, "eth_call", json!([]));
        transport
            .cached_value(
                "eth_blockNumber",
                "eth_blockNumber",
                DEFAULT_TTL,
                block.clone(),
            )
            .unwrap();
        transport
            .cached_value(
                "eth_call:0xabc:bulkReadTokenConfigs",
                "bulkReadTokenConfigs_3423260018()",
                TOKEN_CONFIG_TTL,
                configs.clone(),
            )
            .unwrap();
        transport.invalidate_volatile();
        transport
            .cached_value("eth_blockNumber", "eth_blockNumber", DEFAULT_TTL, block)
            .unwrap();
        transport
            .cached_value(
                "eth_call:0xabc:bulkReadTokenConfigs",
                "bulkReadTokenConfigs_3423260018()",
                TOKEN_CONFIG_TTL,
                configs,
            )
            .unwrap();
        assert_eq!(
            executor.posts.load(Ordering::SeqCst),
            3,
            "block must refetch; token configs must remain cached"
        );
    }
}
