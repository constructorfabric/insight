#!/usr/bin/env bash
# One-time seed: identity_inputs (ClickHouse) -> persons (MariaDB).
#
# Reads identity.identity_inputs via HTTP, groups by source-account,
# assigns deterministic person_id per (tenant, email), INSERT IGNOREs
# every observation into persons.
#
# This script does NOT apply DDL. The `persons` table schema is owned
# by the identity-resolution Rust service and applied by its SeaORM
# Migrator at startup (see ADR-0002 for seed idempotency and ADR-0006
# for the service-owned-migrations policy).
#
# Prerequisites:
#   - Cluster running, ClickHouse + MariaDB healthy
#   - identity_inputs dbt view populated (dbt run --select +identity_inputs)
#   - identity-resolution service has started at least once (its
#     initContainer applies the persons migration), OR run:
#       identity-resolution migrate
#
# Usage:
#   ./src/backend/services/identity/seed/seed-persons.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export KUBECONFIG="${KUBECONFIG:-${HOME}/.kube/insight.kubeconfig}"

echo "=== Seed: identity_inputs -> MariaDB persons ==="

# -- Resolve ClickHouse credentials ---------------------------------------
CH_PASS="${CLICKHOUSE_PASSWORD:-$(kubectl get secret clickhouse-credentials -n data -o jsonpath='{.data.password}' | base64 -d)}"
export CLICKHOUSE_URL="${CLICKHOUSE_URL:-http://localhost:30123}"
export CLICKHOUSE_USER="${CLICKHOUSE_USER:-default}"
export CLICKHOUSE_PASSWORD="$CH_PASS"

# -- Resolve MariaDB credentials ------------------------------------------
# All MARIADB_* values are resolved here and exported to the Python
# subprocess. URL-encode user/password so passwords containing ':', '@',
# '/', or '%' do not break URL parsing in the Python side.
MARIADB_USER="${MARIADB_USER:-insight}"
MARIADB_PASSWORD="${MARIADB_PASSWORD:-insight-pass}"
MARIADB_HOST="${MARIADB_HOST:-localhost}"
MARIADB_PORT="${MARIADB_PORT:-3306}"
MARIADB_DB="${MARIADB_DB:-identity}"
export MARIADB_USER MARIADB_PASSWORD MARIADB_HOST MARIADB_PORT MARIADB_DB

_USER_ENC=$(python3 -c 'import os, urllib.parse; print(urllib.parse.quote(os.environ["MARIADB_USER"], safe=""))')
_PASS_ENC=$(python3 -c 'import os, urllib.parse; print(urllib.parse.quote(os.environ["MARIADB_PASSWORD"], safe=""))')
export MARIADB_URL="mysql://${_USER_ENC}:${_PASS_ENC}@${MARIADB_HOST}:${MARIADB_PORT}/${MARIADB_DB}"

# -- Ensure MariaDB port-forward ------------------------------------------
# Use python3 for port-check instead of nc -- nc is missing on Windows
# Git Bash. Only auto-port-forward against the default Kind pod name; if
# MARIADB_HOST is explicitly overridden to something else (e.g. a remote
# managed instance) trust the caller.
_port_open() {
  python3 -c "
import socket, sys
s = socket.socket()
s.settimeout(0.5)
try:
    s.connect(('${MARIADB_HOST}', ${MARIADB_PORT}))
except OSError:
    sys.exit(1)
"
}
if [[ "$MARIADB_HOST" == "localhost" || "$MARIADB_HOST" == "127.0.0.1" ]] \
    && ! _port_open; then
  echo "  Starting MariaDB port-forward..."
  kubectl -n insight port-forward svc/insight-mariadb "${MARIADB_PORT}:3306" >/dev/null 2>&1 &
  _PF_PID=$!
  # Make sure we do not leave an orphan port-forward if the seed exits
  # for any reason (Ctrl-C, Python error, successful completion).
  trap 'kill $_PF_PID 2>/dev/null || true' EXIT
  _ready=0
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if _port_open; then
      _ready=1
      break
    fi
    sleep 1
  done
  if [[ "$_ready" -ne 1 ]]; then
    echo "ERROR: MariaDB port-forward did not become ready within 10s." >&2
    echo "  Check: kubectl -n insight get pods -l app.kubernetes.io/name=mariadb" >&2
    exit 1
  fi
fi

# -- Run seed -------------------------------------------------------------
echo "  Running seed script..."
pip install pymysql --quiet 2>/dev/null || true
python3 "$SCRIPT_DIR/seed-persons-from-identity-input.py"
