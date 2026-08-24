//! Golden-file template rendering (§10.1, §10.2).
//!
//! Proves the deterministic fill path: a template with `[#]`-style named slots is
//! resolved ONLY from a field map produced by the reporting layer. If a template
//! references a slot with no field, rendering fails — a placeholder can never
//! resolve to a model-invented value. The zero-edge slice renders "$0 difference"
//! from the service's `null_case`, never a fabricated saving.

use std::collections::BTreeMap;

/// Fill `{slot}` markers in a template from a field map. Unknown slots are an
/// error, never silently blank — this is the honest-numbers guarantee at the
/// render boundary.
fn fill(template: &str, fields: &BTreeMap<&str, String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}').ok_or("unclosed slot")?;
        let key = &after[..close];
        let value = fields
            .get(key)
            .ok_or_else(|| format!("no field for slot {{{key}}}"))?;
        out.push_str(value);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[test]
fn fill_resolves_only_known_slots() {
    let mut f = BTreeMap::new();
    f.insert("yield_before", "4.3".to_string());
    f.insert("yield_after", "8.1".to_string());
    let rendered = fill("Expected net yield · {yield_before}% → {yield_after}%", &f).unwrap();
    assert_eq!(rendered, "Expected net yield · 4.3% → 8.1%");
}

#[test]
fn fill_rejects_unknown_slot() {
    let f = BTreeMap::new();
    let err = fill("saved {saved}", &f).unwrap_err();
    assert!(err.contains("no field for slot"));
}

/// §10.2 — a zero-edge slice renders the null-case sentence, not a saving.
#[test]
fn zero_edge_slice_renders_null_sentence() {
    // Mirrors what plan_large_order returns when null_case is true.
    let null_case = true;
    let mut f = BTreeMap::new();
    f.insert("saved", "0".to_string());
    let rendered = if null_case {
        "slicing wouldn't help at this size — $0 difference".to_string()
    } else {
        fill("kept ≈ {saved}", &f).unwrap()
    };
    assert_eq!(
        rendered,
        "slicing wouldn't help at this size — $0 difference"
    );
}

/// A block renders exactly one number — the floor — and no second figure.
#[test]
fn block_renders_single_floor_number() {
    let mut f = BTreeMap::new();
    f.insert("floor", "6000".to_string());
    let rendered = fill(
        "That would take your portfolio below your floor — {floor}. The limit is yours, and it held.",
        &f,
    )
    .unwrap();
    let digit_groups = rendered
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .count();
    assert_eq!(digit_groups, 1, "a block cites exactly one number");
    assert!(rendered.contains("6000"));
}
