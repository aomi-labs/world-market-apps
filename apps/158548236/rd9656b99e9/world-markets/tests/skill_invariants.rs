//! Static skill-payload invariants (eval floor 4b). Ceilings and self-relative
//! checks only — exact-byte pins would fail on every legitimate design-agent edit.

use std::fs;
use std::path::PathBuf;

fn skill_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/skill")
}

fn skill(path: &str) -> String {
    fs::read_to_string(skill_root().join(path)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

fn bytes(path: &str) -> usize {
    fs::metadata(skill_root().join(path))
        .unwrap_or_else(|e| panic!("stat {path}: {e}"))
        .len() as usize
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
}

#[test]
fn workflow_headers_carry_when_mode_budget() {
    let wf = skill("workflows.md");
    let mut missing = Vec::new();
    let lines: Vec<&str> = wf.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if !line.starts_with("## ") {
            continue;
        }
        let mut non_blank = 0;
        let mut found_when = false;
        let mut found_mode = false;
        let mut found_budget = false;
        for next in lines.iter().skip(i + 1) {
            if next.starts_with("## ") {
                break;
            }
            if next.trim().is_empty() {
                continue;
            }
            non_blank += 1;
            if non_blank > 4 {
                break;
            }
            if next.contains("WHEN:") {
                found_when = true;
            }
            if next.contains("MODE:") {
                found_mode = true;
            }
            if next.contains("BUDGET:") {
                found_budget = true;
            }
            if found_when && found_mode && found_budget {
                break;
            }
        }
        if !found_when || !found_mode || !found_budget {
            missing.push(line.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "every ## flow must have WHEN/MODE/BUDGET within 4 non-blank lines: {missing:?}"
    );
}

#[test]
fn payload_form_gates_are_ceilings() {
    let mut payload = 0usize;
    let mut reference = 0usize;
    let mut turn_contract = 0usize;
    for entry in walkdir_skill() {
        let len = fs::metadata(&entry)
            .unwrap_or_else(|e| panic!("{}: {e}", entry.display()))
            .len() as usize;
        payload += len;
        let rel = entry
            .strip_prefix(skill_root())
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with("reference/") {
            reference += len;
        }
        if rel == "turn-contract.md" {
            turn_contract = len;
        }
    }
    assert!(
        payload <= 55_000,
        "static skill payload {payload} B exceeds 55 KB ceiling (round-4 copy was additive; 54,340 B measured)"
    );
    assert!(
        reference <= 4_751,
        "reference/* {reference} B exceeds 4,751 B ceiling"
    );
    assert!(
        turn_contract <= 3_300,
        "turn-contract.md {turn_contract} B exceeds 3,300 B ceiling (round-4 copy was additive)"
    );
    assert_eq!(bytes("turn-contract.md"), turn_contract);
}

fn walkdir_skill() -> Vec<PathBuf> {
    let mut files = Vec::new();
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap().path();
            if entry.is_dir() {
                walk(&entry, out);
            } else if entry.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(entry);
            }
        }
    }
    walk(&skill_root(), &mut files);
    files.sort();
    files
}

#[test]
fn workflow_when_mode_budget_counts_match_headers() {
    let wf = skill("workflows.md");
    let headers = wf.lines().filter(|l| l.starts_with("## ")).count();
    let when = wf.matches("WHEN:").count();
    let mode = wf.matches("MODE:").count();
    let budget = wf.matches("BUDGET:").count();
    assert_eq!(headers, when, "each ## flow needs one WHEN:");
    assert_eq!(headers, mode, "each ## flow needs one MODE:");
    assert_eq!(headers, budget, "each ## flow needs one BUDGET:");
}

#[test]
fn section_refs_resolve_to_workflow_headers() {
    let mut cited = std::collections::BTreeSet::new();
    for path in walkdir_skill() {
        let text = fs::read_to_string(&path).unwrap();
        collect_section_refs(&text, &mut cited);
    }
    let wf = skill("workflows.md");
    let mut defined = std::collections::BTreeSet::new();
    collect_section_refs(&wf, &mut defined);
    let missing: Vec<_> = cited.difference(&defined).cloned().collect();
    assert!(
        missing.is_empty(),
        "§6.x cited in skill/ but missing as a workflows.md header: {missing:?}"
    );
}

fn collect_section_refs(text: &str, out: &mut std::collections::BTreeSet<String>) {
    let mut rest = text;
    while let Some(pos) = rest.find('§') {
        rest = &rest[pos + '§'.len_utf8()..];
        if !rest.starts_with("6.") {
            continue;
        }
        let chars: Vec<char> = rest.chars().collect();
        let mut end = 2;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
        if end < chars.len() && matches!(chars[end], 'a' | 'b' | 'c') {
            end += 1;
        }
        if end > 2 {
            out.insert(format!("§{}", chars[..end].iter().collect::<String>()));
        }
    }
}
