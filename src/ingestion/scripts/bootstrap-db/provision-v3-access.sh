#!/usr/bin/env bash
# Provision the custom assistant's query path: the `insight_v3_ro` role
# (v3-role.sql) + the grant-less `insight_v3_reader` user that carries it.
# Called by create-databases.sh, and only where insight-v3-core is deployed —
# the role reads every table, so an installation without the assistant gets
# neither (#3558). Idempotent.
# Guarded: skips (non-fatal) without access_management or without a password.
# Env: CLICKHOUSE_{URL,USER,PASSWORD} (admin), CLICKHOUSE_V3_READER_{USER,PASSWORD}.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
source "$SCRIPT_DIR/../lib/ch-exec.sh"

reader_user="${CLICKHOUSE_V3_READER_USER:-insight_v3_reader}"

# CREATE ROLE fails without access_management.
if ! printf 'CREATE ROLE IF NOT EXISTS insight_v3_ro' | _ch_http_query >/dev/null 2>&1; then
  echo "  WARN: admin lacks access_management; skipping v3 reader (see bootstrap-db/README.md)"
  exit 0
fi

run_ch < "$SCRIPT_DIR/v3-role.sql"
echo "  insight_v3_ro role ready"

if [[ -z "${CLICKHOUSE_V3_READER_PASSWORD:-}" ]]; then
  echo "  NOTE: CLICKHOUSE_V3_READER_PASSWORD unset; role only (v3 queries as its own user)"
  exit 0
fi

# The password rides the SQL body (run_ch splits on `;`), so reject chars that
# could break or inject past the `IDENTIFIED BY '...'` literal. Auto-gen is
# alphanumeric; this only trips a hand-set BYO password.
case "${CLICKHOUSE_V3_READER_PASSWORD}" in
  *"'"* | *"\\"* | *";"*)
    echo "  WARN: CLICKHOUSE_V3_READER_PASSWORD has a quote/backslash/semicolon; skipping user (use alphanumeric)"
    exit 0
    ;;
esac

case "${reader_user}" in
  *[^A-Za-z0-9_]*)
    echo "  WARN: CLICKHOUSE_V3_READER_USER is not an identifier; skipping user" >&2
    exit 0
    ;;
esac

# Grant-less user: every privilege comes via insight_v3_ro. ALTER converges the
# password on warm clusters (rotation-safe).
run_ch <<SQL
CREATE USER IF NOT EXISTS ${reader_user} IDENTIFIED BY '${CLICKHOUSE_V3_READER_PASSWORD}';
ALTER USER ${reader_user} IDENTIFIED BY '${CLICKHOUSE_V3_READER_PASSWORD}';
GRANT insight_v3_ro TO ${reader_user};
ALTER USER ${reader_user} DEFAULT ROLE insight_v3_ro;
SQL
echo "  ${reader_user} ready"
