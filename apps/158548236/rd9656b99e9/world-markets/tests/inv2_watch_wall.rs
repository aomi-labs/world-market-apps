//! INV-2: watch / research / task-store paths must not reach execution.

use std::fs;
use std::path::PathBuf;

fn crate_src(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("src");
    p.push(name);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

#[test]
fn watch_research_task_modules_do_not_import_execution() {
    for name in ["brain.rs", "research.rs", "tasks.rs"] {
        let src = crate_src(name);
        assert!(
            !src.contains("use crate::execution"),
            "{name} must not import execution"
        );
        assert!(
            !src.contains("ExecutionClient"),
            "{name} must not name the execution client"
        );
        assert!(!src.contains("place_order"), "{name} must not place orders");
        assert!(
            !src.contains("WORLD_PRIVATE_KEY"),
            "{name} must not mention the signer key"
        );
    }
}
