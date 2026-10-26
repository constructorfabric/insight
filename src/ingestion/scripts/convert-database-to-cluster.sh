#!/usr/bin/env bash
# Convert a ClickHouse database of a standalone Insight install into what
# cluster mode creates (epic #2010). The procedure around this script is
# docs/operations/CLICKHOUSE-CLUSTER-CONVERSION.md.
#
# The flag converts nothing that already stands: a plain database holds its
# tables on the one node that created them, and the deploy refuses to run over
# it (lib/ch-topology.sh). This script moves such a database over, on the node
# that holds it:
#
#   1. refuses unless the server can serve a cluster (the named cluster,
#      Keeper, the {shard}/{replica} macros) and every relation in the database
#      is a MergeTree-family table or a view;
#   2. RENAME DATABASE <db> TO standalone_<db> — node-local, instant;
#   3. CREATE DATABASE <db> ON CLUSTER <name>: Replicated for a database this
#      release owns (the engine create-databases.sh renders), Atomic for a
#      `bronze_*` database (what the destination creates);
#   4. for every table, its own CREATE with the Replicated* engine — and, in
#      the Atomic bronze database, the ON CLUSTER clause — then INSERT SELECT,
#      then the two row counts compared;
#   5. for every view, its own CREATE VIEW, once the relations it reads stand.
#
# The standalone copy stays: it is the rollback until the operator drops it
# (DROP DATABASE standalone_<db>, on this node). Re-runnable: a database
# already converted is reported and left alone, a table already copied whole
# is skipped, one copied in part is refused by name.
#
# Usage: convert-database-to-cluster.sh <database> [<database> ...]
#
# Required env (lib/ch-exec.sh):
#   CLICKHOUSE_URL           THE replica holding the standalone data, never the
#                            load balancer: the rename and every read of the
#                            copy's source are local to it
#   CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
#   CLICKHOUSE_CLUSTER_MODE  true
#   CLICKHOUSE_CLUSTER_NAME  the cluster every replica belongs to
# Optional env:
#   CH_HTTP_MAX_TIME_S       ceiling per statement, which a table's copy runs
#                            within (default here: 21600, six hours)
set -euo pipefail

usage() {
  sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
}

DATABASES=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    -*) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    *) DATABASES+=("$1") ;;
  esac
  shift
done
if [[ "${#DATABASES[@]}" -eq 0 ]]; then
  usage >&2
  exit 2
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
# A table's copy is one INSERT SELECT; the deploy's ceiling is sized for a
# statement that moves far less.
: "${CH_HTTP_MAX_TIME_S:=21600}"
source "${SCRIPT_DIR}/lib/ch-topology.sh"

# Where the standalone database goes. A prefix, not a suffix: the deploy's
# bronze sweep reads every `bronze_*` database, and a copy left as
# `bronze_<x>_old` would be refused by name until dropped.
STANDALONE_PREFIX="standalone_"
MERGE_TREE_FAMILY='^(Replacing|Summing|Aggregating|Collapsing|VersionedCollapsing|Graphite)?MergeTree$'

die() {
  echo "ERROR: $*" >&2
  exit 1
}

# One statement, its answer on stdout.
ch() {
  printf '%s' "$1" | _ch_http_query
}

# One DDL statement; a coordinated one answers with a row per replica, which
# is noise once it succeeded.
ddl() {
  printf '%s' "$1" | _ch_http_query > /dev/null
}

quoted() {
  printf '%s' "\`$1\`.\`$2\`"
}

is_bronze() {
  [[ "$1" == bronze_* ]]
}

# name<TAB>engine of every relation in a database, tables before views.
relations_of() {
  ch "SELECT name, engine FROM system.tables WHERE database = '$1' AND NOT is_temporary ORDER BY engine = 'View', name FORMAT TSV"
}

relation_exists() {
  [[ "$(ch_scalar "SELECT count() FROM system.tables WHERE database = '$1' AND name = '$2'")" == 1 ]]
}

# Refuse before anything moves: a relation this script does not carry over.
assert_convertible() {
  local database="$1" unsupported
  unsupported="$(relations_of "${database}" \
    | awk -F'\t' -v family="${MERGE_TREE_FAMILY}" '$2 != "View" && $2 !~ family { print "         " $1 "  " $2 }')"
  [[ -z "${unsupported}" ]] && return 0

  cat >&2 <<MSG
ERROR: the database '${database}' holds relations this conversion does not
       carry over:

${unsupported}

       Only MergeTree-family tables and views are moved. Drop them, or move
       them by hand, then re-run.
MSG
  return 1
}

# A bronze database stays Atomic in both topologies, so whether it is
# converted is told by its tables and its reach: every MergeTree table
# replicated, and the database standing on every replica of the cluster.
# Called under `if`, where a failed probe would read as "not converted" and
# send a converted database into the rename: each one stops the script.
bronze_converted() {
  local database="$1" name plain hosts present
  name="$(ch_cluster_name)"
  plain="$(ch_scalar "SELECT count() FROM system.tables WHERE database = '${database}'
    AND endsWith(engine, 'MergeTree') AND NOT startsWith(engine, 'Replicated')")" \
    || die "could not count the plain MergeTree tables of ${database}"
  hosts="$(ch_scalar "SELECT count() FROM system.clusters WHERE cluster = '${name}' AND database_replica_name = ''")" \
    || die "could not count the hosts of the cluster ${name}"
  present="$(ch_scalar "SELECT count() FROM clusterAllReplicas('${name}', system.databases)
    WHERE name = '${database}' SETTINGS skip_unavailable_shards = 0")" \
    || die "could not read system.databases on every replica of ${name} to look for ${database}"
  [[ "${plain}" -eq 0 && "${present}" -eq "${hosts}" ]]
}

# The CREATE a relation of the standalone database was made with, re-aimed at
# the converted one: the database name, a Replicated* engine, and in an Atomic
# database the cluster clause. SHOW CREATE puts the relation's name on its
# first line and the engine at the start of its own.
converted_create() {
  local create="$1" old="$2" database="$3" on_cluster="$4"
  printf '%s\n' "${create}" | sed -E \
    -e "1s/^(CREATE (TABLE|VIEW) )\`?${old}\`?\.(\`[^\`]+\`|[^ (]+)/\1${database}.\3${on_cluster:+ ${on_cluster}}/" \
    -e 's/^ENGINE = (Replacing|Summing|Aggregating|Collapsing|VersionedCollapsing|Graphite)?MergeTree/ENGINE = Replicated\1MergeTree/'
}

copy_table() {
  local old="$1" database="$2" table="$3" on_cluster="$4" create before after
  before="$(ch_scalar "SELECT count() FROM $(quoted "${old}" "${table}")")"
  if relation_exists "${database}" "${table}"; then
    after="$(ch_scalar "SELECT count() FROM $(quoted "${database}" "${table}")")"
    if [[ "${after}" == "${before}" ]]; then
      echo "  ${database}.${table}: already copied, ${after} row(s)"
      ALREADY_THERE=$(( ALREADY_THERE + 1 ))
      return 0
    fi
    die "${database}.${table} holds ${after} row(s) where ${old}.${table} holds ${before}: a copy stopped part-way. Drop ${database}.${table}${on_cluster:+ ${on_cluster}} and re-run."
  fi

  create="$(ch "SHOW CREATE TABLE $(quoted "${old}" "${table}") FORMAT TSVRaw")"
  converted_create "${create}" "${old}" "${database}" "${on_cluster}" | _ch_http_query > /dev/null
  ddl "INSERT INTO $(quoted "${database}" "${table}") SELECT * FROM $(quoted "${old}" "${table}")"
  after="$(ch_scalar "SELECT count() FROM $(quoted "${database}" "${table}")")"
  [[ "${after}" == "${before}" ]] \
    || die "${database}.${table} holds ${after} row(s) after the copy, ${old}.${table} ${before}"
  echo "  ${database}.${table}: ${after} row(s)"
}

# Views read relations that may be views of the same database, in any order
# system.tables lists them; a pass that creates nothing is a view none of the
# others unblocks.
# INVARIANT: a view's query is stored qualified with the names it was created
# under, and the rename does not rewrite them — which is what points the
# recreated view at the converted database rather than the standalone copy.
create_views() {
  local old="$1" database="$2" on_cluster="$3" view create error
  shift 3
  local pending=("$@") failed=()
  while [[ "${#pending[@]}" -gt 0 ]]; do
    failed=()
    for view in "${pending[@]}"; do
      if relation_exists "${database}" "${view}"; then
        echo "  ${database}.${view}: already stands"
        ALREADY_THERE=$(( ALREADY_THERE + 1 ))
        continue
      fi
      create="$(ch "SHOW CREATE TABLE $(quoted "${old}" "${view}") FORMAT TSVRaw")"
      if error="$(converted_create "${create}" "${old}" "${database}" "${on_cluster}" | _ch_http_query 2>&1)"; then
        echo "  ${database}.${view}: view"
      else
        failed+=("${view}")
      fi
    done
    if [[ "${#failed[@]}" -eq "${#pending[@]}" ]]; then
      die "the view ${database}.${failed[0]} cannot be created: ${error}"
    fi
    pending=(${failed[@]+"${failed[@]}"})
  done
}

convert_database() {
  local database="$1"
  local old="${STANDALONE_PREFIX}${database}" engine old_engine on_cluster target listing
  local tables=() views=() name kind
  ALREADY_THERE=0
  engine="$(_ch_database_engine_of "${database}")"
  old_engine="$(_ch_database_engine_of "${old}")"
  on_cluster="$(ch_on_cluster)"

  echo "=== ${database} ==="
  if [[ -z "${old_engine}" ]]; then
    case "${engine}" in
      "") die "no database '${database}' on the node CLICKHOUSE_URL names" ;;
      Replicated)
        echo "  already Replicated, nothing to do"
        return 0 ;;
      Atomic) ;;
      *) die "the database '${database}' has engine '${engine}'; only an Atomic database can be renamed aside" ;;
    esac
    if is_bronze "${database}" && bronze_converted "${database}"; then
      echo "  already replicated on every replica, nothing to do"
      return 0
    fi
    assert_convertible "${database}"
    echo "  renaming to ${old}"
    ddl "RENAME DATABASE \`${database}\` TO \`${old}\`"
  else
    echo "  ${old} already stands: resuming"
  fi

  if [[ -z "$(_ch_database_engine_of "${database}")" ]]; then
    target="CREATE DATABASE \`${database}\` ${on_cluster}"
    if is_bronze "${database}"; then
      echo "  creating the database ${on_cluster}, Atomic"
    else
      target+=" $(ch_database_engine "${database}")"
      echo "  creating the database ${on_cluster}, Replicated"
    fi
    ddl "${target}"
  fi
  # A table in a Replicated database replays its own DDL; one in the Atomic
  # bronze database needs the clause (lib/ch-topology.sh).
  if ! is_bronze "${database}"; then
    on_cluster=""
  fi

  # Read whole and checked before it is parsed: a listing that failed would
  # otherwise read as an empty database, converted in full.
  listing="$(relations_of "${old}")"
  while IFS=$'\t' read -r name kind; do
    [[ -n "${name}" ]] || continue
    if [[ "${kind}" == View ]]; then
      views+=("${name}")
    else
      tables+=("${name}")
    fi
  done <<< "${listing}"

  for name in ${tables[@]+"${tables[@]}"}; do
    copy_table "${old}" "${database}" "${name}" "${on_cluster}"
  done
  create_views "${old}" "${database}" "${on_cluster}" ${views[@]+"${views[@]}"}
  if [[ "${ALREADY_THERE}" -eq $(( ${#tables[@]} + ${#views[@]} )) ]]; then
    echo "  ${database}: ${#tables[@]} table(s) and ${#views[@]} view(s) already converted, nothing to do; ${old} still stands"
  else
    echo "  ${database}: ${#tables[@]} table(s) and ${#views[@]} view(s) converted; ${old} kept as the rollback"
  fi
}

ch_assert_cluster_server
for database in "${DATABASES[@]}"; do
  convert_database "${database}"
done
