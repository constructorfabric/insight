#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

# KUBECONFIG can be empty when running in-cluster

export RECONCILE_DIR="${SCRIPT_DIR}/../reconcile-connectors"

echo "=== Resolving ClickHouse credentials ==="
CH_PASS="${CLICKHOUSE_PASSWORD:-$(kubectl get secret clickhouse-credentials -n data -o jsonpath='{.data.password}' | base64 -d)}"
if [[ -z "$CH_PASS" ]]; then
  echo "ERROR: clickhouse-credentials Secret not found or empty in namespace 'data'" >&2
  echo "  Run: ./secrets/apply.sh --infra-only" >&2
  exit 1
fi
export CLICKHOUSE_PASSWORD="$CH_PASS"

echo "=== Creating dbt databases (namespace=data) ==="
for db in staging silver insight; do
  if ! kubectl exec -n data deploy/clickhouse -- clickhouse-client --password "$CH_PASS" \
      --query "CREATE DATABASE IF NOT EXISTS ${db}" 2>&1 >/dev/null; then
    echo "ERROR: failed to create ${db} database (namespace=data)" >&2
    exit 1
  fi
done

echo "=== Creating bronze placeholders for missing connectors (namespace=data) ==="
"$SCRIPT_DIR/create-bronze-placeholders.sh"

echo "=== Running ClickHouse migrations (namespace=data) ==="
for migration in "$SCRIPT_DIR/migrations"/*.sql; do
  [ -f "$migration" ] || continue
  echo "  $(basename "$migration")"
  grep -v '^\s*--' "$migration" \
    | kubectl exec -i -n data deploy/clickhouse -- clickhouse-client --password "$CH_PASS" --multiquery
done

# MariaDB migrations: each backend service now owns and applies its own
# migrations at startup (SeaORM Migrator::up). See ADR-0006.
#
# NOTE: connector registration + connection apply are now handled by
# ../reconcile-connectors.sh (called from ../run-init.sh after this script
# finishes the migrations + dbt-database setup above). Do NOT add new
# `register.sh`/`connect.sh`-style invocations here — they were removed
# along with the legacy fan of scripts in the version-driven-reconcile
# refactor.

echo "=== Syncing workflows ==="
./scripts/sync-flows.sh --all
