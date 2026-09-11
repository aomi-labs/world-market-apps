#!/usr/bin/env bash
# Prove brain + Mini App are healthy before copying them to a dedicated box.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

RELEASE=0
STOP_AFTER=0
STARTED_BRAIN=0
STARTED_MINI=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release)
      RELEASE=1
      shift
      ;;
    --stop-after)
      STOP_AFTER=1
      shift
      ;;
    -h | --help)
      cat <<'EOF'
Usage: scripts/smoke-mini-app.sh [--release] [--stop-after]

Checks the same process you will run on a dedicated server:

  1. brain on :8788
  2. world-mini-app on :8080
  3. health, ontology, auth, portfolio, ledger, chart APIs

Uses the laptop .env. MINI_APP_DEV_BYPASS=1 is required for the authenticated
API checks (browser preview). Telegram WebView still needs a bot token and
./scripts/telegram-mini-app.sh.

--release   cargo build --release -p world-mini-app and run that binary
--stop-after  stop processes this script started
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

FAILS=0
pass() { echo "  ok  $1"; }
fail() { echo "  FAIL $1" >&2; FAILS=$((FAILS + 1)); }

cleanup() {
  if [[ "$STOP_AFTER" -eq 1 ]]; then
    [[ "$STARTED_BRAIN" -eq 1 && -n "${BRAIN_PID:-}" ]] && kill "$BRAIN_PID" 2>/dev/null || true
    [[ "$STARTED_MINI" -eq 1 && -n "${MINI_PID:-}" ]] && kill "$MINI_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

if ! dev_http_ok "$BRAIN_HEALTH_URL"; then
  echo "starting brain…"
  dev_install_npm
  dev_start_brain
  dev_wait_http "brain sidecar" "$BRAIN_HEALTH_URL" "$BRAIN_PID" "$BRAIN_LOG" 50
  STARTED_BRAIN=1
fi

if ! dev_http_ok "$MINI_HEALTH_URL"; then
  echo "starting mini-app…"
  if [[ "$RELEASE" -eq 1 ]]; then
    cargo build --release -p world-mini-app
    MINI_LOG="${TMPDIR:-/tmp}/world-markets-mini-app.log"
    "$ROOT/target/release/world-mini-app" >"$MINI_LOG" 2>&1 &
    MINI_PID=$!
    dev_wait_mini_app
  else
    cargo build -p world-mini-app
    dev_start_mini_app background
    dev_wait_mini_app
  fi
  STARTED_MINI=1
fi

echo "smoke"
echo "  mini-app  $MINI_HEALTH_URL"
echo "  brain     $BRAIN_HEALTH_URL"

code="$(curl -sS -o /tmp/wm-brain-health.json -w '%{http_code}' "$BRAIN_HEALTH_URL" || true)"
if [[ "$code" == "200" ]]; then
  pass "brain /health"
else
  fail "brain /health (HTTP $code)"
fi

code="$(curl -sS -o /tmp/wm-mini-health.json -w '%{http_code}' "$MINI_HEALTH_URL" || true)"
if [[ "$code" == "200" ]] && python3 -c "import json; assert json.load(open('/tmp/wm-mini-health.json'))['ok'] is True"; then
  pass "mini-app /api/v1/mini-app/health"
else
  fail "mini-app health (HTTP $code)"
fi

code="$(curl -sS -o /tmp/wm-onto.json -w '%{http_code}' "http://${MINI_HOST}:${MINI_PORT}/api/v1/mini-app/speech-ontology" || true)"
if [[ "$code" == "200" ]]; then
  pass "speech-ontology"
else
  fail "speech-ontology (HTTP $code)"
fi

BYPASS="$(dev_env_val MINI_APP_DEV_BYPASS)"
if [[ "$BYPASS" != "1" && "$BYPASS" != "true" ]]; then
  code="$(curl -sS -o /tmp/wm-unauth.json -w '%{http_code}' "http://${MINI_HOST}:${MINI_PORT}/api/v1/mini-app/portfolio" || true)"
  if [[ "$code" == "401" ]]; then
    pass "portfolio rejects unauthenticated (prod-like)"
  else
    fail "portfolio should 401 without session when bypass is off (HTTP $code)"
  fi
  echo
  echo "Authenticated API checks skipped (MINI_APP_DEV_BYPASS is off)."
  echo "For Telegram: add TELEGRAM_BOT_TOKEN and run ./scripts/telegram-mini-app.sh"
else
  code="$(curl -sS -o /tmp/wm-auth.json -w '%{http_code}' \
    -H 'Content-Type: application/json' \
    -d '{"init_data":"dev"}' \
    "http://${MINI_HOST}:${MINI_PORT}/api/v1/mini-app/auth" || true)"
  if [[ "$code" != "200" ]]; then
    fail "auth with init_data=dev (HTTP $code) — set MINI_APP_DEV_BYPASS=1 for laptop checks"
  else
    TOKEN="$(python3 -c "import json; print(json.load(open('/tmp/wm-auth.json'))['token'])")"
    pass "auth (dev bypass)"
    auth_get() {
      local name="$1"
      local path="$2"
      local expect="${3:-200}"
      local out="/tmp/wm-${name}.json"
      local got
      got="$(curl -sS -o "$out" -w '%{http_code}' -H "Authorization: Bearer $TOKEN" \
        "http://${MINI_HOST}:${MINI_PORT}${path}" || true)"
      if [[ "$got" == "$expect" ]]; then
        pass "$name"
      else
        fail "$name (HTTP $got, expected $expect)"
        python3 -c "print(open('$out').read()[:400])" 2>/dev/null || true
      fi
    }
    auth_get portfolio /api/v1/mini-app/portfolio
    auth_get ledger /api/v1/mini-app/ledger
    auth_get chart "/api/v1/mini-app/chart?symbol=AAPL&period=d"
  fi
fi

echo
if [[ "$FAILS" -gt 0 ]]; then
  echo "$FAILS check(s) failed" >&2
  exit 1
fi
echo "all checks passed"
echo "browser: http://${MINI_HOST}:${MINI_PORT}/?preview=dev"
if [[ -z "$(dev_env_val TELEGRAM_BOT_TOKEN)" ]]; then
  echo "Telegram WebView: add TELEGRAM_BOT_TOKEN, then ./scripts/telegram-mini-app.sh"
else
  echo "Telegram WebView: ./scripts/telegram-mini-app.sh"
fi
