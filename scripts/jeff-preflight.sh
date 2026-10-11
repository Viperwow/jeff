#!/usr/bin/env bash
# Quick jeff install + server check for agents. Exit 0 only if binary and /health OK.
set -euo pipefail

jeff_bin=""
if command -v jeff >/dev/null 2>&1; then
  jeff_bin="$(command -v jeff)"
elif [ -x "./target/release/jeff" ]; then
  jeff_bin="./target/release/jeff"
elif [ -x "./target/debug/jeff" ]; then
  jeff_bin="./target/debug/jeff"
fi

api="${JEFF_URL:-http://127.0.0.1:8080}"
admin="${JEFF_ADMIN_URL:-http://127.0.0.1:8081}"

ok=true
report() { printf '%s\n' "$1"; }

if [ -z "$jeff_bin" ]; then
  report '{"ok":false,"jeff":"missing","hint":"install jeff or cargo build --release"}'
  exit 1
fi

version="$("$jeff_bin" --version 2>/dev/null || true)"
health="down"
if curl -sf "${api%/}/health" >/dev/null 2>&1; then
  health="up"
else
  ok=false
fi

auth="skipped"
if [ -n "${JEFF_API_KEY:-}" ]; then
  if curl -sf -H "Authorization: Bearer ${JEFF_API_KEY}" "${api%/}/v1/models" >/dev/null 2>&1; then
    auth="ok"
  else
    auth="failed"
    ok=false
  fi
fi

admin_reach="skipped"
if curl -sf "${admin%/}/health" >/dev/null 2>&1; then
  admin_reach="up"
elif [ "$health" = "up" ]; then
  admin_reach="down"
fi

report "$(printf '{"ok":%s,"jeff":"%s","version":"%s","api":"%s","health":"%s","auth":"%s","admin":"%s"}' \
  "$([ "$ok" = true ] && echo true || echo false)" \
  "$jeff_bin" \
  "$version" \
  "$api" \
  "$health" \
  "$auth" \
  "$admin_reach")"

[ "$ok" = true ]
