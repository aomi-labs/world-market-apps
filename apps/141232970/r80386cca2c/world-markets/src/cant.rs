//! Heard-path for unfulfillable / near-match / unclear asks.
//! Never executes. Does not mention or call execute tools.
//! Interprets a NormalizedUtterance; does not persist voice records.

use serde_json::{Value, json};

use crate::brain::BrainClient;
use crate::client::{Asset, asset_by_symbol};
use crate::mini_app::load_products;
use crate::speech_ontology::{self, Channel, LexiconEntry};

pub(crate) fn try_heard(account_id: u64, text: &str, extra: Option<&Value>) -> Option<Value> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut body = extra.cloned().unwrap_or_else(|| json!({}));
    let obj = body.as_object_mut()?;
    let has_slots = obj.get("slots").and_then(Value::as_array).is_some();
    if !has_slots {
        attach_normalized(account_id, text, obj);
    }
    obj.insert("account_id".into(), json!(account_id));
    obj.entry("text".to_string()).or_insert_with(|| json!(text));
    obj.entry("universe".to_string())
        .or_insert_with(universe_rows);
    match BrainClient::from_env().heard(&body) {
        Ok(value) => {
            let kind = value.get("kind").and_then(Value::as_str).unwrap_or("");
            if kind == "unmatched" || kind.is_empty() {
                return None;
            }
            Some(wrap(value))
        }
        Err(_) => None,
    }
}

/// Local CANT classifier. Does not need the brain sidecar.
pub(crate) fn cant_wall_for(text: &str) -> Option<Value> {
    let (category, noun) = speech_ontology::unfulfillable_kind(text, &[])?;
    Some(wrap(json!({
        "kind": "cant",
        "skip_llm": true,
        "reply_verbatim": true,
        "matched": true,
        "message": crate::reporting::render_cant_wall(text, category),
        "asked_entity": noun,
        "cant_kind": category,
        "voice_kind": "cant",
    })))
}

/// Heard-path UNCLEAR must use the non-trade register, never a buy-clarification.
pub(crate) fn apply_unclear_copy(mut value: Value) -> Value {
    if value.get("kind").and_then(Value::as_str) == Some("unclear") {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("message".into(), json!(crate::reporting::UNCLEAR_MESSAGE));
            obj.insert("skip_llm".into(), json!(true));
            obj.insert("reply_verbatim".into(), json!(true));
        }
    }
    value
}
/// in the universe. Primary route is model-side (`render_lookup`); this fires
/// when the model wrongly calls preview/check instead.
pub(crate) fn heard_unknown_trade_asset(
    account_id: Option<u64>,
    text: Option<&str>,
    side: &str,
    quantity: &str,
    symbol: &str,
    assets: &[Asset],
) -> Option<Value> {
    if asset_by_symbol(assets, symbol).is_ok() {
        return None;
    }
    let account_id = account_id?;
    let heard_text = text
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| format!("{} {} of {}", side.trim(), quantity.trim(), symbol.trim()));
    if let Some(value) = try_heard(account_id, &heard_text, None) {
        let kind = value.get("kind").and_then(Value::as_str).unwrap_or("");
        if kind == "unclear" {
            if let Some(wall) = cant_wall_for(&heard_text) {
                return Some(wall);
            }
            return Some(apply_unclear_copy(value));
        }
        return matches!(kind, "cant" | "near_match").then_some(value);
    }
    cant_wall_for(&heard_text)
}

fn attach_normalized(account_id: u64, text: &str, obj: &mut serde_json::Map<String, Value>) {
    let channel = obj
        .get("channel")
        .and_then(Value::as_str)
        .map(Channel::parse)
        .unwrap_or(Channel::Text);
    let catalog = catalog_symbols(obj.get("universe"));
    let lexicon = lexicon_for(account_id);
    let normalized = speech_ontology::normalize_utterance(text, channel, &catalog, &lexicon);
    obj.insert("text".into(), json!(normalized.normalized_text));
    obj.insert(
        "slots".into(),
        json!(
            normalized
                .slots
                .iter()
                .map(|row| row.to_json())
                .collect::<Vec<_>>()
        ),
    );
    obj.insert(
        "proposals".into(),
        json!(
            normalized
                .proposals
                .iter()
                .map(|row| row.to_json())
                .collect::<Vec<_>>()
        ),
    );
    obj.insert("grammar".into(), json!(normalized.grammar.as_str()));
    obj.insert(
        "action_ir".into(),
        normalized
            .action_ir
            .as_ref()
            .map(|ir| ir.to_json())
            .unwrap_or(Value::Null),
    );
    obj.insert(
        "unknown_instruments".into(),
        json!(normalized.unknown_instruments),
    );
    obj.insert("channel".into(), json!(channel.as_str()));
    obj.insert(
        "ontology_version".into(),
        json!(normalized.ontology_version),
    );
}

fn catalog_symbols(universe: Option<&Value>) -> Vec<String> {
    if let Some(Value::Array(rows)) = universe {
        return rows
            .iter()
            .filter_map(|row| {
                row.get("symbol")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
            })
            .collect();
    }
    load_products()
        .map(|catalog| catalog.products.into_iter().map(|row| row.symbol).collect())
        .unwrap_or_default()
}

fn lexicon_for(account_id: u64) -> Vec<LexiconEntry> {
    BrainClient::from_env()
        .voice_context(account_id)
        .ok()
        .and_then(|value| value.get("lexicon").and_then(Value::as_array).cloned())
        .map(|rows| rows.iter().filter_map(LexiconEntry::from_json).collect())
        .unwrap_or_default()
}

fn universe_rows() -> Value {
    let Ok(catalog) = load_products() else {
        return json!([]);
    };
    let mut seen = std::collections::HashSet::new();
    let mut rows = Vec::new();
    for product in catalog.products {
        let symbol = product.symbol.trim();
        if symbol.is_empty() {
            continue;
        }
        let key = symbol.to_ascii_uppercase();
        if !seen.insert(key) {
            continue;
        }
        rows.push(json!({
            "symbol": product.symbol,
            "name": product.name,
        }));
    }
    Value::Array(rows)
}

fn wrap(mut value: Value) -> Value {
    if let Some(map) = value.as_object_mut() {
        map.insert("source".into(), json!("world-markets-cant"));
        map.insert("executable".into(), json!(false));
        map.entry("skip_llm".to_string()).or_insert(json!(true));
        map.entry("reply_verbatim".to_string())
            .or_insert(json!(true));
        map.entry("matched".to_string()).or_insert(json!(true));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn cant_module_never_names_execute_tools() {
        let src = fs::read_to_string("src/cant.rs").expect("src/cant.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap_or(&src);
        let forbidden = ["execute_", "ExecuteWorld", "stage_trade"];
        for needle in forbidden {
            assert!(!prod.contains(needle), "cant.rs must not name {needle}");
        }
    }

    #[test]
    fn wrap_forces_not_executable() {
        let value = wrap(json!({ "kind": "cant", "message": "x" }));
        assert_eq!(value["executable"], false);
        assert_eq!(value["skip_llm"], true);
        assert_eq!(value["reply_verbatim"], true);
    }

    #[test]
    fn empty_heard_falls_through() {
        assert!(try_heard(1, "   ", None).is_none());
    }

    #[test]
    fn known_symbol_does_not_enter_heard_backstop() {
        let assets = [Asset {
            token_id: 2,
            symbol: "WETH".into(),
            name: "Wrapped Ether".into(),
            token_type: "crypto".into(),
            erc20_address: "0x0".into(),
            erc20_decimals: 18,
            vault_decimals: 8,
            position_decimals: 8,
            risk_price_percent: 5,
            risk_slippage_percent: 0.5,
        }];
        assert!(
            heard_unknown_trade_asset(
                Some(17),
                Some("buy me $50 of WETH"),
                "buy",
                "50",
                "WETH",
                &assets,
            )
            .is_none()
        );
    }

    #[test]
    fn unknown_symbol_without_account_falls_through() {
        assert!(
            heard_unknown_trade_asset(None, Some("buy me $50 of beef"), "buy", "50", "beef", &[])
                .is_none()
        );
    }

    #[test]
    fn cant_wall_for_beef_uses_unfulfillable_kind() {
        let value = cant_wall_for("buy me $50 of beef").expect("cant");
        assert_eq!(value["kind"], "cant");
        assert_eq!(value["skip_llm"], true);
        let message = value["message"].as_str().unwrap();
        assert!(message.contains("I heard"));
        assert!(message.contains("World doesn't trade"));
        assert!(message.contains("crypto spot, perps, and lending"));
        assert!(!message.to_ascii_lowercase().contains("say buy"));
        assert!(cant_wall_for("my favourite colour is teal").is_none());
        assert!(cant_wall_for("buy $50").is_none());
    }

    #[test]
    fn apply_unclear_copy_replaces_trade_shaped_clarification() {
        let value = apply_unclear_copy(json!({
            "kind": "unclear",
            "message": "I didn't catch an instrument in that. Say buy, a size, and the name.",
        }));
        let message = value["message"].as_str().unwrap();
        assert_eq!(message, crate::reporting::UNCLEAR_MESSAGE);
        assert!(message.contains("I trade crypto spot, perps, and lending"));
        assert!(message.contains("/p"));
        assert!(!message.to_ascii_lowercase().contains("say buy"));
    }
}
