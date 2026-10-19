#!/usr/bin/env bash
# Create every ClickHouse database this release owns, and provision the access
# that reads them.
#
# THE creation site. Nine places used to issue `CREATE DATABASE` — the deploy
# hook, three numbered migrations, the connectors-ddl snapshot headers, three
# dbt on-run-start hooks, the chart's pre-install Job and the compose bootstrap.
# They all route here now, and every one of them may assume its database
# already stands.
#
# Epic #2010 puts the cluster fork on this one statement: inside a `Replicated`
# database every later DDL is logged in Keeper and replayed on every replica,
# so no `ON CLUSTER` clause is needed anywhere else. That holds only while this
# stays the only site. Replaying the DDL is not replicating the rows, though —
# a table still states its own `Replicated*` engine, which is why the dbt
# models and the numbered migrations render a prefix from the same flag.
#
# Bronze is not here and will not be. `bronze_*` is created by
# destination-clickhouse at sync time from the connector's own catalogue, and
# its topology travels with the destination config.
#
# The grants run from here on purpose: a role is granted on a database by name
# long before that database exists (see bootstrap-db/presentation-role.sql), so
# running them together is the only way the two are read together.
#
# Idempotent — `IF NOT EXISTS` throughout, every provisioning script re-runnable
# — so a warm upgrade re-runs this as a no-op.
#
# Required env (same contract as apply-ch-migrations.sh, via lib/ch-exec.sh):
#   CLICKHOUSE_URL       e.g. http://ch-host:8123
#   CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
#   CLICKHOUSE_DATABASE  the Insight app database
#
# Optional env:
#   CLICKHOUSE_CLUSTER_MODE  true -> the DDL emitted here replicates
#   CLICKHOUSE_CLUSTER_NAME  the cluster the ON CLUSTER clause names
#   MCP_ENABLED / SQL_API_ENABLED  provision the MCP SQL-explorer reader
#   INSIGHT_V3_CORE_ENABLED  provision the custom assistant's reader
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set (the Insight app database)}"

source "$SCRIPT_DIR/lib/ch-topology.sh"

# The app database holds gold and is named by the operator; the rest are fixed.
# `staging` and `silver` are the dbt layers, `identity` the resolver's inputs
# and its persons mirror, `config` the operator-authored mappings dbt reads as
# sources, `presentation` the writable namespace for saved results (#1964),
# `product_usage` adoption events (#2573), `ingestion_history` what the sweep
# copies out of the data mover, `insight_datasets` the record tables
# insight-v3-core declares a dataset into.
DATABASES=(
  "${CLICKHOUSE_DATABASE}"
  staging
  silver
  identity
  config
  presentation
  product_usage
  ingestion_history
  insight_datasets
)

# Nothing is created or granted until the server can serve the topology the
# install declared: Keeper for the database engine below, a replicated access
# backend for the roles further down, and no database already standing with the
# wrong engine. A standalone install is checked too, in the one direction that
# loses data (see lib/ch-topology.sh).
ch_assert_topology "${DATABASES[@]}"

ON_CLUSTER="$(ch_on_cluster)"

echo "=== Creating databases${ON_CLUSTER:+ (${ON_CLUSTER})} ==="
for database in "${DATABASES[@]}"; do
  echo "  ${database}"
  printf 'CREATE DATABASE IF NOT EXISTS `%s` %s %s' \
    "${database}" "${ON_CLUSTER}" "$(ch_database_engine "${database}")" | run_ch
done

echo "=== Provisioning presentation access (role + grant-less user) (#1963/#1964) ==="
bash "$SCRIPT_DIR/bootstrap-db/provision-presentation-access.sh"

# Gated like the MCP reader below (#3558): `insight_v3_ro` reads every table,
# and an installation that never runs the custom assistant has no use for a
# role that can.
if [[ "${INSIGHT_V3_CORE_ENABLED:-false}" == "true" ]]; then
  echo "=== Provisioning insight-v3-core query access (grant-less reader) ==="
  bash "$SCRIPT_DIR/bootstrap-db/provision-v3-access.sh"
else
  echo "=== insight-v3-core disabled; stripping the assistant's read ==="
  bash "$SCRIPT_DIR/bootstrap-db/revoke-v3-access.sh"
fi

if [[ "${MCP_ENABLED:-false}" == "true" || "${SQL_API_ENABLED:-false}" == "true" ]]; then
  echo "=== Provisioning MCP SQL explorer access ==="
  bash "$SCRIPT_DIR/bootstrap-db/provision-mcp-access.sh"
else
  echo "=== MCP SQL explorer disabled; skipping access provisioning ==="
fi

echo "=== Provisioning grafana access (SELECT-only role + grant-less user) (#2888) ==="
bash "$SCRIPT_DIR/bootstrap-db/provision-grafana-access.sh"
