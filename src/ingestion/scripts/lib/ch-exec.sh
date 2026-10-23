#!/usr/bin/env bash
# Shared ClickHouse access helpers for the in-cluster migration Job, which
# applies DDL against the EXTERNAL ClickHouse over its HTTP interface.
#
# ClickHouse is always external (both clusters install the L2 infra as
# separate releases via `make deploy` — the umbrella stopped bundling CH in
# #1428), so there is no in-cluster pod to `kubectl exec` into. HTTP is the
# only path.
#
# Sourced by apply-ch-migrations.sh, lib/ch-topology.sh and the bootstrap-db
# access-provisioning scripts.
# Exposes:
#   run_ch           — execute a (multi-statement) SQL block read from stdin.
#   ch_table_exists  — `ch_table_exists <db> <table>`; exit 0 if present.
#   ch_table_engine  — `ch_table_engine <db> <table>`; the engine, empty if
#                      absent, failure if the probe did not run.
#   ch_scalar        — one numeric answer of a SELECT, or failure.
#   ch_cluster_mode  — succeeds where the install declared itself clustered.
#
# Required env:
#   CLICKHOUSE_URL       e.g. http://ch-host:8123
#   CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
# Optional env:
#   CLICKHOUSE_CLUSTER_MODE  true -> every request carries the cluster session
#                            settings below; unset or false -> the request is
#                            exactly the standalone one.

# Idempotent source guard.
[[ -n "${__CH_EXEC_SH:-}" ]] && return 0
__CH_EXEC_SH=1

: "${CLICKHOUSE_URL:?CLICKHOUSE_URL must be set (e.g. http://ch-host:8123)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"

ch_cluster_mode() {
  local mode
  mode="$(printf '%s' "${CLICKHOUSE_CLUSTER_MODE:-false}" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"
  case "${mode}" in
    1 | true | yes | on) return 0 ;;
    *) return 1 ;;
  esac
}

# Per-statement ceiling for coordinated DDL in cluster mode. Only ACTIVE
# replicas are waited for: an inactive one replays its Keeper queue entry when
# it returns, and ch-topology.sh's completeness guard reports the lag until then.
CH_DDL_TASK_TIMEOUT_S=60
# Transport ceiling per request in cluster mode; only a hung connection reaches
# it, a statement that moves rows is allowed the rest. A caller that copies
# whole tables (convert-database-to-cluster.sh) raises it before sourcing.
CH_HTTP_MAX_TIME_S="${CH_HTTP_MAX_TIME_S:-900}"

# The session a clustered deploy runs under, as URL parameters so no `.sql`
# statement has to spell them.
# WORKAROUND: parallel_distributed_insert_select=0 — CH 25.7 (fixed by 26.9)
# fails `INSERT INTO <Replicated*> WITH c AS (...) SELECT ... FROM c` otherwise.
# INVARIANT: insert_quorum=auto + insert_quorum_parallel=0 +
# select_sequential_consistency=1 is what lets a count taken through the load
# balancer judge a rebuild; with a replica down an INSERT fails loudly instead.
# allow_nondeterministic_mutations=1: a Replicated table rejects `ALTER ...
# UPDATE/DELETE` with a subquery without it. mutations_sync/alter_sync=2 wait
# for every replica (3 is accepted by 25.7.5 but waits for one replica only).
CH_CLUSTER_SESSION_SETTINGS="distributed_ddl_output_mode=throw_only_active"
CH_CLUSTER_SESSION_SETTINGS+="&distributed_ddl_task_timeout=${CH_DDL_TASK_TIMEOUT_S}"
CH_CLUSTER_SESSION_SETTINGS+="&parallel_distributed_insert_select=0"
CH_CLUSTER_SESSION_SETTINGS+="&insert_quorum=auto&insert_quorum_parallel=0&select_sequential_consistency=1"
CH_CLUSTER_SESSION_SETTINGS+="&allow_nondeterministic_mutations=1"
CH_CLUSTER_SESSION_SETTINGS+="&mutations_sync=2&alter_sync=2"

# Execute the SQL piped on stdin as one statement over the HTTP interface.
# The body is sent verbatim as the POST payload (--data-binary) — form
# encoding would mangle the SQL into `query=CREATE+...` (Code 62).
#
# Credentials go via ClickHouse's native auth headers, NOT `-u`: the
# username header is harmless in argv, but the password is fed through a
# header file (process substitution) so it never lands in curl's argv (and
# thus /proc/<pid>/cmdline, visible to any process in the pod).
#
# INVARIANT: a standalone install sends exactly this request; cluster mode adds
# the session settings above and the transport ceiling.
_ch_http_query() {
  local url="${CLICKHOUSE_URL%/}/" max_time=()
  if ch_cluster_mode; then
    url+="?${CH_CLUSTER_SESSION_SETTINGS}"
    max_time=(--max-time "${CH_HTTP_MAX_TIME_S}")
  fi
  curl -sS --fail-with-body ${max_time[@]+"${max_time[@]}"} \
    -H "X-ClickHouse-User: ${CLICKHOUSE_USER}" \
    -H @<(printf 'X-ClickHouse-Key: %s' "${CLICKHOUSE_PASSWORD}") \
    --data-binary @- \
    "${url}"
}

# The HTTP interface runs one statement per request, so a multi-statement
# heredoc/file is fanned out statement-by-statement. We first drop full-line
# `--` comments (mirrors the init.sh sed pass + silver.py _split_statements),
# then split on `;`.
#
# DDL-ONLY invariant: splitting on `;` assumes no `;` inside string literals
# or /* */ blocks, and no inline `-- ...; ...` trailer. Every migration +
# placeholder honours this (same simplification silver.py relies on). A
# future migration with an in-string `;` MUST not use this path unguarded.
#
# Returns non-zero at the FIRST statement ClickHouse rejects and sends nothing
# after it. A block that swaps or drops a table must never reach the swap when
# the statement that filled the replacement failed.
run_ch() {
  local sql stmt
  sql="$(sed -E '/^[[:space:]]*--/d')"
  while IFS= read -r -d ';' stmt; do
    # Skip whitespace-only segments (e.g. the tail after the last `;`). CH
    # tolerates leading/trailing whitespace, so non-empty segments are sent
    # as-is — no fragile per-line trim needed.
    [[ "$stmt" =~ [^[:space:]] ]] || continue
    printf '%s' "$stmt" | _ch_http_query || return 1
  done < <(printf '%s;' "$sql")
}

# One numeric scalar from a SELECT, or failure. A guard that reads an empty
# or non-numeric answer must stop the deploy, not fall through as "nothing to
# do" — and never as "safe to proceed".
ch_scalar() {
  local value
  value="$(printf '%s' "$1" | _ch_http_query | tr -d '[:space:]')" || return 1
  [[ "${value}" =~ ^[0-9]+$ ]] || return 1
  printf '%s' "${value}"
}

# NB: callers use `if ! ch_table_exists ...`, which disables `set -e` for
# this body — a probe failure (auth/transient/DNS) reads as "absent" and the
# caller falls through to CREATE ... IF NOT EXISTS, which is idempotent.
ch_table_exists() {
  local db="$1" tbl="$2" result
  result="$(printf "SELECT count() FROM system.tables WHERE database='%s' AND name='%s'" \
    "$db" "$tbl" | _ch_http_query | tr -d '[:space:]')"
  [[ "$result" == "1" ]]
}

# The engine of one table, empty when there is none. Fails when the lookup did
# not run: a caller that acts on the engine (a rebuild copies it) must not read
# a failed probe as an absent table.
ch_table_engine() {
  local db="$1" tbl="$2" engine
  engine="$(printf "SELECT engine FROM system.tables WHERE database='%s' AND name='%s'" \
    "$db" "$tbl" | _ch_http_query)" || return 1
  printf '%s' "${engine}" | tr -d '[:space:]'
}
