#!/usr/bin/env bash
# Drops databases and relations no release creates or reads, only where none of
# them can still hold rows an operator wants.
#
# Sourced by apply-ch-migrations.sh. Exposes:
#   drop_retired_database_if_empty — `<database>`
#   drop_retired_table_if_empty    — `<database> <table>`
#   drop_orphaned_inner_tables     — `<database>`
#
# Required env: whatever lib/ch-topology.sh requires.

[[ -n "${__RETIRED_RELATIONS_SH:-}" ]] && return 0
__RETIRED_RELATIONS_SH=1

_RETIRED_RELATIONS_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
source "${_RETIRED_RELATIONS_DIR}/ch-topology.sh"

# SAFETY: a relation whose engine reports no row count reads as holding rows, and
# a clustered warehouse is never counted from one replica; both are left alone.
_retired_relations_held() {
  local where="$1"
  ch_scalar "SELECT count() FROM system.tables WHERE ${where} AND (total_rows IS NULL OR total_rows > 0)"
}

_retired_relation_kept_on_a_cluster() {
  ch_cluster_mode || return 1
  echo "  $1: clustered warehouse, other replicas are not counted — left in place"
}

drop_retired_database_if_empty() {
  local database="$1"
  local held
  _retired_relation_kept_on_a_cluster "${database}" && return 0

  held="$(_retired_relations_held "database = '${database}'")"
  if [[ "${held}" -gt 0 ]]; then
    echo "  ${database}: ${held} table(s) not provably empty — left in place"
    return 0
  fi

  printf 'DROP DATABASE IF EXISTS `%s`' "${database}" | run_ch
}

drop_retired_table_if_empty() {
  local database="$1" table="$2"
  local held
  _retired_relation_kept_on_a_cluster "${database}.${table}" && return 0

  held="$(_retired_relations_held "database = '${database}' AND name = '${table}'")"
  if [[ "${held}" -gt 0 ]]; then
    echo "  ${database}.${table}: not provably empty — left in place"
    return 0
  fi

  printf 'DROP TABLE IF EXISTS `%s`.`%s`' "${database}" "${table}" | run_ch
}

# INVARIANT: an Atomic database names a materialized view's inner table
# `.inner_id.<view uuid>`; a detached or just-created view still owns one.
drop_orphaned_inner_tables() {
  local database="$1"
  local orphans name
  orphans="$(printf "SELECT name FROM system.tables WHERE database = '%s' AND match(name, '^[.]inner_id[.][0-9a-f-]{36}\$') AND substring(name, 11) NOT IN (SELECT toString(uuid) FROM system.tables WHERE engine = 'MaterializedView') AND substring(name, 11) NOT IN (SELECT toString(uuid) FROM system.detached_tables) AND metadata_modification_time < now() - INTERVAL 1 DAY FORMAT TSV" \
    "${database}" | _ch_http_query)"

  for name in ${orphans}; do
    echo "  ${database}.${name}"
    printf 'DROP TABLE IF EXISTS `%s`.`%s`' "${database}" "${name}" | run_ch
  done
}
