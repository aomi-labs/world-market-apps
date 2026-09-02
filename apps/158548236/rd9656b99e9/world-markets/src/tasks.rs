//! M9 task store: watches + preferences (brain) and policies (signed Mandate).
//!
//! No chat mutation path for policies. Never calls execution.

use serde_json::{Value, json};

use crate::brain::BrainClient;
use crate::mandate::Mandate;

pub(crate) fn policy_items(mandate: &Mandate) -> Vec<Value> {
    let mut items = Vec::new();
    for market in &mandate.markets {
        items.push(json!({
            "id": format!("policy-market-{}-{}-{}", market.product, market.base, market.quote),
            "kind": "market",
            "label": format!("{} {}/{}", market.product, market.base, market.quote),
            "on_chain": true,
        }));
    }
    items.push(json!({
        "id": "policy-leverage",
        "kind": "max_leverage",
        "label": format!("max {}×", mandate.max_leverage),
        "value": mandate.max_leverage,
        "on_chain": true,
    }));
    items.push(json!({
        "id": "policy-floor",
        "kind": "min_risk_adjusted_portfolio_value",
        "label": format!(
            "Portfolio floor {} {}",
            mandate.min_risk_adjusted_portfolio_value.amount,
            mandate.min_risk_adjusted_portfolio_value.quote
        ),
        "value": mandate.min_risk_adjusted_portfolio_value.amount,
        "quote": mandate.min_risk_adjusted_portfolio_value.quote,
        "on_chain": true,
    }));
    items.push(json!({
        "id": "policy-notional",
        "kind": "max_position_notional",
        "label": format!(
            "max position {} {}",
            mandate.max_position_notional.amount,
            mandate.max_position_notional.quote
        ),
        "value": mandate.max_position_notional.amount,
        "on_chain": true,
    }));
    items.push(json!({
        "id": "policy-halt",
        "kind": "halt_if_eligible_for_liquidation",
        "label": if mandate.halt_if_eligible_for_liquidation {
            "Halt if eligible for liquidation"
        } else {
            "No liquidation halt"
        },
        "value": mandate.halt_if_eligible_for_liquidation,
        "on_chain": true,
    }));
    items
}

pub(crate) fn compose(
    brain: &BrainClient,
    account_id: Option<u64>,
    mandate: Result<Mandate, crate::mandate::Verdict>,
    brief: Option<&Value>,
) -> Value {
    let mut watches = json!([]);
    let mut preferences = json!([]);
    let mut ledger = json!({});
    let mut voice = json!({});
    let mut watches_status = "ok";
    let mut preferences_status = "ok";
    if let Some(account_id) = account_id {
        match brain.tasks(account_id) {
            Ok(payload) => {
                watches = payload.get("watches").cloned().unwrap_or(json!([]));
                preferences = payload.get("preferences").cloned().unwrap_or(json!([]));
                ledger = payload.get("ledger").cloned().unwrap_or(json!({}));
                voice = payload.get("voice").cloned().unwrap_or(json!({}));
            }
            Err(_) => {
                watches_status = "unavailable";
                preferences_status = "unavailable";
            }
        }
        if let Some(brief) = brief {
            let _ = brain.seed_brief(account_id, brief);
        }
    } else {
        watches_status = "unavailable";
        preferences_status = "unavailable";
    }

    let (policies, policies_status) = match mandate {
        Ok(bound) => (Value::Array(policy_items(&bound)), "ok"),
        Err(_) => (json!([]), "unavailable"),
    };

    let watch_len = watches.as_array().map(Vec::len).unwrap_or(0);
    let pref_len = preferences.as_array().map(Vec::len).unwrap_or(0);
    let policy_len = policies.as_array().map(Vec::len).unwrap_or(0);

    json!({
        "source": "world-markets-tasks",
        "executable": false,
        "watches": watches,
        "preferences": preferences,
        "policies": policies,
        "ledger": ledger,
        "voice": voice,
        "sections_partial": {
            "watches": watches_status != "ok",
            "preferences": preferences_status != "ok",
            "policies": policies_status != "ok",
        },
        "empty": tasks_empty(watch_len, pref_len, policy_len, &ledger),
        "on_chain_only_on_policies": true,
    })
}

pub(crate) fn load_open_instructions(brain: &BrainClient, account_id: Option<u64>) -> Value {
    let Some(account_id) = account_id else {
        return json!([]);
    };
    match brain.tasks(account_id) {
        Ok(payload) => payload
            .get("ledger")
            .and_then(|ledger| ledger.get("open_instructions"))
            .cloned()
            .unwrap_or(json!([])),
        Err(_) => json!([]),
    }
}

/// Host every-message hook: when the LLM will run, attach standing ledger intent.
pub(crate) fn attach_open_instructions(
    brain: &BrainClient,
    account_id: Option<u64>,
    mut payload: Value,
) -> Value {
    let skip = payload
        .get("skip_llm")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if skip {
        return payload;
    }
    if let Some(map) = payload.as_object_mut() {
        map.insert(
            "open_instructions".into(),
            load_open_instructions(brain, account_id),
        );
    }
    payload
}

fn open_instruction_len(ledger: &Value) -> usize {
    ledger
        .get("open_instructions")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn tasks_empty(watch_len: usize, pref_len: usize, policy_len: usize, ledger: &Value) -> bool {
    watch_len == 0 && pref_len == 0 && policy_len == 0 && open_instruction_len(ledger) == 0
}

pub(crate) fn policy_edit_block(mandate: &Mandate) -> Value {
    json!({
        "source": "world-markets-tasks",
        "executable": false,
        "blocked": true,
        "rule": "policy_signed_on_world",
        "floor": mandate.min_risk_adjusted_portfolio_value.amount,
        "quote": mandate.min_risk_adjusted_portfolio_value.quote,
        "detail": "That's a signed policy. Chat cannot change it.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mandate::Mandate;
    use serde_json::json;

    fn mandate() -> Mandate {
        Mandate::parse(Some(&serde_json::json!({
            "version": 1,
            "markets": [{ "product": "perp", "base": "WETH", "quote": "USDT" }],
            "max_position_notional": { "amount": "25000", "quote": "USDT" },
            "max_leverage": "3",
            "min_risk_adjusted_portfolio_value": { "amount": "5000", "quote": "USDT" },
            "halt_if_eligible_for_liquidation": true,
            "can_withdraw": false
        })))
        .expect("test mandate")
    }

    #[test]
    fn policies_are_on_chain_and_unique() {
        let items = policy_items(&mandate());
        assert!(!items.is_empty());
        let mut ids = std::collections::BTreeSet::new();
        for item in &items {
            assert_eq!(item["on_chain"], true);
            let id = item["id"].as_str().unwrap();
            assert!(ids.insert(id), "duplicate policy id {id}");
        }
    }

    #[test]
    fn empty_is_false_when_only_open_instructions_exist() {
        let ledger = json!({
            "holding": 1,
            "needs_you": 1,
            "open_instructions": [{
                "instruction_id": "i-1",
                "status": "with_aomi",
                "sentence": "If ETH touches 3400, close half"
            }]
        });
        assert!(!super::tasks_empty(0, 0, 0, &ledger));
        assert!(super::tasks_empty(0, 0, 0, &json!({})));
        assert!(super::tasks_empty(
            0,
            0,
            0,
            &json!({ "open_instructions": [] })
        ));
    }

    #[test]
    fn attach_open_instructions_skips_when_host_will_not_call_llm() {
        let brain = crate::brain::BrainClient::from_env();
        let skipped = super::attach_open_instructions(
            &brain,
            None,
            json!({ "skip_llm": true, "matched": true }),
        );
        assert!(skipped.get("open_instructions").is_none());
        let unmatched = super::attach_open_instructions(
            &brain,
            None,
            json!({ "skip_llm": false, "matched": false }),
        );
        assert_eq!(unmatched["open_instructions"], json!([]));
    }
}
