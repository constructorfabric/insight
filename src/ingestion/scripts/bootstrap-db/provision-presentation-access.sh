#!/usr/bin/env bash
# Provision presentation query-path access (#1963/#1964): the read-only
# `presentation_ro` role (presentation-role.sql) + the grant-less `presentation`
# user analytics connects as. Called by apply-ch-migrations.sh. Idempotent.
# Guarded: skips (non-fatal) without access_management or without a user password.
# Env: CLICKHOUSE_{URL,USER,PASSWORD} (admin), CLICKHOUSE_PRESENTATION_PASSWORD (optional).
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
source "$SCRIPT_DIR/../lib/ch-exec.sh"

# CREATE ROLE fails without access_management.
if ! printf 'CREATE ROLE IF NOT EXISTS presentation_ro' | _ch_http_query >/dev/null 2>&1; then
  echo "  WARN: admin lacks access_management; skipping presentation access (see bootstrap-db/README.md)"
  exit 0
fi

run_ch < "$SCRIPT_DIR/presentation-role.sql"
echo "  presentation_ro role ready"

# Bronze read for the ingestion-intensity ops view
# (src/ingestion/gold/bronze_insert_events.sql). Enumerated rather than static
# DDL for two reasons: bronze databases are created per connector, and
# ClickHouse has no wildcard-database grant. Re-runs on every deploy, so a
# connector added since the last one becomes readable at the next.
#
# The view is `merge(REGEXP('^bronze_'), '.*')`, whose table set resolves
# through the CALLER's grants — an ungranted database is not skipped, it makes
# the regexp match nothing and the query fails outright
# (CANNOT_EXTRACT_TABLE_STRUCTURE). So this grant is what makes the surface work
# at all, not an optimisation.
#
# Widening the read-only contract is deliberate and bounded: query_gate.rs
# refuses any caller-authored SQL that names a `bronze_` database, so the
# broadened grant serves this one server-side view and not the public query path.
bronze_databases="$(printf "SELECT name FROM system.databases WHERE name LIKE 'bronze\\_%%' ORDER BY name FORMAT TSV" | _ch_http_query || true)"
if [[ -z "${bronze_databases}" ]]; then
  echo "  NOTE: no bronze_* databases yet; ingestion intensity stays empty until a connector lands"
else
  bronze_count=0
  while IFS= read -r bronze_db; do
    [[ -z "${bronze_db}" ]] && continue
    # Backticked: a database name is an identifier, and the enumeration comes
    # from system.databases rather than from any caller.
    printf 'GRANT SELECT ON `%s`.* TO presentation_ro' "${bronze_db}" | _ch_http_query >/dev/null
    bronze_count=$((bronze_count + 1))
  done <<< "${bronze_databases}"
  echo "  presentation_ro can read ${bronze_count} bronze database(s)"
fi

if [[ -z "${CLICKHOUSE_PRESENTATION_PASSWORD:-}" ]]; then
  echo "  NOTE: CLICKHOUSE_PRESENTATION_PASSWORD unset; role only (analytics stays on admin)"
  exit 0
fi

# The password rides the SQL body (run_ch splits on `;`), so reject chars that
# could break or inject past the `IDENTIFIED BY '...'` literal. Auto-gen is
# alphanumeric; this only trips a hand-set BYO password.
case "${CLICKHOUSE_PRESENTATION_PASSWORD}" in
  *"'"* | *"\\"* | *";"*)
    echo "  WARN: CLICKHOUSE_PRESENTATION_PASSWORD has a quote/backslash/semicolon; skipping user (use alphanumeric)"
    exit 0
    ;;
esac

# Grant-less user: every privilege comes via presentation_ro. ALTER converges
# the password on warm clusters (rotation-safe).
run_ch <<SQL
CREATE USER IF NOT EXISTS presentation IDENTIFIED BY '${CLICKHOUSE_PRESENTATION_PASSWORD}';
ALTER USER presentation IDENTIFIED BY '${CLICKHOUSE_PRESENTATION_PASSWORD}';
GRANT presentation_ro TO presentation;
ALTER USER presentation DEFAULT ROLE presentation_ro;
SQL
echo "  presentation user ready"
