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
#   ./src/backend/services/identity-resolution/seed/seed-persons.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

export KUBECONFIG="${KUBECONFIG:-${HOME}/.kube/insight.kubeconfig}"

echo "=== Seed: identity_inputs -> MariaDB persons ==="

# -- Resolve ClickHouse credentials ---------------------------------------
# Falls back to the umbrella's auto-generated `insight-db-creds` Secret —
# the umbrella chart (charts/insight/templates/secrets.yaml) emits this
# Secret with `clickhouse-password` filled in on every install/upgrade.
# Operators can still override via CLICKHOUSE_PASSWORD env var.
CH_PASS="${CLICKHOUSE_PASSWORD:-$(kubectl -n insight get secret insight-db-creds -o jsonpath='{.data.clickhouse-password}' | base64 -d 2>/dev/null)}"
export CLICKHOUSE_URL="${CLICKHOUSE_URL:-http://localhost:8123}"
export CLICKHOUSE_USER="${CLICKHOUSE_USER:-insight}"
export CLICKHOUSE_PASSWORD="$CH_PASS"

# -- Resolve MariaDB credentials ------------------------------------------
# All MARIADB_* values are resolved here and exported to the Python
# subprocess. URL-encode user/password so passwords containing ':', '@',
# '/', or '%' do not break URL parsing in the Python side. Same fallback
# strategy as CH: pull from `insight-db-creds` when MARIADB_PASSWORD is
# not pre-set.
MARIADB_USER="${MARIADB_USER:-insight}"
MARIADB_PASSWORD="${MARIADB_PASSWORD:-$(kubectl -n insight get secret insight-db-creds -o jsonpath='{.data.mariadb-password}' | base64 -d 2>/dev/null)}"
MARIADB_HOST="${MARIADB_HOST:-localhost}"
MARIADB_PORT="${MARIADB_PORT:-3306}"
MARIADB_DB="${MARIADB_DB:-identity}"
export MARIADB_USER MARIADB_PASSWORD MARIADB_HOST MARIADB_PORT MARIADB_DB

_USER_ENC=$(python3 -c 'import os, urllib.parse; print(urllib.parse.quote(os.environ["MARIADB_USER"], safe=""))')
_PASS_ENC=$(python3 -c 'import os, urllib.parse; print(urllib.parse.quote(os.environ["MARIADB_PASSWORD"], safe=""))')
export MARIADB_URL="mysql://${_USER_ENC}:${_PASS_ENC}@${MARIADB_HOST}:${MARIADB_PORT}/${MARIADB_DB}"

# -- Port-forward helpers -------------------------------------------------
# Use python3 for port-check instead of nc -- nc is missing on Windows
# Git Bash. The seed needs reachable endpoints for BOTH MariaDB (write
# path) and ClickHouse (read path); we auto-port-forward whichever host
# resolves to localhost, since callers running against remote managed
# instances supply their own endpoints via env.
#
# Both PF processes get tracked in `_PF_PIDS` and torn down by a single
# EXIT trap — a previous version only managed the MariaDB PF, which
# meant the script depended on the operator to keep a ClickHouse PF
# alive in another shell and silently failed (`ConnectionRefusedError`)
# the moment that PF died.
_PF_PIDS=()
trap '[[ ${#_PF_PIDS[@]} -gt 0 ]] && kill "${_PF_PIDS[@]}" 2>/dev/null || true' EXIT

_port_open() { # host port
  python3 -c "
import socket, sys
s = socket.socket()
s.settimeout(0.5)
try:
    s.connect(('$1', $2))
except OSError:
    sys.exit(1)
"
}

_ensure_pf() { # label svc host port containerPort
  local label="$1" svc="$2" host="$3" port="$4" containerPort="$5"
  if [[ "$host" != "localhost" && "$host" != "127.0.0.1" ]]; then
    return 0  # remote endpoint — trust caller
  fi
  if _port_open "$host" "$port"; then
    return 0  # already reachable
  fi
  echo "  Starting $label port-forward..."
  kubectl -n insight port-forward "svc/$svc" "${port}:${containerPort}" >/dev/null 2>&1 &
  _PF_PIDS+=("$!")
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if _port_open "$host" "$port"; then
      return 0
    fi
    sleep 1
  done
  echo "ERROR: $label port-forward did not become ready within 10s." >&2
  echo "  Check: kubectl -n insight get pods -l app.kubernetes.io/name=$svc" >&2
  exit 1
}

# -- Ensure both port-forwards --------------------------------------------
_ensure_pf "MariaDB"     "insight-mariadb"    "$MARIADB_HOST" "$MARIADB_PORT" 3306

# Parse ClickHouse host/port from the URL the Python script will use.
_CH_HOST=$(python3 -c "from urllib.parse import urlparse; u=urlparse('$CLICKHOUSE_URL'); print(u.hostname or 'localhost')")
_CH_PORT=$(python3 -c "from urllib.parse import urlparse; u=urlparse('$CLICKHOUSE_URL'); print(u.port or 8123)")
_ensure_pf "ClickHouse"  "insight-clickhouse" "$_CH_HOST"     "$_CH_PORT"     8123

# -- Run seed -------------------------------------------------------------
echo "  Running seed script..."
pip install pymysql --quiet 2>/dev/null || true
python3 "$SCRIPT_DIR/seed-persons-from-identity-input.py"
