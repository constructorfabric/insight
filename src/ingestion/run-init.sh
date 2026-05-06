#!/usr/bin/env bash
# Initialize the ingestion stack: validate Secrets, adopt any pre-existing
# Airbyte resources, then drive the cluster to the descriptor-declared
# state via the single reconcile entrypoint.
#
# Runs from the host machine (requires kubectl, curl, python3).
# Run AFTER: helm install of the umbrella chart + ./secrets/apply.sh
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
cd "${SCRIPT_DIR}"

export KUBECONFIG="${KUBECONFIG:-${HOME}/.kube/insight.kubeconfig}"

# --- Verify infra Secrets exist ---
echo "=== Verifying secrets ==="
if ! kubectl get secret clickhouse-credentials -n data >/dev/null 2>&1; then
  echo "ERROR: clickhouse-credentials Secret not found in namespace 'data'" >&2
  echo "  Run: ./secrets/apply.sh" >&2
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
