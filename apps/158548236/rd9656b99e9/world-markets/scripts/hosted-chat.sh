#!/usr/bin/env bash
# Talk to the hosted World Markets agent (same plugin Telegram uses).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

BACKEND="${AOMI_BACKEND_URL:-https://chat-staging.aomi.dev}"
APP="${AOMI_APP:-world-markets}"
PLATFORM="${AOMI_PLATFORM:-world-market-apps}"
CLIENT_VERSION="${AOMI_CLIENT_VERSION:-0.6.9}"
WORLD_ACCOUNT_ID="${WORLD_HOSTED_ACCOUNT_ID:-19}"
WORLD_RPC_URL="${WORLD_RPC_URL:-https://testnet-unifi-rpc.puffer.fi/}"
WORLD_EXCHANGE_ADDRESS="${WORLD_EXCHANGE_ADDRESS:-0xf6b54e033bb45a583aa642924bcef78b804588ae}"
export AOMI_STATE_DIR="${AOMI_STATE_DIR:-$HOME/.aomi-world-markets}"

LIST_ONLY=0
RESOLVE_ONLY=0
PASS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)
      LIST_ONLY=1
      shift
      ;;
    --resolve-only)
      RESOLVE_ONLY=1
      shift
      ;;
    -h | --help)
      cat <<'EOF'
Usage: scripts/hosted-chat.sh [options] [-- aomi CLI args…]

Connects to the hosted World Markets agent (staging Telegram backend).
`world-market-apps` is the platform; the app name is `world-markets`.

  ./scripts/hosted-chat.sh
  ./scripts/hosted-chat.sh --prompt "how am I doing?"
  ./scripts/hosted-chat.sh --new-session
  ./scripts/hosted-chat.sh --list

Options:
  --list          Print hosted World Markets app ids and exit
  --resolve-only  Print the chosen application id and exit
  -h, --help      Show this help

Env (optional, .env is sourced when present):
  AOMI_BACKEND_URL       default https://chat-staging.aomi.dev
  AOMI_APP               default world-markets
  AOMI_PLATFORM          default world-market-apps
  AOMI_APPLICATION_ID    skip discovery and use this id
  WORLD_HOSTED_ACCOUNT_ID  default 19 (binds that account's owner wallet)
  WORLD_OWNER_WALLET     skip on-chain owner lookup
  AOMI_CLIENT_VERSION    default 0.6.9
  AOMI_STATE_DIR         default ~/.aomi-world-markets
EOF
      exit 0
      ;;
    --)
      PASS+=("${@:2}")
      break
      ;;
    *)
      PASS+=("$1")
      shift
      ;;
  esac
done

resolve_apps() {
  python3 - "$BACKEND" "$PLATFORM" "$APP" <<'PY'
import json, subprocess, sys, uuid

backend, platform, app = sys.argv[1:4]
backend = backend.rstrip("/")


def curl(method, path, token=None, data=None):
    cmd = [
        "curl",
        "-sS",
        "-f",
        "--max-time",
        "20",
        "-X",
        method,
        "-H",
        "accept: application/json",
        "-H",
        "content-type: application/json",
    ]
    if token:
        cmd.extend(
            [
                "-H",
                f"authorization: Bearer {token}",
                "-H",
                f"x-session-id: {uuid.uuid4()}",
            ]
        )
    cmd.append(backend + path)
    if data is not None:
        cmd.extend(["--data", json.dumps(data)])
    try:
        return json.loads(subprocess.check_output(cmd, text=True) or "{}")
    except subprocess.CalledProcessError as err:
        sys.stderr.write(f"{method} {path} failed (curl exit {err.returncode})\n")
        sys.exit(1)


guest = curl("POST", "/api/auth/sign-in/anonymous", data={})
token = guest.get("token") if isinstance(guest, dict) else None
if not token:
    sys.stderr.write("guest sign-in returned no bearer token\n")
    sys.exit(1)

apps = curl("GET", f"/api/thread/apps?platform={platform}", token=token)
if not isinstance(apps, list):
    sys.stderr.write("app list was not an array\n")
    sys.exit(1)

rows = []
for item in apps:
    if not isinstance(item, dict):
        continue
    name = str(item.get("name") or "")
    item_platform = str(item.get("platform") or "")
    if name != app or item_platform != platform:
        continue
    app_id = item.get("applicationId") or item.get("application_id")
    ready = item.get("artifactReady", item.get("artifact_ready"))
    status = item.get("artifactStatus") or item.get("artifact_status") or ""
    active = item.get("isActive", item.get("is_active"))
    rows.append(
        {
            "applicationId": app_id,
            "label": item.get("label") or name,
            "ready": bool(ready),
            "active": bool(active),
            "artifactStatus": status,
            "release": item.get("appReleaseTag")
            or item.get("app_release_tag")
            or "",
        }
    )

json.dump(rows, sys.stdout)
PY
}

choose_ready_id() {
  APPS_JSON="$1" python3 - "$APP" "$PLATFORM" <<'PY'
import json, os, sys

app, platform = sys.argv[1:3]
apps = json.loads(os.environ["APPS_JSON"])
ready = [
    row
    for row in apps
    if row.get("ready") and row.get("applicationId") is not None
]
if not ready:
    sys.stderr.write(f"no ready {app} artifact on {platform}\n")
    sys.exit(1)
ready.sort(key=lambda row: int(row["applicationId"]))
print(ready[-1]["applicationId"])
PY
}

print_app_table() {
  APPS_JSON="$1" python3 - "$APP" "$PLATFORM" <<'PY'
import json, os, sys

app, platform = sys.argv[1:3]
apps = json.loads(os.environ["APPS_JSON"])
if not apps:
    print(f"no {app} apps on platform {platform}")
    raise SystemExit(1)
print(f"{'id':<12} {'ready':<7} {'active':<7} label")
for row in apps:
    print(
        f"{str(row.get('applicationId') or '-'):<12} "
        f"{str(row.get('ready')):<7} "
        f"{str(row.get('active')):<7} "
        f"{row.get('label')}"
    )
PY
}

resolve_owner_wallet() {
  local account_id="$1"
  if [[ -n "${WORLD_OWNER_WALLET:-}" ]]; then
    printf '%s\n' "$WORLD_OWNER_WALLET"
    return
  fi
  # UniFi testnet account 19 owner (getUserAddress). Avoid a slow RPC on every launch.
  if [[ "$account_id" == "19" ]]; then
    printf '%s\n' "0x7742636CD4F174F9e89227842Dcd09c58ee0ca27"
    return
  fi
  if command -v cast >/dev/null 2>&1; then
    local owner
    owner="$(
      cast call "$WORLD_EXCHANGE_ADDRESS" \
        "getUserAddress(uint64)(address)" \
        "$account_id" \
        --rpc-url "$WORLD_RPC_URL" 2>/dev/null || true
    )"
    if [[ "$owner" =~ ^0x[0-9a-fA-F]{40}$ ]]; then
      printf '%s\n' "$owner"
      return
    fi
  fi
  echo "could not resolve owner wallet for World account $account_id" >&2
  echo "install foundry cast, or set WORLD_OWNER_WALLET" >&2
  exit 1
}

pass_has() {
  local flag="$1"
  local arg
  for arg in "${PASS[@]+"${PASS[@]}"}"; do
    [[ "$arg" == "$flag" || "$arg" == "$flag"=* ]] && return 0
  done
  return 1
}

if [[ "$LIST_ONLY" == "1" || -z "${AOMI_APPLICATION_ID:-}" || "$RESOLVE_ONLY" == "1" ]]; then
  APPS_JSON="$(resolve_apps)"
fi

if [[ "$LIST_ONLY" == "1" ]]; then
  print_app_table "$APPS_JSON"
  exit 0
fi

if [[ -n "${AOMI_APPLICATION_ID:-}" ]]; then
  APP_ID="$AOMI_APPLICATION_ID"
else
  APP_ID="$(choose_ready_id "$APPS_JSON")"
fi

if [[ -z "$APP_ID" ]]; then
  echo "could not resolve a hosted application id for $APP on $PLATFORM" >&2
  echo "pass AOMI_APPLICATION_ID or run: $0 --list" >&2
  exit 1
fi

OWNER_WALLET="$(resolve_owner_wallet "$WORLD_ACCOUNT_ID")"

echo "World Markets agent" >&2
echo "  backend  $BACKEND" >&2
echo "  app      $APP  (platform $PLATFORM)" >&2
echo "  id       $APP_ID" >&2
echo "  account  $WORLD_ACCOUNT_ID  ($OWNER_WALLET)" >&2
echo "  state    $AOMI_STATE_DIR" >&2
echo >&2

if [[ "$RESOLVE_ONLY" == "1" ]]; then
  printf '%s\n' "$APP_ID"
  exit 0
fi

AOMI_CMD=(
  npx --yes --silent "@aomi-labs/client@${CLIENT_VERSION}"
  --backend-url "$BACKEND"
  --app "$APP"
  --application-id "$APP_ID"
)
if ! pass_has --public-key; then
  AOMI_CMD+=(--public-key "$OWNER_WALLET")
fi
if [[ ${#PASS[@]} -gt 0 ]]; then
  AOMI_CMD+=("${PASS[@]}")
fi
exec "${AOMI_CMD[@]}"
