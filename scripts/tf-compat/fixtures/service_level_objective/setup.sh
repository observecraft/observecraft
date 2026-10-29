#!/usr/bin/env bash
# Creates the monitor the monitor SLO of this fixture refers to, through ObserveCraft's own API (the
# Datadog-compatible datadog_monitor resource comes with Stage 2 of IMPLEMENTATION_PLAN.md), and
# hands its id to OpenTofu in monitor.auto.tfvars. teardown.sh deletes it.
set -euo pipefail
dir="$1"
id="$(curl -fsS -X POST "$TF_VAR_oc_api_url/api/v1/monitors" \
  -H "Authorization: Bearer $TF_VAR_oc_app_key" -H 'Content-Type: application/json' \
  -d '{"name":"tf-compat checkout errors","type":"metric","query":"sum:trace.http.request.errors{service:order-service}",
       "options":{"detection":"threshold","timeAggregation":"sum","windowMinutes":5,"comparator":"above","thresholds":{"critical":50}},
       "tags":["tf-compat"]}' | sed -E 's/.*"id":([0-9]+).*/\1/')"
case "$id" in '' | *[!0-9]*) echo "could not create the monitor" >&2; exit 1 ;; esac
echo "$id" >"$dir/monitor.id"
echo "monitor_id = $id" >"$dir/monitor.auto.tfvars"
