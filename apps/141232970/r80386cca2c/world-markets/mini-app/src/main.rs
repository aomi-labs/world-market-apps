//! Telegram Mini App server: portfolio snapshot, instruction ledger, charts,
//! the tradeable-product catalog, Desk context (read-only), and hold-to-talk
//! voice notes into the Aomi agent (not The Desk).
//!
//! Init-data HMAC follows Telegram's WebApp algorithm (HMAC-SHA256 keyed by
//! `WebAppData`, then HMAC of the sorted data-check string). The Mini App spec's
//! shorter "HMAC with the bot token as key" does not match Telegram and would
//! reject every real session.
//!
//! `/api/v1/mini-app/ledger*` is GET-only. Compose writes go through
//! `POST /api/v1/mini-app/compose` (Telegram sendData ingress), never `/ledger*`.
//! Voice notes go through `POST /api/v1/mini-app/voice` (not sendData, not
//! `/ledger*`), then the client sendData's the transcript so the host agent runs.
//! Introduction prepare is `POST /api/v1/mini-app/share` (not `/ledger*`; it
//! mutates nothing the Mini App displays).
//! Compose `kind: flush_execute` is a server-side backup after the 3s trade delay.
//!
//! Init-data HMAC follows Telegram's WebApp algorithm (HMAC-SHA256 keyed by
//! `WebAppData`, then HMAC of the sorted data-check string). The Mini App spec's
//! shorter "HMAC with the bot token as key" does not match Telegram and would
//! reject every real session.
//!
//! `/api/v1/mini-app/ledger*` is GET-only. Compose writes go through
//! `POST /api/v1/mini-app/compose` (Telegram sendData ingress), never `/ledger*`.

mod auth;
mod voice_stream;

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::spawn_blocking;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

const SESSION_TTL: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct AppState {
    bot_token: String,
    account_id: Option<u64>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    dev_bypass: bool,
    desk_token: String,
}

struct Session {
    telegram_user_id: u64,
    first_name: Option<String>,
    expires_at: Instant,
}

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

#[derive(Debug, Deserialize)]
struct AuthRequest {
    init_data: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let bot_token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let dev_bypass = std::env::var("MINI_APP_DEV_BYPASS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if bot_token.is_empty() && !dev_bypass {
        eprintln!("TELEGRAM_BOT_TOKEN is required (or set MINI_APP_DEV_BYPASS=1 for local UI)");
        std::process::exit(1);
    }

    let state = AppState {
        bot_token,
        account_id: world_markets::mini_app::account_id_from_env(),
        sessions: Arc::new(Mutex::new(HashMap::new())),
        dev_bypass,
        desk_token: std::env::var("DESK_BRIDGE_TOKEN").unwrap_or_default(),
    };

    let flush_account = state.account_id;
    let app = Router::new()
        .route("/api/v1/mini-app/auth", post(auth_handler))
        .route("/api/v1/mini-app/portfolio", get(portfolio_handler))
        .route("/api/v1/mini-app/products", get(products_handler))
        .route("/api/v1/mini-app/chart", get(chart_handler))
        .route(
            "/api/v1/mini-app/ledger/summary",
            get(ledger_summary_handler),
        )
        .route("/api/v1/mini-app/ledger/{id}", get(ledger_one_handler))
        .route("/api/v1/mini-app/ledger", get(ledger_handler))
        .route("/api/v1/mini-app/compose", post(compose_handler))
        .route("/api/v1/mini-app/voice/live", post(voice_live_handler))
        .route("/api/v1/mini-app/voice/stream", get(voice_stream_handler))
        .route("/api/v1/mini-app/voice", post(voice_handler))
        .route("/api/v1/mini-app/share", post(share_handler))
        .route(
            "/api/v1/mini-app/speech-ontology",
            get(speech_ontology_handler),
        )
        .route("/api/v1/desk/context", get(desk_context_handler))
        .route("/api/v1/mini-app/health", get(health_handler))
        .route("/dev/ontology", get(dev_ontology_handler))
        .fallback(static_handler)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind = std::env::var("MINI_APP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let addr: SocketAddr = bind.parse().expect("MINI_APP_BIND must be host:port");
    tracing::info!("mini app listening on {addr}");
    if let Some(account_id) = flush_account {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(5));
            loop {
                ticker.tick().await;
                let _ =
                    spawn_blocking(move || world_markets::mini_app::flush_due_trades(account_id))
                        .await;
            }
        });
    }
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind mini-app port");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("mini-app server");
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

async fn health_handler() -> Json<Value> {
    Json(json!({ "ok": true }))
}

const SPEECH_ONTOLOGY_JSON: &str = include_str!("../../assets/speech_ontology.json");

async fn speech_ontology_handler() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(axum::body::Body::from(SPEECH_ONTOLOGY_JSON))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

const DEV_ONTOLOGY_HTML: &str = include_str!("dev_ontology.html");

#[derive(Debug, Deserialize)]
struct DevOntologyQuery {
    #[serde(default)]
    preview: Option<String>,
}

fn localhost_host(headers: &HeaderMap) -> bool {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let host = host.split(':').next().unwrap_or(host);
    host == "127.0.0.1" || host == "localhost" || host == "::1"
}

async fn dev_ontology_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<DevOntologyQuery>,
) -> Response {
    if !state.dev_bypass || !localhost_host(&headers) || query.preview.as_deref() != Some("dev") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let account_id = state.account_id;
    match spawn_blocking(move || {
        let summary = world_markets::mini_app::ontology_summary();
        let stats = world_markets::mini_app::ontology_stats(account_id, None, None, false);
        json!({
            "summary": summary.unwrap_or_else(|err| json!({ "ok": false, "error": err })),
            "stats": stats.unwrap_or_else(|err| json!({ "ok": false, "error": err })),
            "account_id": account_id,
        })
    })
    .await
    {
        Ok(payload) => {
            let mut data = payload;
            let error = data
                .pointer("/summary/error")
                .and_then(Value::as_str)
                .or_else(|| data.pointer("/stats/error").and_then(Value::as_str))
                .map(str::to_string);
            if let Some(err) = error {
                if let Some(obj) = data.as_object_mut() {
                    obj.insert("error".into(), json!(err));
                }
            }
            let html = DEV_ONTOLOGY_HTML.replace("__ONTOLOGY_PAYLOAD__", &data.to_string());
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(axum::body::Body::from(html))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn auth_handler(State(state): State<AppState>, Json(body): Json<AuthRequest>) -> Response {
    if state.dev_bypass && (body.init_data.is_empty() || body.init_data == "dev") {
        return issue_session(&state, 0, None);
    }
    if state.bot_token.is_empty() {
        return json_error(StatusCode::UNAUTHORIZED, "invalid_init_data");
    }
    match auth::verify_init_data(&body.init_data, &state.bot_token) {
        Ok(user) => issue_session(&state, user.id, user.first_name),
        Err(err) => {
            tracing::info!(error = %err, "initData rejected");
            json_error(StatusCode::UNAUTHORIZED, "invalid_init_data")
        }
    }
}

fn issue_session(state: &AppState, telegram_user_id: u64, first_name: Option<String>) -> Response {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    {
        let mut sessions = state.sessions.lock().expect("session lock");
        sessions.retain(|_, session| session.expires_at > Instant::now());
        sessions.insert(
            token.clone(),
            Session {
                telegram_user_id,
                first_name,
                expires_at: Instant::now() + SESSION_TTL,
            },
        );
    }
    (
        StatusCode::OK,
        Json(json!({
            "token": token,
            "expires_at": unix_now() + SESSION_TTL.as_secs(),
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct ChartQuery {
    symbol: String,
    #[serde(default = "default_period")]
    period: String,
}

fn default_period() -> String {
    "d".to_string()
}

async fn portfolio_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        tracing::error!("WORLD_ACCOUNT_ID is not set");
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    match spawn_blocking(move || {
        let mut portfolio = world_markets::mini_app::load_portfolio(account_id)?;
        if let Ok(ledger) = world_markets::mini_app::load_ledger(account_id)
            && let Some(counts) = ledger.get("watch_counts").and_then(Value::as_object)
        {
            world_markets::mini_app::apply_watch_counts(&mut portfolio, counts);
        }
        Ok::<_, String>(portfolio)
    })
    .await
    {
        Ok(Ok(portfolio)) => Json(portfolio).into_response(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "portfolio fetch failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
        Err(err) => {
            tracing::error!(error = %err, "portfolio task join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

async fn products_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    match spawn_blocking(world_markets::mini_app::load_products).await {
        Ok(Ok(products)) => Json(products).into_response(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "products fetch failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
        Err(err) => {
            tracing::error!(error = %err, "products task join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

async fn chart_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ChartQuery>,
) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let symbol = query.symbol.clone();
    let period = query.period.clone();
    match spawn_blocking(move || world_markets::mini_app::load_chart(&symbol, &period)).await {
        Ok(Ok(chart)) => Json(chart).into_response(),
        Ok(Err(world_markets::mini_app::ChartError::BadRequest(_))) => {
            json_error(StatusCode::BAD_REQUEST, "bad_request")
        }
        Ok(Err(world_markets::mini_app::ChartError::NotFound(_))) => {
            json_error(StatusCode::NOT_FOUND, "not_found")
        }
        Ok(Err(err)) => {
            tracing::error!(error = %err, "chart fetch failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
        Err(err) => {
            tracing::error!(error = %err, "chart task join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

async fn ledger_summary_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    match spawn_blocking(move || world_markets::mini_app::load_ledger_summary(account_id)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "ledger summary failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
        Err(err) => {
            tracing::error!(error = %err, "ledger summary join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

async fn ledger_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    match spawn_blocking(move || world_markets::mini_app::load_ledger(account_id)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "ledger fetch failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
        Err(err) => {
            tracing::error!(error = %err, "ledger join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

async fn ledger_one_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    match spawn_blocking(move || world_markets::mini_app::load_instruction(account_id, &id)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) if err.contains("not_found") => json_error(StatusCode::NOT_FOUND, "not_found"),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "ledger item failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
        Err(err) => {
            tracing::error!(error = %err, "ledger item join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

#[derive(Debug, Deserialize)]
struct ComposeRequest {
    #[serde(default)]
    correlation_id: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    instruction_id: Option<String>,
    #[serde(default)]
    fire_kind: Option<String>,
    #[serde(default)]
    instrument: Option<String>,
}

async fn compose_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ComposeRequest>,
) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    let chat_id = session_telegram_user(&state, &headers);
    let bot_token = state.bot_token.clone();
    let kind = body.kind.clone().unwrap_or_default();
    let mut payload = json!({
        "account_id": account_id,
        "correlation_id": body.correlation_id,
        "kind": body.kind,
        "message": body.message,
        "instruction_id": body.instruction_id,
        "fire_kind": body.fire_kind,
        "instrument": body.instrument,
    });
    match spawn_blocking(move || -> Result<Value, String> {
        if kind == "flush_execute" {
            let instruction_id = payload
                .get("instruction_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| "instruction_id required".to_string())?;
            return world_markets::mini_app::flush_staged_trade(account_id, instruction_id);
        }
        let mut message = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if matches!(kind.as_str(), "text" | "imperative" | "conditional") && !message.is_empty() {
            if let Ok(ingested) = world_markets::mini_app::ingest_voice_note(
                account_id,
                &json!({ "text": message, "source": "mini_app" }),
            ) {
                if let Some(text) = ingested.get("transcript").and_then(Value::as_str) {
                    message = text.to_string();
                }
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("message".into(), json!(message.clone()));
                    if let Some(id) = ingested.get("utterance_id") {
                        obj.insert("utterance_ref".into(), id.clone());
                    }
                    if let Some(slots) = ingested.get("slots") {
                        obj.insert("slots".into(), slots.clone());
                    }
                    if let Some(proposals) = ingested
                        .get("proposals")
                        .or_else(|| ingested.get("proposed_confusables"))
                    {
                        obj.insert("proposals".into(), proposals.clone());
                    }
                    obj.insert("channel".into(), json!("text"));
                    if let Some(ir) = ingested.get("action_ir") {
                        obj.insert("action_ir".into(), ir.clone());
                    }
                    if let Some(grammar) = ingested.get("grammar") {
                        obj.insert("grammar".into(), grammar.clone());
                    }
                }
            }
        }
        if !matches!(
            kind.as_str(),
            "question" | "pause" | "resume" | "cancel" | "flush_execute" | "archive"
        ) && !message.is_empty()
        {
            if let Some(handled) =
                world_markets::mini_app::submit_heard(account_id, &message, Some(&payload))
            {
                let skip = handled.get("skip_llm") == Some(&json!(true));
                let heard_kind = handled.get("kind").and_then(Value::as_str).unwrap_or("");
                let remaining = handled
                    .get("remaining_text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if skip || heard_kind == "cant" {
                    if let Some(chat_id) = chat_id {
                        if let Some(reply) = handled.get("message").and_then(Value::as_str) {
                            let _ = world_markets::mini_app::post_chat_lines(
                                &bot_token,
                                chat_id,
                                &[reply.to_string()],
                            );
                        }
                    }
                    // Remainder after a wall only — near-match waits for a chip.
                    if heard_kind == "cant" && !remaining.is_empty() && chat_id.is_none() {
                        let _ = dispatch_local_agent_turn(remaining);
                    }
                    return Ok(handled);
                }
                if handled.get("kind").and_then(Value::as_str) == Some("resolved") {
                    let text = handled
                        .get("rewritten_text")
                        .or_else(|| handled.get("remaining_text"))
                        .and_then(Value::as_str)
                        .unwrap_or(&message);
                    if chat_id.is_none() {
                        let _ = dispatch_local_agent_turn(text);
                    }
                    return Ok(handled);
                }
            }
        }
        let value = world_markets::mini_app::submit_compose(&payload)?;
        if chat_id.is_none()
            && value.get("recorded") == Some(&json!(true))
            && !matches!(
                kind.as_str(),
                "cancel" | "archive" | "flush_execute" | "question"
            )
        {
            let sentence = value
                .pointer("/instruction/sentence")
                .or_else(|| payload.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let _ = dispatch_local_agent_turn(&prompt_with_ir(sentence, payload.get("action_ir")));
        }
        if kind == "cancel" {
            let command = value
                .pointer("/thread/command")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| payload_message(&payload));
            let mut lines = vec![command];
            if let Some(reply) = value
                .pointer("/thread/reply")
                .or_else(|| value.get("reply"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                lines.push(reply.to_string());
            }
            if let Some(chat_id) = chat_id {
                if let Err(err) =
                    world_markets::mini_app::post_chat_lines(&bot_token, chat_id, &lines)
                {
                    tracing::warn!(error = %err, "cancel chat echo failed");
                }
            }
        }
        Ok(value)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "compose failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
        Err(err) => {
            tracing::error!(error = %err, "compose join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

#[derive(Debug, Deserialize)]
struct VoiceRequest {
    #[serde(default)]
    audio_base64: Option<String>,
    #[serde(default)]
    mime: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    live_text: Option<String>,
    #[serde(default)]
    duration_secs: Option<f64>,
    #[serde(default)]
    finalized: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct VoiceStreamQuery {
    #[serde(default)]
    sample_rate: Option<u32>,
    #[serde(default)]
    access_token: Option<String>,
}

async fn share_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    let chat_id = session_telegram_user(&state, &headers);
    let first_name = session_first_name(&state, &headers);
    let bot_token = state.bot_token.clone();
    match spawn_blocking(move || {
        world_markets::mini_app::prepare_introduction(
            account_id,
            chat_id,
            first_name.as_deref(),
            &bot_token,
        )
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "share prepare failed");
            json_error(StatusCode::BAD_GATEWAY, "share_unavailable")
        }
        Err(err) => {
            tracing::error!(error = %err, "share join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

async fn desk_context_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !desk_or_session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    match spawn_blocking(move || world_markets::mini_app::load_desk_context(account_id)).await {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "desk context failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
        Err(err) => {
            tracing::error!(error = %err, "desk context join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

async fn voice_live_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VoiceRequest>,
) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    let payload = json!({
        "audio_base64": body.audio_base64,
        "mime": body.mime,
    });
    match spawn_blocking(move || world_markets::mini_app::transcribe_live(account_id, &payload))
        .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "live caption failed");
            Json(json!({ "text": "" })).into_response()
        }
        Err(err) => {
            tracing::warn!(error = %err, "live caption join failed");
            Json(json!({ "text": "" })).into_response()
        }
    }
}

async fn voice_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VoiceRequest>,
) -> Response {
    if !session_ok(&state, &headers) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    let chat_id = session_telegram_user(&state, &headers);
    let bot_token = state.bot_token.clone();
    let payload = json!({
        "audio_base64": body.audio_base64,
        "mime": body.mime,
        "text": body.text,
        "live_text": body.live_text,
        "duration_secs": body.duration_secs,
        "finalized": body.finalized,
        "source": "mini_app",
    });
    match spawn_blocking(move || -> Result<Value, String> {
        let mut value = world_markets::mini_app::ingest_voice_note(account_id, &payload)?;
        let heard = value
            .get("heard_echo")
            .or_else(|| value.get("transcript"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !heard.is_empty() {
            let utterance_id = value
                .get("utterance_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let extra = json!({
                "origin": "voice",
                "source": "mini_app",
                "utterance_ref": utterance_id,
                "peek": chat_id.is_some(),
                "proposed_confusables": value.get("proposed_confusables"),
                "proposals": value.get("proposals").or_else(|| value.get("proposed_confusables")),
                "slots": value.get("slots"),
                "channel": value.get("channel").cloned().unwrap_or(json!("speech")),
            });
            let handled = world_markets::mini_app::submit_heard(account_id, &heard, Some(&extra));
            if let Some(handled) = handled {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "voice_kind".into(),
                        handled.get("kind").cloned().unwrap_or(json!("")),
                    );
                    obj.insert("heard_handled".into(), handled.clone());
                    if handled.get("skip_llm") == Some(&json!(true)) {
                        obj.insert("skip_send_data".into(), json!(chat_id.is_none()));
                    }
                }
                let kind = handled.get("kind").and_then(Value::as_str).unwrap_or("");
                let remaining = handled
                    .get("remaining_text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if chat_id.is_none() {
                    if kind == "cant" || kind == "unclear" || kind == "near_match" {
                        if kind == "cant" && !remaining.is_empty() {
                            let _ = dispatch_local_agent_turn(&remaining);
                        }
                        if let Some(msg) = handled.get("message").and_then(Value::as_str) {
                            if let Some(obj) = value.as_object_mut() {
                                obj.insert("thread_message".into(), json!(msg));
                            }
                        }
                        Ok(value)
                    } else {
                        // resolved: dispatch rewritten
                        let text = handled
                            .get("rewritten_text")
                            .or_else(|| handled.get("remaining_text"))
                            .and_then(Value::as_str)
                            .unwrap_or(&heard);
                        if dispatch_local_agent_turn(text) {
                            if let Some(obj) = value.as_object_mut() {
                                obj.insert("dispatched".into(), json!(true));
                            }
                        }
                        Ok(value)
                    }
                } else {
                    // Telegram: host render_lookup writes; skip extra heard-echo when we will wall
                    if kind != "cant" && kind != "near_match" && kind != "unclear" {
                        let line = format!("heard: {heard}");
                        if let Some(chat_id) = chat_id {
                            if let Err(err) = world_markets::mini_app::post_chat_lines(
                                &bot_token,
                                chat_id,
                                &[line],
                            ) {
                                tracing::warn!(error = %err, "voice heard-echo failed");
                            }
                        }
                    }
                    Ok(value)
                }
            } else if let Some(chat_id) = chat_id {
                let line = format!("heard: {heard}");
                if let Err(err) =
                    world_markets::mini_app::post_chat_lines(&bot_token, chat_id, &[line])
                {
                    tracing::warn!(error = %err, "voice heard-echo failed");
                }
                Ok(value)
            } else if dispatch_local_agent_turn(&prompt_with_ir(&heard, value.get("action_ir"))) {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("dispatched".to_string(), json!(true));
                }
                Ok(value)
            } else {
                let correlation_id = value
                    .get("correlation_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                match world_markets::mini_app::submit_compose(&json!({
                    "account_id": account_id,
                    "kind": "voice",
                    "message": heard,
                    "correlation_id": correlation_id,
                    "instruction_id": correlation_id,
                })) {
                    Ok(composed) => {
                        if let Some(obj) = value.as_object_mut() {
                            obj.insert("composed".to_string(), json!(true));
                            if let Some(id) = composed
                                .pointer("/instruction/instruction_id")
                                .and_then(Value::as_str)
                            {
                                obj.insert("instruction_id".to_string(), json!(id));
                            }
                        }
                    }
                    Err(err) => tracing::warn!(error = %err, "voice compose fallback failed"),
                }
                Ok(value)
            }
        } else {
            Ok(value)
        }
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) => {
            tracing::error!(error = %err, "voice note failed");
            let (code, key) = if err.contains("not configured") {
                (StatusCode::BAD_GATEWAY, "stt_unconfigured")
            } else if err.contains("not reachable") || err.contains("brain sidecar") {
                (StatusCode::BAD_GATEWAY, "voice_unavailable")
            } else if err.contains("didn't catch") || err.contains("empty") {
                (StatusCode::BAD_REQUEST, "empty_transcript")
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, "voice_failed")
            };
            json_error(code, key)
        }
        Err(err) => {
            tracing::error!(error = %err, "voice note join failed");
            json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed")
        }
    }
}

async fn voice_stream_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<VoiceStreamQuery>,
) -> Response {
    if !session_ok_with_query(&state, &headers, q.access_token.as_deref()) {
        return json_error(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let Some(account_id) = state.account_id else {
        return json_error(StatusCode::INTERNAL_SERVER_ERROR, "fetch_failed");
    };
    if !world_markets::mini_app::deepgram_ready() {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "stt_unconfigured");
    }
    let key = std::env::var("DEEPGRAM_API_KEY")
        .unwrap_or_default()
        .trim()
        .to_string();
    if key.is_empty() {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "stt_unconfigured");
    }
    let sample_rate = q.sample_rate.unwrap_or(48_000);
    if !world_markets::mini_app::deepgram_stream_sample_rate_ok(sample_rate) {
        // Do not clamp to a different rate — Deepgram would decode the PCM wrong.
        return json_error(StatusCode::BAD_REQUEST, "unsupported_sample_rate");
    }
    // Do not wait on brain/portfolio here — that delayed the upgrade by seconds
    // and the browser then dumped buffered PCM, which Deepgram heard as noise.
    let keyterms = world_markets::mini_app::voice_stream_keyterms_fast(account_id);
    ws.on_upgrade(move |socket| voice_stream::proxy(socket, key, sample_rate, keyterms))
}

fn prompt_with_ir(text: &str, ir: Option<&Value>) -> String {
    let text = text.trim();
    match ir.filter(|value| value.is_object()) {
        Some(ir) => format!("{text}\n[world_ir] {ir}"),
        None => text.to_string(),
    }
}

/// Local Mini App (`preview=dev`, no Telegram user): feed the transcript to
/// `aomi-run --prompt` so the plugin can `execute_world_order` on the same
/// rails as chat. The sidecar signs with `WORLD_PRIVATE_KEY`. Returns false
/// when the agent binary, plugin, or an LLM key is missing.
fn dispatch_local_agent_turn(transcript: &str) -> bool {
    let transcript = transcript.trim();
    if transcript.is_empty() {
        return false;
    }
    let Some(plugin) = plugin_lib() else {
        tracing::warn!("local agent skipped: build the plugin (`cargo build`)");
        return false;
    };
    let Some(provider) = local_llm_provider() else {
        tracing::warn!("local agent skipped: set OPENROUTER_API_KEY (or OPENAI/ANTHROPIC)");
        return false;
    };
    let Some(bin) = aomi_run_bin() else {
        tracing::warn!("local agent skipped: aomi-run is not on PATH");
        return false;
    };
    let env_file = PathBuf::from(".env");
    let log_path = std::env::temp_dir().join("world-markets-agent.log");
    let transcript = transcript.to_string();
    std::thread::spawn(move || {
        let mut cmd = Command::new(&bin);
        cmd.arg(&plugin)
            .arg("--env-file")
            .arg(&env_file)
            .arg("--provider")
            .arg(provider)
            .arg("--prompt")
            .arg(&transcript)
            .stdin(Stdio::null());
        if let Ok(file) = OpenOptions::new().create(true).append(true).open(&log_path) {
            if let Ok(clone) = file.try_clone() {
                cmd.stdout(Stdio::from(clone));
                cmd.stderr(Stdio::from(file));
            }
        }
        match cmd.status() {
            Ok(status) if status.success() => {
                tracing::info!(prompt = %transcript, "local agent turn finished");
            }
            Ok(status) => {
                tracing::warn!(code = ?status.code(), "local agent turn exited");
            }
            Err(err) => tracing::warn!(error = %err, "local agent turn failed to start"),
        }
    });
    tracing::info!(provider, "dispatched local agent turn");
    true
}

fn local_llm_provider() -> Option<&'static str> {
    let filled = |key: &str| {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .is_some()
    };
    if filled("OPENROUTER_API_KEY") {
        Some("openrouter")
    } else if filled("OPENAI_API_KEY") {
        Some("openai")
    } else if filled("ANTHROPIC_API_KEY") {
        Some("anthropic")
    } else {
        None
    }
}

fn aomi_run_bin() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("AOMI_RUN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join("aomi-run");
            candidate.is_file().then_some(candidate)
        })
    })
}

fn plugin_lib() -> Option<PathBuf> {
    let names = [
        "libworld_markets.dylib",
        "libworld_markets.so",
        "world_markets.dll",
    ];
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
            if let Some(parent) = dir.parent() {
                roots.push(parent.to_path_buf());
            }
        }
    }
    for root in roots {
        for profile in ["debug", "release"] {
            for name in names {
                let path = root.join("target").join(profile).join(name);
                if path.is_file() {
                    return Some(path);
                }
                let path = root.join(profile).join(name);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn payload_message(payload: &Value) -> String {
    payload
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("cancel task")
        .to_string()
}

fn session_ok(state: &AppState, headers: &HeaderMap) -> bool {
    session_token_ok(state, bearer_token(headers).as_deref())
}

fn session_ok_with_query(state: &AppState, headers: &HeaderMap, query_token: Option<&str>) -> bool {
    if session_ok(state, headers) {
        return true;
    }
    session_token_ok(
        state,
        query_token.map(str::trim).filter(|token| !token.is_empty()),
    )
}

fn session_token_ok(state: &AppState, token: Option<&str>) -> bool {
    let Some(token) = token else {
        return false;
    };
    let mut sessions = state.sessions.lock().expect("session lock");
    sessions.retain(|_, session| session.expires_at > Instant::now());
    sessions.contains_key(token)
}

fn desk_or_session_ok(state: &AppState, headers: &HeaderMap) -> bool {
    if session_ok(state, headers) {
        return true;
    }
    let presented = desk_token(headers);
    if !state.desk_token.is_empty() {
        return presented.as_deref() == Some(state.desk_token.as_str());
    }
    state.dev_bypass
}

fn desk_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-desk-token")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn session_telegram_user(state: &AppState, headers: &HeaderMap) -> Option<u64> {
    let token = bearer_token(headers)?;
    let mut sessions = state.sessions.lock().expect("session lock");
    sessions.retain(|_, session| session.expires_at > Instant::now());
    let id = sessions.get(&token)?.telegram_user_id;
    (id != 0).then_some(id)
}

fn session_first_name(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let token = bearer_token(headers)?;
    let mut sessions = state.sessions.lock().expect("session lock");
    sessions.retain(|_, session| session.expires_at > Instant::now());
    sessions
        .get(&token)?
        .first_name
        .clone()
        .filter(|s| !s.is_empty())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn json_error(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() || path == "portfolio" || path == "chart" {
        "index.html"
    } else {
        path
    };
    match Assets::get(path) {
        Some(file) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime_for(path))
            .body(axum::body::Body::from(file.data.into_owned()))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        None => {
            if let Some(index) = Assets::get("index.html") {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                    .body(axum::body::Body::from(index.data.into_owned()))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ledger_routes_are_get_only() {
        let src = include_str!("main.rs");
        assert!(src.contains("get(ledger_summary_handler)"));
        assert!(src.contains("get(ledger_one_handler)"));
        assert!(src.contains("get(ledger_handler)"));
        for line in src.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("//!") {
                continue;
            }
            if !trimmed.contains("/api/v1/mini-app/ledger") && !trimmed.contains("ledger_") {
                continue;
            }
            assert!(
                !trimmed.contains("post(")
                    && !trimmed.contains("put(")
                    && !trimmed.contains("patch(")
                    && !trimmed.contains("delete("),
                "ledger route must not mutate: {trimmed}"
            );
        }
    }

    #[test]
    fn products_route_is_get() {
        let src = include_str!("main.rs");
        assert!(src.contains(r#".route("/api/v1/mini-app/products", get(products_handler))"#));
    }

    #[test]
    fn speech_ontology_is_get() {
        let src = include_str!("main.rs");
        assert!(src.contains(r#"/api/v1/mini-app/speech-ontology"#));
        assert!(src.contains("get(speech_ontology_handler)"));
        assert!(src.contains("assets/speech_ontology.json"));
    }

    #[test]
    fn voice_is_not_a_ledger_route() {
        let src = include_str!("main.rs");
        assert!(src.contains(r#".route("/api/v1/mini-app/voice", post(voice_handler))"#));
        assert!(src.contains(r#".route("/api/v1/mini-app/voice/live", post(voice_live_handler))"#));
        assert!(
            src.contains(r#".route("/api/v1/mini-app/voice/stream", get(voice_stream_handler))"#)
        );
        assert!(src.contains("ingest_voice_note"));
        assert!(src.contains("transcribe_live"));
        let desk_fn = format!("post_{}_voice", "desk");
        assert!(
            !src.contains(&format!("world_markets::mini_app::{desk_fn}")),
            "voice handler must not call The Desk"
        );
        assert!(src.contains(r#".route("/api/v1/desk/context", get(desk_context_handler))"#));
        let live = src
            .split("async fn voice_live_handler")
            .nth(1)
            .and_then(|rest| rest.split("async fn voice_handler").next())
            .expect("voice_live_handler body");
        assert!(live.contains("transcribe_live"));
        assert!(!live.contains("ingest_voice_note"));
        assert!(!live.contains("submit_heard"));
        let stream = src
            .split("async fn voice_stream_handler")
            .nth(1)
            .and_then(|rest| rest.split("fn prompt_with_ir").next())
            .expect("voice_stream_handler body");
        assert!(stream.contains("voice_stream::proxy"));
        assert!(stream.contains("voice_stream_keyterms_fast"));
        assert!(!stream.contains("spawn_blocking"));
        assert!(!stream.contains("ingest_voice_note"));
        assert!(!stream.contains("submit_heard"));
        assert!(!stream.contains("submit_compose"));
    }

    #[test]
    fn dev_ontology_is_get_before_static_fallback() {
        let src = include_str!("main.rs");
        let route = src.find(r#".route("/dev/ontology", get(dev_ontology_handler))"#);
        let fallback = src.find(".fallback(static_handler)");
        assert!(route.is_some() && fallback.is_some());
        assert!(route.unwrap() < fallback.unwrap());
        assert!(src.contains("MINI_APP_DEV_BYPASS") || src.contains("dev_bypass"));
        assert!(src.contains("preview"));
    }

    #[test]
    fn mutating_mini_app_routes_are_allowlisted() {
        // Non-ledger writes. /api/v1/mini-app/share prepares an introduction;
        // it is not /ledger* and mutates nothing the Mini App displays.
        let src = include_str!("main.rs");
        let allowed = [
            r#"/api/v1/mini-app/auth"#,
            r#"/api/v1/mini-app/compose"#,
            r#"/api/v1/mini-app/voice"#,
            r#"/api/v1/mini-app/voice/live"#,
            r#"/api/v1/mini-app/share"#,
        ];
        for line in src.lines() {
            let trimmed = line.trim();
            let is_route = trimmed.contains(".route(");
            let is_post = trimmed.contains("post(");
            if !is_route || !is_post {
                continue;
            }
            if trimmed.contains("/ledger") {
                panic!("no posting to ledger: {trimmed}");
            }
            assert!(
                allowed.iter().any(|path| trimmed.contains(path)),
                "unallowlisted mutating route: {trimmed}"
            );
        }
        assert!(src.contains(r#".route("/api/v1/mini-app/share", post(share_handler))"#));
        assert!(src.contains("prepare_introduction"));
    }

    #[test]
    fn local_voice_dispatches_aomi_run_prompt() {
        let src = include_str!("main.rs");
        assert!(src.contains("dispatch_local_agent_turn"));
        assert!(src.contains("aomi-run"));
        assert!(src.contains("--prompt"));
        assert!(src.contains("live_text"));
        assert!(src.contains("finalized"));
    }

    #[test]
    fn local_typed_compose_dispatches_aomi_run_after_ledger_write() {
        let src = include_str!("main.rs");
        let compose = src
            .split("async fn compose_handler")
            .nth(1)
            .and_then(|rest| rest.split("async fn share_handler").next())
            .expect("compose_handler body");
        assert!(
            compose.contains("submit_compose"),
            "typed compose must write the ledger"
        );
        assert!(
            compose.contains("dispatch_local_agent_turn"),
            "local typed compose must start an agent turn after the ledger write"
        );
        assert!(
            compose.contains("instruction/sentence"),
            "dispatch the recorded sentence, not a second store"
        );
    }
}
