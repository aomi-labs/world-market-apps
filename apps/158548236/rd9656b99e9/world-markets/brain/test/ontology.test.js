import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "aomi-ontology-"));
process.env.WORLD_BRAIN_DIR = dir;

const {
  channelsOf,
  entryCounts,
  instrumentAlias,
  ontologyEntries,
  ontologyFingerprint,
  ontologyFrames,
  ONTOLOGY_VERSION,
} = await import("../src/ontology.js");
const {
  ADD_ALIAS_UNKNOWN_N,
  PROMOTE_CONFUSABLE_ACCEPT_RATE,
  PROMOTE_CONFUSABLE_MIN_N,
  ontologyStats,
  ontologySummary,
  recordCandidateOutcome,
  recordOntologySnapshot,
} = await import("../src/ontology_stats.js");
const { ingestUtterance, setConsent, upsertLexicon } = await import("../src/voice.js");
const { handleHeard, __testables } = await import("../src/cant.js");

test("version stays 3 and confusables are speech-only", () => {
  assert.equal(ONTOLOGY_VERSION, 3);
  for (const row of ontologyEntries()) {
    if (row.kind === "confusable") {
      assert.deepEqual(channelsOf(row), ["speech"]);
    } else {
      assert.deepEqual(channelsOf(row), ["speech", "text"]);
    }
  }
  const counts = entryCounts();
  assert.ok(counts.entry_count > 0);
  assert.ok(counts.channels_speech >= counts.channels_text);
  assert.ok(counts.channels_speech_only > 0);
  assert.ok(counts.channels_both > 0);
  assert.equal(
    counts.channels_speech_only + counts.channels_both,
    counts.channels_speech,
  );
  assert.equal(typeof ontologyFingerprint(), "string");
  assert.equal(ontologyFingerprint().length, 64);
  assert.ok(ontologyFrames().some((row) => row.id === "buy_sell"));
  assert.ok(ontologyFrames().some((row) => row.role === "instrument_slot"));
});

test("upsertLexicon still works from ontology tests", () => {
  const out = upsertLexicon("ont-1", {
    surface_form: "loop",
    normalized_target: "WETH",
    kind: "instrument",
  });
  assert.equal(out.ok, true);
  assert.equal(out.lexicon.length, 1);
});

test("snapshot is append-only on fingerprint change", () => {
  const first = recordOntologySnapshot(1_700_000_000);
  const again = recordOntologySnapshot(1_700_000_010);
  assert.equal(again.fingerprint, first.fingerprint);
  assert.equal(again.ts, first.ts);
  const summary = ontologySummary();
  assert.equal(summary.version, 3);
  assert.equal(summary.last_snapshot.ts, first.ts);
  assert.ok(summary.fingerprint_short);
});

test("candidates roll up proposals and unknown tokens", () => {
  ingestUtterance("ont-2", {
    transcript: "buy fifty dollars worth of beef",
    channel: "speech",
    proposals: [{ surface: "beef", target: "ETH", kind: "confusable" }],
    grammar: "partial",
    unknown_instruments: ["beef"],
  });
  setConsent("ont-2", { kind: "training_use", status: "granted" });
  ingestUtterance("ont-2", {
    transcript: "buy fifty dollars worth of zzzcoin",
    channel: "text",
    grammar: "partial",
    unknown_instruments: ["zzzcoin"],
    slots: [{ kind: "size", surface: "550", target: "fifty", source: "size_rule" }],
  });
  recordCandidateOutcome("ont-2", {
    surface: "beef",
    target: "ETH",
    slotKind: "confusable",
    channel: "speech",
    outcome: "accepted",
    trainingUse: true,
  });
  const stats = ontologyStats({ accountId: "ont-2" });
  assert.equal(stats.ok, true);
  assert.equal(stats.thresholds.PROMOTE_CONFUSABLE_MIN_N, PROMOTE_CONFUSABLE_MIN_N);
  assert.equal(stats.thresholds.PROMOTE_CONFUSABLE_ACCEPT_RATE, PROMOTE_CONFUSABLE_ACCEPT_RATE);
  assert.ok(stats.candidates.some((row) => row.surface === "beef" && row.proposed >= 1));
  assert.ok(stats.candidates.some((row) => row.surface === "zzzcoin" && row.unknown >= 1));
  assert.ok(stats.all_time.speech.n + stats.all_time.text.n >= 2);
});

test("extractEntity uses instrument slot surface and skips cancel these", () => {
  const { extractEntity } = __testables();
  assert.equal(
    extractEntity("buy fifty dollars worth of ether", [
      { kind: "instrument", surface: "ether", target: "WETH", source: "alias" },
    ]),
    "weth",
  );
  assert.equal(
    extractEntity("buy fifty dollars worth of ETH", [
      { kind: "instrument", surface: "ETH", target: "WETH", source: "alias" },
    ]),
    "weth",
  );
  assert.equal(extractEntity("cancel these watches", []), null);
  const heard = handleHeard("ont-3", {
    text: "cancel these watches",
    universe: [{ symbol: "WETH", name: "Wrapped Ether" }],
  });
  assert.notEqual(heard.kind, "near_match");
});

test("eth ether ethereum alias to weth like btc aliases to wbtc", () => {
  assert.equal(instrumentAlias("eth"), "WETH");
  assert.equal(instrumentAlias("ether"), "WETH");
  assert.equal(instrumentAlias("ethereum"), "WETH");
  assert.equal(instrumentAlias("weth"), "WETH");
  assert.equal(instrumentAlias("btc"), "WBTC");
  assert.equal(instrumentAlias("bitcoin"), "WBTC");
});

test("order_type surfaces include market limit twap dca", () => {
  const rows = ontologyEntries().filter((row) => row.kind === "order_type");
  const targets = rows.map((row) => row.normalized_target);
  for (const want of ["market", "limit", "twap", "dca"]) {
    assert.ok(targets.includes(want), want);
  }
  const surfaces = rows.map((row) => String(row.surface_form).toLowerCase());
  for (const want of ["twap", "dca", "dollar cost average", "over time", "in slices"]) {
    assert.ok(surfaces.includes(want), want);
  }
});

test("promote threshold constant is documented", () => {
  assert.equal(PROMOTE_CONFUSABLE_MIN_N, 5);
  assert.equal(ADD_ALIAS_UNKNOWN_N, 5);
});

test("promote-ready pair includes a suggested JSON entry", () => {
  for (let i = 0; i < PROMOTE_CONFUSABLE_MIN_N; i++) {
    ingestUtterance("ont-promo", {
      transcript: "buy fifty dollars worth of beef",
      channel: "speech",
      proposals: [{ surface: "beef", target: "ETH", kind: "confusable" }],
    });
    recordCandidateOutcome("ont-promo", {
      surface: "beef",
      target: "ETH",
      slotKind: "confusable",
      channel: "speech",
      outcome: "accepted",
      trainingUse: false,
    });
  }
  const stats = ontologyStats({ accountId: "ont-promo" });
  const promo = stats.decisions.find((row) => row.action === "promote_confusable");
  assert.ok(promo);
  assert.equal(promo.suggested_entry.kind, "confusable");
  assert.equal(promo.suggested_entry.surface_form, "beef");
  assert.equal(String(promo.suggested_entry.normalized_target).toLowerCase(), "eth");
  assert.ok(promo.suggested_test);
});
