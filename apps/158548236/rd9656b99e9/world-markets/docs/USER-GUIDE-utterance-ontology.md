# User guide: utterance language + ontology analytics

This is the operator runbook for World’s shared speech/text language and the local analytics page that tells you when to edit the checked-in vocabulary.

You do **not** need this for day-to-day trading. Use it when STT or typed compose mishears a name, when you want to add an alias, or when you want to see whether a JSON edit actually landed.

## What this is

Hold-to-talk and typed compose both run through one server-side normalizer (`normalize_utterance`). The vocabulary lives in one checked-in file:

[`assets/speech_ontology.json`](../assets/speech_ontology.json)

That file is compiled into the plugin and also loaded by brain. There is no second `world_ontology.json`.

The normalizer:

- Rewrites exact aliases on **both** channels (`ether` → `WETH`, `bitcoin` → `WBTC`).
- **Proposes** speech confusables (`beef` → WETH) and never silently maps them.
- Does **not** apply speech confusables on typed text (`buy fifty dollars worth of beef` stays `beef`).
- Slot-gates control phrases: `cancel these watches` / `watch these` do not propose WETH.
- Speech-only size repair: `$550` / `550` in a buy/sell + worth-of frame → `fifty`. Typed `550` stays `550`.
- Speech-only dollars-worth restore: Nova-3 often keeps the number and ticker and drops unstressed `dollars worth of`. Restores `buy fifty ETH`, `open 50 ETH long`, `buy ETH 50`, `put 50 into ether`, and `buy 50 wrapped ether`. Whole amounts ≥ 10. Decimals and 1–9 stay base. Typed `buy fifty ETH` stays base.
- Speech-only opener repair: a transcript that starts with a number (`5.05 ETH`, `5 ETH`) and has no command/question word → `buy 5 WETH`. “buy 5” is often fused to `5.05` because *buy* and *five* share a diphthong. Typed `5.05 ETH` is unchanged. Leading `I have` / `I've` / `about` / `by` / `wait` on a sized trade is treated as `buy`. Leading `well` / `cell` / `sale` / `so` on a sized trade is treated as `sell`. Typed `I have 50 ETH` and `well 50 ETH` are unchanged.
- Speech-only ETH/eight collapse: `five five eight` / `58` / `buy 5 eight` → `buy 5 WETH`. “eth” is heard as “eight”; SOL does not sound like a digit so `buy 5 SOL` is left alone. Typed `58` stays `58`.
- Attaches grammar (`matched` / `partial` / `none`) and an action IR for **logging**. IR does not place a trade and is not stuffed into the LLM system prompt.

Per-account confirmations still go to the **lexicon** under the brain data dir, not into git.

## What this is not

- Production never writes `assets/speech_ontology.json`.
- The Mini App never places orders. Confirm still happens in the agent thread.
- Grammar “matched” is not auto-execute.
- Do not put the ontology dump into the skill / LLM prompt.
- Hold-to-talk UX, STT model (`nova-3`), ledger layout, and `aomi-run` dispatch are unchanged by this page.

## How to open the analytics page

1. Start the Mini App stack (brain on `:8788`, mini-app on `:8080`):

   ```sh
   ./scripts/dev-mini-app.sh --open
   ```

   Or the full local experience: `./scripts/dev-full.sh`.

2. In `.env` you need:

   - `WORLD_ACCOUNT_ID` — the bound account the page reports on
   - `MINI_APP_DEV_BYPASS=1` — localhost without Telegram

3. Open:

   [http://127.0.0.1:8080/dev/ontology?preview=dev](http://127.0.0.1:8080/dev/ontology?preview=dev)

Gate: `MINI_APP_DEV_BYPASS`, Host `127.0.0.1` / `localhost`, and `?preview=dev`. Without the query param the route is **404**. This is not a product URL for Telegram users.

Optional JSON dump (not a substitute for the page):

```sh
./scripts/ontology-report.sh
```

Brain APIs (localhost):

- `GET http://127.0.0.1:8788/v1/ontology/summary` — version, fingerprint, snapshot history
- `GET http://127.0.0.1:8788/v1/ontology/stats?account_id=$WORLD_ACCOUNT_ID` — 7-day + all-time, speech vs text, decision queue
- `?all=1` iterates every `voice/*.json` on this machine. Operator-local only. Do not export other users’ utterances unless `training_use` is granted.

Snapshots and candidates live under the brain data dir (gitignored), default:

`$XDG_DATA_HOME/aomi/world-markets/brain` or `~/.local/share/aomi/world-markets/brain`

Files: `ontology/snapshots.json`, `ontology/candidates.json`. Utterances: `voice/<account_id>.json`.

## Everyday use (speech and text)

Same sentence, two inputs:

| You say / type | Speech | Text |
|---|---|---|
| `buy fifty dollars worth of ETH` | normalized WETH, grammar matched | same |
| `buy fifty dollars worth of ether` | WETH (alias) | WETH (alias) |
| `buy fifty dollars worth of beef` | stays `beef`, **proposes** WETH | stays `beef`, **no** proposal |
| `cancel these watches` | no WETH proposal | no WETH proposal |
| `5.05 ETH` (said “buy 5 ETH”) | `buy 5 WETH` | stays `5.05 WETH` |
| `five five eight` / `58` (said “buy 5 ETH”) | `buy 5 WETH` | stays `58` |

Typed compose in the Mini App (`type instead` → slide to send) POSTs `/api/v1/mini-app/compose`. Local `?preview=dev` is the same path as the hosted Mini App compose, not a second client-side dictionary.

Hold-to-talk is still `POST /api/v1/mini-app/voice` (Deepgram `nova-3` + keyterm prompting). After STT, the same normalizer runs with `channel: speech`.

A successful normalize writes a full record (channel, slots, proposals, grammar, action IR). The agent still sees `normalized_text`. Unknown in-universe names can still wall as `can't` if the live catalog does not trade that symbol — that is catalog, not a missed rewrite.

## Reading the page

### Now

- **version** — `assets/speech_ontology.json` `"version"` (currently 3).
- **fingerprint** — hash of entries + frames + repairs. Changes when you edit the JSON.
- **entries / speech-only / both channels** — speech-only rows are acoustic confusables; omitted `channels` means both speech and text.
- **last snapshot** — last time brain recorded a version or fingerprint change (boot, not every stats request).

### Over time

Append-only snapshot table. A new row appears when you change the JSON **and restart brain**. Same fingerprint on restart does not add a row. Use this to see whether an edit actually loaded.

### Behavior

Last 7 days and all-time, split speech vs text:

- utterance count, repair rate, proposal rate
- grammar mix (matched / partial / none)
- cant rate and correction rate
- size-rule fires, grammar-none + act (frame-gap signal)

Repair rate high on speech after you promoted an alias is a smell: the rewrite may not have landed.

### Decision queue

Top suggestions. Thresholds live in `brain/src/ontology_stats.js`, not in the HTML. Banners include a suggested JSON entry and a suggested fixture name when the page has enough n.

| Banner | Meaning | What you do |
|---|---|---|
| `promote_confusable` | Same speech pair proposed n≥5 and accept ≥80% | Add a `kind: "confusable"` row with `"channels": ["speech"]`. Keep proposing — do not silent-map. Add the suggested test. |
| `add_alias` | Unknown instrument-slot token n≥5 (speech or text) | Add `kind: "instrument"` alias (`ether` → `WETH` style). Omit `channels` so it applies to both. |
| `add_negative_fixture` | Same confusable rejected ≥3 in a non-instrument frame (`cancel these` …) | Do **not** add a rewrite. Add a slot-gate test so it never proposes. |
| `alias-did-not-land` / `needs_more_n` | After a snapshot bump, that pair still repairs a lot | Check the JSON actually changed, restart brain, wait for more n. |
| `frame_gap` | grammar-none + has act is rising | The utterance is trade-shaped but no frame matched. Extend `frames` in the JSON, with a test. |
| _(empty)_ | Leave it | Not enough n. Do not edit from a single mishear. |

Accept rate is `accepted / proposed` (confirms vs how often it was offered).

## How to edit the vocabulary

1. Edit **only** [`assets/speech_ontology.json`](../assets/speech_ontology.json).
2. Bump `"version"` when the language change is intentional (channels-only was v1→v2 when frames/repairs landed). Fingerprint will change even if you forget the version bump.
3. Restart **brain** so it records a snapshot. Restart **mini-app** if you need the binary’s `include_str!` copy (speech-ontology GET and tests). `./scripts/dev-mini-app.sh` rebuilds the mini-app.
4. Reload `/dev/ontology?preview=dev` and confirm a new snapshot row (new fingerprint, entry count).
5. Add a fixture (Rust `src/speech_ontology.rs` tests and/or `brain/test/ontology.test.js`) for the behavior you wanted.
6. Run:

   ```sh
   cargo test --lib speech_ontology
   cargo test -p world-mini-app
   node --test brain/test/*.test.js
   ```

### Entry shapes

```json
{ "surface_form": "ether", "normalized_target": "WETH", "kind": "instrument", "confidence": 1.0 }
```

Omitted `"channels"` = both speech and text.

```json
{ "surface_form": "beef", "normalized_target": "WETH", "kind": "confusable", "confidence": 1.0, "channels": ["speech"] }
```

Speech-only. Propose, never rewrite.

Kinds you will actually add: `instrument` (alias), `confusable` (speech near-miss), occasionally `act` / `order_type` / `size_frame`. Frames and repairs are for grammar and the `$550`→fifty speech rule — change those only with a test.

### Per-account vs checked-in

| Store | When | Where |
|---|---|---|
| Checked-in JSON | Shared language: aliases, speech confusables, frames | `assets/speech_ontology.json` (git) |
| Per-account lexicon | User confirmed “beef meant BIFI” | `WORLD_BRAIN_DIR/voice/<id>.json` |
| Candidates / snapshots | Analytics only | `WORLD_BRAIN_DIR/ontology/` |

Never copy a one-user confirm into git until the decision queue says the pair is promote-ready.

## Smoke after an edit

Typed (Mini App `type instead`, or `POST /api/v1/mini-app/compose` with a dev session):

- `buy fifty dollars worth of ETH`
- `buy fifty dollars worth of ether` → record `text` is WETH, `repaired_from` is the ether sentence
- `cancel these watches` → no proposals

Speech: hold-to-talk the same phrases (needs mic + Deepgram). Confusables should propose, not rewrite.

Then open the analytics page and check the new snapshot plus speech vs text counts.

## Troubleshooting

| Symptom | Check |
|---|---|
| `/dev/ontology` is 404 | Need `?preview=dev`, `MINI_APP_DEV_BYPASS=1`, and Host localhost |
| Page says brain unreachable | Brain not on `:8788`, or `WORLD_ACCOUNT_ID` missing |
| No new snapshot after JSON edit | Restart brain; fingerprint is of parsed entries/frames/repairs, not file whitespace |
| Typed `beef` became WETH | Bug — text must not apply speech confusables |
| `cancel these` proposed WETH | Slot-gate regression — add/keep a negative fixture |
| `ether` stayed ether | Alias missing or mini-app/plugin not rebuilt after JSON change |
| Promote banner but you already added the row | `alias-did-not-land` — confirm snapshot fingerprint matches the file you think is loaded |
