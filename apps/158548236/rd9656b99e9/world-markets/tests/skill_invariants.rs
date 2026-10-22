//! Static invariants over the always-active turn contract.

use std::fs;
use std::path::PathBuf;

fn skill(path: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/skill");
    fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[test]
fn turn_contract_routing_order_is_load_bearing() {
    let contract = skill("turn-contract.md");
    let cant = contract.find("CANT").expect("CANT in turn-contract");
    let action = contract.find("ACTION").expect("ACTION in turn-contract");
    let verdict = contract
        .find("ADVISORY-VERDICT")
        .expect("ADVISORY-VERDICT in turn-contract");
    let explain = contract
        .find("ADVISORY-EXPLAIN")
        .expect("ADVISORY-EXPLAIN in turn-contract");
    assert!(
        cant < action,
        "CANT must precede ACTION in the classify table (out-of-universe trades are not ACTION)"
    );
    assert!(
        verdict < explain,
        "ADVISORY-VERDICT must precede ADVISORY-EXPLAIN (should-I is not an explain turn)"
    );
    assert!(contract.contains("policy engine verdict > blocked-means-blocked"));
}

#[test]
fn always_active_files_carry_no_removed_surfaces() {
    for path in ["turn-contract.md", "safety.md"] {
        let text = skill(path);
        for banned in [
            "sidecar",
            "Telegram",
            "Mini App",
            "WATCH",
            "GUEST",
            "SHARE",
            "get_world_tasks",
            "render_lookup",
            "skip_llm",
        ] {
            assert!(!text.contains(banned), "{path} must not mention {banned}");
        }
    }
}
