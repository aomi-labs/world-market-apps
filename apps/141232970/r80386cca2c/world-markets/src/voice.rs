//! Mini App / chat voice ingest. STT + brain records + compose into the agent.
//! Does not submit orders. Does not call The Desk.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{Value, json};

use crate::brain::BrainClient;
use crate::speech_ontology::{self, Channel, LexiconEntry};
use crate::stt::{self, SttErrorKind, Transcript};

const CATALOG_TTL: Duration = Duration::from_secs(60);

pub fn ingest_voice(account_id: u64, body: &Value) -> Result<Value, String> {
    let brain = BrainClient::with_timeout(90);
    let typed = body
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let live_text = body
        .get("live_text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let channel = if typed.is_some() {
        Channel::Text
    } else {
        Channel::Speech
    };
    let transcript = if let Some(text) = typed {
        Transcript {
            text: text.to_string(),
            words: Vec::new(),
            lang: "en".to_string(),
            stt_version: String::new(),
            keyterm_applied: false,
        }
    } else {
        transcribe_ingest_audio(account_id, body, &brain, live_text)?
    };

    let catalog = cached_catalog_symbols();
    let lexicon = lexicon_for(account_id, &brain);
    let mut normalized =
        speech_ontology::normalize_utterance(&transcript.text, channel, &catalog, &lexicon);
    if typed.is_some() {
        normalized.stt_version = None;
        normalized.keyterm_applied = false;
    } else {
        normalized.stt_version = Some(transcript.stt_version.clone());
        normalized.keyterm_applied = transcript.keyterm_applied;
    }
    let proposed_confusables: Vec<Value> = normalized
        .proposals
        .iter()
        .map(|row| row.to_json())
        .collect();
    let lexicon_hits: Vec<Value> = normalized
        .lexicon_hits
        .iter()
        .map(|hit| hit.to_json())
        .collect();
    let slots: Vec<Value> = normalized.slots.iter().map(|row| row.to_json()).collect();

    let duration_secs = body
        .get("duration_secs")
        .and_then(Value::as_f64)
        .or_else(|| {
            body.get("duration_ms")
                .and_then(Value::as_f64)
                .map(|ms| ms / 1000.0)
        });

    let words: Vec<Value> = transcript
        .words
        .iter()
        .map(|w| {
            json!({
                "w": w.w,
                "conf": w.conf,
                "t0": w.t0,
                "t1": w.t1,
            })
        })
        .collect();

    let recorded = brain
        .ingest_utterance(&json!({
            "account_id": account_id,
            "transcript": normalized.normalized_text,
            "text": normalized.normalized_text,
            "repaired_from": normalized.repaired_from,
            "words": words,
            "lang": transcript.lang,
            "stt_version": normalized.stt_version,
            "keyterm_applied": normalized.keyterm_applied,
            "duration_secs": duration_secs,
            "audio_base64": body.get("audio_base64"),
            "source": body.get("source").and_then(Value::as_str).unwrap_or("mini_app"),
            "foreign": false,
            "channel": normalized.channel.as_str(),
            "ontology_version": normalized.ontology_version,
            "slots": slots,
            "proposals": proposed_confusables,
            "proposed_confusables": proposed_confusables,
            "grammar": normalized.grammar.as_str(),
            "action_ir": normalized.action_ir.as_ref().map(|ir| ir.to_json()),
            "lexicon_hits": lexicon_hits,
            "unknown_instruments": normalized.unknown_instruments,
        }))
        .unwrap_or_else(|_| json!({ "heard_echo": normalized.normalized_text }));

    let utterance_id = recorded
        .pointer("/utterance/id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let episode_id = recorded
        .pointer("/episode/id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let correlation_id = if utterance_id.is_empty() {
        format!(
            "voice-{account_id}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        )
    } else {
        utterance_id.clone()
    };

    let send_payload = json!({
        "kind": "voice",
        "message": normalized.normalized_text,
        "utterance_id": utterance_id,
        "correlation_id": correlation_id,
        "episode_id": episode_id,
        "grammar": normalized.grammar.as_str(),
        "action_ir": normalized.action_ir.as_ref().map(|ir| ir.to_json()),
        "slots": slots,
        "channel": normalized.channel.as_str(),
    });

    Ok(json!({
        "ok": true,
        "transcript": normalized.normalized_text,
        "heard_echo": recorded.get("heard_echo").and_then(Value::as_str).unwrap_or(&normalized.normalized_text),
        "utterance_id": utterance_id,
        "episode_id": episode_id,
        "correlation_id": correlation_id,
        "stt_version": normalized.stt_version,
        "keyterm_applied": normalized.keyterm_applied,
        "channel": normalized.channel.as_str(),
        "grammar": normalized.grammar.as_str(),
        "action_ir": normalized.action_ir.as_ref().map(|ir| ir.to_json()),
        "slots": slots,
        "proposals": proposed_confusables,
        "long_note": recorded.get("long_note"),
        "long_note_line": recorded.get("long_note_line"),
        "split_parse": recorded.get("split_parse"),
        "proposed_confusables": proposed_confusables,
        "send_payload": send_payload,
    }))
}

const MIN_LIVE_AUDIO: usize = 1200;
const MIN_LIVE_WAV: usize = 4000;

/// Partial STT for hold-to-talk captions. Deepgram only; never ingests or dispatches.
pub fn transcribe_live(account_id: u64, body: &Value) -> Result<Value, String> {
    Ok(json!({ "text": transcribe_live_text(account_id, body) }))
}

fn transcribe_live_text(account_id: u64, body: &Value) -> String {
    if !stt::deepgram_configured() {
        return String::new();
    }
    let Some(audio) = decode_live_audio(body) else {
        return String::new();
    };
    let mime = body
        .get("mime")
        .and_then(Value::as_str)
        .unwrap_or("audio/webm");
    let keyterms = voice_stream_keyterms_fast(account_id);
    match stt::transcribe_partial(&audio, mime, &keyterms) {
        Ok(transcript) => transcript.text.trim().to_string(),
        Err(_) => String::new(),
    }
}

fn transcribe_ingest_audio(
    account_id: u64,
    body: &Value,
    brain: &BrainClient,
    live_text: Option<&str>,
) -> Result<Transcript, String> {
    let extra = seed_keyterms(account_id);
    let keyterms = brain
        .voice_keyterms(account_id, &extra)
        .unwrap_or_else(|_| extra.clone());
    let audio = decode_audio(body)?;
    let mime = body
        .get("mime")
        .and_then(Value::as_str)
        .unwrap_or("audio/webm");
    match stt::transcribe(&audio, mime, &keyterms) {
        Ok(transcript) if !transcript.text.trim().is_empty() => Ok(transcript),
        Ok(_) => live_fallback_or_err(live_text, "didn't catch any speech"),
        Err(err) => live_fallback_or_err(live_text, &stt_message(err)),
    }
}

fn live_fallback_or_err(live_text: Option<&str>, err: &str) -> Result<Transcript, String> {
    if let Some(live) = live_text.filter(|value| !value.is_empty()) {
        return Ok(Transcript {
            text: live.to_string(),
            words: Vec::new(),
            lang: "en".to_string(),
            stt_version: String::new(),
            keyterm_applied: false,
        });
    }
    Err(err.to_string())
}

fn decode_live_audio(body: &Value) -> Option<Vec<u8>> {
    let raw = body.get("audio_base64").and_then(Value::as_str)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .ok()?;
    let mime = body.get("mime").and_then(Value::as_str).unwrap_or("");
    let min = if mime.contains("wav") {
        MIN_LIVE_WAV
    } else {
        MIN_LIVE_AUDIO
    };
    if bytes.len() < min || bytes.len() > 5_000_000 {
        return None;
    }
    Some(bytes)
}

fn decode_audio(body: &Value) -> Result<Vec<u8>, String> {
    let raw = body
        .get("audio_base64")
        .and_then(Value::as_str)
        .ok_or_else(|| "audio is required".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|_| "audio is not valid base64".to_string())?;
    if bytes.is_empty() {
        return Err("didn't catch any speech".to_string());
    }
    if bytes.len() > 5_000_000 {
        return Err("voice note is too long".to_string());
    }
    Ok(bytes)
}

#[cfg(test)]
fn choose_transcript(stt: &str, live_text: Option<&str>) -> String {
    let stt = stt.trim();
    if !stt.is_empty() {
        return stt.to_string();
    }
    live_text
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(stt)
        .to_string()
}

struct KeytermCache {
    at: Instant,
    account_id: u64,
    terms: Vec<String>,
}

static KEYTERM_CACHE: Mutex<Option<KeytermCache>> = Mutex::new(None);

fn peek_keyterm_cache(account_id: u64) -> Option<Vec<String>> {
    let Ok(guard) = KEYTERM_CACHE.lock() else {
        return None;
    };
    let cache = guard.as_ref()?;
    if cache.account_id == account_id && cache.at.elapsed() < CATALOG_TTL {
        Some(cache.terms.clone())
    } else {
        None
    }
}

pub(crate) fn voice_keyterms_for(account_id: u64) -> Vec<String> {
    if let Some(terms) = peek_keyterm_cache(account_id) {
        return terms;
    }
    let extra = seed_keyterms(account_id);
    let terms = BrainClient::with_timeout(8)
        .voice_keyterms(account_id, &extra)
        .unwrap_or(extra);
    if let Ok(mut guard) = KEYTERM_CACHE.lock() {
        *guard = Some(KeytermCache {
            at: Instant::now(),
            account_id,
            terms: terms.clone(),
        });
    }
    terms
}

/// Live captions cannot wait on brain or portfolio RPC. Use a warm cache, else
/// the compiled ontology seed (buy/sell/ETH/…) with no network.
pub(crate) fn voice_stream_keyterms_fast(account_id: u64) -> Vec<String> {
    if let Some(terms) = peek_keyterm_cache(account_id) {
        return terms;
    }
    speech_ontology::seed_keyterms(&[], &[])
}

fn seed_keyterms(account_id: u64) -> Vec<String> {
    speech_ontology::seed_keyterms(&cached_catalog_symbols(), &holdings(account_id))
}

fn lexicon_for(account_id: u64, brain: &BrainClient) -> Vec<LexiconEntry> {
    brain
        .voice_context(account_id)
        .ok()
        .and_then(|value| value.get("lexicon").and_then(Value::as_array).cloned())
        .map(|rows| rows.iter().filter_map(LexiconEntry::from_json).collect())
        .unwrap_or_default()
}

fn holdings(account_id: u64) -> Vec<String> {
    match crate::mini_app::load_portfolio(account_id) {
        Ok(snap) => snap
            .positions
            .into_iter()
            .map(|row| row.symbol)
            .filter(|symbol| symbol.len() >= 2)
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn cached_catalog_symbols() -> Vec<String> {
    struct CatalogCache {
        at: Instant,
        symbols: Vec<String>,
    }
    static CACHE: Mutex<Option<CatalogCache>> = Mutex::new(None);
    let Ok(mut guard) = CACHE.lock() else {
        return catalog_symbols_uncached();
    };
    if let Some(cache) = guard.as_ref() {
        if cache.at.elapsed() < CATALOG_TTL {
            return cache.symbols.clone();
        }
    }
    let symbols = catalog_symbols_uncached();
    *guard = Some(CatalogCache {
        at: Instant::now(),
        symbols: symbols.clone(),
    });
    symbols
}

fn catalog_symbols_uncached() -> Vec<String> {
    match crate::mini_app::load_products() {
        Ok(snap) => {
            let mut seen = std::collections::HashSet::new();
            snap.products
                .into_iter()
                .map(|row| row.symbol)
                .filter(|symbol| symbol.len() >= 2 && seen.insert(symbol.to_ascii_lowercase()))
                .collect()
        }
        Err(_) => Vec::new(),
    }
}

fn stt_message(err: crate::stt::SttError) -> String {
    match err.kind {
        SttErrorKind::Empty => err.detail,
        SttErrorKind::Unconfigured => err.detail,
        SttErrorKind::Provider => err.detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_audio_stt_is_not_replaced_by_live_captions() {
        assert_eq!(
            choose_transcript("Hi", Some("buy fifty dollars of eth")),
            "Hi"
        );
        assert_eq!(
            choose_transcript("buy 51", Some("buy fifty dollars of ether")),
            "buy 51"
        );
        assert_eq!(
            choose_transcript("buy fifty dollars of ETH", Some("by 15 of it")),
            "buy fifty dollars of ETH"
        );
    }

    #[test]
    fn empty_stt_uses_live_words() {
        assert_eq!(choose_transcript("", Some("sell all sol")), "sell all sol");
    }

    #[test]
    fn ingest_transcribes_hold_audio_not_live_captions() {
        let src = include_str!("voice.rs");
        let start = src.find("pub fn ingest_voice").expect("ingest_voice");
        let rest = &src[start..];
        let end = rest.find("\nconst MIN_LIVE_AUDIO").unwrap_or(rest.len());
        let body = &rest[..end];
        assert!(body.contains("transcribe_ingest_audio"));
        assert!(body.contains("Channel::Speech"));
        assert!(!body.contains("deepgram:nova-3:stream"));
        assert!(!body.contains("finalized"));
        let transcribe = src
            .find("fn transcribe_ingest_audio")
            .expect("transcribe_ingest_audio");
        let transcribe_body = &src[transcribe..];
        assert!(transcribe_body.contains("stt::transcribe"));
        assert!(transcribe_body.contains("live_fallback_or_err"));
    }

    #[test]
    fn live_captions_empty_without_audio() {
        let out = transcribe_live(1, &json!({})).unwrap();
        assert_eq!(out.get("text").and_then(Value::as_str), Some(""));
        let tiny = transcribe_live(
            1,
            &json!({ "audio_base64": base64::engine::general_purpose::STANDARD.encode(b"too-small") }),
        )
        .unwrap();
        assert_eq!(tiny.get("text").and_then(Value::as_str), Some(""));
    }

    #[test]
    fn transcribe_live_fn_does_not_ingest() {
        let src = include_str!("voice.rs");
        let start = src.find("pub fn transcribe_live").expect("transcribe_live");
        let rest = &src[start..];
        let end = rest.find("\nfn decode_audio").unwrap_or(rest.len());
        let body = &rest[..end];
        assert!(!body.contains("ingest_utterance"));
        assert!(!body.contains("submit_heard"));
        assert!(!body.contains("ingest_voice"));
        assert!(body.contains("transcribe_partial"));
        assert!(body.contains("voice_stream_keyterms_fast"));
    }

    #[test]
    fn stream_keyterms_fast_are_local_ontology() {
        let terms = voice_stream_keyterms_fast(1);
        let lower: Vec<String> = terms.iter().map(|t| t.to_ascii_lowercase()).collect();
        assert!(lower.iter().any(|t| t == "buy"), "{lower:?}");
        assert!(lower.iter().any(|t| t == "eth"), "{lower:?}");
        assert!(lower.iter().any(|t| t == "sell"), "{lower:?}");
    }

    #[test]
    fn lexicon_hits_do_not_auto_map_confusable_beef() {
        let repair = speech_ontology::repair_transcript("buy fifty dollars worth of beef", &[]);
        assert_eq!(repair.text, "buy fifty dollars worth of beef");
        assert!(repair.hits.is_empty());
        assert_eq!(repair.proposed_confusables.len(), 1);
        assert_eq!(repair.proposed_confusables[0].target, "WETH");
        let typed = speech_ontology::normalize_utterance(
            "buy fifty dollars worth of beef",
            speech_ontology::Channel::Text,
            &[],
            &[],
        );
        assert!(typed.proposals.is_empty());
        assert_eq!(typed.channel, speech_ontology::Channel::Text);
    }
}
