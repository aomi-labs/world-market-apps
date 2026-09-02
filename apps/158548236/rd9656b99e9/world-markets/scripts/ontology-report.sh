#!/usr/bin/env bash
# Operator-local ontology stats. Not a substitute for GET /dev/ontology.
set -euo pipefail
BRAIN="${WORLD_BRAIN_URL:-http://127.0.0.1:8788}"
ACCOUNT="${WORLD_ACCOUNT_ID:-}"
if [[ -z "$ACCOUNT" && -f .env ]]; then
  ACCOUNT="$(grep -E '^WORLD_ACCOUNT_ID=' .env | head -n1 | cut -d= -f2- | tr -d '"' || true)"
fi
echo "== summary =="
curl -sS "${BRAIN}/v1/ontology/summary"
echo
echo "== stats =="
if [[ -n "$ACCOUNT" ]]; then
  curl -sS "${BRAIN}/v1/ontology/stats?account_id=${ACCOUNT}"
else
  echo "set WORLD_ACCOUNT_ID for per-account stats; using operator-local all=1"
  curl -sS "${BRAIN}/v1/ontology/stats?all=1"
fi
echo
