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

for database in silver insight; do
  outfile="${DDL_DIR}/${database}.sql"
  echo "dumping ${database} -> $(basename "${outfile}")"
  printf 'CREATE DATABASE IF NOT EXISTS `%s`;\n\n' "${database}" > "${outfile}"
  dump_tables "${database}" "${outfile}"
  dump_views "${database}" "${outfile}"
done

ls -l "${DDL_DIR}"
