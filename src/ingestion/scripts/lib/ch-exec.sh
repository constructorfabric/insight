#!/usr/bin/env bash
# Shared ClickHouse access helpers for the in-cluster migration Job, which
# applies DDL against the EXTERNAL ClickHouse over its HTTP interface.
#
# ClickHouse is always external (both clusters install the L2 infra as
# separate releases via `make deploy` — the umbrella stopped bundling CH in
# #1428), so there is no in-cluster pod to `kubectl exec` into. HTTP is the
# only path.
#
# Sourced by apply-ch-migrations.sh and create-bronze-placeholders.sh.
# Exposes:
#   run_ch           — execute a (multi-statement) SQL block read from stdin.
#   ch_table_exists  — `ch_table_exists <db> <table>`; exit 0 if present.
#
# Required env:
#   CLICKHOUSE_URL       e.g. http://ch-host:8123
#   CLICKHOUSE_USER, CLICKHOUSE_PASSWORD

# Idempotent source guard.
[[ -n "${__CH_EXEC_SH:-}" ]] && return 0
__CH_EXEC_SH=1

: "${CLICKHOUSE_URL:?CLICKHOUSE_URL must be set (e.g. http://ch-host:8123)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"

# Execute the SQL piped on stdin as one statement over the HTTP interface.
# The body is sent verbatim as the POST payload (--data-binary), mirroring
# clickhouse-init-svcdbs-job.yaml — form encoding would mangle the SQL into
# `query=CREATE+...` (Code 62).
#
# Credentials go via ClickHouse's native auth headers, NOT `-u`: the
# username header is harmless in argv, but the password is fed through a
# header file (process substitution) so it never lands in curl's argv (and
# thus /proc/<pid>/cmdline, visible to any process in the pod).
_ch_http_query() {
  curl -sS --fail-with-body \
    -H "X-ClickHouse-User: ${CLICKHOUSE_USER}" \
    -H @<(printf 'X-ClickHouse-Key: %s' "${CLICKHOUSE_PASSWORD}") \
    --data-binary @- \
    "${CLICKHOUSE_URL%/}/"
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
run_ch() {
  local sql stmt
  sql="$(sed -E '/^[[:space:]]*--/d')"
  while IFS= read -r -d ';' stmt; do
    # Skip whitespace-only segments (e.g. the tail after the last `;`). CH
    # tolerates leading/trailing whitespace, so non-empty segments are sent
    # as-is — no fragile per-line trim needed.
    [[ "$stmt" =~ [^[:space:]] ]] || continue
    printf '%s' "$stmt" | _ch_http_query
  done < <(printf '%s;' "$sql")
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
