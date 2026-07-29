# World Markets Agent

An Aomi app for live World Markets context on MegaETH mainnet.

The app reads the World exchange contract directly and exposes four typed tools:

- `list_world_assets`
- `get_world_account`
- `get_world_market`
- `preview_world_trade`

Version 0.1 is intentionally read-only. Trade previews are never policy
approval and cannot sign, submit, or execute transactions.

## Validate

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test reads_live_world -- --ignored --nocapture
cargo build --release
```
