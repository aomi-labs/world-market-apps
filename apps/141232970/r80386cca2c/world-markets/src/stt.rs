//! Speech-to-text for Mini App / chat voice ingest.
//!
//! Deepgram Nova-3 is required for F11 keyterm prompting. Whisper is a fallback that
//! transcribes but cannot apply the lexicon to the recognizer.

use serde_json::Value;

const DEEPGRAM_URL: &str = "https://api.deepgram.com/v1/listen";
const WHISPER_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const MAX_KEYTERMS: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub words: Vec<Word>,
    pub lang: String,
    pub stt_version: String,
    pub keyterm_applied: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub w: String,
    pub conf: f64,
    pub t0: f64,
    pub t1: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SttError {
    pub kind: SttErrorKind,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SttErrorKind {
    Empty,
    Unconfigured,
    Provider,
}

impl SttError {
    fn empty() -> Self {
        Self {
            kind: SttErrorKind::Empty,
            detail: "didn't catch any speech".to_string(),
        }
    }

    fn unconfigured() -> Self {
        Self {
            kind: SttErrorKind::Unconfigured,
            detail: "speech recognition is not configured — set DEEPGRAM_API_KEY or OPENAI_API_KEY"
                .to_string(),
        }
    }

    fn provider(detail: impl Into<String>) -> Self {
        Self {
            kind: SttErrorKind::Provider,
            detail: detail.into(),
        }
    }
}

pub fn deepgram_configured() -> bool {
    std::env::var("DEEPGRAM_API_KEY")
        .map(|key| !key.trim().is_empty())
        .unwrap_or(false)
}

pub fn transcribe(audio: &[u8], mime: &str, keyterms: &[String]) -> Result<Transcript, SttError> {
    transcribe_with(audio, mime, keyterms, false)
}

/// Faster partial captions: no endpointing/punctuation so words land before a pause.
pub fn transcribe_partial(
    audio: &[u8],
    mime: &str,
    keyterms: &[String],
) -> Result<Transcript, SttError> {
    transcribe_with(audio, mime, keyterms, true)
}

fn transcribe_with(
    audio: &[u8],
    mime: &str,
    keyterms: &[String],
    partial: bool,
) -> Result<Transcript, SttError> {
    if audio.is_empty() {
        return Err(SttError::empty());
    }
    let content_type = content_type_for(audio, mime);
    if deepgram_configured() {
        let key = std::env::var("DEEPGRAM_API_KEY")
            .unwrap_or_default()
            .trim()
            .to_string();
        return deepgram(audio, content_type, &key, keyterms, partial);
    }
    if partial {
        return Err(SttError::unconfigured());
    }
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return whisper(audio, content_type, &key);
        }
    }
    Err(SttError::unconfigured())
}

fn deepgram(
    audio: &[u8],
    content_type: &str,
    key: &str,
    keyterms: &[String],
    partial: bool,
) -> Result<Transcript, SttError> {
    let timeout = if partial {
        std::time::Duration::from_secs(12)
    } else {
        std::time::Duration::from_secs(60)
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|err| SttError::provider(err.to_string()))?;
    let mut request = client
        .post(DEEPGRAM_URL)
        .query(&deepgram_query(partial))
        .header("Authorization", format!("Token {key}"))
        .header("Content-Type", content_type)
        .body(audio.to_vec());
    for term in keyterm_params(keyterms) {
        request = request.query(&[("keyterm", term)]);
    }
    for (from, to) in deepgram_replace_pairs() {
        request = request.query(&[("replace", format!("{from}:{to}"))]);
    }
    let keyterm_applied = !keyterms.is_empty();
    let response = request
        .send()
        .map_err(|err| SttError::provider(format!("deepgram is not reachable ({err})")))?;
    if !response.status().is_success() {
        return Err(SttError::provider(format!(
            "deepgram rejected the audio (HTTP {})",
            response.status()
        )));
    }
    let value: Value = response
        .json()
        .map_err(|err| SttError::provider(format!("deepgram returned invalid JSON ({err})")))?;
    parse_deepgram(value, keyterm_applied)
}

fn deepgram_query(partial: bool) -> Vec<(&'static str, &'static str)> {
    // Containerized audio (WAV / WebM / Ogg). Omit encoding, sample_rate, and
    // channels — Deepgram reads those from the container. Declaring linear16
    // at 16 kHz on 8 kHz mu-law (or the reverse) yields fluent-but-wrong text.
    if partial {
        vec![
            ("model", "nova-3"),
            ("smart_format", "true"),
            ("punctuate", "false"),
            ("language", "en"),
        ]
    } else {
        vec![
            ("model", "nova-3"),
            ("smart_format", "true"),
            ("punctuate", "true"),
            ("language", "en"),
        ]
    }
}

pub(crate) fn stream_sample_rate_ok(sample_rate: u32) -> bool {
    (8_000..=48_000).contains(&sample_rate)
}

/// Raw headerless PCM. `encoding` / `sample_rate` / `channels` must match the
/// bytes on the wire exactly — never clamp or guess a different rate.
pub(crate) fn deepgram_stream_query(sample_rate: u32) -> Vec<(&'static str, String)> {
    vec![
        ("model", "nova-3".into()),
        ("encoding", "linear16".into()),
        ("sample_rate", sample_rate.to_string()),
        ("channels", "1".into()),
        ("interim_results", "true".into()),
        ("smart_format", "true".into()),
        ("language", "en".into()),
        // Hold-to-talk already has an endpoint (pointer up). Default 10ms
        // silence splits a sentence into spans; the client concatenates
        // `is_final` results, but disabling endpointing keeps the caption
        // growing as one utterance until CloseStream.
        ("endpointing", "false".into()),
    ]
}

/// Exact Deepgram `replace` pairs. "buy 5 eth" is often emitted as "five five eight".
pub(crate) fn deepgram_replace_pairs() -> &'static [(&'static str, &'static str)] {
    &[
        ("five five eight", "buy 5 ETH"),
        ("five eight", "buy 5 ETH"),
    ]
}

/// Live WebSocket Results payload (`channel.alternatives`), not prerecorded `results.channels`.
pub(crate) fn stream_transcript(value: &Value) -> Option<(String, bool, f64)> {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    if kind == "Error" || kind == "UtteranceEnd" || kind == "Metadata" {
        return None;
    }
    let alt = value
        .pointer("/channel/alternatives/0")
        .or_else(|| value.pointer("/results/channels/0/alternatives/0"))
        .cloned()
        .unwrap_or(Value::Null);
    let text = alt
        .get("transcript")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return None;
    }
    let is_final = value
        .get("is_final")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || value
            .get("speech_final")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let confidence = alt.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);
    Some((text, is_final, confidence))
}

/// Nova-3 `keyterm` is a plain phrase — no `:intensifier` suffix (that is nova-2
/// `keywords` only, and nova-3 rejects `keywords` with HTTP 400).
pub(crate) fn keyterm_params(keyterms: &[String]) -> Vec<String> {
    keyterms
        .iter()
        .take(MAX_KEYTERMS)
        .map(|term| term.trim())
        .filter(|term| term.len() >= 2)
        .map(|term| term.to_string())
        .collect()
}

/// nova-2 `keywords` intensifier is roughly 1–10. Kept for tests and any
/// remaining nova-2 callers. Instruments use 5 so ETH/WETH beat near-misses.
pub(crate) fn keyword_params(keyterms: &[String]) -> Vec<String> {
    keyterms
        .iter()
        .take(MAX_KEYTERMS)
        .filter_map(|term| keyword_param(term))
        .collect()
}

fn keyword_param(term: &str) -> Option<String> {
    let trimmed = term.trim();
    if trimmed.len() < 2 || trimmed.contains(char::is_whitespace) {
        return None;
    }
    let intensifier = crate::speech_ontology::intensifier_for(trimmed);
    Some(format!("{trimmed}:{intensifier}"))
}

fn parse_deepgram(value: Value, keyterm_applied: bool) -> Result<Transcript, SttError> {
    let alt = value
        .pointer("/results/channels/0/alternatives/0")
        .cloned()
        .unwrap_or(Value::Null);
    let text = alt
        .get("transcript")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(SttError::empty());
    }
    let words = alt
        .get("words")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let w = row
                        .get("word")
                        .or_else(|| row.get("punctuated_word"))
                        .and_then(Value::as_str)?
                        .to_string();
                    Some(Word {
                        w,
                        conf: row.get("confidence").and_then(Value::as_f64).unwrap_or(0.0),
                        t0: row.get("start").and_then(Value::as_f64).unwrap_or(0.0),
                        t1: row.get("end").and_then(Value::as_f64).unwrap_or(0.0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Transcript {
        text,
        words,
        lang: "en".to_string(),
        stt_version: "deepgram:nova-3".to_string(),
        keyterm_applied,
    })
}

fn whisper(audio: &[u8], content_type: &str, key: &str) -> Result<Transcript, SttError> {
    let ext = extension_for(content_type);
    let part = reqwest::blocking::multipart::Part::bytes(audio.to_vec())
        .file_name(format!("note.{ext}"))
        .mime_str(content_type)
        .map_err(|err| SttError::provider(err.to_string()))?;
    let form = reqwest::blocking::multipart::Form::new()
        .text("model", "whisper-1")
        .part("file", part);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|err| SttError::provider(err.to_string()))?;
    let response = client
        .post(WHISPER_URL)
        .header("Authorization", format!("Bearer {key}"))
        .multipart(form)
        .send()
        .map_err(|err| SttError::provider(format!("whisper is not reachable ({err})")))?;
    if !response.status().is_success() {
        return Err(SttError::provider(format!(
            "whisper rejected the audio (HTTP {})",
            response.status()
        )));
    }
    let value: Value = response
        .json()
        .map_err(|err| SttError::provider(format!("whisper returned invalid JSON ({err})")))?;
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(SttError::empty());
    }
    Ok(Transcript {
        text,
        words: Vec::new(),
        lang: value
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or("en")
            .to_string(),
        stt_version: "openai:whisper-1".to_string(),
        keyterm_applied: false,
    })
}

fn sniff_audio_type(audio: &[u8]) -> Option<&'static str> {
    if audio.len() >= 12 && audio.starts_with(b"RIFF") && &audio[8..12] == b"WAVE" {
        return Some("audio/wav");
    }
    if audio.starts_with(b"OggS") {
        return Some("audio/ogg");
    }
    if audio.len() >= 4 && audio[..4] == [0x1a, 0x45, 0xdf, 0xa3] {
        return Some("audio/webm");
    }
    None
}

fn content_type_for<'a>(audio: &[u8], mime: &'a str) -> &'a str {
    if let Some(sniffed) = sniff_audio_type(audio) {
        return sniffed;
    }
    if mime.is_empty() { "audio/webm" } else { mime }
}

fn extension_for(content_type: &str) -> &'static str {
    if content_type.contains("ogg") {
        "ogg"
    } else if content_type.contains("mp4") || content_type.contains("m4a") {
        "m4a"
    } else if content_type.contains("mpeg") || content_type.contains("mp3") {
        "mp3"
    } else if content_type.contains("wav") {
        "wav"
    } else {
        "webm"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_audio_fails_closed() {
        let err = transcribe(&[], "audio/webm", &[]).unwrap_err();
        assert_eq!(err.kind, SttErrorKind::Empty);
    }

    #[test]
    fn deepgram_parse_reads_words() {
        let parsed = parse_deepgram(
            json!({
                "results": {
                    "channels": [{
                        "alternatives": [{
                            "transcript": "buy weth",
                            "words": [
                                { "word": "buy", "confidence": 0.99, "start": 0.0, "end": 0.2 },
                                { "word": "weth", "confidence": 0.8, "start": 0.2, "end": 0.5 }
                            ]
                        }]
                    }]
                }
            }),
            true,
        )
        .unwrap();
        assert_eq!(parsed.text, "buy weth");
        assert_eq!(parsed.words.len(), 2);
        assert_eq!(parsed.words[1].w, "weth");
        assert!(parsed.keyterm_applied);
    }

    #[test]
    fn keyword_params_use_kind_intensifiers() {
        let params = keyword_params(&[
            "ETH".to_string(),
            "buy".to_string(),
            "worth".to_string(),
            "the loop".to_string(),
            "x".to_string(),
            "nickname".to_string(),
        ]);
        assert_eq!(params, vec!["ETH:5", "buy:3", "worth:3", "nickname:2"]);
    }

    #[test]
    fn live_query_uses_nova3() {
        let q = deepgram_query(true);
        assert!(q.contains(&("model", "nova-3")));
        assert!(q.contains(&("smart_format", "true")));
        assert!(q.contains(&("language", "en")));
        assert!(!q.iter().any(|(k, _)| *k == "endpointing"));
    }

    #[test]
    fn stream_query_asks_for_interims() {
        let q = deepgram_stream_query(48_000);
        assert!(
            q.iter()
                .any(|(k, v)| *k == "interim_results" && v == "true")
        );
        assert!(q.iter().any(|(k, v)| *k == "model" && v == "nova-3"));
        assert!(q.iter().any(|(k, v)| *k == "encoding" && v == "linear16"));
        assert!(q.iter().any(|(k, v)| *k == "sample_rate" && v == "48000"));
        assert!(q.iter().any(|(k, v)| *k == "channels" && v == "1"));
        assert!(q.iter().any(|(k, v)| *k == "smart_format" && v == "true"));
        assert!(q.iter().any(|(k, v)| *k == "endpointing" && v == "false"));
        assert!(!q.iter().any(|(k, _)| *k == "keywords"));
        assert!(
            deepgram_replace_pairs()
                .iter()
                .any(|(from, to)| { *from == "five five eight" && *to == "buy 5 ETH" })
        );
    }

    #[test]
    fn stream_query_keeps_the_capture_rate() {
        let q = deepgram_stream_query(44_100);
        assert_eq!(
            q.iter()
                .find(|(k, _)| *k == "sample_rate")
                .map(|(_, v)| v.as_str()),
            Some("44100")
        );
        assert!(stream_sample_rate_ok(44_100));
        assert!(stream_sample_rate_ok(16_000));
        assert!(!stream_sample_rate_ok(0));
        assert!(!stream_sample_rate_ok(8_000 - 1));
        assert!(!stream_sample_rate_ok(96_000));
        let high = deepgram_stream_query(96_000);
        assert_eq!(
            high.iter()
                .find(|(k, _)| *k == "sample_rate")
                .map(|(_, v)| v.as_str()),
            Some("96000"),
            "never advertise 48 kHz for 96 kHz PCM"
        );
    }

    #[test]
    fn prerecorded_query_omits_raw_pcm_hints() {
        for partial in [false, true] {
            let q = deepgram_query(partial);
            assert!(!q.iter().any(|(k, _)| *k == "encoding"));
            assert!(!q.iter().any(|(k, _)| *k == "sample_rate"));
            assert!(!q.iter().any(|(k, _)| *k == "channels"));
        }
    }

    #[test]
    fn content_type_sniffs_wav_over_a_webm_label() {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&[0, 0, 0, 0]);
        wav.extend_from_slice(b"WAVE");
        wav.extend_from_slice(&[0; 8]);
        assert_eq!(content_type_for(&wav, "audio/webm"), "audio/wav");
        assert_eq!(content_type_for(b"OggS....", ""), "audio/ogg");
        assert_eq!(
            content_type_for(&[0x1a, 0x45, 0xdf, 0xa3, 0, 0], "audio/wav"),
            "audio/webm"
        );
        assert_eq!(content_type_for(b"???", ""), "audio/webm");
        assert_eq!(content_type_for(b"???", "audio/mp4"), "audio/mp4");
    }

    #[test]
    fn keyterm_params_are_plain_phrases() {
        let params = keyterm_params(&[
            "ETH".to_string(),
            "wrapped ether".to_string(),
            "x".to_string(),
            "buy".to_string(),
        ]);
        assert_eq!(params, vec!["ETH", "wrapped ether", "buy"]);
        assert!(!params.iter().any(|t| t.contains(':')));
    }

    #[test]
    fn stream_transcript_reads_live_payload() {
        let interim = stream_transcript(&json!({
            "type": "Results",
            "is_final": false,
            "channel": { "alternatives": [{ "transcript": "buy ether" }] }
        }))
        .unwrap();
        assert_eq!(interim, ("buy ether".to_string(), false, 0.0));
        let fin = stream_transcript(&json!({
            "type": "Results",
            "is_final": true,
            "speech_final": true,
            "channel": { "alternatives": [{ "transcript": "buy fifty ether", "confidence": 0.91 }] }
        }))
        .unwrap();
        assert_eq!(fin, ("buy fifty ether".to_string(), true, 0.91));
        assert!(stream_transcript(&json!({ "type": "Metadata" })).is_none());
        assert!(
            stream_transcript(&json!({
                "type": "Results",
                "channel": { "alternatives": [{ "transcript": "  " }] }
            }))
            .is_none()
        );
    }
}
