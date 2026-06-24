#!/usr/bin/env bash
# Apply the ClickHouse gold-view migrations against an EXTERNAL ClickHouse.
#
# This is the in-cluster, network-mode counterpart to the ClickHouse half
# of scripts/init.sh. init.sh `kubectl exec`s into a bundled CH StatefulSet
# (retired in #1428 when the umbrella stopped bundling L2 infra), so it
# cannot reach an external CH. This script talks to CH over its HTTP
# interface via lib/ch-exec.sh (selected by CLICKHOUSE_URL) and is invoked
# by the clickhouse-migrate Helm Hook Job (post-install,post-upgrade).
#
# Steps (same order and contract as init.sh):
#   1. Create the core databases (staging, silver, app db).
#   2. Run create-bronze-placeholders.sh — minimum-viable bronze/silver
#      stubs so gold-view CREATE VIEW type-checks on a fresh cluster
#      (CH validates referenced tables at parse time). See ADR-0007.
#   3. Apply migrations/*.sql in lexicographic order.
#
# Bookkeeping: none — every migration is re-run on every invocation and
# MUST stay idempotent/re-runnable (CREATE OR REPLACE / IF NOT EXISTS).
# This matches the existing init.sh contract (see ingestion DESIGN §migrations).
#
# Required env (set by the Hook Job from chart values + insight-db-creds):
#   CLICKHOUSE_URL       e.g. http://ch-host:8123  (selects the HTTP backend)
#   CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
#   CLICKHOUSE_DATABASE  the Insight app database
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

: "${CLICKHOUSE_URL:?CLICKHOUSE_URL must be set (e.g. http://ch-host:8123)}"
: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set (the Insight app database)}"

source "$SCRIPT_DIR/lib/ch-exec.sh"

echo "=== Creating core databases (staging, silver, ${CLICKHOUSE_DATABASE}) ==="
run_ch <<SQL
CREATE DATABASE IF NOT EXISTS staging;
CREATE DATABASE IF NOT EXISTS silver;
CREATE DATABASE IF NOT EXISTS ${CLICKHOUSE_DATABASE};
SQL

echo "=== Creating bronze/silver placeholders (ADR-0007) ==="
bash "$SCRIPT_DIR/create-bronze-placeholders.sh"

echo "=== Applying ClickHouse migrations ==="
shopt -s nullglob
for migration in "$SCRIPT_DIR/migrations"/*.sql; do
  echo "  $(basename "$migration")"
  run_ch < "$migration"
done

echo "=== ClickHouse migrations complete ==="
