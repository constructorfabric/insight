#!/usr/bin/env bash
# Build a full ClickHouse from real connectors + dbt + migrations, in the same
# order a fresh cluster converges to — used both for local dev and to
# stand a throwaway warehouse up from the connectors and dbt.
#
# Order matters (this is the fix for #1831/#1763):
#   1. Databases + access (create-databases.sh, the one creation site).
#   2. Connectors -> bronze (ReplacingMergeTree via append_dedup).
#   3. dbt run (all): staging + silver + dbt-owned gold.
#   4. Migrations (apply-ch-migrations.sh): the schema migrations and the
#      warm-cluster heals, on top of what dbt just built.
set -euo pipefail

CONFIG_FILE="${1:?usage: bootstrap-db.sh <connectors-config.yaml>}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

set -a
source "${SCRIPT_DIR}/pins.env"
set +a

if [[ -f "${SCRIPT_DIR}/.env" ]]; then
  set -a
  source "${SCRIPT_DIR}/.env"
  set +a
fi

: "${CLICKHOUSE_HOST:?CLICKHOUSE_HOST must be set}"
: "${CLICKHOUSE_PORT:?CLICKHOUSE_PORT must be set}"
: "${CLICKHOUSE_PROTOCOL:?CLICKHOUSE_PROTOCOL must be set (http or https)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"
: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set}"

# Every step below talks to ClickHouse through lib/ch-exec.sh, which selects
# its backend from CLICKHOUSE_URL.
export CLICKHOUSE_URL="${CLICKHOUSE_PROTOCOL}://${CLICKHOUSE_HOST}:${CLICKHOUSE_PORT}"
# The topology every step reads: create-databases.sh and the migrations through
# lib/ch-topology.sh, the bronze destination through compose_destination_config.py.
# Unset is a standalone server.
export CLICKHOUSE_CLUSTER_MODE="${CLICKHOUSE_CLUSTER_MODE:-false}"
export CLICKHOUSE_CLUSTER_NAME="${CLICKHOUSE_CLUSTER_NAME:-}"

echo "=== 1. Databases + access ==="
bash "${SCRIPT_DIR}/../create-databases.sh"

echo "=== 2. Creating connector tables (bronze) ==="
"${SCRIPT_DIR}/seed-connectors.sh" "${CONFIG_FILE}"

echo "=== 3. Running all dbt models ==="
# Fail loud: a partial dbt run means missing staging/silver/gold relations, and
# this is their only build — step 4 runs no dbt. A swallowed failure would leave
# an incomplete warehouse behind while bootstrap still "succeeds", so let its
# non-zero exit abort under `set -e`.
"${SCRIPT_DIR}/run-dbt.sh"

echo "=== 4. Applying ClickHouse migrations ==="
# apply-ch-migrations re-creates the databases (no-op), applies the migrations,
# and heals warm-cluster schemas on top of what dbt just built. It needs no
# `dbt` on PATH — which outside the prod toolbox (local dev, a CI workflow) is
# absent or the wrong build (dbt-fusion 2.0).
bash "${SCRIPT_DIR}/../apply-ch-migrations.sh"
