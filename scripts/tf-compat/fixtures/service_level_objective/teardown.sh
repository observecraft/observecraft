#!/usr/bin/env bash
# Deletes the monitor setup.sh created (our API deletes monitors in bulk).
set -euo pipefail
dir="$1"
[ -f "$dir/monitor.id" ] || exit 0
curl -fsS -o /dev/null -X POST "$TF_VAR_oc_api_url/api/v1/monitors/bulk" \
  -H "Authorization: Bearer $TF_VAR_oc_app_key" -H 'Content-Type: application/json' \
  -d "{\"ids\":[$(cat "$dir/monitor.id")],\"action\":\"delete\"}"
rm -f "$dir/monitor.id"
