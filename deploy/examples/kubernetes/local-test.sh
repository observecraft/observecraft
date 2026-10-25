#!/usr/bin/env bash
# Sends a local Kubernetes cluster's logs, metrics and objects to the ObserveCraft running on this
# machine (make up / server / intake), with demo workloads.
#
#   KUBE_CONTEXT=orbstack OBSERVECRAFT_API_KEY=ocik_… ./local-test.sh        # or kind-<name>, docker-desktop
#   KUBE_CONTEXT=orbstack ./local-test.sh delete
#
# The context is required and used for every command, so the current kubectl context (possibly a
# real cluster) is never touched.
set -euo pipefail
: "${KUBE_CONTEXT:?set KUBE_CONTEXT to a local cluster context (orbstack, kind-…, docker-desktop)}"
case "$KUBE_CONTEXT" in
  orbstack|kind-*|docker-desktop|k3d-*|minikube) ;;
  *) echo "Refusing to use '$KUBE_CONTEXT': this script is for local clusters only." >&2; exit 1 ;;
esac
k() { kubectl --context "$KUBE_CONTEXT" "$@"; }
here="$(cd "$(dirname "$0")" && pwd)"

if [ "${1:-}" = "delete" ]; then
  k delete -f "$here/demo-workloads.yaml" --ignore-not-found
  k delete -f "$here/observecraft-collector.yaml" --ignore-not-found
  exit 0
fi

: "${OBSERVECRAFT_API_KEY:?create one under Organization Settings → API Keys}"
# Pods reach the host's intake (port 4318) through the runtime's host alias.
ENDPOINT="${OBSERVECRAFT_ENDPOINT:-http://host.docker.internal:4318}"
CLUSTER_NAME="${CLUSTER_NAME:-local-$KUBE_CONTEXT}"

k apply -f "$here/observecraft-collector.yaml"
k -n observecraft create secret generic observecraft \
  --from-literal=api-key="$OBSERVECRAFT_API_KEY" --from-literal=endpoint="$ENDPOINT" --from-literal=cluster-name="$CLUSTER_NAME" \
  --dry-run=client -o yaml | k apply -f -
k -n observecraft rollout restart daemonset/observecraft-agent deployment/observecraft-cluster
k apply -f "$here/demo-workloads.yaml"
k -n observecraft rollout status daemonset/observecraft-agent --timeout=180s
k -n observecraft rollout status deployment/observecraft-cluster --timeout=180s
echo "Sending to $ENDPOINT as cluster '$CLUSTER_NAME'. Open Infrastructure → Kubernetes in ObserveCraft."
