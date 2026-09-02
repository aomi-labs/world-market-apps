#!/usr/bin/env bash
# Full local experience: brain + execution sidecar + Mini App UI + interactive aomi-run CLI.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OPEN_BROWSER=1
SKIP_SIDECAR=0
SKIP_CLI=0
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --open)
      OPEN_BROWSER=1
      shift
      ;;
    --no-open)
      OPEN_BROWSER=0
      shift
      ;;
    --no-sidecar)
      SKIP_SIDECAR=1
      shift
      ;;
    --no-cli)
      SKIP_CLI=1
      shift
      ;;
    -h | --help)
      cat <<'EOF'
Usage: scripts/dev-full.sh [options] [-- aomi-run args…]

Starts everything for local end-to-end testing:
  - brain sidecar (ledger, watches, compose, voice records)
  - execution sidecar (when WORLD_PRIVATE_KEY is in .env)
  - world-mini-app HTTP server (Mini App UI in the browser)
  - interactive aomi-run CLI in this terminal (agent chat thread)

Requires .env:
  WORLD_ACCOUNT_ID, OPENROUTER_API_KEY (or OPENAI/ANTHROPIC), MINI_APP_DEV_BYPASS=1

Options:
  --open        Open portfolio + chart browser tabs (default)
  --no-open     Skip browser
  --no-sidecar  Skip execution sidecar even when WORLD_PRIVATE_KEY is set
  --no-cli      Services + browser only; no aomi-run REPL

Examples:
  ./scripts/dev-full.sh
  ./scripts/dev-full.sh --no-open
  ./scripts/dev-full.sh -- --prompt "how am I doing?"
EOF
      exit 0
      ;;
    --)
      EXTRA_ARGS=("${@:2}")
      break
      ;;
    *)
      EXTRA_ARGS+=("$1")
      shift
      ;;
  esac
done

# shellcheck disable=SC1091
source "$ROOT/scripts/dev-lib.sh"
dev_cd_root
dev_require_env
dev_configure_ports
dev_plan_sidecar

if [[ -z "$(dev_env_val WORLD_ACCOUNT_ID)" ]]; then
  echo "WORLD_ACCOUNT_ID is not set in .env — portfolio and ledger will fail" >&2
fi

if [[ "$(dev_env_val MINI_APP_DEV_BYPASS)" != "1" ]] \
  && [[ "$(dev_env_val MINI_APP_DEV_BYPASS)" != "true" ]] \
  && [[ -z "$(dev_env_val TELEGRAM_BOT_TOKEN)" ]]; then
  echo "hint: set MINI_APP_DEV_BYPASS=1 for local browser testing without Telegram" >&2
fi

if [[ "$SKIP_CLI" -eq 0 ]]; then
  if ! command -v aomi-run >/dev/null 2>&1; then
    echo "aomi-run not found on PATH — install aomi-sdk with cli,dev-runtime features" >&2
    echo "  cargo install --git https://github.com/aomi-labs/aomi-sdk --features cli,dev-runtime aomi-sdk" >&2
    exit 1
  fi
  provider="$(dev_llm_provider)"
  if [[ "$provider" == "openrouter" ]] && [[ -z "$(dev_env_val OPENROUTER_API_KEY)" ]]; then
    echo "OPENROUTER_API_KEY is not set in .env (required for aomi-run with openrouter)" >&2
    exit 1
  fi
fi

DEV_SKIP_SIDECAR="$SKIP_SIDECAR"
dev_preflight_ports
dev_install_npm
dev_build_rust

dev_start_brain
dev_start_sidecar
trap dev_cleanup EXIT INT TERM

dev_wait_sidecars
dev_start_mini_app background
dev_wait_mini_app

dev_print_stack_banner full

if [[ "$OPEN_BROWSER" -eq 1 ]]; then
  dev_open_mini_app_tabs
fi

if [[ "$SKIP_CLI" -eq 1 ]]; then
  echo "Press Ctrl+C to stop all services."
  echo
  wait "$MINI_PID"
  exit 0
fi

PLUGIN="$(dev_plugin_path)"
PROVIDER="$(dev_llm_provider)"

# Seed post-trade RAPV from live RAPV when ATLAS projection fails (stubbed evm-core).
export WORLD_DEV_SEED_POST_TRADE_RAPV="${WORLD_DEV_SEED_POST_TRADE_RAPV:-1}"

cat <<EOF
Starting agent CLI (type messages here — same thread as Mini App compose/voice).
Quit the REPL or Ctrl+C to stop brain, sidecar, and mini-app.

EOF

if [[ ${#EXTRA_ARGS[@]} -gt 0 ]]; then
  exec aomi-run "$PLUGIN" --env-file .env --provider "$PROVIDER" "${EXTRA_ARGS[@]}"
fi

exec aomi-run "$PLUGIN" --env-file .env --provider "$PROVIDER"
