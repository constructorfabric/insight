#!/usr/bin/env bash
# Initialize the ingestion stack: validate the umbrella install, run dbt
# database setup + ClickHouse migrations, adopt any pre-existing Airbyte
# resources, then drive the cluster to the descriptor-declared state via
# the single reconcile entrypoint.
#
# Runs from the host machine (requires kubectl, curl, python3).
# Run AFTER: helm install of the umbrella chart + ./secrets/apply.sh
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
cd "${SCRIPT_DIR}"

: "${KUBECONFIG:?KUBECONFIG must be set to your cluster kubeconfig path}"
: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"
export KUBECONFIG

# Single-namespace umbrella (PR #224). All Insight components live in the
# release namespace.
INSIGHT_NS="${INSIGHT_NAMESPACE}"

# --- Verify the umbrella is installed ---
echo "=== Verifying umbrella install ==="
if ! kubectl get -n "$INSIGHT_NS" statefulset/insight-clickhouse >/dev/null 2>&1; then
  echo "ERROR: insight-clickhouse StatefulSet not found in namespace '$INSIGHT_NS'" >&2
  echo "  Run: ./dev-compose.sh up  (or: cd deploy/gitops && make deploy ENV=local)" >&2
  exit 1
fi
if ! kubectl get -n "$INSIGHT_NS" secret insight-db-creds >/dev/null 2>&1; then
  echo "ERROR: insight-db-creds Secret not found in namespace '$INSIGHT_NS'" >&2
  echo "  The umbrella chart should have created it on install." >&2
  exit 1
fi

# --- Migrations + dbt databases (still managed by scripts/init.sh) ---
source ./scripts/init.sh

# --- Single declarative reconcile chain ---
# Per ADR-0007 / KEY DECISION #13: Secret validation is now an INTERNAL pre-step
# of reconcile-connectors/main.sh (valsec_check_secret), not a standalone script.
# 1. one-shot adopt: annotate any pre-existing Airbyte resources so the
#    new cfg-hash / version invariants hold before the diff pass
# 2. reconcile: descriptor.yaml + Secret-driven, idempotent
echo "=== Adopting pre-existing Airbyte resources ==="
bash "${SCRIPT_DIR}/reconcile-connectors/main.sh" adopt

echo "=== Reconciling Airbyte to descriptor state ==="
bash "${SCRIPT_DIR}/reconcile-connectors/main.sh"

echo "=== Init complete ==="
