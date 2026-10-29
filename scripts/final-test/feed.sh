#!/usr/bin/env bash
# Feeds one stack of the before/after test (README.md here) with NEW's oc-tracegen, the same settings
# for both stacks:
#
#   scripts/final-test/feed.sh <old|new> load     metrics load profile (Stage 1 baseline: 4 orgs,
#                                                 80,000 + 3 x 3,300 series, 20 labels, flush 18 s,
#                                                 churn 5 % / 10 min, 10 % OTLP, seed 1)
#   scripts/final-test/feed.sh <old|new> traces   oc-tracegen's demo traces and their logs, into load-org-1
#   scripts/final-test/feed.sh <old|new> k8s      Kubernetes objects (k8s-objects.mjs), into load-org-1
#   scripts/final-test/feed.sh <old|new> stop <load|traces|k8s>
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
state="$root/.ft.local"
version="${1:?old or new}"
action="${2:?action}"
case "$version" in
  old) p=182 ;;
  new) p=183 ;;
  *) echo "version must be old or new" >&2; exit 2 ;;
esac
tracegen="$root/server/target/release/oc-tracegen"
intake="http://localhost:${p}10"

start() { # name, command...
  local what="$1"; shift
  local pidfile="$state/$version-feed-$what.pid"
  if [[ -f "$pidfile" ]] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
    echo "$version feed $what already running (pid $(cat "$pidfile"))"; return
  fi
  nohup "$@" >>"$state/$version-feed-$what.log" 2>&1 &
  echo $! >"$pidfile"
  echo "$version feed $what started (pid $!)"
}

case "$action" in
  load)
    start load env OTLP_ENDPOINT="$intake" OC_LOAD_ORGS=4 OC_LOAD_LARGE_SERIES=80000 OC_LOAD_SMALL_SERIES=3300 \
      OC_LOAD_LABELS=20 OC_LOAD_FLUSH_SECS=18 OC_LOAD_CHURN_FRACTION=0.05 OC_LOAD_CHURN_MINUTES=10 \
      OC_LOAD_OTLP_SHARE=0.1 OC_LOAD_SEED=1 RUST_LOG=info "$tracegen" load
    ;;
  traces)
    # TRACES_PER_SEC as given; CLICKHOUSE_HTTP: its CI / profile backfill checks read this stack only.
    start traces env OTLP_ENDPOINT="$intake" OC_INGEST_KEY=ocik_load_org_1_local_only \
      TRACES_PER_SEC="${TRACES_PER_SEC:-20}" CLICKHOUSE_HTTP="http://localhost:${p}01" \
      CLICKHOUSE_USER=observecraft CLICKHOUSE_PASSWORD=observecraft RUST_LOG=warn "$tracegen"
    ;;
  k8s)
    start k8s node "$here/k8s-objects.mjs" --intake "$intake" --key ocik_load_org_1_local_only
    ;;
  stop)
    what="${3:?load, traces or k8s}"
    pidfile="$state/$version-feed-$what.pid"
    if [[ -f "$pidfile" ]]; then
      pid="$(cat "$pidfile")"
      kill "$pid" 2>/dev/null || true
      rm -f "$pidfile"
      echo "$version feed $what stopped (pid $pid)"
    fi
    ;;
  *) echo "unknown action $action" >&2; exit 2 ;;
esac
