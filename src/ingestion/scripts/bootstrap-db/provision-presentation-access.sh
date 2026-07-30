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
