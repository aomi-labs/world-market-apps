# World Markets Agent

An Aomi app for live World Markets context on MegaETH mainnet.

The app reads the World exchange contract directly and exposes seven typed tools:

- `list_world_assets`
- `get_world_account`
- `get_world_market`
- `preview_world_trade`
- `check_world_mandate`
- `get_world_agent_permission`
- `get_world_open_orders`

Version 0.3 is mandate-aware and intentionally non-executable. It verifies the
active actor as the World account owner or an on-chain permitted trader, parses
mandate v1 with fail-closed unknown-key handling, evaluates structured intents
against live account and market state, and reads resting orders. Its app-scoped
skill cannot be discovered or activated by another Aomi app.

The app fails closed when it cannot prove post-trade risk-adjusted portfolio
value. Transaction staging remains disabled until the host can guarantee that
app policy cannot be bypassed through a host wallet tool and the app can compute
the full post-trade World risk state.

## Validate

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test reads_live_world -- --ignored --nocapture
cargo build --release
```
