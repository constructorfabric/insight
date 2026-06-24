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

# Single-namespace umbrella (PR #224). Exported so child scripts
# (reconcile-connectors/*.sh, sync-flows.sh) inherit the value.
: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"
export INSIGHT_NAMESPACE

# ClickHouse migrations + dbt databases (staging/silver/app) + bronze/silver
# placeholders are NOT applied here anymore. They moved to the
# clickhouse-migrate Helm Hook Job
# (charts/insight/templates/clickhouse-migrate-job.yaml), which applies them
# over HTTP against the external ClickHouse on every install/upgrade. The
# bundled-CH `kubectl exec` path this script used died with the
# insight-clickhouse StatefulSet in #1428.
#
# MariaDB migrations: each backend service owns and applies its own at
# startup (SeaORM Migrator::up). See ADR-0006.
#
# Connector registration + connection apply: ../reconcile-connectors/main.sh
# (called from ../run-init.sh). Do NOT add register.sh/connect.sh-style
# invocations here — removed in the version-driven-reconcile refactor (ADR-0001).

echo "=== Syncing workflows ==="
./scripts/sync-flows.sh --all
