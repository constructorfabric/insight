#!/usr/bin/env bash
# Move a warm install's stored tenant from its legacy key to the canonical
# tenant UUID (deploy/TENANT_IDENTITY_UPGRADE.md §3.4).
#
#   migrate-tenant-identity.sh --canonical-tenant <uuid> --legacy-tenant-key <key> [--dry-run]
#
# Rewrites the tenant column of every table whose rows a dbt full refresh cannot
# reproduce: Bronze (`bronze_*`), operator config (`config`), the sync ledger
# (`ingestion_history`) and the dbt models tagged `history`
# (tenant-history-tables.txt). `unique_key` is never touched: rows keep the
# legacy key segment, connectors keep writing it through `insight_tenant_key`
# and dbt through `tenant_key()`, so ReplacingMergeTree still collapses old and
# new rows. Every other Staging, Silver and Gold relation is rebuilt by a dbt
# full refresh afterwards.
#
# Refuses before changing anything when a table holds a tenant that is neither
# of the two: that install keeps data under several tenants and needs its own
# plan. Idempotent — a second run changes nothing.
#
# Required env: CLICKHOUSE_URL, CLICKHOUSE_USER, CLICKHOUSE_PASSWORD.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/ch-exec.sh
source "${SCRIPT_DIR}/lib/ch-exec.sh"

UUID_RE='^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
DATABASES_SQL="(database LIKE 'bronze\\_%' OR database IN ('config', 'ingestion_history'))"
HISTORY_TABLES_FILE="${SCRIPT_DIR}/tenant-history-tables.txt"

usage() {
  echo "usage: $0 --canonical-tenant <uuid> --legacy-tenant-key <key> [--dry-run]" >&2
  exit 64
}

canonical="" legacy="" dry_run=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --canonical-tenant) [[ $# -ge 2 ]] || usage; canonical="$2"; shift 2 ;;
    --legacy-tenant-key) [[ $# -ge 2 ]] || usage; legacy="$2"; shift 2 ;;
    --dry-run) dry_run=1; shift ;;
    *) usage ;;
  esac
done
[[ "${canonical}" =~ ${UUID_RE} ]] || { echo "--canonical-tenant must be a lowercase UUID, got '${canonical}'" >&2; exit 64; }
[[ -n "${legacy}" && "${legacy}" != *"'"* && "${legacy}" != *'\'* ]] \
  || { echo "--legacy-tenant-key must be non-empty and contain no quote or backslash" >&2; exit 64; }
[[ "${legacy}" != "${canonical}" ]] || { echo "the legacy key equals the canonical tenant: nothing to migrate" >&2; exit 64; }

query() { printf '%s' "$1" | _ch_http_query; }

history_sql="$(grep -v '^[[:space:]]*\(#\|$\)' "${HISTORY_TABLES_FILE}" \
  | awk -F. '{ printf "%s(\x27%s\x27, \x27%s\x27)", (NR > 1 ? ", " : ""), $1, $2 }')"
[[ -n "${history_sql}" ]] || { echo "no tables listed in ${HISTORY_TABLES_FILE}" >&2; exit 1; }

# database \t table \t tenant column \t engine \t 1 when that column is in the
# sorting or partition key. dbt history models name it tenant_id or, for a few
# silver-shaped ones, insight_tenant_id; a UUID-typed column is already canonical.
tables="$(query "
  SELECT c.database, c.table, c.name, t.engine,
         toUInt8(has(splitByRegexp('[^A-Za-z0-9_]+', concat(t.sorting_key, ' ', t.partition_key)), c.name))
  FROM system.columns AS c
  INNER JOIN system.tables AS t ON t.database = c.database AND t.name = c.table
  WHERE t.engine LIKE '%MergeTree'
    AND ((c.name = 'tenant_id' AND ${DATABASES_SQL//database/c.database})
      OR (c.name IN ('tenant_id', 'insight_tenant_id') AND c.type NOT LIKE '%UUID%'
          AND (c.database, c.table) IN (${history_sql})))
  ORDER BY c.database, c.table
  FORMAT TSV")"

refused=0
while IFS=$'\t' read -r db tbl col _engine _keyed; do
  [[ -n "${db}" ]] || continue
  others="$(query "SELECT arrayStringConcat(groupUniqArray(10)(toString(${col})), ', ')
    FROM ${db}.\`${tbl}\`
    WHERE ${col} IS NOT NULL AND toString(${col}) NOT IN ('', '${legacy}', '${canonical}')
    FORMAT TSV")"
  if [[ -n "${others}" ]]; then
    echo "REFUSED ${db}.${tbl}: holds other tenants (${others})" >&2
    refused=1
  fi
done <<<"${tables}"
[[ "${refused}" -eq 0 ]] || { echo "nothing changed: this install holds more than one tenant" >&2; exit 1; }

total=0
while IFS=$'\t' read -r db tbl col engine keyed; do
  [[ -n "${db}" ]] || continue
  n="$(ch_scalar "SELECT count() FROM ${db}.\`${tbl}\` WHERE toString(${col}) = '${legacy}'")"
  total=$((total + n))
  if [[ "${n}" -eq 0 ]]; then
    printf '%-60s %10s\n' "${db}.${tbl}.${col}" "0"
    continue
  fi
  if [[ "${keyed}" == "1" && "${engine}" != *Replacing* ]]; then
    echo "REFUSED ${db}.${tbl}: ${col} is in the ${engine} key, and copying rows there is not idempotent" >&2
    exit 1
  fi
  printf '%-60s %10s%s\n' "${db}.${tbl}.${col}" "${n}" "$([[ "${dry_run}" -eq 1 ]] && echo '  (dry run)')"
  [[ "${dry_run}" -eq 0 ]] || continue
  if [[ "${keyed}" == "1" ]]; then
    # ALTER UPDATE cannot rewrite a key column: copy under the new tenant, then
    # drop the originals. A rerun after a failure re-copies onto the same keys.
    run_ch <<SQL
INSERT INTO ${db}.\`${tbl}\` SELECT * REPLACE ('${canonical}' AS ${col}) FROM ${db}.\`${tbl}\` WHERE toString(${col}) = '${legacy}';
ALTER TABLE ${db}.\`${tbl}\` DELETE WHERE toString(${col}) = '${legacy}' SETTINGS mutations_sync = 2
SQL
  else
    run_ch <<SQL
ALTER TABLE ${db}.\`${tbl}\` UPDATE ${col} = '${canonical}' WHERE toString(${col}) = '${legacy}' SETTINGS mutations_sync = 2
SQL
  fi
done <<<"${tables}"

without="$(query "
  SELECT arrayStringConcat(groupArray(concat(database, '.', name)), ', ')
  FROM system.tables
  WHERE ${DATABASES_SQL} AND engine LIKE '%MergeTree'
    AND (database, name) NOT IN (SELECT database, table FROM system.columns WHERE name = 'tenant_id')
  FORMAT TSV")"
[[ -z "${without}" ]] || echo "tables without a tenant_id column (not migrated): ${without}"
echo "rows under '${legacy}': ${total}$([[ "${dry_run}" -eq 1 ]] && echo ' (dry run: nothing changed)')"
