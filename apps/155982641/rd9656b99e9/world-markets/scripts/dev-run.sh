#!/usr/bin/env bash
# Local aomi-run with the execution sidecar. The plugin never sees WORLD_PRIVATE_KEY.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env ]]; then
  echo "copy .env.example to .env and set OPENROUTER_API_KEY, WORLD_ACCOUNT_ID, WORLD_PRIVATE_KEY" >&2
  exit 1
fi

cargo build

if [[ ! -d sidecar/node_modules ]]; then
  (cd sidecar && npm install)
fi

SIDECAR_LOG="${TMPDIR:-/tmp}/world-markets-sidecar.log"
(cd sidecar && npm start) >"$SIDECAR_LOG" 2>&1 &
SIDECAR_PID=$!
cleanup() {
  kill "$SIDECAR_PID" 2>/dev/null || true
}
trap cleanup EXIT

PORT="${WORLD_EXECUTION_PORT:-}"
if [[ -z "$PORT" ]]; then
  PORT="$(awk -F= '/^WORLD_EXECUTION_PORT=/{v=$2} END{print v}' .env | tr -d '\"' || true)"
fi
PORT="${PORT:-8787}"

healthy=0
for _ in $(seq 1 75); do
  if curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null; then
    healthy=1
    break
  fi
  if ! kill -0 "$SIDECAR_PID" 2>/dev/null; then
    echo "execution sidecar exited; see $SIDECAR_LOG" >&2
    cat "$SIDECAR_LOG" >&2 || true
    exit 1
  fi
  sleep 0.2
done
if [[ "$healthy" -ne 1 ]]; then
  echo "execution sidecar did not become healthy; see $SIDECAR_LOG" >&2
  cat "$SIDECAR_LOG" >&2 || true
  exit 1
fi

PLUGIN="target/debug/libworld_markets.dylib"
if [[ ! -f "$PLUGIN" ]]; then
  PLUGIN="target/debug/libworld_markets.so"
fi

aomi-run "$PLUGIN" --env-file .env --provider openrouter "$@"
