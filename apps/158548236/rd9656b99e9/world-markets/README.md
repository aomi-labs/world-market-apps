# World Markets Agent

An Aomi app for live World Markets context and mandate-gated execution on the
UniFi testnet (chain ID 2092151908).

The app reads the World exchange contract directly over its own RPC path,
evaluates every trade intent against the signed mandate (ATLAS post-trade
risk included), and ships its own skill catalog — including the execution
procedure and the guard table the host enforces on every staged transaction.

Reads (never execute):

- `list_world_assets`
- `get_world_account`
- `get_health_snapshot` (account card + PnL)
- `get_world_market`
- `get_world_rates`
- `get_world_loans`
- `get_world_open_orders`
- `get_world_pnl`

Verdicts (never execute):

- `preview_world_trade` — resolved size, book, mark, limit price, and the
  deterministic mandate verdict under `preview.verdict`
- `check_world_mandate` — same body as the preview
- `compute_resize` — the one number a block cites: the signed RAPV floor

Execution helpers (pure computation / read-only; the host stages, simulates,
and commits):

- `world_resolve_book` — the order-book contract for a product and pair
- `world_pack_order` — the packed `uint256` order word a
  `new*Order(address,uint256)` call takes

The execution skill (`src/skill/execution.md`) is the only path to the chain:
allow verdict → `world_resolve_book` → `world_pack_order` → one `evm_stage_tx`
(Encode mode, `to` = the exchange) → one `simulate_batch` → one
`evm_commit_txs`. `src/skill/guard.json` restricts staged calls to the exchange
contract and the fourteen trading selectors (six `new*Order`, six
`cancel*Order`, `renewLoan`, `payInterestAndFees`) on chain 2092151908.

The mandate fails closed: without a bound handover mandate every verdict is
`missing_mandate` and nothing is staged. The bound account comes from
`handover.account_ref`, then the mandate's `account.id`. Post-trade RAPV is
derived from ATLAS `evaluate` at unit risk, anchored to the live contract RAPV;
if that derivation cannot run, the verdict is `post_trade_risk_unavailable`.

## Validate

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo test reads_live_world -- --ignored --nocapture
cargo build --release
```

## Interactive sanity (aomi-run)

[`aomi-run`](https://aomi.dev/docs/build/toolchain/aomi-run) is the local dev
runtime: it loads this app, calls a real LLM, and shows which tools the model
selects. It is **not** the hosted backend.

```sh
chmod +x scripts/dev-run.sh
./scripts/dev-run.sh
```

`dev-run.sh` builds with `--features local-dev`, which is the only build that
honours `WORLD_ACCOUNT_ID` (set it in `.env` so the app process inherits it via
`--env-file`). `aomi-run` stubs every handover attribute, so there is no
mandate locally: previews return `missing_mandate` and nothing can be staged.
Reads and lookups work. `WORLD_RPC_URL` / `WORLD_EXCHANGE_ADDRESS` are
local-only overrides; a hosted run takes the chain from the host and the
exchange from the handover.

On Linux use `libworld_markets.so`. Terse lookups (`b`, `p`, `r`) are plain
messages, not slash commands.

Hosted chat against staging: `./scripts/hosted-chat.sh`.

PnL persistence: realized and closed-position figures are written under
`WORLD_PNL_DIR`, else `$XDG_DATA_HOME/aomi/world-markets/pnl`. Open PnL is
live from the contract.

## Deploy

This app requires Aomi SDK 5.0.1 in both `Cargo.toml` and `Cargo.lock` (5.0.0 hashes app-skill guard tables differently from the host).
Its hosted instructions are the three SDK 5 skills in `src/skill/`; each must
pass the 4,000-token validation limit. `cargo test --locked` checks the skills,
the guard table, and every provider-facing tool schema before publication.

An owner or repository administrator must first open the
[World Markets staging import page](https://build-staging.aomi.dev/operate/deployments/new?platform=world-market-apps&mode=import),
confirm the `world-market-apps` platform, and connect `World-Markets-Inc/aomi`.
Scope the staging Aomi GitHub App to this repository rather than every
organization repository.

![Connect the World Markets repository to its Aomi platform](docs/images/aomi-build-connect.jpg)

If Build reports that `.aomi/config.json` uses `world-market-apps` but the
Project uses `community`, no Project was created. Reopen the scoped staging
link above and retry on `world-market-apps`.

After the Project is connected:

```sh
cargo install --git https://github.com/aomi-labs/aomi-sdk \
  --features cli,dev-runtime aomi-sdk
aomi-build login \
  --build-url https://build-staging.aomi.dev \
  --backend https://api-staging.aomi.dev
```

After each code update, validate, commit, and push the exact revision that
should run:

```sh
cargo test --locked
cargo build --release
git push

aomi-build deploy preflight --repo World-Markets-Inc/aomi
aomi-build deploy --repo World-Markets-Inc/aomi
aomi-build deploy status
```

Deployment uses the pushed Git commit, not uncommitted working-tree changes.
`.aomi/deployment.json` is local lifecycle state and must not be committed.
