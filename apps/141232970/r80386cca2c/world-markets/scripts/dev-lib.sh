# Shared helpers for scripts/dev-*.sh. Source from repo root; do not execute.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  echo "source scripts/dev-lib.sh from another dev script" >&2
  exit 1
fi

[[ -n "${DEV_LIB_LOADED:-}" ]] && return 0
DEV_LIB_LOADED=1

DEV_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

dev_cd_root() {
  cd "$DEV_ROOT"
}

dev_require_env() {
  if [[ ! -f .env ]]; then
    echo "copy .env.example to .env and set WORLD_ACCOUNT_ID, OPENROUTER_API_KEY, MINI_APP_DEV_BYPASS=1" >&2
    exit 1
  fi
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
}

dev_env_val() {
  local key="$1"
  local default="${2:-}"
  if [[ -n "${!key:-}" ]]; then
    printf '%s' "${!key}"
  else
    printf '%s' "$default"
  fi
}

dev_configure_ports() {
  BRAIN_PORT="$(dev_env_val WORLD_BRAIN_PORT 8788)"
  BRAIN_HOST="$(dev_env_val WORLD_BRAIN_HOST 127.0.0.1)"
  EXEC_PORT="$(dev_env_val WORLD_EXECUTION_PORT 8787)"
  EXEC_HOST="$(dev_env_val WORLD_EXECUTION_HOST 127.0.0.1)"
  MINI_BIND="$(dev_env_val MINI_APP_BIND 127.0.0.1:8080)"
  MINI_HOST="${MINI_BIND%%:*}"
  MINI_PORT="${MINI_BIND##*:}"
  if [[ "$MINI_HOST" == "$MINI_PORT" ]]; then
    MINI_HOST="127.0.0.1"
    MINI_PORT="$MINI_BIND"
  fi
  MINI_PREVIEW_URL="http://${MINI_HOST}:${MINI_PORT}/?preview=dev"
  MINI_CHART_URL="http://${MINI_HOST}:${MINI_PORT}/chart?symbol=AAPL&period=d&preview=dev"
  MINI_HEALTH_URL="http://${MINI_HOST}:${MINI_PORT}/api/v1/mini-app/health"
  BRAIN_HEALTH_URL="http://${BRAIN_HOST}:${BRAIN_PORT}/health"
  EXEC_HEALTH_URL="http://${EXEC_HOST}:${EXEC_PORT}/health"
}

dev_http_ok() {
  curl -sf --max-time 1 "$1" >/dev/null 2>&1
}

dev_wait_http() {
  local name="$1"
  local url="$2"
  local pid="$3"
  local log="$4"
  local attempts="${5:-60}"
  local i=0
  while ((i < attempts)); do
    if dev_http_ok "$url"; then
      return 0
    fi
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "$name exited before becoming healthy; see $log" >&2
      cat "$log" >&2 || true
      exit 1
    fi
    sleep 0.2
    i=$((i + 1))
  done
  echo "$name did not become healthy; see $log" >&2
  cat "$log" >&2 || true
  exit 1
}

dev_install_npm() {
  if [[ ! -d brain/node_modules ]]; then
    echo "installing brain dependencies…"
    (cd brain && npm install)
  fi
  if [[ "${DEV_START_SIDECAR:-0}" -eq 1 ]] && [[ ! -d sidecar/node_modules ]]; then
    echo "installing execution sidecar dependencies…"
    (cd sidecar && npm install)
  fi
}

dev_build_rust() {
  echo "building plugin and mini-app…"
  cargo build --lib -p world-markets
  cargo build -p world-mini-app
}

dev_plugin_path() {
  local plugin="$DEV_ROOT/target/debug/libworld_markets.dylib"
  if [[ ! -f "$plugin" ]]; then
    plugin="$DEV_ROOT/target/debug/libworld_markets.so"
  fi
  if [[ ! -f "$plugin" ]]; then
    echo "plugin not built; run cargo build" >&2
    exit 1
  fi
  printf '%s' "$plugin"
}

dev_llm_provider() {
  local key
  key="$(dev_env_val AOMI_PROVIDER)"
  if [[ -n "$key" ]]; then
    printf '%s' "$key"
    return
  fi
  if [[ -n "$(dev_env_val OPENROUTER_API_KEY)" ]]; then
    printf '%s' "openrouter"
  elif [[ -n "$(dev_env_val OPENAI_API_KEY)" ]]; then
    printf '%s' "openai"
  elif [[ -n "$(dev_env_val ANTHROPIC_API_KEY)" ]]; then
    printf '%s' "anthropic"
  else
    printf '%s' "openrouter"
  fi
}

dev_preflight_ports() {
  if dev_http_ok "$BRAIN_HEALTH_URL"; then
    echo "brain already listening on ${BRAIN_HOST}:${BRAIN_PORT} — stop it or set WORLD_BRAIN_PORT" >&2
    exit 1
  fi
  if dev_http_ok "$MINI_HEALTH_URL"; then
    echo "mini-app already on ${MINI_HOST}:${MINI_PORT} — stop it or set MINI_APP_BIND" >&2
    exit 1
  fi
  if [[ "${DEV_START_SIDECAR:-0}" -eq 1 ]] && dev_http_ok "$EXEC_HEALTH_URL"; then
    echo "execution sidecar already on ${EXEC_HOST}:${EXEC_PORT} — stop it or set WORLD_EXECUTION_PORT" >&2
    exit 1
  fi
}

dev_plan_sidecar() {
  DEV_START_SIDECAR=0
  if [[ "${DEV_SKIP_SIDECAR:-0}" -eq 1 ]]; then
    return
  fi
  if [[ -n "$(dev_env_val WORLD_PRIVATE_KEY)" ]]; then
    DEV_START_SIDECAR=1
  fi
}

dev_start_brain() {
  BRAIN_LOG="${TMPDIR:-/tmp}/world-markets-brain.log"
  (cd brain && npm start) >"$BRAIN_LOG" 2>&1 &
  BRAIN_PID=$!
}

dev_start_sidecar() {
  if [[ "${DEV_START_SIDECAR:-0}" -ne 1 ]]; then
    EXEC_PID=""
    return
  fi
  EXEC_LOG="${TMPDIR:-/tmp}/world-markets-sidecar.log"
  (cd sidecar && npm start) >"$EXEC_LOG" 2>&1 &
  EXEC_PID=$!
}

dev_start_mini_app() {
  local mode="${1:-background}"
  MINI_LOG="${TMPDIR:-/tmp}/world-markets-mini-app.log"
  if [[ "$mode" == "foreground" ]]; then
    exec cargo run -p world-mini-app
  fi
  cargo run -p world-mini-app >"$MINI_LOG" 2>&1 &
  MINI_PID=$!
}

dev_wait_sidecars() {
  dev_wait_http "brain sidecar" "$BRAIN_HEALTH_URL" "$BRAIN_PID" "$BRAIN_LOG" 50
  if [[ -n "${EXEC_PID:-}" ]]; then
    dev_wait_http "execution sidecar" "$EXEC_HEALTH_URL" "$EXEC_PID" "$EXEC_LOG" 75
  else
    echo "execution sidecar skipped (no WORLD_PRIVATE_KEY or --no-sidecar)"
  fi
}

dev_wait_mini_app() {
  dev_wait_http "mini-app" "$MINI_HEALTH_URL" "$MINI_PID" "$MINI_LOG" 75
}

dev_cleanup() {
  local pids=()
  [[ -n "${BRAIN_PID:-}" ]] && pids+=("$BRAIN_PID")
  [[ -n "${EXEC_PID:-}" ]] && pids+=("$EXEC_PID")
  [[ -n "${MINI_PID:-}" ]] && pids+=("$MINI_PID")
  if [[ ${#pids[@]} -gt 0 ]]; then
    kill "${pids[@]}" 2>/dev/null || true
  fi
}

dev_open_mini_app_tabs() {
  if command -v open >/dev/null; then
    open "$MINI_PREVIEW_URL"
    open "$MINI_CHART_URL"
  fi
}

dev_print_stack_banner() {
  local mode="${1:-mini}"
  cat <<EOF

Local stack is up:
  agent CLI       aomi-run (this terminal when using dev-full.sh)
  UI (portfolio)  ${MINI_PREVIEW_URL}
  UI (chart)      ${MINI_CHART_URL}
  brain           ${BRAIN_HEALTH_URL}
EOF
  if [[ -n "${EXEC_PID:-}" ]]; then
    echo "  execution       ${EXEC_HEALTH_URL}"
  fi
  if [[ "$mode" == "mini" ]]; then
    echo "  agent log       ${TMPDIR:-/tmp}/world-markets-agent.log (hold-to-talk --prompt bridge)"
  fi
  cat <<EOF

Logs:
  brain           ${BRAIN_LOG}
EOF
  if [[ -n "${EXEC_PID:-}" ]]; then
    echo "  execution       ${EXEC_LOG}"
  fi
  if [[ -n "${MINI_PID:-}" ]]; then
    echo "  mini-app        ${MINI_LOG}"
  fi
}
