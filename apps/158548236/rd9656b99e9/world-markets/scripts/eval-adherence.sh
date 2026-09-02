#!/usr/bin/env bash
# Local-only adherence eval (4a) and round-2 probe harness (4c). Not CI.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export WORLD_DEV_SEED_POST_TRADE_RAPV="${WORLD_DEV_SEED_POST_TRADE_RAPV:-1}"
exec python3 tests/adherence-eval/run.py "$@"
