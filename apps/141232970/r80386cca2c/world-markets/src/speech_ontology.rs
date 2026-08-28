//! World utterance language. Vocabulary, frames, repairs, and channel policy
//! live in `assets/speech_ontology.json`. Rust is the only matcher.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

const ONTOLOGY_JSON: &str = include_str!("../assets/speech_ontology.json");
const ONTOLOGY_VERSION: u32 = 3;
const EXTRA_KEYTERM_BUDGET: usize = 40;
const MAX_EDIT_DISTANCE: usize = 1;

#[derive(Debug, Deserialize)]
struct OntologyFile {
    version: u32,
    entries: Vec<OntologyEntry>,
    #[serde(default)]
    frames: Vec<OntologyFrame>,
    #[serde(default)]
    repairs: Vec<OntologyRepair>,
    #[serde(default)]
    gates: OntologyGates,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OntologyEntry {
    pub surface_form: String,
    pub normalized_target: String,
    pub kind: String,
    #[serde(default)]
    pub confidence: f64,
    /// Omitted or empty means both speech and text.
    #[serde(default)]
    pub channels: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OntologyFrame {
    id: String,
    role: String,
    #[serde(default)]
    tokens: Vec<String>,
    #[serde(default)]
    fuzzy: bool,
    #[serde(default)]
    acts: Vec<String>,
    #[serde(default)]
    fuzzy_acts: Vec<String>,
    #[serde(default)]
    size: Option<String>,
    #[serde(default)]
    instrument: Option<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    referents: Vec<String>,
    #[serde(default)]
    open_prefix: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct OntologyRepair {
    id: String,
    kind: String,
    surface: String,
    target: String,
    #[serde(default)]
    insert_before_worth: Option<String>,
    #[serde(default)]
    require_acts: Vec<String>,
    #[serde(default)]
    require_span: Vec<String>,
    #[serde(default)]
    channels: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct OntologyGates {
    #[serde(default)]
    confusable_pronouns: Vec<String>,
    #[serde(default)]
    control_acts: Vec<String>,
    #[serde(default)]
    watch_referents: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Speech,
    Text,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Speech => "speech",
            Channel::Text => "text",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("speech") {
            Channel::Speech
        } else {
            Channel::Text
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrammarStatus {
    Matched,
    Partial,
    None,
}

impl GrammarStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GrammarStatus::Matched => "matched",
            GrammarStatus::Partial => "partial",
            GrammarStatus::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UtteranceSlot {
    pub kind: String,
    pub surface: String,
    pub target: String,
    pub source: String,
}

impl UtteranceSlot {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "kind": self.kind,
            "surface": self.surface,
            "target": self.target,
            "source": self.source,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionIr {
    pub act: String,
    pub instrument: Option<String>,
    pub size: Option<String>,
    pub product: Option<String>,
    pub referent: Option<String>,
    pub frame_id: Option<String>,
    pub order_type: Option<String>,
    pub size_kind: Option<String>,
}

impl ActionIr {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "act": self.act,
            "instrument": self.instrument,
            "size": self.size,
            "product": self.product,
            "referent": self.referent,
            "frame_id": self.frame_id,
            "order_type": self.order_type,
            "size_kind": self.size_kind,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeKind {
    Quote,
    Base,
    Ambiguous,
    None,
}

impl SizeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SizeKind::Quote => "quote",
            SizeKind::Base => "base",
            SizeKind::Ambiguous => "ambiguous",
            SizeKind::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeSpan {
    pub kind: SizeKind,
    pub surface: String,
    pub amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedVeto {
    pub asset: String,
    pub absolute: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexiconEntry {
    pub surface_form: String,
    pub normalized_target: String,
    pub kind: String,
}

impl LexiconEntry {
    pub fn from_json(value: &Value) -> Option<Self> {
        let surface = value
            .get("surface_form")
            .or_else(|| value.get("surface"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("instrument");
        if kind == "confusable" {
            return None;
        }
        let target = value
            .get("normalized_target")
            .or_else(|| value.get("target"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(surface);
        Some(Self {
            surface_form: surface.to_string(),
            normalized_target: target.to_string(),
            kind: kind.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedUtterance {
    pub channel: Channel,
    pub raw: String,
    pub normalized_text: String,
    pub ontology_version: u32,
    pub stt_version: Option<String>,
    pub keyterm_applied: bool,
    pub slots: Vec<UtteranceSlot>,
    pub proposals: Vec<ProposedConfusable>,
    pub grammar: GrammarStatus,
    pub action_ir: Option<ActionIr>,
    pub lexicon_hits: Vec<LexiconHit>,
    pub repaired_from: Option<String>,
    pub unknown_instruments: Vec<String>,
}

impl NormalizedUtterance {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "channel": self.channel.as_str(),
            "raw": self.raw,
            "normalized_text": self.normalized_text,
            "ontology_version": self.ontology_version,
            "stt_version": self.stt_version,
            "keyterm_applied": self.keyterm_applied,
            "slots": self.slots.iter().map(UtteranceSlot::to_json).collect::<Vec<_>>(),
            "proposals": self.proposals.iter().map(ProposedConfusable::to_json).collect::<Vec<_>>(),
            "grammar": self.grammar.as_str(),
            "action_ir": self.action_ir.as_ref().map(ActionIr::to_json),
            "lexicon_hits": self.lexicon_hits.iter().map(LexiconHit::to_json).collect::<Vec<_>>(),
            "repaired_from": self.repaired_from,
            "unknown_instruments": self.unknown_instruments,
        })
    }

    fn blank(raw: impl Into<String>, channel: Channel) -> Self {
        let raw = raw.into();
        Self {
            channel,
            raw: raw.clone(),
            normalized_text: raw,
            ontology_version: ONTOLOGY_VERSION,
            stt_version: None,
            keyterm_applied: false,
            slots: Vec::new(),
            proposals: Vec::new(),
            grammar: GrammarStatus::None,
            action_ir: None,
            lexicon_hits: Vec::new(),
            repaired_from: None,
            unknown_instruments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexiconHit {
    pub surface_form: String,
    pub normalized_target: String,
    pub kind: String,
    pub source: String,
}

impl LexiconHit {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "surface_form": self.surface_form,
            "normalized_target": self.normalized_target,
            "kind": self.kind,
            "source": self.source,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedConfusable {
    pub surface: String,
    pub target: String,
}

impl ProposedConfusable {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "surface": self.surface,
            "target": self.target,
            "kind": "confusable",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repair {
    pub text: String,
    pub hits: Vec<LexiconHit>,
    pub repaired_from: Option<String>,
    pub proposed_confusables: Vec<ProposedConfusable>,
}

impl Repair {
    #[allow(dead_code)]
    fn blank(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            hits: Vec::new(),
            repaired_from: None,
            proposed_confusables: Vec::new(),
        }
    }
}

enum SlotHit {
    Canonical {
        surface: String,
        target: String,
        source: String,
    },
    Confusable {
        surface: String,
        target: String,
    },
}

struct MappedSurface {
    target: String,
    source: SlotSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotSource {
    Exact,
    Alias,
    Lexicon,
}

impl SlotSource {
    fn as_str(self) -> &'static str {
        match self {
            SlotSource::Exact => "exact",
            SlotSource::Alias => "alias",
            SlotSource::Lexicon => "lexicon",
        }
    }

    fn to_string(self) -> String {
        self.as_str().to_string()
    }
}

struct Ontology {
    entries: Vec<OntologyEntry>,
    kind_by_surface: HashMap<String, String>,
    instrument_by_surface: HashMap<String, String>,
    instrument_channels: HashMap<String, Vec<String>>,
    confusable_by_surface: HashMap<String, String>,
    confusable_channels: HashMap<String, Vec<String>>,
    size_words: HashSet<String>,
    units: HashSet<String>,
    frames: Vec<OntologyFrame>,
    repairs: Vec<OntologyRepair>,
    gates: OntologyGates,
}

fn channels_of(channels: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for row in channels {
        let key = row.trim().to_ascii_lowercase();
        if key.is_empty() || !seen.insert(key.clone()) {
            continue;
        }
        out.push(key);
    }
    if out.is_empty() {
        vec!["speech".to_string(), "text".to_string()]
    } else {
        out
    }
}

fn allows_channel(channels: &[String], channel: Channel) -> bool {
    channels_of(channels)
        .iter()
        .any(|row| row == channel.as_str())
}

fn ontology() -> &'static Ontology {
    static ONTOLOGY: OnceLock<Ontology> = OnceLock::new();
    ONTOLOGY.get_or_init(load_ontology)
}

fn load_ontology() -> Ontology {
    let file: OntologyFile =
        serde_json::from_str(ONTOLOGY_JSON).expect("assets/speech_ontology.json must parse");
    assert_eq!(
        file.version, ONTOLOGY_VERSION,
        "assets/speech_ontology.json version"
    );
    let mut kind_by_surface = HashMap::new();
    let mut instrument_by_surface = HashMap::new();
    let mut instrument_channels = HashMap::new();
    let mut confusable_by_surface = HashMap::new();
    let mut confusable_channels = HashMap::new();
    let mut size_words = HashSet::new();
    let mut units = HashSet::new();
    for entry in &file.entries {
        let key = normalize_key(&entry.surface_form);
        if key.is_empty() {
            continue;
        }
        let ch = channels_of(&entry.channels);
        kind_by_surface.insert(key.clone(), entry.kind.clone());
        match entry.kind.as_str() {
            "instrument" => {
                instrument_by_surface.insert(key.clone(), entry.normalized_target.clone());
                instrument_channels.insert(key, ch);
            }
            "confusable" => {
                confusable_by_surface.insert(key.clone(), entry.normalized_target.clone());
                confusable_channels.insert(key, ch);
            }
            "size" => {
                size_words.insert(key);
            }
            "unit" | "size_frame" => {
                if entry.surface_form.eq_ignore_ascii_case("dollars")
                    || entry.surface_form.eq_ignore_ascii_case("bucks")
                    || entry.kind == "unit"
                {
                    units.insert(normalize_key(&entry.surface_form));
                }
            }
            _ => {}
        }
    }
    units.insert("dollars".to_string());
    units.insert("bucks".to_string());
    Ontology {
        entries: file.entries,
        kind_by_surface,
        instrument_by_surface,
        instrument_channels,
        confusable_by_surface,
        confusable_channels,
        size_words,
        units,
        frames: file.frames,
        repairs: file.repairs,
        gates: file.gates,
    }
}

fn normalize_key(surface: &str) -> String {
    surface
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn intensifier_for(term: &str) -> u8 {
    match kind_for(term).as_deref() {
        Some("instrument") => 5,
        Some("act" | "opener" | "size_frame" | "size" | "unit" | "product" | "order_type") => 3,
        _ => 2,
    }
}

pub fn kind_for(term: &str) -> Option<String> {
    let key = normalize_key(term);
    ontology().kind_by_surface.get(&key).cloned()
}

pub fn boostable_keyterms() -> Vec<String> {
    let ont = ontology();
    let mut ranked: Vec<&OntologyEntry> = ont
        .entries
        .iter()
        .filter(|row| row.kind != "confusable" && allows_channel(&row.channels, Channel::Speech))
        .collect();
    ranked.sort_by(|a, b| {
        kind_rank(&a.kind).cmp(&kind_rank(&b.kind)).then(
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for row in ranked {
        let term = row.surface_form.trim();
        if !is_boost_token(term, &row.kind) {
            continue;
        }
        let key = term.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        out.push(term.to_string());
    }
    out
}

fn is_boost_token(term: &str, kind: &str) -> bool {
    let trimmed = term.trim();
    if trimmed.len() < 2 || trimmed.eq_ignore_ascii_case("if") {
        return false;
    }
    let words = trimmed.split_whitespace().count();
    if words == 0 || words > 3 {
        return false;
    }
    if words > 1 {
        return matches!(kind, "act" | "opener");
    }
    true
}

fn kind_rank(kind: &str) -> u8 {
    match kind {
        "opener" | "act" => 0,
        "instrument" => 1,
        "order_type" => 2,
        "size_frame" => 3,
        "size" | "unit" => 4,
        "product" => 5,
        "level" => 6,
        "phrase" => 7,
        _ => 9,
    }
}

pub fn seed_keyterms(catalog: &[String], holdings: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let push = |seen: &mut HashSet<String>, out: &mut Vec<String>, term: &str| {
        if out.len() >= EXTRA_KEYTERM_BUDGET {
            return;
        }
        let trimmed = term.trim();
        if trimmed.len() < 2 || trimmed.eq_ignore_ascii_case("if") {
            return;
        }
        let words = trimmed.split_whitespace().count();
        if words == 0 || words > 3 {
            return;
        }
        let key = trimmed.to_ascii_lowercase();
        if !seen.insert(key) {
            return;
        }
        out.push(trimmed.to_string());
    };
    for term in boostable_keyterms() {
        push(&mut seen, &mut out, &term);
    }
    for symbol in catalog {
        push(&mut seen, &mut out, symbol);
    }
    for symbol in holdings {
        push(&mut seen, &mut out, symbol);
    }
    out
}

/// Thin wrapper around speech-channel normalize with an empty lexicon.
pub fn repair_transcript(raw: &str, catalog: &[String]) -> Repair {
    let normalized = normalize_utterance(raw, Channel::Speech, catalog, &[]);
    Repair {
        text: normalized.normalized_text,
        hits: normalized.lexicon_hits,
        repaired_from: normalized.repaired_from,
        proposed_confusables: normalized.proposals,
    }
}

/// Shared speech/text normalizer. Confusables honor JSON `channels`.
pub fn normalize_utterance(
    raw: &str,
    channel: Channel,
    catalog: &[String],
    lexicon: &[LexiconEntry],
) -> NormalizedUtterance {
    let original = raw.trim();
    if original.is_empty() {
        return NormalizedUtterance::blank("", channel);
    }
    let mut tokens = tokenize(original);
    if tokens.is_empty() {
        return NormalizedUtterance::blank(original, channel);
    }

    let universe = InstrumentUniverse::new(catalog, lexicon, channel);
    let mut slots: Vec<UtteranceSlot> = Vec::new();
    if let Some(eth_slot) = repair_eth_heard_as_eight(&mut tokens, channel) {
        slots.push(eth_slot);
    }
    if let Some(opener_slot) = restore_speech_opener(&mut tokens, channel) {
        slots.push(opener_slot);
    }
    if let Some(size_slot) = apply_repairs(&mut tokens, channel) {
        slots.push(size_slot);
    }

    rewrite_exact_aliases(&mut tokens, &universe, &mut slots);
    let mut instrument_slots = instrument_slots(&tokens);
    instrument_slots.sort_by_key(|slot| slot.index);
    instrument_slots.dedup_by_key(|slot| slot.index);

    let mut proposed_confusables: Vec<ProposedConfusable> = Vec::new();
    let mut proposed_seen = HashSet::new();
    let mut unknown_instruments: Vec<String> = Vec::new();
    let mut unknown_seen = HashSet::new();
    let mut mapped_indices: HashSet<usize> = HashSet::new();
    for (idx, token) in tokens.iter().enumerate() {
        if universe.canonical_instrument(token).is_some() {
            mapped_indices.insert(idx);
        }
    }

    for slot in instrument_slots.into_iter().rev() {
        if mapped_indices.contains(&slot.index) {
            continue;
        }
        if should_gate_confusable(&tokens, slot.index) {
            continue;
        }
        let Some((consumed, hit)) =
            resolve_instrument(&tokens, slot.index, slot.fuzzy, channel, &universe)
        else {
            if let Some(surface) = tokens.get(slot.index) {
                if unknown_seen.insert(surface.to_ascii_lowercase()) {
                    unknown_instruments.push(surface.clone());
                }
            }
            continue;
        };
        match hit {
            SlotHit::Confusable { surface, target } => {
                if proposed_seen.insert(surface.to_ascii_lowercase()) {
                    proposed_confusables.push(ProposedConfusable { surface, target });
                }
            }
            SlotHit::Canonical {
                surface,
                target,
                source,
            } => {
                tokens[slot.index] = target.clone();
                for _ in 1..consumed {
                    if slot.index + 1 < tokens.len() {
                        tokens.remove(slot.index + 1);
                    }
                }
                mapped_indices.insert(slot.index);
                push_instrument_slot(&mut slots, surface, target, source);
            }
        }
    }

    let mut hit_seen = HashSet::new();
    let mut lexicon_hits: Vec<LexiconHit> = Vec::new();
    for slot in &slots {
        if slot.kind != "instrument" {
            continue;
        }
        if hit_seen.insert(slot.target.to_ascii_lowercase()) {
            lexicon_hits.push(LexiconHit {
                surface_form: slot.target.clone(),
                normalized_target: slot.target.clone(),
                kind: "instrument".to_string(),
                source: slot.source.clone(),
            });
        }
    }

    let text = tokens.join(" ");
    let repaired_from = if text != original {
        Some(original.to_string())
    } else {
        None
    };
    let clause = first_clause(&tokens);
    let (grammar, action_ir) = parse_grammar(original, clause, &universe);
    let mut slots = slots;
    if let Some(ir) = &action_ir {
        if let Some(size) = &ir.size {
            ensure_size_slot(&mut slots, size);
        }
        if let Some(referent) = &ir.referent {
            if !slots.iter().any(|row| row.kind == "referent") {
                slots.push(UtteranceSlot {
                    kind: "referent".to_string(),
                    surface: referent.clone(),
                    target: referent.clone(),
                    source: "exact".to_string(),
                });
            }
        }
    }
    NormalizedUtterance {
        channel,
        raw: original.to_string(),
        normalized_text: text,
        ontology_version: ONTOLOGY_VERSION,
        stt_version: None,
        keyterm_applied: false,
        slots,
        proposals: proposed_confusables,
        grammar,
        action_ir,
        lexicon_hits,
        repaired_from,
        unknown_instruments,
    }
}

fn first_clause(tokens: &[String]) -> &[String] {
    tokens
        .iter()
        .position(|t| t == "and" || t == "then")
        .map(|i| &tokens[..i])
        .filter(|clause| !clause.is_empty())
        .unwrap_or(tokens)
}

fn ensure_size_slot(slots: &mut Vec<UtteranceSlot>, size: &str) {
    if slots.iter().any(|row| row.kind == "size") {
        return;
    }
    slots.push(UtteranceSlot {
        kind: "size".to_string(),
        surface: size.to_string(),
        target: size.to_string(),
        source: "exact".to_string(),
    });
}

fn push_instrument_slot(
    slots: &mut Vec<UtteranceSlot>,
    surface: String,
    target: String,
    source: String,
) {
    let key = target.to_ascii_lowercase();
    if slots
        .iter()
        .any(|row| row.kind == "instrument" && row.target.eq_ignore_ascii_case(&key))
    {
        return;
    }
    slots.push(UtteranceSlot {
        kind: "instrument".to_string(),
        surface,
        target,
        source,
    });
}

fn rewrite_exact_aliases(
    tokens: &mut Vec<String>,
    universe: &InstrumentUniverse,
    slots: &mut Vec<UtteranceSlot>,
) {
    let ont = ontology();
    let mut i = 0;
    while i < tokens.len() {
        if i + 1 < tokens.len() {
            let phrase = format!("{} {}", tokens[i], tokens[i + 1]);
            if let Some(mapped) = universe.exact(&phrase) {
                let surface = phrase;
                tokens[i] = mapped.target.clone();
                tokens.remove(i + 1);
                push_instrument_slot(
                    slots,
                    surface,
                    mapped.target.clone(),
                    mapped.source.to_string(),
                );
                i += 1;
                continue;
            }
        }
        if is_protected_token(&tokens[i], ont) {
            i += 1;
            continue;
        }
        if let Some(mapped) = universe.exact(&tokens[i]) {
            let surface = tokens[i].clone();
            if tokens[i] != mapped.target {
                tokens[i] = mapped.target.clone();
            }
            push_instrument_slot(
                slots,
                surface,
                mapped.target.clone(),
                mapped.source.to_string(),
            );
        }
        i += 1;
    }
}

fn is_protected_token(token: &str, ont: &Ontology) -> bool {
    if matches!(
        token,
        "of" | "a" | "an" | "the" | "and" | "then" | "to" | "for" | "me" | "my"
    ) {
        return true;
    }
    matches!(
        ont.kind_by_surface.get(token).map(String::as_str),
        Some("act" | "size" | "unit" | "size_frame" | "product" | "order_type")
    )
}

fn should_gate_confusable(tokens: &[String], index: usize) -> bool {
    let ont = ontology();
    let surface = tokens.get(index).map(String::as_str).unwrap_or("");
    if !ont
        .gates
        .confusable_pronouns
        .iter()
        .any(|row| row == surface)
    {
        return false;
    }
    if index > 0
        && ont
            .gates
            .control_acts
            .iter()
            .any(|row| row == tokens[index - 1].as_str())
    {
        return true;
    }
    let control = tokens
        .iter()
        .any(|t| matches!(t.as_str(), "cancel" | "pause" | "resume"));
    let watches = tokens.iter().any(|t| {
        ont.gates
            .watch_referents
            .iter()
            .any(|row| row == t.as_str())
    });
    control && watches
}

struct InstrumentUniverse {
    by_surface: HashMap<String, MappedSurface>,
    confusable: HashMap<String, String>,
    alias_surfaces: Vec<(String, String)>,
}

impl InstrumentUniverse {
    fn new(catalog: &[String], lexicon: &[LexiconEntry], channel: Channel) -> Self {
        let ont = ontology();
        let mut by_surface: HashMap<String, MappedSurface> = HashMap::new();
        for (surface, target) in &ont.instrument_by_surface {
            let ch = ont
                .instrument_channels
                .get(surface)
                .cloned()
                .unwrap_or_else(|| vec!["speech".into(), "text".into()]);
            if !allows_channel(&ch, channel) {
                continue;
            }
            let source = if surface.eq_ignore_ascii_case(target) {
                SlotSource::Exact
            } else {
                SlotSource::Alias
            };
            by_surface.insert(
                surface.clone(),
                MappedSurface {
                    target: target.clone(),
                    source,
                },
            );
        }
        for symbol in catalog {
            let trimmed = symbol.trim();
            if trimmed.len() < 2 {
                continue;
            }
            by_surface
                .entry(trimmed.to_ascii_lowercase())
                .or_insert_with(|| MappedSurface {
                    target: trimmed.to_string(),
                    source: SlotSource::Exact,
                });
        }
        for row in lexicon {
            if row.kind == "confusable" {
                continue;
            }
            let key = normalize_key(&row.surface_form);
            let target = row.normalized_target.trim();
            if key.is_empty() || target.is_empty() {
                continue;
            }
            by_surface.insert(
                key,
                MappedSurface {
                    target: target.to_string(),
                    source: SlotSource::Lexicon,
                },
            );
        }
        let confusable: HashMap<String, String> = ont
            .confusable_by_surface
            .iter()
            .filter(|(surface, _)| {
                let ch = ont
                    .confusable_channels
                    .get(*surface)
                    .cloned()
                    .unwrap_or_else(|| vec!["speech".into()]);
                allows_channel(&ch, channel)
            })
            .map(|(surface, target)| (surface.clone(), target.clone()))
            .collect();
        let alias_surfaces: Vec<(String, String)> = by_surface
            .iter()
            .filter(|(surface, _)| !surface.contains(' ') && surface.len() >= 3)
            .map(|(surface, mapped)| (surface.clone(), mapped.target.clone()))
            .collect();
        Self {
            by_surface,
            confusable,
            alias_surfaces,
        }
    }

    fn canonical_instrument(&self, token: &str) -> Option<String> {
        self.by_surface
            .get(&token.to_ascii_lowercase())
            .map(|row| row.target.clone())
    }

    fn exact(&self, surface: &str) -> Option<&MappedSurface> {
        self.by_surface.get(&normalize_key(surface))
    }

    fn resolve_slot(&self, surface: &str, fuzzy: bool) -> Option<SlotHit> {
        let key = normalize_key(surface);
        if key.is_empty() {
            return None;
        }
        if let Some(mapped) = self.by_surface.get(&key) {
            return Some(SlotHit::Canonical {
                surface: surface.to_string(),
                target: mapped.target.clone(),
                source: mapped.source.to_string(),
            });
        }
        if !fuzzy {
            return None;
        }
        if let Some(target) = self.confusable.get(&key) {
            return Some(SlotHit::Confusable {
                surface: surface.to_string(),
                target: target.clone(),
            });
        }
        if key.contains(' ') || key.len() < 3 {
            return None;
        }
        nearest_alias(&key, &self.alias_surfaces).map(|target| SlotHit::Canonical {
            surface: surface.to_string(),
            target,
            source: SlotSource::Alias.to_string(),
        })
    }
}

fn nearest_alias(token: &str, aliases: &[(String, String)]) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for (surface, target) in aliases {
        let dist = levenshtein(token, surface);
        if dist == 0 {
            return Some(target.clone());
        }
        if dist > MAX_EDIT_DISTANCE {
            continue;
        }
        match &best {
            None => best = Some((dist, target.clone())),
            Some((best_dist, best_target)) => {
                if dist < *best_dist {
                    best = Some((dist, target.clone()));
                } else if dist == *best_dist && best_target != target {
                    return None;
                }
            }
        }
    }
    best.map(|(_, target)| target)
}

fn tokenize(raw: &str) -> Vec<String> {
    let lower = raw.to_ascii_lowercase();
    let mut tokens = Vec::new();
    let mut cur = String::new();
    for ch in lower.chars() {
        if ch == '.' && !cur.is_empty() && cur.chars().all(|c| c.is_ascii_digit()) {
            cur.push('.');
        } else if ch.is_ascii_alphanumeric() {
            cur.push(ch);
        } else {
            flush_token(&mut cur, &mut tokens);
        }
    }
    flush_token(&mut cur, &mut tokens);
    tokens
}

fn flush_token(cur: &mut String, tokens: &mut Vec<String>) {
    if cur.is_empty() {
        return;
    }
    tokens.push(std::mem::take(cur));
}

fn is_number_token(token: &str) -> bool {
    parse_amount_token(token).is_some()
}

/// Parse "200", "0.02", "$50", "5k", "fifty" into a decimal string.
pub fn parse_amount_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches('$').replace(',', "");
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if let Some(word) = number_word(&lower) {
        return Some(word);
    }
    let (body, mult) = if let Some(stripped) = lower.strip_suffix('k') {
        (stripped, 1_000i64)
    } else if let Some(stripped) = lower.strip_suffix('m') {
        (stripped, 1_000_000i64)
    } else {
        (lower.as_str(), 1i64)
    };
    if body.is_empty() {
        return None;
    }
    if !body.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    if body == "." || body.chars().filter(|c| *c == '.').count() > 1 {
        return None;
    }
    if mult == 1 {
        return Some(body.trim_start_matches('0').to_string()).map(|s| {
            if s.is_empty() || s.starts_with('.') {
                format!("0{s}")
            } else {
                s
            }
        });
    }
    let n: f64 = body.parse().ok()?;
    Some(((n * mult as f64).round() as i64).to_string())
}

fn number_word(token: &str) -> Option<String> {
    let n = match token {
        "zero" | "oh" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        "hundred" => 100,
        _ => return None,
    };
    Some(n.to_string())
}

const MONEY_VERBS: &[&str] = &["put", "spend", "invest", "deploy"];
const CURRENCY_MARKERS: &[&str] = &["usd", "usdt", "dollar", "dollars", "worth", "bucks", "buck"];
const BUY_ACTS: &[&str] = &[
    "buy", "sell", "long", "short", "shorts", "longs", "purchase", "dump",
];

pub fn parse_size(sentence: &str, instrument: Option<&str>) -> SizeSpan {
    let tokens = tokenize(sentence);
    let mut span = classify_size_tokens(&tokens, instrument);
    if sentence.contains('$') && !matches!(span.kind, SizeKind::None) {
        span.kind = SizeKind::Quote;
    }
    span
}

pub fn classify_size_tokens(tokens: &[String], instrument: Option<&str>) -> SizeSpan {
    let Some(idx) = tokens.iter().position(|t| is_number_token(t)) else {
        return SizeSpan {
            kind: SizeKind::None,
            surface: String::new(),
            amount: String::new(),
        };
    };
    let mut amount = parse_amount_token(&tokens[idx]).unwrap_or_else(|| tokens[idx].clone());
    let mut surface = tokens[idx].clone();
    if tokens.get(idx + 1).map(String::as_str) == Some("hundred") {
        if let Ok(n) = amount.parse::<i64>() {
            amount = (n * 100).to_string();
            surface = format!("{} hundred", tokens[idx]);
        }
    }
    let window_lo = idx.saturating_sub(3);
    let window_hi = (idx + 4).min(tokens.len());
    let window = &tokens[window_lo..window_hi];
    let has_currency = window
        .iter()
        .any(|t| CURRENCY_MARKERS.iter().any(|m| m == t))
        || tokens.iter().any(|t| {
            CURRENCY_MARKERS.iter().any(|m| m == t) && {
                let pos = tokens.iter().position(|x| x == t).unwrap_or(0);
                pos.abs_diff(idx) <= 4
            }
        });
    let money_verb = tokens.iter().any(|t| MONEY_VERBS.iter().any(|v| v == t));
    let inst = instrument
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty());
    let next = tokens.get(idx + 1).map(String::as_str);
    let unit_is_asset = match (next, inst.as_deref()) {
        (Some(n), Some(i)) if n == i || n.eq_ignore_ascii_case(i) => true,
        (Some("eth" | "ether" | "weth" | "btc" | "wbtc" | "sol" | "usdt"), _) => true,
        (Some(n), _) if inst.as_deref() == Some(n) => true,
        _ => false,
    };
    let has_of_asset =
        tokens.get(idx + 1).map(String::as_str) == Some("of") && tokens.get(idx + 2).is_some();
    let kind = if has_currency {
        SizeKind::Quote
    } else if money_verb {
        SizeKind::Quote
    } else if unit_is_asset {
        SizeKind::Base
    } else if has_of_asset && !has_currency {
        SizeKind::Quote
    } else {
        SizeKind::Ambiguous
    };
    SizeSpan {
        kind,
        surface,
        amount,
    }
}

pub fn infer_side(sentence: &str) -> Option<String> {
    let tokens = tokenize(sentence);
    infer_side_tokens(&tokens)
}

pub fn infer_side_tokens(tokens: &[String]) -> Option<String> {
    for t in tokens {
        match t.as_str() {
            "short" | "shorts" | "shorting" => return Some("sell".to_string()),
            "long" | "longs" | "longing" => return Some("buy".to_string()),
            "sell" | "selling" | "dump" => return Some("sell".to_string()),
            "buy" | "buying" | "purchase" | "purchasing" => return Some("buy".to_string()),
            _ => {}
        }
    }
    None
}

pub fn infer_product(sentence: &str) -> Option<String> {
    let tokens = tokenize(sentence);
    if tokens.iter().any(|t| {
        matches!(
            t.as_str(),
            "perp" | "perpetual" | "perps" | "short" | "long"
        )
    }) {
        return Some("perp".to_string());
    }
    if tokens
        .iter()
        .any(|t| matches!(t.as_str(), "lend" | "lending" | "borrow"))
    {
        return Some("lend".to_string());
    }
    if tokens.iter().any(|t| matches!(t.as_str(), "spot")) {
        return Some("spot".to_string());
    }
    None
}

pub fn classify_protected_veto(sentence: &str) -> Option<ProtectedVeto> {
    let lower = sentence.to_ascii_lowercase();
    let protect = lower.contains("never sell")
        || lower.contains("don't ever sell")
        || lower.contains("dont ever sell")
        || lower.contains("do not ever sell")
        || lower.contains("protect my")
        || lower.contains("don't sell my")
        || lower.contains("dont sell my")
        || (lower.contains("never") && lower.contains("sell"));
    if !protect {
        return None;
    }
    let tokens = tokenize(sentence);
    let asset = tokens
        .iter()
        .rev()
        .find(|t| {
            !matches!(
                t.as_str(),
                "never"
                    | "sell"
                    | "ever"
                    | "dont"
                    | "don't"
                    | "do"
                    | "not"
                    | "my"
                    | "the"
                    | "protect"
                    | "please"
                    | "stack"
                    | "position"
                    | "holdings"
            ) && !is_number_token(t)
        })
        .cloned()
        .unwrap_or_else(|| "that".to_string());
    let absolute = lower.contains("never")
        || lower.contains("ever")
        || lower.contains("no matter what")
        || lower.contains("under any circumstance");
    Some(ProtectedVeto { asset, absolute })
}

const FOOD: &[&str] = &[
    "beef", "pork", "chicken", "steak", "pizza", "burger", "coffee", "milk", "eggs", "bread",
    "rice", "corn", "wheat", "soy", "soybeans",
];
const COMMODITY: &[&str] = &[
    "gold",
    "silver",
    "oil",
    "crude",
    "gas",
    "wheat",
    "corn",
    "copper",
    "platinum",
    "palladium",
];
const EQUITY: &[&str] = &[
    "tsla", "aapl", "nvda", "msft", "amzn", "goog", "meta", "spy", "qqq", "stock", "stocks",
    "share", "shares", "equity", "equities",
];
const FX: &[&str] = &[
    "euro", "euros", "yen", "gbp", "pound", "pounds", "franc", "cad", "aud", "fx", "forex",
];

pub fn instrument_category(noun: &str) -> Option<&'static str> {
    let key = noun.trim().trim_end_matches('s').to_ascii_lowercase();
    if FOOD.contains(&key.as_str()) || FOOD.iter().any(|w| *w == noun.trim().to_ascii_lowercase()) {
        return Some("food");
    }
    if COMMODITY.contains(&key.as_str())
        || COMMODITY
            .iter()
            .any(|w| *w == noun.trim().to_ascii_lowercase())
    {
        return Some("commodities");
    }
    if EQUITY.contains(&noun.trim().to_ascii_lowercase().as_str()) || key == "sp" {
        return Some("equities");
    }
    if FX.contains(&noun.trim().to_ascii_lowercase().as_str()) {
        return Some("fx");
    }
    None
}

/// Buy-verb + a concrete noun with no universe hit → CANT category. No noun → unclear.
pub fn unfulfillable_kind(sentence: &str, unknown: &[String]) -> Option<(&'static str, String)> {
    let tokens = tokenize(sentence);
    let has_act = tokens
        .iter()
        .any(|t| BUY_ACTS.iter().any(|a| a == t) || MONEY_VERBS.iter().any(|a| a == t));
    if !has_act {
        return None;
    }
    if let Some(noun) = unknown.iter().find(|n| !n.trim().is_empty()) {
        let category = instrument_category(noun).unwrap_or("that");
        return Some((category, noun.clone()));
    }
    for t in tokens.iter().rev() {
        if is_number_token(t)
            || CURRENCY_MARKERS.iter().any(|m| m == t)
            || BUY_ACTS.iter().any(|a| a == t)
            || MONEY_VERBS.iter().any(|a| a == t)
            || matches!(
                t.as_str(),
                "of" | "the" | "my" | "me" | "a" | "an" | "some" | "into" | "on" | "with"
            )
        {
            continue;
        }
        if let Some(cat) = instrument_category(t) {
            return Some((cat, t.clone()));
        }
        if t.len() >= 3 {
            return Some(("that", t.clone()));
        }
    }
    None
}

fn is_size_filler(token: &str, ont: &Ontology) -> bool {
    is_number_token(token)
        || ont.size_words.contains(token)
        || ont.units.contains(token)
        || token == "a"
        || token == "the"
        || token == "open"
}

/// "buy 5 eth" is often heard as "five five eight" or "58": buy→five, eth→eight,
/// then smart_format concatenates the digits. SOL does not sound like a number,
/// so "buy 5 SOL" survives. Collapse only when no other instrument is present.
fn repair_eth_heard_as_eight(tokens: &mut Vec<String>, channel: Channel) -> Option<UtteranceSlot> {
    if channel != Channel::Speech || tokens.is_empty() {
        return None;
    }
    if has_money_frame(tokens) || has_named_instrument(tokens) {
        return None;
    }
    let act = tokens.first().filter(|t| is_trade_act(t)).cloned();
    if tokens.first().is_some_and(|t| is_question_opener(t)) {
        return None;
    }
    if !eth_collapsed_as_eight(tokens) {
        return None;
    }
    let surface = tokens.join(" ");
    let verb = act.unwrap_or_else(|| "buy".to_string());
    tokens.clear();
    tokens.push(verb);
    tokens.push("5".to_string());
    tokens.push("eth".to_string());
    Some(UtteranceSlot {
        kind: "instrument".to_string(),
        surface,
        target: "ETH".to_string(),
        source: "eth_eight_rule".to_string(),
    })
}

fn is_trade_act(token: &str) -> bool {
    matches!(
        token,
        "buy" | "sell" | "long" | "short" | "lend" | "borrow" | "close" | "unwind"
    )
}

fn is_question_opener(token: &str) -> bool {
    matches!(
        token,
        "what" | "whats" | "why" | "how" | "who" | "when" | "where" | "can" | "could" | "should"
    )
}

fn is_five_token(token: &str) -> bool {
    token == "5" || token == "five"
}

fn is_eight_token(token: &str) -> bool {
    matches!(token, "8" | "eight" | "ate")
}

fn is_five_eight_num(token: &str) -> bool {
    token == "58"
}

fn eth_collapsed_as_eight(tokens: &[String]) -> bool {
    let rest: &[String] = if tokens.first().is_some_and(|t| is_trade_act(t)) {
        &tokens[1..]
    } else {
        tokens
    };
    match rest {
        [n] if is_five_eight_num(n) => true,
        [a, b] if is_five_token(a) && is_eight_token(b) => true,
        [a, b, c] if is_five_token(a) && is_five_token(b) && is_eight_token(c) => true,
        _ => false,
    }
}

fn has_money_frame(tokens: &[String]) -> bool {
    tokens.iter().any(|t| {
        matches!(
            t.as_str(),
            "dollars" | "bucks" | "worth" | "percent" | "notional"
        )
    })
}

fn has_named_instrument(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(kind_for(token).as_deref(), Some("instrument"))
            || matches!(
                token.as_str(),
                "eth"
                    | "weth"
                    | "ether"
                    | "ethereum"
                    | "btc"
                    | "wbtc"
                    | "bitcoin"
                    | "sol"
                    | "solana"
                    | "usdc"
                    | "usdt"
            )
    })
}

/// Speech often drops the first word. "buy 5 ETH" lands as "5.05 ETH" because
/// "buy" and "five" share a diphthong and smart_format fuses the two numbers.
/// Users almost never start with a numeral; restore `buy` when they did.
fn restore_speech_opener(tokens: &mut Vec<String>, channel: Channel) -> Option<UtteranceSlot> {
    if channel != Channel::Speech || tokens.is_empty() {
        return None;
    }
    if tokens.iter().any(|t| is_utterance_opener(t)) {
        return None;
    }
    if !looks_like_sized_instrument(tokens) {
        return None;
    }
    let qty = leading_command_qty(&tokens[0])?;
    let surface = tokens[0].clone();
    tokens[0] = qty;
    tokens.insert(0, "buy".to_string());
    Some(UtteranceSlot {
        kind: "act".to_string(),
        surface,
        target: "buy".to_string(),
        source: "opener_rule".to_string(),
    })
}

fn leading_command_qty(token: &str) -> Option<String> {
    if !is_number_token(token) {
        return None;
    }
    if let Some((whole, frac)) = token.split_once('.') {
        if !whole.is_empty()
            && whole.chars().all(|c| c.is_ascii_digit())
            && !frac.is_empty()
            && frac.chars().all(|c| c.is_ascii_digit())
        {
            if frac.chars().all(|c| c == '0') {
                return Some(whole.to_string());
            }
            if frac.starts_with('0') && frac.trim_start_matches('0') == whole {
                return Some(whole.to_string());
            }
        }
    }
    Some(token.to_string())
}

fn is_utterance_opener(token: &str) -> bool {
    match kind_for(token).as_deref() {
        Some("act" | "opener") => true,
        _ => matches!(
            token,
            "what"
                | "whats"
                | "why"
                | "how"
                | "who"
                | "when"
                | "where"
                | "can"
                | "could"
                | "should"
                | "show"
                | "list"
                | "tell"
                | "buy"
                | "sell"
                | "unwind"
                | "leverage"
                | "long"
                | "short"
                | "lend"
                | "borrow"
                | "close"
                | "cancel"
                | "watch"
                | "pause"
                | "resume"
                | "open"
        ),
    }
}

fn looks_like_sized_instrument(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            kind_for(token).as_deref(),
            Some("instrument") | Some("size_frame")
        ) || matches!(
            token.as_str(),
            "eth"
                | "weth"
                | "ether"
                | "ethereum"
                | "btc"
                | "wbtc"
                | "bitcoin"
                | "sol"
                | "solana"
                | "usdc"
                | "usdt"
                | "dollars"
                | "bucks"
                | "worth"
        )
    })
}

fn apply_repairs(tokens: &mut Vec<String>, channel: Channel) -> Option<UtteranceSlot> {
    let ont = ontology();
    for repair in &ont.repairs {
        if repair.kind != "size_mishear" {
            continue;
        }
        if !allows_channel(&repair.channels, channel) {
            continue;
        }
        if !repair.require_acts.is_empty()
            && !tokens
                .iter()
                .any(|t| repair.require_acts.iter().any(|a| a == t))
        {
            continue;
        }
        if !repair.require_span.is_empty() {
            let span = &repair.require_span;
            let hit = tokens
                .windows(span.len())
                .any(|w| w.iter().zip(span.iter()).all(|(tok, want)| tok == want));
            if !hit {
                continue;
            }
        }
        let Some(idx) = tokens.iter().position(|t| t == &repair.surface) else {
            continue;
        };
        if !repair.require_acts.is_empty()
            && !tokens[..idx]
                .iter()
                .any(|t| repair.require_acts.iter().any(|a| a == t))
        {
            continue;
        }
        tokens[idx] = repair.target.clone();
        if let Some(insert) = &repair.insert_before_worth {
            let after = idx + 1;
            if after < tokens.len() && tokens[after] == "worth" {
                tokens.insert(after, insert.clone());
            }
        }
        return Some(UtteranceSlot {
            kind: "size".to_string(),
            surface: repair.surface.clone(),
            target: repair.target.clone(),
            source: "size_rule".to_string(),
        });
    }
    None
}

struct Slot {
    index: usize,
    fuzzy: bool,
}

fn instrument_slots(tokens: &[String]) -> Vec<Slot> {
    let ont = ontology();
    let mut slots = Vec::new();
    let mut marked = HashSet::new();

    for frame in ont
        .frames
        .iter()
        .filter(|frame| frame.role == "instrument_slot")
    {
        if frame.tokens.is_empty() {
            continue;
        }
        let mut i = 0;
        while i + frame.tokens.len() <= tokens.len() {
            if tokens[i..i + frame.tokens.len()] == frame.tokens[..] {
                let index = i + frame.tokens.len();
                if index < tokens.len() && marked.insert(index) {
                    slots.push(Slot {
                        index,
                        fuzzy: frame.fuzzy,
                    });
                }
                i += frame.tokens.len();
            } else {
                i += 1;
            }
        }
    }

    for frame in ont
        .frames
        .iter()
        .filter(|frame| frame.role == "act_instrument")
    {
        let mut i = 0;
        while i < tokens.len() {
            if frame.acts.iter().any(|act| act == &tokens[i]) {
                let fuzzy = frame.fuzzy_acts.iter().any(|act| act == &tokens[i]);
                let mut j = i + 1;
                while j < tokens.len() && is_size_filler(&tokens[j], ont) {
                    j += 1;
                }
                if j < tokens.len() && tokens[j] != "worth" && tokens[j] != "of" && marked.insert(j)
                {
                    slots.push(Slot { index: j, fuzzy });
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
    }
    slots
}

fn resolve_instrument(
    tokens: &[String],
    start: usize,
    fuzzy: bool,
    _channel: Channel,
    universe: &InstrumentUniverse,
) -> Option<(usize, SlotHit)> {
    if start >= tokens.len() {
        return None;
    }
    if start + 1 < tokens.len() {
        let phrase = format!("{} {}", tokens[start], tokens[start + 1]);
        if let Some(hit) = universe.resolve_slot(&phrase, fuzzy) {
            return Some((2, hit));
        }
    }
    universe
        .resolve_slot(&tokens[start], fuzzy)
        .map(|hit| (1, hit))
}

fn parse_grammar(
    raw: &str,
    tokens: &[String],
    universe: &InstrumentUniverse,
) -> (GrammarStatus, Option<ActionIr>) {
    if raw.trim().is_empty() || tokens.is_empty() {
        return (GrammarStatus::None, None);
    }
    if is_question(raw) || is_lookup_tokens(tokens) {
        return (GrammarStatus::None, None);
    }
    let product = tokens.iter().find(|t| is_product_token(t)).cloned();
    let ont = ontology();

    for frame in ont.frames.iter().filter(|frame| frame.role == "grammar") {
        if !frame.referents.is_empty() {
            if let Some(ir) = match_referent_frame(tokens, frame, product.clone()) {
                return (GrammarStatus::Matched, Some(ir));
            }
            continue;
        }
        if frame.level.as_deref() == Some("required") {
            if let Some(hit) = match_level_frame(tokens, frame, universe, product.clone()) {
                return hit;
            }
            continue;
        }
        if let Some(hit) = match_trade_frame(tokens, frame, universe, product.clone()) {
            return hit;
        }
    }
    (GrammarStatus::None, None)
}

fn match_referent_frame(
    tokens: &[String],
    frame: &OntologyFrame,
    product: Option<String>,
) -> Option<ActionIr> {
    let act_idx = tokens
        .iter()
        .position(|t| frame.acts.iter().any(|act| act == t))?;
    let rest = &tokens[act_idx + 1..];
    if !rest
        .iter()
        .any(|t| frame.referents.iter().any(|row| row == t))
    {
        return None;
    }
    let referent = rest.iter().find(|t| *t == "these").cloned().or_else(|| {
        rest.iter()
            .find(|t| frame.referents.iter().any(|row| row == *t))
            .cloned()
    });
    Some(ActionIr {
        act: tokens[act_idx].clone(),
        instrument: None,
        size: None,
        product,
        referent,
        frame_id: Some(frame.id.clone()),
        order_type: None,
        size_kind: None,
    })
}

fn match_level_frame(
    tokens: &[String],
    frame: &OntologyFrame,
    universe: &InstrumentUniverse,
    product: Option<String>,
) -> Option<(GrammarStatus, Option<ActionIr>)> {
    let act_idx = tokens
        .iter()
        .position(|t| frame.acts.iter().any(|act| act == t))?;
    let act = tokens[act_idx].clone();
    let instrument = tokens.iter().find_map(|t| universe.canonical_instrument(t));
    let level = tokens.iter().rev().find(|t| is_number_token(t)).cloned();
    if instrument.is_none() {
        return Some((GrammarStatus::None, None));
    }
    if level.is_some() {
        return Some((
            GrammarStatus::Matched,
            Some(ActionIr {
                act,
                instrument,
                size: level,
                product,
                referent: None,
                frame_id: Some(frame.id.clone()),
                order_type: None,
                size_kind: Some("base".to_string()),
            }),
        ));
    }
    Some((
        GrammarStatus::Partial,
        Some(ActionIr {
            act,
            instrument,
            size: None,
            product,
            referent: None,
            frame_id: Some(frame.id.clone()),
            order_type: None,
            size_kind: None,
        }),
    ))
}

fn match_trade_frame(
    tokens: &[String],
    frame: &OntologyFrame,
    universe: &InstrumentUniverse,
    product: Option<String>,
) -> Option<(GrammarStatus, Option<ActionIr>)> {
    let (act, _) = find_frame_act(tokens, frame)?;
    let instrument = tokens.iter().find_map(|t| universe.canonical_instrument(t));
    let size = find_size_token(tokens);
    let order_type = find_order_type(tokens);
    let size_kind = Some(
        classify_size_tokens(tokens, instrument.as_deref())
            .kind
            .as_str()
            .to_string(),
    );
    if instrument.is_some() {
        if frame.size.as_deref() == Some("required") && size.is_none() {
            return Some((
                GrammarStatus::Partial,
                Some(ActionIr {
                    act,
                    instrument,
                    size: None,
                    product,
                    referent: None,
                    frame_id: Some(frame.id.clone()),
                    order_type,
                    size_kind,
                }),
            ));
        }
        return Some((
            GrammarStatus::Matched,
            Some(ActionIr {
                act,
                instrument,
                size,
                product,
                referent: None,
                frame_id: Some(frame.id.clone()),
                order_type,
                size_kind,
            }),
        ));
    }
    Some((
        GrammarStatus::Partial,
        Some(ActionIr {
            act,
            instrument: None,
            size,
            product,
            referent: None,
            frame_id: Some(frame.id.clone()),
            order_type,
            size_kind,
        }),
    ))
}

fn find_frame_act(tokens: &[String], frame: &OntologyFrame) -> Option<(String, usize)> {
    for i in 0..tokens.len() {
        if frame.open_prefix
            && tokens[i] == "open"
            && i + 1 < tokens.len()
            && frame.acts.iter().any(|act| act == &tokens[i + 1])
        {
            return Some((tokens[i + 1].clone(), i));
        }
        if frame.acts.iter().any(|act| act == &tokens[i]) {
            return Some((tokens[i].clone(), i));
        }
    }
    None
}

fn find_size_token(tokens: &[String]) -> Option<String> {
    let ont = ontology();
    tokens
        .iter()
        .find(|t| ont.size_words.contains(*t) || is_number_token(t))
        .cloned()
}

fn find_order_type(tokens: &[String]) -> Option<String> {
    let ont = ontology();
    let max = tokens.len().min(4);
    for width in (1..=max).rev() {
        for window in tokens.windows(width) {
            let key = window.join(" ");
            if ont.kind_by_surface.get(&key).map(String::as_str) != Some("order_type") {
                continue;
            }
            if let Some(entry) = ont
                .entries
                .iter()
                .find(|row| row.kind == "order_type" && normalize_key(&row.surface_form) == key)
            {
                return Some(normalize_key(&entry.normalized_target));
            }
        }
    }
    None
}

fn is_product_token(token: &str) -> bool {
    ontology().kind_by_surface.get(token).map(String::as_str) == Some("product")
}

fn is_question(raw: &str) -> bool {
    let t = raw.trim().to_ascii_lowercase();
    if t.ends_with('?') {
        return true;
    }
    let first = t
        .split(|c: char| !c.is_ascii_alphabetic())
        .find(|part| !part.is_empty())
        .unwrap_or("");
    matches!(first, "what" | "why" | "how" | "who" | "when" | "where")
        || t.starts_with("walk me")
        || t.starts_with("tell me")
}

fn is_lookup_tokens(tokens: &[String]) -> bool {
    if tokens.len() != 1 {
        return false;
    }
    matches!(
        tokens[0].as_str(),
        "b" | "p"
            | "r"
            | "a"
            | "d"
            | "balance"
            | "positions"
            | "risk"
            | "available"
            | "dollarpower"
            | "commands"
            | "shortcuts"
    )
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repair(raw: &str) -> Repair {
        repair_transcript(raw, &[])
    }

    fn text(raw: &str) -> String {
        repair(raw).text
    }

    fn norm(raw: &str, channel: Channel) -> NormalizedUtterance {
        normalize_utterance(raw, channel, &[], &[])
    }

    #[test]
    fn ontology_version_is_three() {
        assert_eq!(ONTOLOGY_VERSION, 3);
        assert!(!boostable_keyterms().is_empty());
        assert!(!ontology().frames.is_empty());
        assert!(!ontology().repairs.is_empty());
    }

    #[test]
    fn beef_in_worth_of_frame_proposes_eth_not_rewrite() {
        let out = repair("buy fifty dollars worth of beef");
        assert_eq!(out.text, "buy fifty dollars worth of beef");
        assert!(out.repaired_from.is_none());
        assert!(out.hits.is_empty());
        assert_eq!(out.proposed_confusables.len(), 1);
        assert_eq!(out.proposed_confusables[0].surface, "beef");
        assert_eq!(out.proposed_confusables[0].target, "WETH");
    }

    #[test]
    fn these_in_worth_of_frame_proposes_eth_not_rewrite() {
        let out = repair("buy fifty dollars worth of these");
        assert_eq!(out.text, "buy fifty dollars worth of these");
        assert_eq!(out.proposed_confusables[0].target, "WETH");
        assert!(out.hits.is_empty());
    }

    #[test]
    fn eth_in_worth_of_frame_rewrites_to_weth() {
        let out = repair("buy fifty dollars worth of ETH");
        assert_eq!(out.text, "buy fifty dollars worth of WETH");
        assert!(out.hits.iter().any(|h| h.normalized_target == "WETH"));
        assert_eq!(out.hits.len(), 1);
    }

    #[test]
    fn cancel_these_watches_is_not_rewritten() {
        assert_eq!(text("cancel these watches"), "cancel these watches");
        let out = repair("cancel these watches");
        assert!(out.hits.is_empty());
        assert!(
            out.proposed_confusables.is_empty(),
            "slot-gate: these must not propose ETH"
        );
        let watch = repair("watch these");
        assert!(watch.proposed_confusables.is_empty());
    }

    #[test]
    fn dollar_550_in_worth_of_frame_becomes_fifty() {
        assert_eq!(
            text("buy $550 worth of ETH"),
            "buy fifty dollars worth of WETH"
        );
        assert_eq!(
            text("buy 550 dollars worth of ETH"),
            "buy fifty dollars worth of WETH"
        );
    }

    #[test]
    fn fifteen_is_not_rewritten() {
        assert_eq!(
            text("buy 15 dollars worth of ETH"),
            "buy 15 dollars worth of WETH"
        );
    }

    #[test]
    fn empty_portfolio_keyterms_include_core_tokens() {
        let terms = seed_keyterms(&[], &[]);
        let lower: Vec<String> = terms.iter().map(|t| t.to_ascii_lowercase()).collect();
        assert!(lower.iter().any(|t| t == "eth"));
        assert!(lower.iter().any(|t| t == "buy"));
        assert!(lower.iter().any(|t| t == "worth"));
        assert!(lower.iter().any(|t| t == "twap"));
        assert!(lower.iter().any(|t| t == "dca"));
        assert!(lower.iter().any(|t| t == "unwind"));
        assert!(lower.iter().any(|t| t == "leverage up"));
        assert!(lower.iter().any(|t| t == "how much"));
        assert!(!lower.iter().any(|t| t == "beef" || t == "these"));
        let buy = lower.iter().position(|t| t == "buy").expect("buy");
        let eth = lower.iter().position(|t| t == "eth").expect("eth");
        assert!(
            buy < eth,
            "command openers must seed before instruments: {lower:?}"
        );
        assert!(terms.len() <= EXTRA_KEYTERM_BUDGET);
    }

    #[test]
    fn speech_restores_buy_when_five_fuses_to_decimal() {
        assert_eq!(text("5.05 ETH"), "buy 5 WETH");
        assert_eq!(text("5.05 ether"), "buy 5 WETH");
        assert_eq!(text("5 ETH"), "buy 5 WETH");
        assert_eq!(text("5.0 sol"), "buy 5 SOL");
        assert_eq!(text("buy 5.05 ETH"), "buy 5.05 WETH");
        assert_eq!(text("sell 5 ETH"), "sell 5 WETH");
        assert_eq!(text("how much is 5 ETH"), "how much is 5 WETH");
        let typed = normalize_utterance("5.05 ETH", Channel::Text, &[], &[]);
        assert_eq!(typed.normalized_text, "5.05 WETH");
    }

    #[test]
    fn speech_collapses_five_five_eight_to_buy_5_eth() {
        assert_eq!(text("five five eight"), "buy 5 WETH");
        assert_eq!(text("58"), "buy 5 WETH");
        assert_eq!(text("5 8"), "buy 5 WETH");
        assert_eq!(text("five eight"), "buy 5 WETH");
        assert_eq!(text("buy 5 eight"), "buy 5 WETH");
        assert_eq!(text("buy 58"), "buy 5 WETH");
        assert_eq!(text("sell 5 eight"), "sell 5 WETH");
        assert_eq!(text("buy 58 SOL"), "buy 58 SOL");
        assert_eq!(text("buy 5 SOL"), "buy 5 SOL");
        let typed = normalize_utterance("58", Channel::Text, &[], &[]);
        assert_eq!(typed.normalized_text, "58");
    }

    #[test]
    fn catalog_symbols_join_the_universe() {
        let out = repair_transcript("buy fifty dollars worth of xyz", &["XYZ".to_string()]);
        assert_eq!(out.text, "buy fifty dollars worth of XYZ");
    }

    #[test]
    fn eath_near_miss_proposes_eth() {
        let out = repair("buy fifty dollars worth of eath");
        assert_eq!(out.text, "buy fifty dollars worth of eath");
        assert_eq!(out.proposed_confusables[0].target, "WETH");
    }

    #[test]
    fn buy_it_proposes_eth_cancel_it_does_not() {
        let buy = repair("buy it");
        assert_eq!(buy.text, "buy it");
        assert_eq!(buy.proposed_confusables[0].target, "WETH");
        assert_eq!(text("cancel it"), "cancel it");
        assert_eq!(text("watch these"), "watch these");
        let sell = repair("sell these");
        assert_eq!(sell.text, "sell these");
        assert_eq!(sell.proposed_confusables[0].target, "WETH");
    }

    #[test]
    fn spoken_eth_names_map_to_weth() {
        assert_eq!(
            text("buy fifty dollars worth of ether"),
            "buy fifty dollars worth of WETH"
        );
        assert_eq!(
            text("buy fifty dollars worth of ethereum"),
            "buy fifty dollars worth of WETH"
        );
        assert_eq!(
            text("buy fifty dollars worth of ETH"),
            "buy fifty dollars worth of WETH"
        );
        assert_eq!(
            text("buy fifty dollars worth of wrapped ether"),
            "buy fifty dollars worth of WETH"
        );
        assert_eq!(
            text("buy fifty dollars worth of wrapped ethereum"),
            "buy fifty dollars worth of WETH"
        );
    }

    #[test]
    fn confusable_entries_are_speech_channel() {
        let ont = ontology();
        for entry in &ont.entries {
            if entry.kind == "confusable" {
                assert_eq!(entry.channels, vec!["speech".to_string()]);
            } else {
                assert!(entry.channels.is_empty());
            }
        }
    }

    #[test]
    fn intensifier_ranks_instruments_above_acts() {
        assert_eq!(intensifier_for("ETH"), 5);
        assert_eq!(intensifier_for("buy"), 3);
        assert_eq!(intensifier_for("worth"), 3);
        assert_eq!(intensifier_for("the loop"), 2);
    }

    #[test]
    fn text_channel_does_not_propose_speech_confusables() {
        let out = norm("buy fifty dollars worth of beef", Channel::Text);
        assert_eq!(out.normalized_text, "buy fifty dollars worth of beef");
        assert!(out.proposals.is_empty());
        assert_eq!(out.channel, Channel::Text);
        assert!(
            out.unknown_instruments
                .iter()
                .any(|row| row.eq_ignore_ascii_case("beef")),
            "out-of-universe 'beef' must land as unknown, not unmatched-silent: {:?}",
            out.unknown_instruments
        );
    }

    #[test]
    fn buy_me_50_of_beef_is_unknown_instrument_not_canonical() {
        let out = norm("buy me $50 of beef", Channel::Text);
        assert!(
            out.action_ir
                .as_ref()
                .is_none_or(|ir| ir.instrument.is_none()),
            "beef must not resolve to a universe instrument: {:?}",
            out.action_ir
        );
        assert!(out.proposals.is_empty());
        assert!(!out.normalized_text.to_ascii_lowercase().contains("eth"));
    }

    #[test]
    fn text_ether_rewrites_to_weth() {
        let out = norm("buy fifty dollars worth of ether", Channel::Text);
        assert_eq!(out.normalized_text, "buy fifty dollars worth of WETH");
        assert!(out.proposals.is_empty());
        assert!(
            out.slots
                .iter()
                .any(|s| s.kind == "instrument" && s.target == "WETH" && s.source == "alias")
        );
    }

    #[test]
    fn text_channel_skips_speech_size_mishear() {
        let out = norm("buy 550 dollars worth of ETH", Channel::Text);
        assert_eq!(out.normalized_text, "buy 550 dollars worth of WETH");
        assert!(!out.slots.iter().any(|s| s.source == "size_rule"));
        assert_eq!(out.grammar, GrammarStatus::Matched);
        assert_eq!(out.action_ir.unwrap().size.as_deref(), Some("550"));
    }

    #[test]
    fn lexicon_rewrite_on_both_channels() {
        let lex = [LexiconEntry {
            surface_form: "the loop".to_string(),
            normalized_target: "WETH".to_string(),
            kind: "instrument".to_string(),
        }];
        let out = normalize_utterance(
            "buy fifty dollars worth of the loop",
            Channel::Text,
            &[],
            &lex,
        );
        assert_eq!(out.normalized_text, "buy fifty dollars worth of WETH");
        assert!(
            out.slots
                .iter()
                .any(|s| s.source == "lexicon" && s.target == "WETH")
        );
    }

    #[test]
    fn grammar_matched_partial_none() {
        let matched = norm("buy fifty dollars worth of ETH", Channel::Speech);
        assert_eq!(matched.grammar, GrammarStatus::Matched);
        let ir = matched.action_ir.expect("matched IR");
        assert_eq!(ir.act, "buy");
        assert_eq!(ir.instrument.as_deref(), Some("WETH"));
        assert_eq!(ir.size.as_deref(), Some("fifty"));
        assert_eq!(ir.frame_id.as_deref(), Some("buy_sell"));

        let twap = norm("buy fifty ETH twap", Channel::Text);
        assert_eq!(twap.action_ir.unwrap().order_type.as_deref(), Some("twap"));
        let over_time = norm("buy fifty ETH over time", Channel::Text);
        assert_eq!(
            over_time.action_ir.unwrap().order_type.as_deref(),
            Some("twap")
        );
        let dca = norm("dca buy fifty ETH", Channel::Text);
        assert_eq!(dca.action_ir.unwrap().order_type.as_deref(), Some("dca"));
        let dca_phrase = norm("dollar cost average buy fifty ETH", Channel::Text);
        assert_eq!(
            dca_phrase.action_ir.unwrap().order_type.as_deref(),
            Some("dca")
        );

        let partial = norm("buy fifty", Channel::Text);
        assert_eq!(partial.grammar, GrammarStatus::Partial);
        assert!(partial.action_ir.unwrap().instrument.is_none());

        let none = norm("what is ETH doing", Channel::Text);
        assert_eq!(none.grammar, GrammarStatus::None);
        assert!(none.action_ir.is_none());

        let lookup = norm("positions", Channel::Text);
        assert_eq!(lookup.grammar, GrammarStatus::None);

        let cancel = norm("cancel these watches", Channel::Speech);
        assert_eq!(cancel.grammar, GrammarStatus::Matched);
        assert_eq!(cancel.action_ir.unwrap().referent.as_deref(), Some("these"));
        assert!(cancel.proposals.is_empty());

        let close = norm("close ETH", Channel::Text);
        assert_eq!(close.grammar, GrammarStatus::Matched);
        assert_eq!(close.action_ir.unwrap().act, "close");
    }

    #[test]
    fn size_rule_slot_uses_size_rule_source() {
        let out = norm("buy $550 worth of ETH", Channel::Speech);
        assert_eq!(out.normalized_text, "buy fifty dollars worth of WETH");
        assert!(
            out.slots
                .iter()
                .any(|s| s.kind == "size" && s.source == "size_rule" && s.target == "fifty")
        );
    }

    #[test]
    fn json_channels_gate_confusables_not_hardcoded_speech_enum() {
        let speech = norm("buy fifty dollars worth of beef", Channel::Speech);
        assert_eq!(speech.proposals.len(), 1);
        let text = norm("buy fifty dollars worth of beef", Channel::Text);
        assert!(text.proposals.is_empty());
    }

    #[test]
    fn parse_size_quote_vs_base() {
        assert_eq!(
            parse_size("buy $200 of ETH", Some("ETH")).kind,
            SizeKind::Quote
        );
        assert_eq!(
            parse_size("put 300 into ether", Some("ETH")).kind,
            SizeKind::Quote
        );
        assert_eq!(
            parse_size("spend $200 on WETH", Some("WETH")).kind,
            SizeKind::Quote
        );
        assert_eq!(
            parse_size("buy 200 dollars worth of WETH", Some("WETH")).kind,
            SizeKind::Quote
        );
        assert_eq!(
            parse_size("buy 0.02 WETH", Some("WETH")).kind,
            SizeKind::Base
        );
        assert_eq!(
            parse_size("buy 200 WETH", Some("WETH")).kind,
            SizeKind::Base
        );
        assert_eq!(parse_size("buy 200", None).kind, SizeKind::Ambiguous);
        assert_eq!(parse_amount_token("5k").as_deref(), Some("5000"));
        assert_eq!(parse_amount_token("0.02").as_deref(), Some("0.02"));
    }

    #[test]
    fn short_verb_infers_sell_side() {
        assert_eq!(infer_side("short $5k of WBTC").as_deref(), Some("sell"));
        assert_eq!(infer_side("long another ETH perp").as_deref(), Some("buy"));
    }

    #[test]
    fn protected_veto_and_category() {
        let veto = classify_protected_veto("don't ever sell my SOL").unwrap();
        assert_eq!(veto.asset, "sol");
        assert!(veto.absolute);
        assert_eq!(instrument_category("beef"), Some("food"));
        assert_eq!(instrument_category("gold"), Some("commodities"));
        assert_eq!(instrument_category("TSLA"), Some("equities"));
        let (cat, noun) = unfulfillable_kind("buy me $50 of beef", &["beef".into()]).unwrap();
        assert_eq!(cat, "food");
        assert_eq!(noun, "beef");
        let (cat, noun) = unfulfillable_kind("buy me $50 of beef", &[]).unwrap();
        assert_eq!(cat, "food");
        assert_eq!(noun, "beef");
        assert!(unfulfillable_kind("buy $50", &[]).is_none());
        assert!(unfulfillable_kind("my favourite colour is teal", &[]).is_none());
    }
}
