#!/usr/bin/env bash
set -euo pipefail

# KUBECONFIG must be set by the caller (up.sh sets it)
# KUBECONFIG can be empty when running in-cluster

# Single-namespace umbrella (PR #224). Override via INSIGHT_NAMESPACE.
INSIGHT_NS="${INSIGHT_NAMESPACE:-insight}"
TIMEOUT=180

echo "  Waiting for ClickHouse..."
kubectl wait --for=condition=ready pod \
  -l app.kubernetes.io/name=clickhouse,app.kubernetes.io/instance=insight \
  -n "$INSIGHT_NS" \
  --timeout="${TIMEOUT}s"

echo "  Waiting for Argo Workflows..."
kubectl wait --for=condition=ready pod \
  -l app.kubernetes.io/name=argo-workflows-server -n "$INSIGHT_NS" \
  --timeout="${TIMEOUT}s"

echo "  Waiting for Airbyte..."
# `airbyte-server` is the chart's actual pod label
# (app.kubernetes.io/name=airbyte-server, see deploy/gitops/system/airbyte);
# the previous `name=server` selector matched no pods and silently fell
# through to the curl localhost:8001 fallback below — which only works
# when the caller has already started a port-forward, an undocumented
# requirement that masked the real wait.
kubectl wait --for=condition=ready pod \
  -l app.kubernetes.io/name=airbyte-server -n "$INSIGHT_NS" \
  --timeout="${TIMEOUT}s" 2>/dev/null || {
  # Fallback: poll health endpoint
  printf "  Polling Airbyte health..."
  for i in $(seq 1 60); do
    if curl -sf "http://localhost:8001/api/v1/health" >/dev/null 2>&1; then
      echo " ok"
      break
    fi
    sleep 3
  done
}

echo "  All services ready"
