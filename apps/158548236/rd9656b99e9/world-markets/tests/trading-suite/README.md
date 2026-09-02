# Local trading suite (development only)

UniFi testnet cases through `aomi-run`. Swap is not included. The runner binds
`mandate.json` (WETH/SOL/WBTC/USDT). Logs and results are gitignored.

```sh
cargo build
(cd sidecar && npm start) &
python3 tests/trading-suite/run.py
python3 tests/trading-suite/run.py --from-id 16   # new cases only
```
