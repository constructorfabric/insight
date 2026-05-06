#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

# KUBECONFIG can be empty when running in-cluster

export RECONCILE_DIR="${SCRIPT_DIR}/../reconcile-connectors"

# Single-namespace umbrella (PR #224). All Insight components — including the
# bundled ClickHouse StatefulSet — live in the release namespace, default
# `insight`. Exported so child scripts (airbyte-toolkit/*.sh, sync-flows.sh)
# inherit the value.
export INSIGHT_NAMESPACE="${INSIGHT_NAMESPACE:-insight}"
INSIGHT_NS="$INSIGHT_NAMESPACE"
CH_POD="${CLICKHOUSE_POD:-statefulset/insight-clickhouse}"

# clickhouse-client inside the StatefulSet pod inherits CLICKHOUSE_USER /
# CLICKHOUSE_PASSWORD from the container env (set by the chart from
# auth.existingSecret), so we do not pass --user / --password.
ch_exec() {
  kubectl exec -n "$INSIGHT_NS" "$CH_POD" -- clickhouse-client "$@"
}
ch_exec_stdin() {
  kubectl exec -i -n "$INSIGHT_NS" "$CH_POD" -- clickhouse-client "$@"
}

echo "=== Verifying ClickHouse pod ==="
if ! kubectl get -n "$INSIGHT_NS" "$CH_POD" >/dev/null 2>&1; then
  echo "ERROR: ClickHouse not found at -n $INSIGHT_NS $CH_POD" >&2
  echo "  Ensure the umbrella chart is installed with clickhouse.deploy=true" >&2
  echo "  (helm list -n $INSIGHT_NS | grep insight)" >&2
  exit 1
fi

echo "=== Creating dbt databases (namespace=$INSIGHT_NS) ==="
for db in staging silver insight; do
  if ! ch_exec --query "CREATE DATABASE IF NOT EXISTS $db"; then
    echo "ERROR: failed to create $db database (namespace=$INSIGHT_NS)" >&2
    exit 1
  fi
done

echo "=== Creating bronze placeholders for missing connectors (namespace=$INSIGHT_NS) ==="
"$SCRIPT_DIR/create-bronze-placeholders.sh"

echo "=== Running ClickHouse migrations (namespace=$INSIGHT_NS) ==="
for migration in "$SCRIPT_DIR/migrations"/*.sql; do
  [ -f "$migration" ] || continue
  echo "  $(basename "$migration")"
  # `sed` instead of `grep -v` so a comment-only migration (matching every
  # line) does not return exit 1 and abort the pipeline under `set -o pipefail`.
  sed -E '/^[[:space:]]*--/d' "$migration" | ch_exec_stdin --multiquery
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
