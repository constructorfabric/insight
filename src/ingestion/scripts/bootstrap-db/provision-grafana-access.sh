#!/usr/bin/env bash
# Provision Grafana datasource access (#2888): the SELECT-only `grafana_ro`
# role (grafana-role.sql) + the grant-less `grafana` user the Grafana
# ClickHouse datasource connects as. Called by apply-ch-migrations.sh.
# Idempotent. Guarded: skips (non-fatal) without access_management or without
# a user password.
# Env: CLICKHOUSE_{URL,USER,PASSWORD} (admin), CLICKHOUSE_GRAFANA_PASSWORD (optional).
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
source "$SCRIPT_DIR/../lib/ch-exec.sh"

# CREATE ROLE fails without access_management.
if ! printf 'CREATE ROLE IF NOT EXISTS grafana_ro' | _ch_http_query >/dev/null 2>&1; then
  echo "  WARN: admin lacks access_management; skipping grafana access (see bootstrap-db/README.md)"
  exit 0
fi

run_ch < "$SCRIPT_DIR/grafana-role.sql"
echo "  grafana_ro role ready"

if [[ -z "${CLICKHOUSE_GRAFANA_PASSWORD:-}" ]]; then
  echo "  NOTE: CLICKHOUSE_GRAFANA_PASSWORD unset; role only (no grafana user)"
  exit 0
fi

# The password rides the SQL body (run_ch splits on `;`), so reject chars that
# could break or inject past the `IDENTIFIED BY '...'` literal. Auto-gen is
# alphanumeric; this only trips a hand-set BYO password.
case "${CLICKHOUSE_GRAFANA_PASSWORD}" in
  *"'"* | *"\\"* | *";"*)
    echo "  WARN: CLICKHOUSE_GRAFANA_PASSWORD has a quote/backslash/semicolon; skipping user (use alphanumeric)"
    exit 0
    ;;
esac

# Grant-less user: every privilege comes via grafana_ro. Drop-and-recreate
# converges a warm user — ALTER would leave direct grants or extra roles
# handed out out-of-band effective, voiding the SELECT-only guarantee.
# SAFETY: unlike `presentation` (a live service connection pool), Grafana
# reconnects per query, so the sub-second no-user window during a deploy
# hook is harmless.
run_ch <<SQL
DROP USER IF EXISTS grafana;
CREATE USER grafana IDENTIFIED BY '${CLICKHOUSE_GRAFANA_PASSWORD}';
GRANT grafana_ro TO grafana;
ALTER USER grafana DEFAULT ROLE grafana_ro;
SQL
echo "  grafana user ready"
