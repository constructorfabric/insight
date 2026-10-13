#!/usr/bin/env bash
# What the declared ClickHouse topology means for the DDL this deploy emits,
# and whether the server can serve it at all (epic #2010).
#
# One decision reaches the shell as CLICKHOUSE_CLUSTER_MODE /
# CLICKHOUSE_CLUSTER_NAME, and three things follow from it: a `CREATE DATABASE`
# carries a cluster clause and a `Replicated` engine, a table engine carries a
# `Replicated` prefix, and roles need an access backend the whole cluster
# shares. The spellings accepted for the flag are the ones every other reader
# accepts (charts/insight/README.md, "ClickHouse topology").
#
# Sourced by create-databases.sh and apply-ch-migrations.sh. Exposes:
#   ch_cluster_mode     — succeeds where the install declared itself clustered
#   ch_on_cluster       — `ON CLUSTER <name>`, or nothing
#   ch_database_engine  — the `ENGINE =` clause a CREATE DATABASE carries
#   ch_render_topology  — expand the topology marker in SQL read from stdin
#   ch_assert_topology  — refuse a deploy this server cannot serve
#
# Required env: whatever lib/ch-exec.sh requires, plus the optional pair above.

# Idempotent source guard.
[[ -n "${__CH_TOPOLOGY_SH:-}" ]] && return 0
__CH_TOPOLOGY_SH=1

_CH_TOPOLOGY_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
source "${_CH_TOPOLOGY_DIR}/ch-exec.sh"

# The Keeper path a Replicated database coordinates through. Deterministic per
# database, so a re-run rejoins its path rather than minting a second one.
CH_DATABASE_KEEPER_ROOT="/clickhouse/databases"

ch_cluster_mode() {
  local mode
  mode="$(printf '%s' "${CLICKHOUSE_CLUSTER_MODE:-false}" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"
  case "${mode}" in
    1 | true | yes | on) return 0 ;;
    *) return 1 ;;
  esac
}

# A name without the flag names no cluster: the flag is what turns on the
# replicated engines a clause would qualify. The flag without a name never
# reaches here — the chart refuses that pair.
ch_on_cluster() {
  local name
  ch_cluster_mode || return 0
  name="$(printf '%s' "${CLICKHOUSE_CLUSTER_NAME:-}" | tr -d '[:space:]')"
  [[ -n "${name}" ]] || return 0

  printf 'ON CLUSTER %s' "${name}"
}

# The engine a database is created with. `Replicated` is what logs every later
# DDL in Keeper and replays it on every replica, which is what lets the
# migrations and the dbt hooks state a table without naming a cluster.
#
# `{shard}` / `{replica}` are server macros; a clustered server defines them in
# the same config that points it at Keeper.
ch_database_engine() {
  local database="${1:?ch_database_engine <database>}"
  ch_cluster_mode || return 0

  printf "ENGINE = Replicated('%s/%s', '{shard}', '{replica}')" \
    "${CH_DATABASE_KEEPER_ROOT}" "${database}"
}

# The `Replicated*` prefix a MergeTree family takes where the warehouse
# replicates. Rendered without Keeper coordinates, leaving the server's
# `default_replica_path` / `default_replica_name` to supply them — the same
# shape the dbt `insight_engine` macro emits.
ch_replicated_prefix() {
  if ch_cluster_mode; then
    printf 'Replicated'
  fi
}

# The marker a numbered migration spells its engine behind. A SQL comment on
# purpose: this tree has no template engine, and the migrations are executed
# unrendered by the data-path harness and by the services' live tests, which
# want the standalone spelling the comment already reads as.
CH_TOPOLOGY_MARKER='/*@@REPLICATED@@*/'

# Expand the marker in a SQL file read from stdin, so
# `ENGINE = /*@@REPLICATED@@*/ReplacingMergeTree(_version)` states one engine
# per topology from one line.
#
# INVARIANT: a Replicated database replays DDL on every replica but does not
# replicate rows — only the table engine does. A migration whose engine stays
# the plain family therefore stands on every replica holding different data.
ch_render_topology() {
  local sql
  sql="$(cat)"

  printf '%s' "${sql//"${CH_TOPOLOGY_MARKER}"/$(ch_replicated_prefix)}"
}

# A probe that could not run is not a probe that passed: an unanswered question
# reads as "no such database" or "nothing configured", which is exactly the
# state the checks below exist to refuse.
_ch_probe_unreadable() {
  cat >&2 <<MSG
ERROR: could not read ${1}.

       The topology this deploy is about to emit cannot be verified, and a
       check that could not run is not a check that passed. Nothing has been
       created or granted. Fix the connection, or the deploy admin's access to
       that table, and re-run.
MSG
}

_ch_database_engine_of() {
  local database="$1" engine
  if ! engine="$(printf "SELECT engine FROM system.databases WHERE name = '%s'" "${database}" | _ch_http_query)"; then
    _ch_probe_unreadable system.databases
    return 1
  fi

  printf '%s' "${engine}" | tr -d '[:space:]'
}

_ch_require_keeper() {
  if printf "SELECT count() FROM system.zookeeper WHERE path = '/'" | _ch_http_query >/dev/null 2>&1; then
    return 0
  fi

  cat >&2 <<MSG
ERROR: clickhouse.clusterMode is on, but reading system.zookeeper failed, so
       this server is not reachable or has no Keeper.

       Keeper is a hard prerequisite of both the Replicated database engine and
       the ON CLUSTER queue, even on a one-node cluster. Point every node at a
       Keeper ensemble (<zookeeper> or <keeper_server> in the server config),
       and define the {shard} and {replica} macros in the same config — the
       database engine names them.

       A warehouse without Keeper runs standalone: leave clickhouse.clusterMode
       off.
MSG
  return 1
}

_ch_require_replicated_access_storage() {
  local replicated
  if ! replicated="$(ch_scalar "SELECT count() FROM system.user_directories WHERE type = 'replicated'")"; then
    _ch_probe_unreadable system.user_directories
    return 1
  fi
  [[ "${replicated}" -gt 0 ]] && return 0

  cat >&2 <<MSG
ERROR: clickhouse.clusterMode is on, but this server stores access entities
       locally — no replicated user directory is configured.

       Roles and users created with SQL live outside any database DDL log, so
       on a cluster they exist on whichever replica the deploy connected to.
       Authentication through the load balancer then succeeds or fails
       depending on which replica answers.

       Add a replicated directory to every node's config and restart them:

           <user_directories>
               <replicated>
                   <zookeeper_path>/clickhouse/access</zookeeper_path>
               </replicated>
           </user_directories>

       Or provision access out of band — create the roles and users on each
       replica yourself, and set clickhouse.createDatabases to false so this
       deploy stops provisioning them.
MSG
  return 1
}

# The flag does not convert a database that already stands: a plain one holds
# its tables on the replica that created them, and `IF NOT EXISTS` would hide
# that for the rest of the installation's life.
_ch_require_replicated_databases() {
  local database engine
  for database in "$@"; do
    engine="$(_ch_database_engine_of "${database}")" || return 1
    [[ -z "${engine}" || "${engine}" == "Replicated" ]] && continue

    cat >&2 <<MSG
ERROR: clickhouse.clusterMode is on, but the database '${database}' already
       exists with engine '${engine}'.

       Turning the flag on does not convert it, and every table created in it
       from now on would reach one replica only. Convert the database first —
       create a Replicated one beside it, move the tables over, rename — then
       re-run the deploy.
MSG
    return 1
  done
}

_ch_refuse_replicated_databases() {
  local database engine
  for database in "$@"; do
    engine="$(_ch_database_engine_of "${database}")" || return 1
    [[ "${engine}" == "Replicated" ]] || continue

    cat >&2 <<MSG
ERROR: the database '${database}' is Replicated, but clickhouse.clusterMode is
       off.

       Every creator in this release would now emit standalone DDL into a
       replicated database, and the engines would disagree table by table. Turn
       clickhouse.clusterMode back on (with clickhouse.clusterName), or convert
       the database to a plain one before deploying standalone.
MSG
    return 1
  done
}

# Refuse a deploy this warehouse cannot serve, BEFORE anything is created or
# granted. Every probe exits non-zero through `set -e` in the caller, so the
# first failure stops the deploy with its own remedy.
ch_assert_topology() {
  if ! ch_cluster_mode; then
    _ch_refuse_replicated_databases "$@"
    return 0
  fi

  echo "=== Verifying the warehouse can serve a clustered install ==="
  _ch_require_keeper
  _ch_require_replicated_access_storage
  _ch_require_replicated_databases "$@"
}
