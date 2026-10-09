#!/usr/bin/env bash
# Move a warm install's stored tenant from its legacy key to the canonical
# tenant UUID (deploy/TENANT_IDENTITY_UPGRADE.md §3.4).
#
#   migrate-tenant-identity.sh --canonical-tenant <uuid> --legacy-tenant-key <key> [--dry-run]
#
# Rewrites `tenant_id` in every table of the databases connectors and operators
# write — Bronze (`bronze_*`), operator config (`config`) and the sync ledger
# (`ingestion_history`). `unique_key` is never touched: rows keep the legacy key
# segment, and connectors keep writing it through `insight_tenant_key`, so
# ReplacingMergeTree still collapses old and new rows. Staging, Silver and Gold
# are derived and are rebuilt by a dbt full refresh afterwards.
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

# database \t table \t engine \t 1 when tenant_id is in the sorting or partition key
tables="$(query "
  SELECT c.database, c.table, t.engine,
         toUInt8(hasAny(splitByRegexp('[^A-Za-z0-9_]+', concat(t.sorting_key, ' ', t.partition_key)), ['tenant_id']))
  FROM system.columns AS c
  INNER JOIN system.tables AS t ON t.database = c.database AND t.name = c.table
  WHERE c.name = 'tenant_id' AND t.engine LIKE '%MergeTree'
    AND ${DATABASES_SQL//database/c.database}
  ORDER BY c.database, c.table
  FORMAT TSV")"

refused=0
while IFS=$'\t' read -r db tbl _engine _keyed; do
  [[ -n "${db}" ]] || continue
  others="$(query "SELECT arrayStringConcat(groupUniqArray(10)(toString(tenant_id)), ', ')
    FROM ${db}.\`${tbl}\`
    WHERE tenant_id IS NOT NULL AND toString(tenant_id) NOT IN ('', '${legacy}', '${canonical}')
    FORMAT TSV")"
  if [[ -n "${others}" ]]; then
    echo "REFUSED ${db}.${tbl}: holds other tenants (${others})" >&2
    refused=1
  fi
done <<<"${tables}"
[[ "${refused}" -eq 0 ]] || { echo "nothing changed: this install holds more than one tenant" >&2; exit 1; }

total=0
while IFS=$'\t' read -r db tbl engine keyed; do
  [[ -n "${db}" ]] || continue
  n="$(ch_scalar "SELECT count() FROM ${db}.\`${tbl}\` WHERE toString(tenant_id) = '${legacy}'")"
  total=$((total + n))
  if [[ "${n}" -eq 0 ]]; then
    printf '%-60s %10s\n' "${db}.${tbl}" "0"
    continue
  fi
  if [[ "${keyed}" == "1" && "${engine}" != *Replacing* ]]; then
    echo "REFUSED ${db}.${tbl}: tenant_id is in the ${engine} key, and copying rows there is not idempotent" >&2
    exit 1
  fi
  printf '%-60s %10s%s\n' "${db}.${tbl}" "${n}" "$([[ "${dry_run}" -eq 1 ]] && echo '  (dry run)')"
  [[ "${dry_run}" -eq 0 ]] || continue
  if [[ "${keyed}" == "1" ]]; then
    # ALTER UPDATE cannot rewrite a key column: copy under the new tenant, then
    # drop the originals. A rerun after a failure re-copies onto the same keys.
    run_ch <<SQL
INSERT INTO ${db}.\`${tbl}\` SELECT * REPLACE ('${canonical}' AS tenant_id) FROM ${db}.\`${tbl}\` WHERE toString(tenant_id) = '${legacy}';
ALTER TABLE ${db}.\`${tbl}\` DELETE WHERE toString(tenant_id) = '${legacy}' SETTINGS mutations_sync = 2
SQL
  else
    run_ch <<SQL
ALTER TABLE ${db}.\`${tbl}\` UPDATE tenant_id = '${canonical}' WHERE toString(tenant_id) = '${legacy}' SETTINGS mutations_sync = 2
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
