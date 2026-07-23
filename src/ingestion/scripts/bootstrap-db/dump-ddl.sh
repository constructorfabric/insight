#!/usr/bin/env bash
set -euo pipefail

: "${CLICKHOUSE_HOST:?CLICKHOUSE_HOST must be set}"
: "${CLICKHOUSE_PORT:?CLICKHOUSE_PORT must be set}"
: "${CLICKHOUSE_PROTOCOL:?CLICKHOUSE_PROTOCOL must be set (http or https)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONNECTORS_DIR="$(cd "${SCRIPT_DIR}/../../connectors" && pwd)"
DDL_DIR="${SCRIPT_DIR}/../connectors-ddl"

ch() {
  curl -sS --fail-with-body "${CLICKHOUSE_PROTOCOL}://${CLICKHOUSE_HOST}:${CLICKHOUSE_PORT}/" \
    -H "X-ClickHouse-User: ${CLICKHOUSE_USER}" \
    -H @<(printf 'X-ClickHouse-Key: %s' "${CLICKHOUSE_PASSWORD}") \
    --data-binary "$1"
}

connector_for_namespace() {
  local namespace="$1" descriptor
  for descriptor in "${CONNECTORS_DIR}"/*/*/descriptor.yaml; do
    if [[ "$(yq -r '.connection.namespace' "${descriptor}")" == "${namespace}" ]]; then
      yq -r '.name' "${descriptor}"
      return
    fi
  done
  echo "${namespace}"
}

dump_tables() {
  local database="$1" outfile="$2" table
  while IFS= read -r table; do
    [[ -n "${table}" ]] || continue
    ch "SHOW CREATE TABLE \`${database}\`.\`${table}\` FORMAT TSVRaw" \
      | sed -e '1s/^CREATE TABLE /CREATE TABLE IF NOT EXISTS /' >> "${outfile}"
    printf ';\n\n' >> "${outfile}"
  done < <(ch "SELECT name FROM system.tables
               WHERE database = '${database}' AND engine NOT IN ('View', 'MaterializedView')
                 AND name NOT LIKE '.inner%'
               ORDER BY name FORMAT TSVRaw")
}

dump_views() {
  local database="$1" outfile="$2" view
  while IFS= read -r view; do
    [[ -n "${view}" ]] || continue
    ch "SHOW CREATE TABLE \`${database}\`.\`${view}\` FORMAT TSVRaw" \
      | sed -e '1s/^CREATE VIEW /CREATE OR REPLACE VIEW /' \
            -e '1s/^CREATE MATERIALIZED VIEW /CREATE MATERIALIZED VIEW IF NOT EXISTS /' >> "${outfile}"
    printf ';\n\n' >> "${outfile}"
  done < <(ch "SELECT name FROM system.tables
               WHERE database = '${database}' AND engine IN ('View', 'MaterializedView')
               ORDER BY name FORMAT TSVRaw")
}

mkdir -p "${DDL_DIR}"
rm -f "${DDL_DIR}"/*.sql

while IFS= read -r database; do
  [[ -n "${database}" ]] || continue
  connector="$(connector_for_namespace "${database}")"
  outfile="${DDL_DIR}/${connector}.sql"
  echo "dumping ${database} -> $(basename "${outfile}")"
  printf 'CREATE DATABASE IF NOT EXISTS `%s`;\n\n' "${database}" > "${outfile}"
  dump_tables "${database}" "${outfile}"
done < <(ch "SELECT DISTINCT database FROM system.tables
             WHERE database LIKE 'bronze\\_%' ORDER BY database FORMAT TSVRaw")

# Dump one relation (table or view) with the right CREATE prefix.
dump_relation() {
  local database="$1" name="$2" outfile="$3" engine
  engine="$(ch "SELECT engine FROM system.tables
                WHERE database = '${database}' AND name = '${name}' FORMAT TSVRaw")"
  [[ -n "${engine}" ]] || { echo "  skip: ${database}.${name} not found" >&2; return; }
  if [[ "${engine}" == "View" || "${engine}" == "MaterializedView" ]]; then
    ch "SHOW CREATE TABLE \`${database}\`.\`${name}\` FORMAT TSVRaw" \
      | sed -e '1s/^CREATE VIEW /CREATE OR REPLACE VIEW /' \
            -e '1s/^CREATE MATERIALIZED VIEW /CREATE MATERIALIZED VIEW IF NOT EXISTS /' >> "${outfile}"
  else
    ch "SHOW CREATE TABLE \`${database}\`.\`${name}\` FORMAT TSVRaw" \
      | sed -e '1s/^CREATE TABLE /CREATE TABLE IF NOT EXISTS /' >> "${outfile}"
  fi
  printf ';\n\n' >> "${outfile}"
}

# person/identity precede silver/insight: the gold-view migrations reference
# person.persons and identity.aliases/identity_inputs, so keeping them in the
# snapshot lets create-bronze-placeholders.sh satisfy those on a fresh cluster
# (#1763). These are migration-owned tables (init-identity) and anti-join /
# full-refresh dbt models — safe to pre-create empty. create-bronze-placeholders
# applies them in its first batch (before silver at 900, insight at 950),
# resolving order via its retry loop.
for database in person identity silver insight; do
  outfile="${DDL_DIR}/${database}.sql"
  echo "dumping ${database} -> $(basename "${outfile}")"
  printf 'CREATE DATABASE IF NOT EXISTS `%s`;\n\n' "${database}" > "${outfile}"
  dump_tables "${database}" "${outfile}"
  dump_views "${database}" "${outfile}"
done

# staging: dump ONLY the tables the gold views (insight) / silver reference — NOT
# all of staging. Pre-creating an incremental staging model as an EMPTY table
# poisons its first real build: dbt sees the table, takes the is_incremental()
# path, and a model whose bound reads `max(<date>) FROM {{ this }}` reads the
# Date type-MAX over the empty MergeTree (CH 25.7 min-max metadata short-circuit)
# as the cutoff, excluding every row — permanently starving the model on a fresh
# cluster AND in the e2e. The handful of staging relations gold references (today
# just m365__collab_email_activity, whose bound reads the bronze source) are safe
# to pre-create; anything not referenced must not enter the snapshot.
staging_out="${DDL_DIR}/staging.sql"
echo "dumping staging (gold-referenced only) -> $(basename "${staging_out}")"
printf 'CREATE DATABASE IF NOT EXISTS `staging`;\n\n' > "${staging_out}"
while IFS= read -r tbl; do
  [[ -n "${tbl}" ]] || continue
  echo "  staging.${tbl}"
  dump_relation staging "${tbl}" "${staging_out}"
done < <(grep -ohrE 'staging\.[a-zA-Z0-9_]+' "${DDL_DIR}/insight.sql" "${DDL_DIR}/silver.sql" \
           | sed 's/^staging\.//' | sort -u)

ls -l "${DDL_DIR}"
