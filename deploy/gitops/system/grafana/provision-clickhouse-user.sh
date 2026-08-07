#!/usr/bin/env bash
# Provision Grafana's ClickHouse access: the read-only `grafana_ro` role
# (clickhouse-role.sql) plus the grant-less user of the same name that the
# provisioned datasource connects as. Called by `make system-grafana`.
#
# This lives with Grafana rather than with the umbrella's ClickHouse
# migrations on purpose. Ingestion is always installed; Grafana is optional
# (inventory.system.grafana). A cluster without Grafana should carry no
# grafana_ro, and the thing that installs Grafana is the thing that should
# create it.
#
# Idempotent — `make system-grafana` re-runs it on every invocation and it
# converges an existing user (password and settings) rather than failing.
#
# Two modes:
#   cluster (default) — resolves credentials from Secrets in NAMESPACE and
#                       reaches ClickHouse through a temporary port-forward
#   direct            — set CLICKHOUSE_URL (plus USER/PASSWORD/GRAFANA_PASSWORD)
#                       and no kubectl is used at all; this is the path
#                       tests/test_clickhouse_role.py drives
#
# Env:
#   CLICKHOUSE_URL              http://host:8123 — enables direct mode
#   CLICKHOUSE_USER/_PASSWORD   admin, needs access_management
#   CLICKHOUSE_GRAFANA_PASSWORD the grafana_ro password
#   NAMESPACE                   cluster mode, default insight-infra
#   KUBE_CTX                    cluster mode, optional
#   CH_SECRET/CH_SECRET_KEY     cluster mode, where the admin password lives
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
NAMESPACE="${NAMESPACE:-insight-infra}"
CH_SECRET="${CH_SECRET:-clickhouse-creds}"
GRAFANA_SECRET="${GRAFANA_SECRET:-grafana-clickhouse}"
PORT_FORWARD_PID=""

cleanup() {
  [[ -z "$PORT_FORWARD_PID" ]] && return 0
  kill "$PORT_FORWARD_PID" 2>/dev/null || true
  wait "$PORT_FORWARD_PID" 2>/dev/null || true
}
trap cleanup EXIT

kube() { kubectl ${KUBE_CTX:+--context "$KUBE_CTX"} -n "$NAMESPACE" "$@"; }

# Secret value or empty string — a missing Secret is a skip, not a failure.
secret_value() {
  kube get secret "$1" -o "jsonpath={.data.$2}" 2>/dev/null | base64 -d 2>/dev/null || true
}

if [[ -z "${CLICKHOUSE_URL:-}" ]]; then
  # The admin password key differs between deployments (the shared baseline
  # names it in system/clickhouse/values.yaml), so read it from there rather
  # than hardcoding, and let the caller override.
  if [[ -z "${CH_SECRET_KEY:-}" ]]; then
    CH_SECRET_KEY="$(yq -r '.auth.existingSecretKey // "admin-password"' \
      "$SCRIPT_DIR/../clickhouse/values.yaml" 2>/dev/null || echo admin-password)"
  fi
  CLICKHOUSE_USER="${CLICKHOUSE_USER:-$(yq -r '.auth.username // "insight"' \
    "$SCRIPT_DIR/../clickhouse/values.yaml" 2>/dev/null || echo insight)}"
  CLICKHOUSE_PASSWORD="${CLICKHOUSE_PASSWORD:-$(secret_value "$CH_SECRET" "$CH_SECRET_KEY")}"
  CLICKHOUSE_GRAFANA_PASSWORD="${CLICKHOUSE_GRAFANA_PASSWORD:-$(secret_value "$GRAFANA_SECRET" password)}"

  if [[ -z "$CLICKHOUSE_PASSWORD" ]]; then
    echo "  NOTE: no $CH_SECRET/$CH_SECRET_KEY in $NAMESPACE; skipping grafana_ro provisioning"
    exit 0
  fi
  if ! kube get svc clickhouse >/dev/null 2>&1; then
    echo "  NOTE: no svc/clickhouse in $NAMESPACE (external ClickHouse?); skipping grafana_ro provisioning"
    echo "        Provision it out-of-band — see system/grafana/SECRETS.md."
    exit 0
  fi

  # An ephemeral local port: the operator's machine may already be forwarding
  # something, and this must not collide with a long-lived tunnel.
  LOCAL_PORT="${LOCAL_PORT:-$(( 20000 + RANDOM % 20000 ))}"
  # Deliberately not through the `kube` helper: backgrounding a shell function
  # makes $! the subshell's pid, and the real kubectl survives as its child —
  # the tunnel then outlives this script.
  kubectl ${KUBE_CTX:+--context "$KUBE_CTX"} -n "$NAMESPACE" \
    port-forward svc/clickhouse "${LOCAL_PORT}:8123" >/dev/null 2>&1 &
  PORT_FORWARD_PID=$!
  CLICKHOUSE_URL="http://127.0.0.1:${LOCAL_PORT}"

  for _ in $(seq 1 40); do
    curl -sf -o /dev/null "${CLICKHOUSE_URL}/ping" && break
    sleep 0.25
  done
  if ! curl -sf -o /dev/null "${CLICKHOUSE_URL}/ping"; then
    echo "  ERROR: port-forward to svc/clickhouse in $NAMESPACE never became ready"
    exit 1
  fi
fi

: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"

ch_query() {
  curl -sS --fail-with-body "${CLICKHOUSE_URL}/" \
    -H "X-ClickHouse-User: ${CLICKHOUSE_USER}" \
    -H "X-ClickHouse-Key: ${CLICKHOUSE_PASSWORD}" \
    --data-binary @-
}

# Statement-at-a-time: the HTTP interface takes one per request.
ch_script() {
  local statement
  while IFS= read -r -d ';' statement; do
    [[ -z "${statement//[[:space:]]/}" ]] && continue
    printf '%s' "$statement" | ch_query >/dev/null
  done
}

# CREATE ROLE fails without access_management.
if ! printf 'CREATE ROLE IF NOT EXISTS grafana_ro' | ch_query >/dev/null 2>&1; then
  echo "  WARN: ClickHouse admin lacks access_management; skipping grafana_ro"
  exit 0
fi

grep -v '^[[:space:]]*--' "$SCRIPT_DIR/clickhouse-role.sql" | ch_script
echo "  grafana_ro role ready"

if [[ -z "${CLICKHOUSE_GRAFANA_PASSWORD:-}" ]]; then
  echo "  NOTE: no $GRAFANA_SECRET/password; role only, no user"
  echo "        Grafana's ClickHouse datasource will fail its health check until"
  echo "        that Secret exists — see system/grafana/SECRETS.md."
  exit 0
fi

# The password rides the SQL body, so reject characters that could break out of
# the IDENTIFIED BY '...' literal.
case "${CLICKHOUSE_GRAFANA_PASSWORD}" in
  *"'"* | *"\\"* | *";"*)
    echo "  WARN: grafana_ro password has a quote/backslash/semicolon; skipping user (use alphanumeric)"
    exit 0
    ;;
esac

# Grant-less user: every privilege arrives via grafana_ro. readonly=2 is
# belt-and-braces on top of the SELECT-only role — it blocks writes and DDL at
# the settings level while still allowing SET, which the Grafana ClickHouse
# plugin needs. max_execution_time matches the datasource's queryTimeout in
# values.yaml, so a runaway panel query dies here instead of outliving the HTTP
# request that already gave up on it.
USER_SETTINGS="readonly = 2, max_execution_time = 60, max_result_rows = 2000000, result_overflow_mode = 'throw'"

ch_script <<SQL
CREATE USER IF NOT EXISTS grafana_ro IDENTIFIED BY '${CLICKHOUSE_GRAFANA_PASSWORD}' SETTINGS ${USER_SETTINGS};
ALTER USER grafana_ro IDENTIFIED BY '${CLICKHOUSE_GRAFANA_PASSWORD}' SETTINGS ${USER_SETTINGS};
GRANT grafana_ro TO grafana_ro;
ALTER USER grafana_ro DEFAULT ROLE grafana_ro;
SQL
echo "  grafana_ro user ready"
