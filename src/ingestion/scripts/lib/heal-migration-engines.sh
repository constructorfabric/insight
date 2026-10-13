#!/usr/bin/env bash
# Tables a numbered migration creates were once created with a literal plain
# engine. Inside a Replicated database that replays the DDL on every replica but
# never the rows: each replica keeps its own. `CREATE TABLE IF NOT EXISTS` never
# changes the engine of a table that stands, so a clustered install that already
# holds one is rebuilt here on the engine `ch_engine` names.
#
# The copy reads every replica, since each holds rows the others never saw; it
# is counted against the original, swapped in, counted again, and only then is
# the original dropped. A single node already has the engine it needs, so there
# this costs one probe per table.
#
# Sourced by apply-ch-migrations.sh after lib/ch-exec.sh.

# INVARIANT: every table a migration creates with `{{ch_engine:...}}` is listed
# here — tests/test_migration_engines_follow_the_topology.py holds the two equal.
MIGRATION_ENGINE_TABLES=(
  product_usage.usage_events
  ingestion_history.sync_events
  silver.class_person_absences
  silver.class_ai_credit_usage
)

# One column of the table's system.tables row, or failure on an empty answer.
_table_attribute() {
  local db="$1" table="$2" column="$3" value
  value="$(printf "SELECT %s FROM system.tables WHERE database = '%s' AND name = '%s' FORMAT TSVRaw" \
    "${column}" "${db}" "${table}" | _ch_http_query)" || return 1
  [[ -n "${value}" ]] || return 1
  printf '%s' "${value}"
}

# Distinct sorting-key tuples: a merge collapses rows of a ReplacingMergeTree
# but never a key, so a copy that merged mid-count still compares equal.
_key_count() {
  local relation="$1" key="$2"
  ch_scalar "SELECT uniqExact(${key}) FROM ${relation} SETTINGS select_sequential_consistency = 1"
}

heal_migration_engine() {
  local db="${1%%.*}" table="${1#*.}"
  local relation="$1" copy="$1__replicated"
  local engine needed engine_full key cluster replicas before copied live

  ch_table_exists "${db}" "${table}" || return 0
  engine="$(_table_attribute "${db}" "${table}" engine)" || return 1
  [[ "${engine}" == Replicated* ]] && return 0
  needed="$(ch_engine "${engine}")"
  [[ "${needed}" == "${engine}" ]] && return 0

  cluster="$(printf '%s' "${CLICKHOUSE_CLUSTER_NAME:-}" | tr -d '[:space:]')"
  if [[ -z "${cluster}" ]]; then
    echo "  ${relation}: a ${engine} on a clustered install, but CLICKHOUSE_CLUSTER_NAME names no cluster to read its replicas from" >&2
    return 1
  fi
  engine_full="$(_table_attribute "${db}" "${table}" engine_full)" || return 1
  key="$(_table_attribute "${db}" "${table}" sorting_key)" || return 1
  replicas="clusterAllReplicas('${cluster}', ${relation})"

  before="$(_key_count "${replicas}" "${key}")" || return 1
  echo "  ${relation}: rebuilding ${before} key(s) from every replica of '${cluster}' as ${needed}"
  run_ch <<SQL || return 1
DROP TABLE IF EXISTS ${copy};
CREATE TABLE ${copy} AS ${relation} ENGINE = Replicated${engine_full};
INSERT INTO ${copy} SETTINGS insert_deduplicate = 0, insert_quorum = 'auto' SELECT * FROM ${replicas};
SQL

  copied="$(_key_count "${copy}" "${key}")" || return 1
  if [[ "${copied}" -lt "${before}" ]]; then
    echo "  ${relation}: copy holds ${copied} key(s), expected ${before} — leaving the table as it is" >&2
    run_ch <<SQL
DROP TABLE IF EXISTS ${copy};
SQL
    return 1
  fi

  run_ch <<SQL || return 1
EXCHANGE TABLES ${relation} AND ${copy};
SQL
  if ! live="$(_key_count "${relation}" "${key}")" || [[ "${live}" -lt "${before}" ]]; then
    echo "  ${relation}: rebuilt table holds ${live:-?} key(s), expected ${before} — swapping the original back" >&2
    run_ch <<SQL
EXCHANGE TABLES ${relation} AND ${copy};
SQL
    return 1
  fi

  run_ch <<SQL
DROP TABLE IF EXISTS ${copy};
SQL
}

heal_migration_engines() {
  local relation
  for relation in "${MIGRATION_ENGINE_TABLES[@]}"; do
    heal_migration_engine "${relation}"
  done
}
