#!/usr/bin/env bash
# Build a full ClickHouse from real connectors + dbt + migrations, in the same
# order a fresh cluster converges to — used both for local dev and to
# regenerate the committed connectors-ddl snapshot (see dump-ddl.sh).
#
# Order matters (this is the fix for #1831/#1763):
#   1. Databases + access (create-databases.sh, the one creation site).
#   2. Connectors -> bronze (ReplacingMergeTree via append_dedup).
#   3. dbt run (all): staging + silver + dbt-owned gold.
#   4. Migrations (apply-ch-migrations.sh): the schema migrations and the
#      warm-cluster heals, on top of what dbt just built. The snapshot
#      applicator (create-bronze-placeholders.sh) is SKIPPED here
#      (BOOTSTRAP_SKIP_SNAPSHOT=1) — during generation the on-disk snapshot is
#      stale and would pollute the dump.
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

echo "=== 1. Databases + access ==="
bash "${SCRIPT_DIR}/../create-databases.sh"

echo "=== 2. Creating connector tables (bronze) ==="
"${SCRIPT_DIR}/seed-connectors.sh" "${CONFIG_FILE}"

echo "=== 3. Running all dbt models ==="
# Fail loud: a partial dbt run means missing staging/silver/gold relations, and
# this is their only build — step 4 runs no dbt. A swallowed failure would
# silently ship an incomplete connectors-ddl snapshot while bootstrap still
# "succeeds". A complete dbt run is a precondition for the dump, so let its
# non-zero exit abort under `set -e`.
"${SCRIPT_DIR}/run-dbt.sh"

echo "=== 4. Applying ClickHouse migrations ==="
# Snapshot applicator is a no-op here (generation mode); the real relations
# already exist from steps 1-3. apply-ch-migrations re-creates the databases
# (no-op), applies the migrations, and heals warm-cluster schemas. It needs no
# `dbt` on PATH — which outside the prod toolbox (local dev, the connectors-ddl
# CI workflow) is absent or the wrong build (dbt-fusion 2.0).
export BOOTSTRAP_SKIP_SNAPSHOT=1
bash "${SCRIPT_DIR}/../apply-ch-migrations.sh"
