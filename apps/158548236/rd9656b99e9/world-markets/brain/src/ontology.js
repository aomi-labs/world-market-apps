/**
 * World Markets speech ontology. Same JSON the plugin compiles in.
 * Used to seed STT keyterms globally (not persisted per account).
 */

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ONTOLOGY_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  "../../assets/speech_ontology.json",
);

const file = JSON.parse(readFileSync(ONTOLOGY_PATH, "utf8"));

export const ONTOLOGY_VERSION = file.version;

export function ontologyEntries() {
  return file.entries || [];
}

/** Omitted or empty channels means both speech and text. */
export function channelsOf(entry) {
  const raw = Array.isArray(entry?.channels) ? entry.channels : [];
  const out = [];
  const seen = new Set();
  for (const row of raw) {
    const key = String(row || "")
      .trim()
      .toLowerCase();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    out.push(key);
  }
  if (!out.length) return ["speech", "text"];
  out.sort();
  return out;
}

/** Hash of sorted entries plus frames and repairs. Snapshot when any of these change. */
export function ontologyFingerprint() {
  const rows = ontologyEntries()
    .map((entry) =>
      [
        normalizeKey(entry.surface_form),
        String(entry.kind || ""),
        String(entry.normalized_target || ""),
        channelsOf(entry).join(","),
      ].join("\0"),
    )
    .sort();
  const frames = JSON.stringify(file.frames || []);
  const repairs = JSON.stringify(file.repairs || []);
  return createHash("sha256")
    .update(`${rows.join("\n")}\n${frames}\n${repairs}`)
    .digest("hex");
}

export function ontologyFrames() {
  return file.frames || [];
}

export function entryCounts() {
  const counts_by_kind = {};
  let channels_speech = 0;
  let channels_text = 0;
  let channels_speech_only = 0;
  let channels_text_only = 0;
  let channels_both = 0;
  const entries = ontologyEntries();
  for (const entry of entries) {
    const kind = entry.kind || "unknown";
    counts_by_kind[kind] = (counts_by_kind[kind] || 0) + 1;
    const channels = channelsOf(entry);
    const speech = channels.includes("speech");
    const text = channels.includes("text");
    if (speech) channels_speech += 1;
    if (text) channels_text += 1;
    if (speech && text) channels_both += 1;
    else if (speech) channels_speech_only += 1;
    else if (text) channels_text_only += 1;
  }
  return {
    counts_by_kind,
    entry_count: entries.length,
    channels_speech,
    channels_text,
    channels_speech_only,
    channels_text_only,
    channels_both,
  };
}

function normalizeKey(surface) {
  return String(surface || "")
    .trim()
    .toLowerCase()
    .replace(/\s+/g, " ");
}

const byKind = new Map();
const kindBySurface = new Map();
const instrumentBySurface = new Map();
const confusableBySurface = new Map();
for (const row of ontologyEntries()) {
  const key = normalizeKey(row.surface_form);
  if (!key) continue;
  kindBySurface.set(key, row.kind);
  if (!byKind.has(row.kind)) byKind.set(row.kind, []);
  byKind.get(row.kind).push(row);
  if (row.kind === "instrument") {
    instrumentBySurface.set(key, row.normalized_target);
  }
  if (row.kind === "confusable") {
    confusableBySurface.set(key, row.normalized_target);
  }
}

export function kindOf(surface) {
  return kindBySurface.get(normalizeKey(surface)) || null;
}

export function instrumentAlias(surface) {
  return instrumentBySurface.get(normalizeKey(surface)) || null;
}

export function confusableTarget(surface) {
  return confusableBySurface.get(normalizeKey(surface)) || null;
}

export function surfacesOfKind(kind) {
  return (byKind.get(kind) || []).map((row) => row.surface_form);
}

export function actSet() {
  return new Set((byKind.get("act") || []).map((row) => normalizeKey(row.surface_form)));
}

export function fillerSet() {
  const out = new Set(["a", "an", "the", "me", "my", "of", "and", "then", "to", "for", "open"]);
  for (const kind of ["size", "unit", "size_frame"]) {
    for (const row of byKind.get(kind) || []) {
      out.add(normalizeKey(row.surface_form));
    }
  }
  return out;
}

export function kindRank(kind) {
  const rank = {
    opener: 0,
    act: 0,
    size_frame: 1,
    instrument: 2,
    order_type: 3,
    size: 4,
    unit: 4,
    product: 5,
    level: 6,
    phrase: 7,
  };
  return rank[kind] ?? 9;
}

function isBoostToken(term, kind) {
  const trimmed = String(term || "").trim();
  if (trimmed.length < 2 || trimmed.toLowerCase() === "if") return false;
  const words = trimmed.split(/\s+/).filter(Boolean).length;
  if (words === 0 || words > 3) return false;
  if (words > 1) {
    return kind === "act" || kind === "opener" || kind === "size_frame";
  }
  return true;
}

/** Command and question openers first, then instruments. Phrases allowed. */
export function ontologyKeyterms() {
  const ranked = [...ontologyEntries()]
    .filter((row) => row.kind !== "confusable" && channelsOf(row).includes("speech"))
    .sort((a, b) => {
      const d = kindRank(a.kind) - kindRank(b.kind);
      if (d !== 0) return d;
      return (b.confidence || 0) - (a.confidence || 0);
    });
  const seen = new Set();
  const out = [];
  for (const row of ranked) {
    const term = String(row.surface_form || "").trim();
    if (!isBoostToken(term, row.kind)) continue;
    const key = term.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(term);
  }
  return out;
}
