//! Stage a user trade on the ledger, wait the cancel window, then fill
//! (including TWAP/DCA child slices over time).

use std::thread;
use std::time::Duration;

use aomi_sdk::DynToolCallCtx;
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};

use crate::brain::BrainClient;
use crate::mandate::parse_decimal;
use crate::order_intent::{self, ExecutionPlan};
use crate::tool::{ExecuteWorldOrderArgs, WorldMarketsApp, place_world_order};

const DELAY: Duration = Duration::from_secs(3);

pub fn stage_and_schedule(
    brain: &BrainClient,
    account_id: u64,
    args: &ExecuteWorldOrderArgs,
    sentence: &str,
    mandate: Option<&Value>,
    plan: &ExecutionPlan,
    extra: Option<&Value>,
) -> Result<Value, String> {
    let mut params = json!({
        "product": args.product,
        "side": args.side,
        "base_symbol": args.base_symbol,
        "quote_symbol": args.quote_symbol,
        "quantity": args.quantity,
        "price": args.price,
        "order_type": plan.order_type,
        "slippage": args.slippage,
        "account_id": account_id,
        "wallet_address": args.wallet_address,
        "sentence": sentence,
        "schedule": plan.to_schedule_json(&args.quantity),
    });
    if let Some(mandate) = mandate {
        params["handover_mandate"] = mandate.clone();
    }
    if let Some(obj) = extra.and_then(Value::as_object) {
        if let Some(params_obj) = params.as_object_mut() {
            for (k, v) in obj {
                params_obj.insert(k.clone(), v.clone());
            }
        }
    }
    let staged = brain.stage_trade(&json!({
        "account_id": account_id,
        "sentence": sentence,
        "delay_secs": DELAY.as_secs(),
        "instrument": args.base_symbol,
        "params": params,
    }))?;
    let instruction_id = staged
        .pointer("/instruction/instruction_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !instruction_id.is_empty() {
        thread::spawn(move || {
            let mut wait = DELAY;
            loop {
                thread::sleep(wait);
                match flush_staged_trade(account_id, &instruction_id) {
                    Ok(value) if value.get("more_slices") == Some(&Value::Bool(true)) => {
                        let secs = value
                            .get("interval_secs")
                            .and_then(Value::as_u64)
                            .filter(|n| *n > 0)
                            .unwrap_or(60);
                        wait = Duration::from_secs(secs);
                    }
                    _ => break,
                }
            }
        });
    }
    Ok(json!({
        "source": "world-markets-ledger",
        "executable": false,
        "staged": true,
        "execute_delay_secs": DELAY.as_secs(),
        "order_type": plan.order_type,
        "schedule": plan.to_schedule_json(&args.quantity),
        "instruction": staged.get("instruction"),
        "cancel": staged
            .pointer("/instruction/task_id")
            .and_then(Value::as_str)
            .map(|id| format!("cancel task {id}")),
        "sentence": sentence,
        "hint": if plan.is_sliced() {
            format!(
                "on the ledger for 3 seconds — cancel if that's wrong, then it fills in {} slices",
                plan.slices
            )
        } else {
            "on the ledger for 3 seconds — cancel if that's wrong, then it fills".to_string()
        },
    }))
}

pub fn flush_staged_trade(account_id: u64, instruction_id: &str) -> Result<Value, String> {
    let app = WorldMarketsApp::default();
    let brain = BrainClient::from_env();
    let claimed = match brain.claim_slice(account_id, instruction_id) {
        Ok(value) => value,
        Err(err) => {
            if skippable_claim(&err) {
                return Ok(json!({ "ok": true, "skipped": true, "detail": err }));
            }
            return Err(err);
        }
    };
    if claimed.get("done") == Some(&Value::Bool(true)) {
        return Ok(json!({ "ok": true, "skipped": true, "done": true }));
    }
    let params = claimed.get("params").cloned().unwrap_or(json!({}));
    let slice_i = claimed.get("slice_i").and_then(Value::as_u64).unwrap_or(1) as u32;
    let slice_n = claimed
        .get("slice_n")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as u32;
    let last = claimed.get("last") == Some(&Value::Bool(true)) || slice_i >= slice_n;
    let mut args = args_from_params(&params, account_id);
    let fill_qty = child_quantity(&params, slice_i, slice_n)?;
    args.quantity = fill_qty.clone();
    args.order_type = Some(order_intent::venue_order_type(
        params
            .get("order_type")
            .and_then(Value::as_str)
            .unwrap_or(""),
        args.price.as_deref(),
    ));
    let ctx = flush_ctx(&params);
    match place_world_order(&app, args, ctx) {
        Ok(value) if value.get("executable") == Some(&Value::Bool(false)) => {
            let detail = value
                .pointer("/policy_result/detail")
                .and_then(Value::as_str)
                .unwrap_or("mandate blocked");
            let _ = brain.complete_execute(
                account_id,
                instruction_id,
                &json!({
                    "failed": true,
                    "error": detail,
                    "receipt": format!("stopped after slice {slice_i} — {detail}"),
                }),
            );
            Ok(value)
        }
        Ok(value) => {
            let receipt = value.get("receipt").cloned().unwrap_or(json!({}));
            let filled = filled_after(&params, &fill_qty);
            let avg = avg_price(&receipt);
            let progress = brain.record_slice(
                account_id,
                instruction_id,
                &json!({
                    "slice_i": slice_i,
                    "avg_price": avg,
                    "filled_quantity": filled,
                    "receipt": format!("slice {slice_i} of {slice_n}"),
                    "result_ref": receipt.get("transaction_hash"),
                    "fill": {
                        "hash": receipt.get("transaction_hash"),
                        "quantity": fill_qty,
                        "price": avg,
                    },
                }),
            )?;
            let graduating = graduate_kind(&brain, account_id, &params);
            let avg_s = value_as_price(&avg);
            let fill_message =
                fill_receipt_message(&params, &fill_qty, avg_s.as_deref(), graduating);
            if let Some(message) = fill_message.as_ref() {
                deliver_fill_receipt(&params, instruction_id, message);
            }
            if last || progress.get("more") != Some(&Value::Bool(true)) {
                let stored = fill_message
                    .clone()
                    .unwrap_or_else(|| receipt_line(&value, &receipt));
                let _ = brain.complete_execute(
                    account_id,
                    instruction_id,
                    &json!({
                        "receipt": stored,
                        "avg_price": avg,
                        "result_ref": receipt.get("transaction_hash"),
                        "filled_quantity": filled,
                    }),
                );
                return Ok(json!({
                    "ok": true,
                    "more_slices": false,
                    "receipt": receipt,
                    "graduating": graduating,
                }));
            }
            Ok(json!({
                "ok": true,
                "more_slices": true,
                "interval_secs": progress.get("interval_secs"),
                "next_slice_at": progress.get("next_slice_at"),
                "instruction": progress.get("instruction"),
            }))
        }
        Err(err) => {
            let _ = brain.complete_execute(
                account_id,
                instruction_id,
                &json!({
                    "failed": true,
                    "error": err,
                    "receipt": err,
                }),
            );
            Err(err)
        }
    }
}

pub fn flush_due_trades(account_id: u64) -> Result<Value, String> {
    let brain = BrainClient::from_env();
    let listed = brain.due_trades(Some(account_id))?;
    let trades = listed
        .get("trades")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut flushed = 0u32;
    let mut skipped = 0u32;
    for trade in trades {
        let id = trade
            .get("instruction_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if id.is_empty() {
            continue;
        }
        match flush_staged_trade(account_id, id) {
            Ok(value) if value.get("skipped") == Some(&Value::Bool(true)) => skipped += 1,
            Ok(_) => flushed += 1,
            Err(_) => skipped += 1,
        }
    }
    Ok(json!({ "ok": true, "flushed": flushed, "skipped": skipped }))
}

fn skippable_claim(err: &str) -> bool {
    err.contains("too_soon")
        || err.contains("cancelled")
        || err.contains("not_pending")
        || err.contains("in_flight")
        || err.contains("not_executing")
}

/// Kind graduates only after a successful send, never on stage or cancel.
fn graduate_kind(brain: &BrainClient, account_id: u64, params: &Value) -> bool {
    let Some(kind) = params
        .get("action_kind")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    match brain.confirm_action_kind(account_id, kind) {
        Ok(value) => value
            .get("graduating")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        Err(_) => false,
    }
}

fn value_as_price(avg: &Value) -> Option<String> {
    avg.as_str()
        .map(str::to_string)
        .or_else(|| avg.as_number().map(|n| n.to_string()))
        .filter(|s| !s.is_empty() && s != "null")
}

fn fill_receipt_message(
    params: &Value,
    fill_qty: &str,
    avg: Option<&str>,
    graduating: bool,
) -> Option<String> {
    if !graduating {
        return None;
    }
    let asset = params
        .get("base_symbol")
        .and_then(Value::as_str)
        .unwrap_or("");
    let product = params
        .get("product")
        .and_then(Value::as_str)
        .unwrap_or("spot");
    let sentence = params.get("sentence").and_then(Value::as_str).unwrap_or("");
    let mark = params
        .get("mark")
        .and_then(Value::as_str)
        .and_then(|s| parse_decimal(s, "mark").ok())
        .unwrap_or(Decimal::ZERO);
    let qty = parse_decimal(fill_qty, "qty")
        .ok()
        .or_else(|| {
            params
                .get("quantity")
                .and_then(Value::as_str)
                .and_then(|s| parse_decimal(s, "quantity").ok())
        })
        .unwrap_or(Decimal::ZERO);
    let notional = params
        .get("notional")
        .and_then(Value::as_str)
        .and_then(|s| parse_decimal(s, "notional").ok())
        .unwrap_or_else(|| {
            if mark > Decimal::ZERO {
                qty * mark
            } else {
                Decimal::ZERO
            }
        });
    let resolved = crate::size::ResolvedSize {
        input: String::new(),
        denomination: "quote",
        mark,
        base_qty: qty,
        notional,
        size: crate::size::Size::Quote(notional),
    };
    let happened = crate::reporting::render_size_happened(&resolved, asset, product, avg, false);
    Some(crate::reporting::render_receipt(
        &happened,
        &format!("You asked to {sentence}."),
        None,
        avg,
        None,
        "within limits.",
        "Nothing to watch. I'll only message you if it moves enough to change your risk band.",
        Some(crate::reporting::GRADUATION_NOTICE),
        true,
    ))
}

fn deliver_fill_receipt(params: &Value, instruction_id: &str, message: &str) {
    eprintln!("[world-markets] thread_message");
    for line in message.lines() {
        eprintln!("bot ▸ {line}");
    }
    let brain = BrainClient::from_env();
    let _ = brain.enqueue_outbound(&json!({
        "kind": "receipt",
        "message": message,
        "account_id": params.get("account_id"),
        "instruction_id": instruction_id,
    }));
    if let Some(chat_id) = params.get("telegram_chat_id").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    }) {
        let token = std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
        let _ = crate::mini_app::post_chat_lines(&token, chat_id, &[message.to_string()]);
    }
}

fn child_quantity(params: &Value, slice_i: u32, slice_n: u32) -> Result<String, String> {
    let total = parse_decimal(
        params
            .get("quantity")
            .and_then(Value::as_str)
            .unwrap_or("0"),
        "quantity",
    )
    .map_err(|verdict| format!("[world-markets] {}: {}", verdict.rule, verdict.detail))?;
    let filled = parse_decimal(
        params
            .pointer("/schedule/filled_quantity")
            .and_then(Value::as_str)
            .unwrap_or("0"),
        "filled_quantity",
    )
    .unwrap_or(Decimal::ZERO);
    Ok(
        order_intent::slice_quantity(total, filled, slice_n, slice_i)
            .normalize()
            .to_string(),
    )
}

fn filled_after(params: &Value, fill_qty: &str) -> String {
    let prior = parse_decimal(
        params
            .pointer("/schedule/filled_quantity")
            .and_then(Value::as_str)
            .unwrap_or("0"),
        "filled_quantity",
    )
    .unwrap_or(Decimal::ZERO);
    let add = parse_decimal(fill_qty, "fill").unwrap_or(Decimal::ZERO);
    (prior + add).normalize().to_string()
}

fn args_from_params(params: &Value, account_id: u64) -> ExecuteWorldOrderArgs {
    ExecuteWorldOrderArgs {
        product: params
            .get("product")
            .and_then(Value::as_str)
            .unwrap_or("spot")
            .to_string(),
        side: params
            .get("side")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        base_symbol: params
            .get("base_symbol")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        quote_symbol: params
            .get("quote_symbol")
            .and_then(Value::as_str)
            .map(str::to_string),
        quantity: params
            .get("quantity")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        price: params
            .get("price")
            .and_then(Value::as_str)
            .map(str::to_string),
        order_type: params
            .get("order_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        slippage: params
            .get("slippage")
            .and_then(Value::as_str)
            .map(str::to_string),
        slices: params
            .pointer("/schedule/slices")
            .and_then(Value::as_u64)
            .map(|n| n as u32),
        window_minutes: None,
        interval_secs: params
            .pointer("/schedule/interval_secs")
            .and_then(Value::as_u64),
        cadence: params
            .pointer("/schedule/cadence")
            .and_then(Value::as_str)
            .map(str::to_string),
        account_id: Some(
            params
                .get("account_id")
                .and_then(Value::as_u64)
                .unwrap_or(account_id),
        ),
        wallet_address: params
            .get("wallet_address")
            .and_then(Value::as_str)
            .map(str::to_string),
        sentence: None,
        instruction_id: None,
        size_usd: params
            .get("size_usd")
            .and_then(Value::as_str)
            .map(str::to_string),
        size_base: params
            .get("size_base")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

fn flush_ctx(params: &Value) -> DynToolCallCtx {
    let mut attrs = Map::new();
    if let Some(mandate) = params.get("handover_mandate") {
        attrs.insert("handover_mandate".to_string(), mandate.clone());
    }
    let mut world = Map::new();
    if let Some(id) = params.get("account_id") {
        world.insert("account_id".to_string(), id.clone());
    }
    if let Some(wallet) = params.get("wallet_address") {
        world.insert("owner_wallet".to_string(), wallet.clone());
    }
    if !world.is_empty() {
        attrs.insert("world".to_string(), Value::Object(world));
    }
    DynToolCallCtx {
        session_id: "flush-staged".to_string(),
        tool_name: "execute_world_order".to_string(),
        call_id: "flush-staged-1".to_string(),
        state_attributes: attrs,
        secrets: Default::default(),
    }
}

fn receipt_line(value: &Value, receipt: &Value) -> String {
    receipt
        .get("transaction_hash")
        .or_else(|| value.pointer("/receipt/transaction_hash"))
        .and_then(Value::as_str)
        .filter(|hash| !hash.is_empty())
        .map(|hash| format!("filled · {hash}"))
        .or_else(|| receipt.as_str().map(str::to_string))
        .unwrap_or_else(|| "filled".to_string())
}

fn avg_price(receipt: &Value) -> Value {
    receipt
        .get("avg_price")
        .or_else(|| receipt.get("price"))
        .cloned()
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_args_round_trip_schedule() {
        let params = json!({
            "product": "spot",
            "side": "buy",
            "base_symbol": "ETH",
            "quote_symbol": "USDT",
            "quantity": "10",
            "order_type": "twap",
            "slippage": "0.005",
            "account_id": 7,
            "wallet_address": "0xabc",
            "schedule": {
                "slices": 4,
                "interval_secs": 60,
                "cadence": "twap",
                "quantity_per_slice": "2.5",
                "filled_quantity": "2.5"
            }
        });
        let args = args_from_params(&params, 42);
        assert_eq!(args.order_type.as_deref(), Some("twap"));
        assert_eq!(args.slices, Some(4));
        assert_eq!(args.interval_secs, Some(60));
        assert_eq!(args.cadence.as_deref(), Some("twap"));
        assert_eq!(args.quantity, "10");
        assert_eq!(args.base_symbol, "ETH");
        assert_eq!(args.account_id, Some(7));
        assert_eq!(args.wallet_address.as_deref(), Some("0xabc"));
        assert_eq!(
            child_quantity(&params, 2, 4).unwrap(),
            order_intent::slice_quantity(Decimal::new(10, 0), Decimal::new(25, 1), 4, 2)
                .normalize()
                .to_string()
        );
    }

    #[test]
    fn fill_receipt_only_when_graduating_and_never_asks_yes() {
        let params = json!({
            "product": "spot",
            "base_symbol": "WETH",
            "notional": "200",
            "mark": "2500",
            "quantity": "0.08",
            "sentence": "buy $200 of WETH",
        });
        assert!(fill_receipt_message(&params, "0.08", Some("2500"), false).is_none());
        let message = fill_receipt_message(&params, "0.08", Some("2500"), true).unwrap();
        assert!(message.contains(crate::reporting::GRADUATION_NOTICE));
        assert!(message.contains("$200"), "{message}");
        assert!(!message.to_ascii_lowercase().contains("yes, send"));
        let qty = message
            .split('`')
            .find(|t| t.starts_with("0."))
            .unwrap_or("");
        let frac = qty.split('.').nth(1).unwrap_or("");
        assert!(frac.len() <= 4, "{qty}");
    }

    #[test]
    fn skippable_claim_does_not_count_as_send() {
        assert!(skippable_claim("cancelled"));
        assert!(skippable_claim("too_soon"));
        assert!(!skippable_claim("filled"));
    }
}
