#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

# KUBECONFIG can be empty when running in-cluster.

export RECONCILE_DIR="${SCRIPT_DIR}/../reconcile-connectors"

# Host preflight (yq / jq / kubectl / port-forward to airbyte-server) is
# no longer triggered from init.sh: connector registration / connection
# apply moved to the in-cluster reconcile loop (per ADR-0001), and the
# legacy fan of host scripts (register.sh, connect.sh, sync-state.sh,
# upload-manifests.sh) was removed along with airbyte-toolkit/. Operators
# running `reconcile-connectors/main.sh` from the host install yq / jq
# themselves; the toolbox image ships them pre-installed for cron pods.

# Single-namespace umbrella (PR #224). All Insight components — including the
# bundled ClickHouse StatefulSet — live in the release namespace, default
# `insight`. Exported so child scripts (airbyte-toolkit/*.sh, sync-flows.sh)
# inherit the value.
: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"
export INSIGHT_NAMESPACE
INSIGHT_NS="$INSIGHT_NAMESPACE"
CH_POD="${CLICKHOUSE_POD:-statefulset/insight-clickhouse}"  # RULE-DEFAULTS-OK: bundled umbrella deploys this exact StatefulSet name; override only for non-bundled CH

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

# Resolve the configured ClickHouse database name from the
# `insight-platform` ConfigMap (or CLICKHOUSE_DATABASE env override). The
# umbrella chart's `clickhouse.database` value drives both the bitnami
# subchart's CREATE DATABASE on first boot AND every consumer (Airbyte
# destination, analytics-api DSN, …) — keeping this loop in lock-step
# means a non-default `clickhouse.database` no longer breaks first-run
# init by silently creating the wrong DB. `staging` and `silver` are
# project-internal dbt schemas, those names are stable.
#
# Fail-fast: no silent default. If neither env var nor ConfigMap key is
# set, abort with a clear message instead of guessing `insight` and
# creating a database the rest of the platform won't use.
CH_DB="${CLICKHOUSE_DATABASE:-}"  # RULE-DEFAULTS-OK: empty sentinel; resolved from ConfigMap on next line, then asserted non-empty
if [[ -z "$CH_DB" ]]; then
  CH_DB=$(kubectl get configmap -n "$INSIGHT_NS" insight-platform \
    -o jsonpath='{.data.CLICKHOUSE_DATABASE}')
fi
: "${CH_DB:?CLICKHOUSE_DATABASE not resolvable: set the env var explicitly, or ensure the umbrella chart is installed and the insight-platform ConfigMap has CLICKHOUSE_DATABASE populated (mirrors clickhouse.database in chart values).}"

echo "=== Creating dbt databases (namespace=$INSIGHT_NS, app db=$CH_DB) ==="
for db in staging silver "$CH_DB"; do
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
# ../reconcile-connectors/main.sh (called from ../run-init.sh after this
# script finishes the migrations + dbt-database setup above). Do NOT add
# new `register.sh`/`connect.sh`-style invocations here — they were
# removed along with the legacy fan of scripts in the version-driven-
# reconcile refactor (ADR-0001).

echo "=== Syncing workflows ==="
./scripts/sync-flows.sh --all
