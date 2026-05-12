#!/usr/bin/env bash
# Insight platform — stop all services (data preserved).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ENV="${ENV:-local}"
CLUSTER_NAME="insight"
KUBECONFIG_PATH="${KUBECONFIG:-${HOME}/.kube/insight.kubeconfig}"

if [[ "$ENV" == "local" ]]; then
  kind export kubeconfig --name "${CLUSTER_NAME}" --kubeconfig "${KUBECONFIG_PATH}" 2>/dev/null || true
fi

export KUBECONFIG="${KUBECONFIG_PATH}"

echo "=== Stopping services (data preserved) ==="

# Stop app services
echo "  Stopping Frontend & API Gateway..."
kubectl scale deployment -n insight --all --replicas=0 2>/dev/null || true

# Stop ingestion services
"$ROOT_DIR/src/ingestion/down.sh"

# Stop port-forwards
pkill -f 'port-forward.*airbyte' 2>/dev/null || true

# Stop Kind cluster (local only) — preserves all data inside
if [[ "$ENV" == "local" ]]; then
  echo "  Stopping Kind cluster..."
  docker stop "${CLUSTER_NAME}-control-plane" 2>/dev/null || true
fi

echo "=== Done (data preserved, run ./dev-up.sh to restart) ==="
