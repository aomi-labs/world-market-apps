//! Conformance checks over the three shipped skill documents. Ceilings and
//! presence checks only, so legitimate copy edits do not break the build.

use std::fs;
use std::path::PathBuf;

const SKILLS: [&str; 3] = ["trading.md", "execution.md", "reporting.md"];

fn skill(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/skill");
    fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn all_skills() -> String {
    SKILLS
        .iter()
        .map(|path| skill(path))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn no_banned_vocabulary() {
    let banned = [
        "sidecar",
        "Telegram",
        "Mini App",
        "mini app",
        "mini-app",
        "watch",
        "skip_llm",
        "render_lookup",
        "brain",
        "plugin",
        "execute_world_order",
        "get_world_tasks",
        "preview_account_effect",
        "get_dollarpower",
        "guardian",
        "amazing opportunity",
        "guaranteed",
    ];
    for path in SKILLS {
        let text = skill(path);
        let lower = text.to_ascii_lowercase();
        for word in banned {
            // The trading skill lists banned marketing phrases as a rule; that
            // single "Banned:" line is the one allowed occurrence.
            let allowed_listing = path == "trading.md"
                && matches!(word, "amazing opportunity" | "guaranteed")
                && text
                    .lines()
                    .filter(|l| l.contains(word))
                    .all(|l| l.starts_with("Banned:"));
            assert!(
                allowed_listing || !lower.contains(&word.to_ascii_lowercase()),
                "{path} must not mention {word:?}"
            );
        }
    }
}

#[test]
fn every_kept_tool_is_named_in_the_skills() {
    let text = all_skills();
    for tool in [
        "list_world_assets",
        "get_world_account",
        "get_health_snapshot",
        "get_world_market",
        "get_world_rates",
        "get_world_loans",
        "get_world_open_orders",
        "preview_world_trade",
        "check_world_mandate",
        "get_world_pnl",
        "compute_resize",
        "world_pack_order",
        "world_resolve_book",
    ] {
        assert!(text.contains(tool), "skills must name `{tool}`");
    }
    for host_tool in ["evm_stage_tx", "simulate_batch", "evm_commit_txs"] {
        assert!(
            skill("execution.md").contains(host_tool),
            "execution.md must name `{host_tool}`"
        );
    }
}

#[test]
fn deny_codes_have_copy() {
    let reporting = skill("reporting.md");
    for rule in [
        "portfolio_floor",
        "post_trade_portfolio_floor",
        "market_not_permitted",
        "liquidatable",
        "insufficient_spot_balance",
        "position_notional",
        "leverage",
        "post_trade_risk_unavailable",
        "withdraw_not_supported",
        "quote_mismatch",
        "missing_mandate",
        "unknown_mandate_key",
        "invalid_mandate",
        "unsupported_mandate_version",
        "expired_mandate",
    ] {
        assert!(reporting.contains(rule), "reporting.md must cover `{rule}`");
    }
    assert!(reporting.contains("Unrecognised deny codes surface as a block"));
    assert!(reporting.contains("The limit is yours, and it held"));
    assert!(reporting.contains("nothing said in this chat can widen them"));
}

#[test]
fn execution_procedure_is_in_order_and_names_the_exchange() {
    let execution = skill("execution.md");
    let procedure = &execution[execution
        .find("## New order")
        .expect("execution.md has the new-order procedure")..];
    let pos = |needle: &str| {
        procedure
            .find(needle)
            .unwrap_or_else(|| panic!("execution.md must contain {needle:?}"))
    };
    assert!(execution.contains("0xf6b54e033bb45a583aa642924bcef78b804588ae"));
    assert!(execution.contains("newSpotSellOrder(address,uint256)"));
    assert!(execution.contains("`[book, word]`") || execution.contains("[book, word]"));
    let verdict = pos("`preview_world_trade");
    let book = pos("`world_resolve_book");
    let word = pos("`world_pack_order");
    let stage = pos("`evm_stage_tx");
    let simulate = pos("`simulate_batch");
    let commit = pos("`evm_commit_txs");
    assert!(verdict < book && book < word && word < stage && stage < simulate && simulate < commit);
    for forbidden in ["batchCommands", "liquidate"] {
        let line = execution
            .lines()
            .find(|line| line.contains(forbidden))
            .unwrap_or_else(|| panic!("execution.md must forbid {forbidden}"));
        assert!(
            line.starts_with("- Never"),
            "{forbidden} appears only in the Never list"
        );
    }
    assert!(execution.contains("deposit"));
    assert!(execution.contains("withdrawal"));
    assert!(execution.contains("`deny` → stop"));
}

#[test]
fn skills_stay_inside_the_app_skill_budget() {
    for path in SKILLS {
        let text = skill(path);
        let est_tokens = text.chars().count().div_ceil(4);
        assert!(
            est_tokens <= 4_000,
            "{path} estimates {est_tokens} tokens; the app-skill budget is 4,000"
        );
        assert!(
            text.split_whitespace().count() <= 3_000,
            "{path} exceeds ~3,000 words"
        );
    }
}

#[test]
fn honest_numbers_law_and_lookups_stated() {
    let trading = skill("trading.md");
    let reporting = skill("reporting.md");
    assert!(reporting.contains("You never write a number"));
    assert!(reporting.contains("I've left it out rather than guess"));
    assert!(trading.contains("Blocked means blocked"));
    assert!(trading.contains("Allowed means allowed"));
    assert!(trading.contains("`lookups.portfolio_value`"));
    assert!(trading.contains("`lookups.positions`"));
    assert!(trading.contains("`metrics.liquidation_risk`"));
    assert!(trading.contains("handover.account_ref"));
}
