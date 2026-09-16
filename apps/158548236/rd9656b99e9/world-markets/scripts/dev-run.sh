#!/usr/bin/env bash
# Local aomi-run: build the app with the local-dev feature and start the REPL.
# Reads WORLD_ACCOUNT_ID / WORLD_RPC_URL / WORLD_EXCHANGE_ADDRESS from .env.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env ]]; then
  echo "copy .env.example to .env and set OPENROUTER_API_KEY and WORLD_ACCOUNT_ID" >&2
  exit 1
fi

cargo build --features local-dev

APP="target/debug/libworld_markets.dylib"
if [[ ! -f "$APP" ]]; then
  APP="target/debug/libworld_markets.so"
fi

exec aomi-run "$APP" --env-file .env --provider openrouter "$@"
