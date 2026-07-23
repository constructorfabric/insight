#!/usr/bin/env bash
# Build a full ClickHouse from real connectors + dbt + migrations, in the same
# order a fresh cluster converges to — used both for local dev and to
# regenerate the committed connectors-ddl snapshot (see dump-ddl.sh).
#
# Order matters (this is the fix for #1831/#1763):
#   1. Core databases + person/identity schema (init-identity migration).
#      The identity dbt models (seed_persons_*, seed_aliases_*) ANTI-JOIN
#      person.persons, so that table MUST exist BEFORE dbt runs — otherwise
#      those models fail and identity/person tables never make it into the
#      snapshot.
#   2. Connectors -> bronze (+ per-connector bronze_promoted).
#   3. dbt run (all): staging + silver + dbt-owned gold.
#   4. Gold-view migrations (apply-ch-migrations.sh): CREATE OR REPLACE the
#      migration-owned gold views on top of the silver dbt just built, then
#      rebuild dbt tag:gold. The snapshot applicator (create-bronze-
#      placeholders.sh) is SKIPPED here (BOOTSTRAP_SKIP_SNAPSHOT=1) — during
#      generation the on-disk snapshot is stale and would pollute the dump.
set -euo pipefail

CONFIG_FILE="${1:?usage: bootstrap-db.sh <connectors-config.yaml>}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MIGRATIONS_DIR="$(cd "${SCRIPT_DIR}/../migrations" && pwd)"

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

# run_ch (lib/ch-exec.sh) fans a multi-statement SQL block out statement-by-
# statement over the HTTP interface (CH runs one statement per request).
export CLICKHOUSE_URL="${CLICKHOUSE_PROTOCOL}://${CLICKHOUSE_HOST}:${CLICKHOUSE_PORT}"
source "${SCRIPT_DIR}/../lib/ch-exec.sh"

echo "=== 1. Core databases + identity/person schema (init-identity) ==="
run_ch <<SQL
CREATE DATABASE IF NOT EXISTS staging;
CREATE DATABASE IF NOT EXISTS silver;
CREATE DATABASE IF NOT EXISTS ${CLICKHOUSE_DATABASE};
SQL
# person.persons + identity.aliases must exist before the identity dbt models
# (LEFT ANTI JOIN person.persons). init-identity is the first, idempotent
# migration (CREATE DATABASE/TABLE IF NOT EXISTS).
run_ch < "${MIGRATIONS_DIR}/20260408000000_init-identity.sql"

echo "=== 2. Creating connector tables (bronze + promote) ==="
"${SCRIPT_DIR}/seed-connectors.sh" "${CONFIG_FILE}"

echo "=== 3. Running all dbt models ==="
"${SCRIPT_DIR}/run-dbt.sh" || echo "dbt run finished with errors, continuing" >&2

echo "=== 4. Applying ClickHouse migrations (gold views) + dbt gold ==="
# Snapshot applicator is a no-op here (generation mode); the real relations
# already exist from steps 1-3. apply-ch-migrations re-runs init-identity
# (no-op), applies the gold-view migrations, and heals warm-cluster schemas.
# SKIP_DBT_GOLD: step 3 already built every tag:gold model with the pinned dbt
# venv; skip apply-ch-migrations' own gold build, which would need a `dbt` on
# PATH (absent / dbt-fusion outside the prod toolbox).
export BOOTSTRAP_SKIP_SNAPSHOT=1
export SKIP_DBT_GOLD=1
bash "${SCRIPT_DIR}/../apply-ch-migrations.sh"
