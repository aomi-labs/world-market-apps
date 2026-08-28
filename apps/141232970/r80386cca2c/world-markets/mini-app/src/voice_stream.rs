//! Proxy Mini App hold-to-talk PCM to Deepgram live listen. Does not ingest.

use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as DgMessage;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

const MAX_STREAM: Duration = Duration::from_secs(120);

pub async fn proxy(client: WebSocket, api_key: String, sample_rate: u32, keyterms: Vec<String>) {
    if let Err(err) = proxy_inner(client, api_key, sample_rate, keyterms).await {
        tracing::warn!(error = %err, "voice stream proxy failed");
    }
}

fn listen_url(sample_rate: u32, keyterms: &[String]) -> String {
    let mut url = String::from("wss://api.deepgram.com/v1/listen");
    let mut first = true;
    for (key, value) in world_markets::mini_app::deepgram_stream_query(sample_rate) {
        url.push(if first { '?' } else { '&' });
        first = false;
        url.push_str(key);
        url.push('=');
        url.push_str(&urlencoding::encode(&value));
    }
    for term in world_markets::mini_app::voice_keyterm_boosts(keyterms) {
        url.push_str("&keyterm=");
        url.push_str(&urlencoding::encode(&term));
    }
    for (from, to) in world_markets::mini_app::deepgram_replace_pairs() {
        url.push_str("&replace=");
        url.push_str(&urlencoding::encode(&format!("{from}:{to}")));
    }
    url
}

async fn proxy_inner(
    mut client: WebSocket,
    api_key: String,
    sample_rate: u32,
    keyterms: Vec<String>,
) -> Result<(), String> {
    let url = listen_url(sample_rate, &keyterms);
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|err| err.to_string())?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Token {api_key}")).map_err(|err| err.to_string())?,
    );
    let (mut deepgram, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|err| format!("deepgram stream: {err}"))?;
    client
        .send(Message::Text(json!({ "type": "ready" }).to_string().into()))
        .await
        .map_err(|err| err.to_string())?;

    let outcome = tokio::time::timeout(MAX_STREAM, async {
        loop {
            tokio::select! {
                incoming = client.next() => {
                    match incoming {
                        Some(Ok(Message::Binary(bytes))) => {
                            if bytes.is_empty() {
                                continue;
                            }
                            deepgram
                                .send(DgMessage::Binary(bytes.to_vec().into()))
                                .await
                                .map_err(|err| err.to_string())?;
                        }
                        Some(Ok(Message::Text(text))) => {
                            if wants_finalize(text.as_str()) {
                                deepgram
                                    .send(DgMessage::Text(r#"{"type":"Finalize"}"#.into()))
                                    .await
                                    .map_err(|err| err.to_string())?;
                            } else if wants_keepalive(text.as_str()) {
                                deepgram
                                    .send(DgMessage::Text(r#"{"type":"KeepAlive"}"#.into()))
                                    .await
                                    .map_err(|err| err.to_string())?;
                            } else if wants_close(text.as_str()) {
                                deepgram
                                    .send(DgMessage::Text(r#"{"type":"CloseStream"}"#.into()))
                                    .await
                                    .map_err(|err| err.to_string())?;
                            }
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = client.send(Message::Pong(payload)).await;
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => {}
                        Some(Err(err)) => return Err(err.to_string()),
                    }
                }
                incoming = deepgram.next() => {
                    match incoming {
                        Some(Ok(DgMessage::Text(text))) => {
                            let value: Value = serde_json::from_str(text.as_str()).unwrap_or(Value::Null);
                            if value.get("type").and_then(Value::as_str) == Some("Error") {
                                let msg = json!({
                                    "type": "error",
                                    "message": value.get("description")
                                        .or_else(|| value.get("message"))
                                        .and_then(Value::as_str)
                                        .unwrap_or("deepgram error"),
                                });
                                client
                                    .send(Message::Text(msg.to_string().into()))
                                    .await
                                    .map_err(|err| err.to_string())?;
                                continue;
                            }
                            if let Some((heard, is_final, confidence)) =
                                world_markets::mini_app::stream_transcript_caption(&value)
                            {
                                let msg = json!({
                                    "type": "transcript",
                                    "text": heard,
                                    "is_final": is_final,
                                    "confidence": confidence,
                                });
                                client
                                    .send(Message::Text(msg.to_string().into()))
                                    .await
                                    .map_err(|err| err.to_string())?;
                            }
                        }
                        Some(Ok(DgMessage::Ping(payload))) => {
                            let _ = deepgram.send(DgMessage::Pong(payload)).await;
                        }
                        Some(Ok(DgMessage::Close(_))) | None => break,
                        Some(Ok(_)) => {}
                        Some(Err(err)) => return Err(err.to_string()),
                    }
                }
            }
        }
        Ok::<(), String>(())
    })
    .await;

    let _ = deepgram.close(None).await;
    let _ = client.send(Message::Close(None)).await;
    match outcome {
        Ok(inner) => inner,
        Err(_) => Err("voice stream timed out".into()),
    }
}

fn wants_close(text: &str) -> bool {
    command_type(text).is_some_and(|kind| kind.eq_ignore_ascii_case("close"))
}

fn wants_finalize(text: &str) -> bool {
    command_type(text).is_some_and(|kind| kind.eq_ignore_ascii_case("finalize"))
}

fn wants_keepalive(text: &str) -> bool {
    command_type(text).is_some_and(|kind| kind.eq_ignore_ascii_case("keepalive"))
}

fn command_type(text: &str) -> Option<String> {
    serde_json::from_str::<Value>(text)
        .ok()?
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_message_is_detected() {
        assert!(wants_close(r#"{"type":"close"}"#));
        assert!(wants_close(r#"{"type":"Close"}"#));
        assert!(wants_finalize(r#"{"type":"finalize"}"#));
        assert!(wants_keepalive(r#"{"type":"KeepAlive"}"#));
        assert!(wants_keepalive(r#"{"type":"keepalive"}"#));
        assert!(!wants_finalize(r#"{"type":"close"}"#));
        assert!(!wants_close(r#"{"type":"keepalive"}"#));
        assert!(!wants_close("nope"));
    }

    #[test]
    fn listen_url_is_linear16_interims() {
        let url = listen_url(48_000, &["ETH".into(), "buy".into()]);
        assert!(url.starts_with("wss://api.deepgram.com/v1/listen?"));
        assert!(url.contains("model=nova-3"));
        assert!(url.contains("interim_results=true"));
        assert!(url.contains("endpointing=false"));
        assert!(url.contains("encoding=linear16"));
        assert!(url.contains("sample_rate=48000"));
        assert!(url.contains("channels=1"));
        let native = listen_url(44_100, &[]);
        assert!(native.contains("sample_rate=44100"));
        assert!(!native.contains("sample_rate=48000"));
        assert!(url.contains("keyterm="));
        assert!(url.contains("replace="));
        assert!(!url.contains("keywords="));
        assert!(!url.contains("ingest"));
    }
}
