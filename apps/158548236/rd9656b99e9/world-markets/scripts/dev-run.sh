#!/usr/bin/env bash
# Local aomi-run with the execution sidecar. The plugin never sees WORLD_PRIVATE_KEY.
# Wipe residual brain state: WORLD_BRAIN_WIPE=1 or --wipe (deletes WORLD_BRAIN_DIR).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ ! -f .env ]]; then
  echo "copy .env.example to .env and set OPENROUTER_API_KEY, WORLD_ACCOUNT_ID, WORLD_PRIVATE_KEY" >&2
  exit 1
fi

WIPE=0
ARGS=()
for arg in "$@"; do
  if [[ "$arg" == "--wipe" ]]; then
    WIPE=1
  else
    ARGS+=("$arg")
  fi
done
if [[ "${WORLD_BRAIN_WIPE:-}" == "1" ]]; then
  WIPE=1
fi
if [[ "$WIPE" == "1" ]]; then
  BRAIN_DIR="${WORLD_BRAIN_DIR:-}"
  if [[ -z "$BRAIN_DIR" ]]; then
    BRAIN_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/aomi/world-markets/brain"
  fi
  rm -rf "$BRAIN_DIR"
  echo "wiped brain dir $BRAIN_DIR (per-account files under this env)"
fi

cargo build

if [[ ! -d brain/node_modules ]]; then
  (cd brain && npm install)
fi
if [[ ! -d sidecar/node_modules ]]; then
  (cd sidecar && npm install)
fi

BRAIN_LOG="${TMPDIR:-/tmp}/world-markets-brain.log"
(cd brain && npm start) >"$BRAIN_LOG" 2>&1 &
BRAIN_PID=$!

SIDECAR_LOG="${TMPDIR:-/tmp}/world-markets-sidecar.log"
(cd sidecar && npm start) >"$SIDECAR_LOG" 2>&1 &
SIDECAR_PID=$!
cleanup() {
  kill "$BRAIN_PID" "$SIDECAR_PID" 2>/dev/null || true
}
trap cleanup EXIT

BRAIN_PORT="${WORLD_BRAIN_PORT:-}"
if [[ -z "$BRAIN_PORT" ]]; then
  BRAIN_PORT="$(awk -F= '/^WORLD_BRAIN_PORT=/{v=$2} END{print v}' .env | tr -d '\"' || true)"
fi
BRAIN_PORT="${BRAIN_PORT:-8788}"

PORT="${WORLD_EXECUTION_PORT:-}"
if [[ -z "$PORT" ]]; then
  PORT="$(awk -F= '/^WORLD_EXECUTION_PORT=/{v=$2} END{print v}' .env | tr -d '\"' || true)"
fi
PORT="${PORT:-8787}"

brain_ok=0
for _ in $(seq 1 50); do
  if curl -sf "http://127.0.0.1:${BRAIN_PORT}/health" >/dev/null; then
    brain_ok=1
    break
  fi
  if ! kill -0 "$BRAIN_PID" 2>/dev/null; then
    echo "brain sidecar exited; see $BRAIN_LOG" >&2
    cat "$BRAIN_LOG" >&2 || true
    exit 1
  fi
  sleep 0.2
done
if [[ "$brain_ok" -ne 1 ]]; then
  echo "brain sidecar did not become healthy; see $BRAIN_LOG" >&2
  cat "$BRAIN_LOG" >&2 || true
  exit 1
fi


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

# Seed post-trade RAPV from live RAPV when ATLAS projection fails (stubbed evm-core).
export WORLD_DEV_SEED_POST_TRADE_RAPV="${WORLD_DEV_SEED_POST_TRADE_RAPV:-1}"

aomi-run "$PLUGIN" --env-file .env --provider openrouter "${ARGS[@]}"
