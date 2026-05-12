#!/usr/bin/env bash
# Insight platform — DELETE the cluster and all data.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

ENV="${ENV:-local}"
CLUSTER_NAME="insight"

echo "=== WARNING: This will DELETE the cluster and ALL data ==="
read -p "Are you sure? [y/N] " -r
[[ "$REPLY" =~ ^[Yy]$ ]] || { echo "Cancelled."; exit 0; }

if [[ "$ENV" == "local" ]]; then
  echo "  Deleting Kind cluster '${CLUSTER_NAME}'..."
  kind delete cluster --name "${CLUSTER_NAME}" 2>/dev/null || true
  rm -f "${HOME}/.kube/insight.kubeconfig"
fi

# Kill port-forwards
pkill -f 'port-forward.*airbyte' 2>/dev/null || true

# Clean ingestion state
rm -f src/ingestion/airbyte-toolkit/state.yaml 2>/dev/null || true
# Clean generated tenant workflows (preserve templates/ and schedules/)
for d in src/ingestion/workflows/*/; do
  case "$(basename "$d")" in templates|schedules) continue;; esac
  rm -rf "$d" 2>/dev/null || true
done

echo "=== Cleaned. Run ./dev-up.sh for fresh install ==="
