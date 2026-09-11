#!/usr/bin/env bash
# Expose local world-mini-app over HTTPS and set this bot's Telegram menu button.
# Does not touch Aomi's webhook. Requires TELEGRAM_BOT_TOKEN in .env.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

MENU_TEXT="${TELEGRAM_MENU_TEXT:-Open}"
SKIP_STACK=0
MENU_URL=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --url)
      MENU_URL="${2:-}"
      shift 2
      ;;
    --menu-only)
      SKIP_STACK=1
      shift
      ;;
    -h | --help)
      cat <<'EOF'
Usage: scripts/telegram-mini-app.sh [--url https://host] [--menu-only]

Puts the Mini App on this Telegram bot:

  1. Starts brain + world-mini-app (unless --menu-only)
  2. Opens a Cloudflare HTTPS tunnel to :8080
  3. Calls setChatMenuButton so the chat shows Open (not only the paperclip)

Does not change the Aomi webhook.

Required in .env:
  TELEGRAM_BOT_TOKEN     BotFather token for this bot
Optional:
  WORLD_MINI_APP_URL     Skip the tunnel and use this origin
  TELEGRAM_CHAT_ID       Also send a View portfolio web_app message
  TELEGRAM_MENU_TEXT     Menu button label (default Open)
  WORLD_ACCOUNT_ID       Mini App portfolio/ledger account

Ctrl+C stops the tunnel and local stack. The menu button keeps the last URL
until you run this again or reset it in BotFather.
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

TOKEN="$(dev_env_val TELEGRAM_BOT_TOKEN)"
if [[ -z "$TOKEN" ]]; then
  echo "TELEGRAM_BOT_TOKEN is not set in .env" >&2
  echo "Open @BotFather → your bot → API Token, then add:" >&2
  echo "  TELEGRAM_BOT_TOKEN=123456:AAH..." >&2
  exit 1
fi

ensure_cloudflared() {
  if command -v cloudflared >/dev/null 2>&1; then
    return 0
  fi
  if ! command -v brew >/dev/null 2>&1; then
    echo "install cloudflared (https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/)" >&2
    exit 1
  fi
  echo "installing cloudflared…"
  brew install cloudflared
}

telegram_api() {
  local method="$1"
  local body="$2"
  curl -sfS "https://api.telegram.org/bot${TOKEN}/${method}" \
    -H 'Content-Type: application/json' \
    -d "$body"
}

set_menu_button() {
  local origin="$1"
  origin="${origin%/}"
  local payload
  payload="$(python3 - "$MENU_TEXT" "$origin" <<'PY'
import json, sys
text, origin = sys.argv[1], sys.argv[2]
print(json.dumps({
    "menu_button": {
        "type": "web_app",
        "text": text[:16],
        "web_app": {"url": origin + "/"},
    }
}))
PY
)"
  local result
  result="$(telegram_api setChatMenuButton "$payload")"
  python3 -c "import json,sys; d=json.loads(sys.argv[1]);
raise SystemExit(0 if d.get('ok') else 1)" "$result" || {
    echo "setChatMenuButton failed: $result" >&2
    exit 1
  }
}

ping_chat() {
  local origin="$1"
  local chat_id
  chat_id="$(dev_env_val TELEGRAM_CHAT_ID)"
  [[ -n "$chat_id" ]] || return 0
  origin="${origin%/}"
  local payload
  payload="$(python3 - "$chat_id" "$origin" <<'PY'
import json, sys
chat_id, origin = sys.argv[1], sys.argv[2]
print(json.dumps({
    "chat_id": int(chat_id) if chat_id.lstrip("-").isdigit() else chat_id,
    "text": "View portfolio",
    "disable_notification": True,
    "reply_markup": {
        "inline_keyboard": [[{
            "text": "View portfolio",
            "web_app": {"url": origin + "/"},
        }]]
    },
}))
PY
)"
  telegram_api sendMessage "$payload" >/dev/null
  echo "sent View portfolio button to chat $chat_id"
}

wait_tunnel_url() {
  local log="$1"
  local i=0
  local url=""
  while ((i < 40)); do
    url="$(python3 - "$log" <<'PY'
import re, sys
from pathlib import Path
text = Path(sys.argv[1]).read_text(errors="ignore")
m = re.search(r"https://[a-z0-9-]+\.trycloudflare\.com", text)
print(m.group(0) if m else "")
PY
)"
    if [[ -n "$url" ]]; then
      printf '%s\n' "$url"
      return 0
    fi
    sleep 0.25
    i=$((i + 1))
  done
  echo "cloudflared did not print a trycloudflare URL; see $log" >&2
  cat "$log" >&2 || true
  exit 1
}

ORIGIN="${MENU_URL:-$(dev_env_val WORLD_MINI_APP_URL)}"
TUNNEL_PID=""
TUNNEL_LOG="${TMPDIR:-/tmp}/world-markets-cloudflared.log"

cleanup() {
  if [[ -n "${TUNNEL_PID:-}" ]]; then
    kill "$TUNNEL_PID" 2>/dev/null || true
  fi
  dev_cleanup
}
trap cleanup EXIT INT TERM

if [[ "$SKIP_STACK" -eq 0 ]]; then
  DEV_SKIP_SIDECAR=1
  dev_plan_sidecar
  if [[ -z "$(dev_env_val WORLD_ACCOUNT_ID)" ]]; then
    echo "WORLD_ACCOUNT_ID is not set — portfolio/ledger will fail" >&2
  fi
  if ! dev_http_ok "$BRAIN_HEALTH_URL"; then
    dev_install_npm
    dev_start_brain
    dev_wait_http "brain sidecar" "$BRAIN_HEALTH_URL" "$BRAIN_PID" "$BRAIN_LOG" 50
  else
    echo "brain already on ${BRAIN_HOST}:${BRAIN_PORT}"
    BRAIN_PID=""
  fi
  if ! dev_http_ok "$MINI_HEALTH_URL"; then
    dev_build_rust
    dev_start_mini_app background
    dev_wait_mini_app
  else
    echo "mini-app already on ${MINI_HOST}:${MINI_PORT}"
    MINI_PID=""
  fi
fi

if [[ -z "$ORIGIN" ]]; then
  ensure_cloudflared
  : >"$TUNNEL_LOG"
  cloudflared tunnel --no-autoupdate --url "http://${MINI_HOST}:${MINI_PORT}" \
    >"$TUNNEL_LOG" 2>&1 &
  TUNNEL_PID=$!
  ORIGIN="$(wait_tunnel_url "$TUNNEL_LOG")"
fi

ORIGIN="${ORIGIN%/}"
export WORLD_MINI_APP_URL="$ORIGIN"
set_menu_button "$ORIGIN"
ping_chat "$ORIGIN"

echo
echo "Mini App is on this Telegram bot."
echo "  origin     $ORIGIN"
echo "  menu       $MENU_TEXT  (replaces the / command button, not the paperclip)"
echo
echo "In the Telegram thread: look left of the message box for \"$MENU_TEXT\","
echo "or tap the bot name → Open."
echo

if [[ -n "$TUNNEL_PID" || -n "${MINI_PID:-}" || -n "${BRAIN_PID:-}" ]]; then
  echo "Press Ctrl+C to stop the tunnel / local stack."
  if [[ -n "$TUNNEL_PID" ]]; then
    wait "$TUNNEL_PID"
  else
    wait
  fi
fi
