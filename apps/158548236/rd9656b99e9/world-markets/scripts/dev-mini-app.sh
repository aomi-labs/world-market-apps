#!/usr/bin/env bash
# Mini App UI only (no interactive aomi-run). For full local experience use dev-full.sh.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

OPEN_BROWSER=0
SKIP_SIDECAR=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --open)
      OPEN_BROWSER=1
      shift
      ;;
    --no-sidecar)
      SKIP_SIDECAR=1
      shift
      ;;
    -h | --help)
      cat <<'EOF'
Usage: scripts/dev-mini-app.sh [--open] [--no-sidecar]

Mini App stack only (no interactive agent CLI). For ledger + browser + aomi-run:
  ./scripts/dev-full.sh

Starts:
  - brain sidecar (ledger, watches, compose, voice records)
  - execution sidecar (only if WORLD_PRIVATE_KEY is in .env and not --no-sidecar)
  - world-mini-app HTTP server (UI + API)

Options:
  --open        Open http://127.0.0.1:8080/?preview=dev in the default browser
  --no-sidecar  Skip execution sidecar even when WORLD_PRIVATE_KEY is set
EOF
      exit 0
      ;;
    *)
      echo "unknown option: $1 (try --help)" >&2
      exit 1
      ;;
  esac
done

# shellcheck disable=SC1091
source "$ROOT/scripts/dev-lib.sh"
dev_cd_root
dev_require_env
dev_configure_ports

DEV_SKIP_SIDECAR="$SKIP_SIDECAR"
dev_plan_sidecar

if [[ -z "$(dev_env_val WORLD_ACCOUNT_ID)" ]]; then
  echo "WORLD_ACCOUNT_ID is not set in .env — portfolio and ledger will fail" >&2
fi

if [[ "$(dev_env_val MINI_APP_DEV_BYPASS)" != "1" ]] \
  && [[ "$(dev_env_val MINI_APP_DEV_BYPASS)" != "true" ]] \
  && [[ -z "$(dev_env_val TELEGRAM_BOT_TOKEN)" ]]; then
  echo "hint: set MINI_APP_DEV_BYPASS=1 for local browser testing without Telegram" >&2
fi

dev_preflight_ports
dev_install_npm
dev_build_rust

dev_start_brain
dev_start_sidecar
trap dev_cleanup EXIT INT TERM

dev_wait_sidecars

dev_print_stack_banner mini
echo "Press Ctrl+C to stop all services."
echo

if [[ "$OPEN_BROWSER" -eq 1 ]]; then
  dev_open_mini_app_tabs
fi

dev_start_mini_app foreground
