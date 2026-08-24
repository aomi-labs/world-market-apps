# World Markets Agent

An Aomi app for live World Markets context on the UniFi testnet.

The app reads the World exchange contract directly and exposes typed tools in two
groups.

Live contract reads and execution (mandate-aware):

- `list_world_assets`
- `get_world_account`
- `get_world_market`
- `get_world_rates`
- `get_world_loans`
- `preview_world_trade`
- `check_world_mandate`
- `execute_world_order`
- `cancel_world_order`
- `execute_world_swap`
- `renew_world_loans`
- `get_world_agent_permission`
- `get_world_open_orders`

Reporting-service tools (the honest-numbers layer — deterministic derived figures
so the message layer never authors a number; see `src/skill/` and the
`TELEGRAM-MESSAGING-UX-SPEC`):

- `get_world_pnl`
- `preview_account_effect`
- `compute_resize`
- `preview_exit`
- `plan_large_order`
- `get_dollarpower`
- `simulate_guardian_unwind`
- `check_negative_carry`

Version 0.4 is mandate-aware. Local `aomi-run` can place, cancel, swap, and
extend loans through a Node sidecar (`sidecar/`) that holds `WORLD_PRIVATE_KEY`
and calls `@wcm-inc/sdk`. The Rust plugin never sees the key. Hosted Aomi
signing is not in this release.

The mandate still fail-closes without a bound policy document. Post-trade RAPV is
derived from ATLAS `evaluate` at unit risk, anchored to the live contract RAPV,
and labeled as an estimate. If that derivation cannot run, the mandate fail-closes.

## Validate

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test reads_live_world -- --ignored --nocapture
cargo build --release
```

## Interactive sanity (aomi-run)

[`aomi-run`](https://aomi.dev/docs/build/toolchain/aomi-run) is the local dev
runtime: it loads this plugin, calls a real LLM, and shows which tools the model
selects. It is **not** the hosted Telegram backend.

```sh
# plugin only (reads, previews)
cargo build
aomi-run target/debug/libworld_markets.dylib \
  --env-file .env --provider openrouter

# plugin + local execution sidecar (requires WORLD_PRIVATE_KEY)
chmod +x scripts/dev-run.sh
./scripts/dev-run.sh
```

On Linux use `libworld_markets.so`. `/help` inside the REPL lists only host
commands (`/quit`, `/reset`, …) — not agent lookup tokens. Terse lookups (`b`,
`p`, `r`, …) are plain messages, not slash commands.

Set `WORLD_ACCOUNT_ID` in `.env` (not only on the shell command line) so the
plugin process inherits it via `--env-file`. `aomi-run` returns `None` for all
handover state attributes per the
[aomi-run docs](https://aomi.dev/docs/build/toolchain/aomi-run#what-the-dev-runtime-stubs).
`aomi-run` has no `handover_mandate`, so the plugin uses a bundled placeholder
policy (WETH/USDT, see `mandate.dev.example.json`) unless you set
`WORLD_MANDATE_PATH` to a real file or to `none`. Start the sidecar so execute
tools can submit.

Smoke prompts:

- `b` → one line: `Portfolio [#].` (calls `get_world_account`)
- "What can't you do?" → §6.1 incapacity message (no numbers)
- "How am I doing?" → multi-line health card via `get_world_account` /
  `get_world_pnl` / `get_dollarpower`

PnL persistence (until Aomi host storage is agreed): realized and closed-position
figures are written under `WORLD_PNL_DIR`, else
`$XDG_DATA_HOME/aomi/world-markets/pnl`. Open PnL is live from the contract.

Deploy against the real backend for live handover and mandate context. Hosted
execution waits on Aomi's key-holding design; local execution uses the sidecar.

## Deploy

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
cargo test
cargo build --release
git push

aomi-build deploy preflight --repo World-Markets-Inc/aomi
aomi-build deploy --repo World-Markets-Inc/aomi
aomi-build deploy status
```

Deployment uses the pushed Git commit, not uncommitted working-tree changes.
`.aomi/deployment.json` is local lifecycle state and must not be committed.
