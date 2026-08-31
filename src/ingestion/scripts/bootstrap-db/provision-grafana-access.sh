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

# Grant-less user: every privilege comes via grafana_ro, and a warm user is
# converged back to exactly that — REVOKE ALL strips direct grants handed
# out out-of-band (role-carried privileges live on the role, not the user),
# and the loop below strips stray roles.
# SAFETY: converge in place, never DROP+CREATE — run_ch sends one statement
# per HTTP request, so a mid-sequence failure after a DROP would leave no
# `grafana` user until the hook retries; this order keeps the user
# authenticatable with grafana_ro at every step.
run_ch <<SQL
CREATE USER IF NOT EXISTS grafana IDENTIFIED BY '${CLICKHOUSE_GRAFANA_PASSWORD}';
ALTER USER grafana IDENTIFIED BY '${CLICKHOUSE_GRAFANA_PASSWORD}';
GRANT grafana_ro TO grafana;
ALTER USER grafana DEFAULT ROLE grafana_ro;
REVOKE ALL ON *.* FROM grafana;
SQL

# Role names come from the server, but only plain identifiers are revoked —
# an exotic name would need quoting this DDL-only path does not do.
extra_roles="$(printf "SELECT granted_role_name FROM system.role_grants WHERE user_name = 'grafana' AND granted_role_name != 'grafana_ro'" | _ch_http_query)"
while IFS= read -r role; do
  [[ -n "$role" ]] || continue
  if [[ ! "$role" =~ ^[A-Za-z0-9_]+$ ]]; then
    echo "  WARN: not revoking oddly-named role '${role}' from grafana (needs manual review)"
    continue
  fi
  printf 'REVOKE %s FROM grafana' "$role" | _ch_http_query >/dev/null
done <<< "$extra_roles"
echo "  grafana user ready"
