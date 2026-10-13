#!/usr/bin/env bash
# staging.jira__catalogue_first_seen was a plain MergeTree keyed by source. The
# model is now a ReplacingMergeTree keyed by tenant and source that refuses a
# full refresh, and dbt never re-creates a relation that already stands, so the
# old table is rebuilt into the new shape here. Its rows exist nowhere else —
# bronze forgets the first extraction as its parts merge — so this is a copy,
# counted against the original, swapped in, counted again, and only then is the
# original dropped. The tenant comes from the source's bronze rows; a source
# bronze no longer holds keeps an empty tenant.
#
# Guarded on the exact engine the topology needs, so a converged warehouse pays
# one SELECT and a table left on the wrong family is rebuilt. Column order and
# types are the model's: dbt appends to this table positionally.
#
# Sourced by apply-ch-migrations.sh after lib/ch-exec.sh.

_catalogue_source_count() {
  ch_scalar "SELECT uniqExact(insight_source_id) FROM staging.$1"
}

# SAFETY: an unreadable probe must stop the heal. Read as "no bronze", it would
# copy every source with an empty tenant, and the swapped-in table would then
# pass the engine guard on every later deploy.
_catalogue_tenants_select() {
  local bronze
  bronze="$(ch_scalar "SELECT count() FROM system.tables WHERE database = 'bronze_jira' AND name = 'jira_fields'")" || return 1
  if [[ "${bronze}" -eq 0 ]]; then
    echo "SELECT '' AS insight_source_id, '' AS tenant_id WHERE 0"
    return 0
  fi
  echo "SELECT DISTINCT COALESCE(source_id, '') AS insight_source_id, COALESCE(tenant_id, '') AS tenant_id
          FROM bronze_jira.jira_fields"
}

heal_jira_catalogue_first_seen() {
  local table="staging.jira__catalogue_first_seen" copy="staging.jira__catalogue_first_seen__rebuild"
  local engine converged before tenants copied live

  ch_table_exists staging jira__catalogue_first_seen || return 0
  engine="$(ch_engine ReplacingMergeTree)"
  converged="$(ch_scalar "SELECT count() FROM system.tables
    WHERE database = 'staging' AND name = 'jira__catalogue_first_seen' AND engine = '${engine}'")" || return 1
  [[ "${converged}" -eq 0 ]] || return 0

  before="$(_catalogue_source_count jira__catalogue_first_seen)" || return 1
  tenants="$(_catalogue_tenants_select)" || return 1
  echo "  ${table}: rebuilding ${before} source(s) as a ${engine} keyed by tenant and source"
  run_ch <<SQL || return 1
DROP TABLE IF EXISTS ${copy};
CREATE TABLE ${copy}
(
    unique_key String,
    tenant_id String,
    insight_source_id String,
    catalogue_first_sync DateTime64(3),
    recorded_at DateTime64(3),
    _version UInt64
)
ENGINE = ${engine}(_version)
ORDER BY unique_key
SETTINGS replicated_deduplication_window = 0;
INSERT INTO ${copy}
SELECT
    concat(t.tenant_id, '-', c.insight_source_id),
    t.tenant_id,
    c.insight_source_id,
    c.catalogue_first_sync,
    c.recorded_at,
    toUInt64(4102444800000 - toUnixTimestamp64Milli(c.catalogue_first_sync))
FROM
(
    SELECT insight_source_id, min(catalogue_first_sync) AS catalogue_first_sync, min(recorded_at) AS recorded_at
    FROM ${table}
    GROUP BY insight_source_id
) AS c
LEFT JOIN (${tenants}) AS t ON t.insight_source_id = c.insight_source_id;
SQL

  copied="$(_catalogue_source_count jira__catalogue_first_seen__rebuild)" || return 1
  if [[ "${copied}" -lt "${before}" ]]; then
    echo "  ${table}: copy holds ${copied} source(s), expected at least ${before} — leaving the table as it is" >&2
    run_ch <<SQL
DROP TABLE IF EXISTS ${copy};
SQL
    return 1
  fi

  run_ch <<SQL || return 1
EXCHANGE TABLES ${table} AND ${copy};
SQL
  if ! live="$(_catalogue_source_count jira__catalogue_first_seen)" || [[ "${live}" -lt "${before}" ]]; then
    echo "  ${table}: rebuilt table holds ${live:-?} source(s), expected ${before} — swapping the original back" >&2
    run_ch <<SQL
EXCHANGE TABLES ${table} AND ${copy};
SQL
    return 1
  fi

  run_ch <<SQL
DROP TABLE IF EXISTS ${copy};
SQL
}
