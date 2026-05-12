#!/usr/bin/env bash
# Force an immediate refresh of the refreshable materialized views that
# back the task delivery pipeline. The migration that creates them
# (`20260429000000_task-delivery-silver-rewrite.sql`) intentionally does
# NOT trigger this — `SYSTEM REFRESH VIEW` is synchronous and would
# block the migration runner on large tenants.
#
# Run this script once after the migration finishes if you don't want to
# wait for the natural 1-hour tick. Safe to re-run; the worst case is a
# redundant recompute.
#
# Usage:
#   CH_HOST=clickhouse.svc CH_USER=default CH_PASSWORD=... ./refresh-task-views.sh
#
# Defaults are tuned for the in-cluster ClickHouse via a port-forward on
# localhost:9000 with no auth — adjust env vars for prod.

set -euo pipefail

CH_HOST="${CH_HOST:-localhost}"
CH_PORT="${CH_PORT:-9000}"
CH_USER="${CH_USER:-default}"
CH_PASSWORD="${CH_PASSWORD:-}"
CH_DATABASE="${CH_DATABASE:-insight}"

VIEWS=(
    "insight.task_issue_current_state"
    "insight.task_status_intervals"
)

CLIENT_ARGS=(
    --host "$CH_HOST"
    --port "$CH_PORT"
    --user "$CH_USER"
    --database "$CH_DATABASE"
)
if [[ -n "$CH_PASSWORD" ]]; then
    CLIENT_ARGS+=(--password "$CH_PASSWORD")
fi

for view in "${VIEWS[@]}"; do
    echo "Refreshing $view ..."
    clickhouse-client "${CLIENT_ARGS[@]}" --query "SYSTEM REFRESH VIEW $view"
    echo "  done"
done

echo "All task delivery views refreshed."
