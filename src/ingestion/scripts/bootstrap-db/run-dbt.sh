#!/usr/bin/env bash
set -euo pipefail

: "${CLICKHOUSE_HOST:?CLICKHOUSE_HOST must be set}"
: "${CLICKHOUSE_PORT:?CLICKHOUSE_PORT must be set}"
: "${CLICKHOUSE_PROTOCOL:?CLICKHOUSE_PROTOCOL must be set (http or https)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DBT_DIR="$(cd "${SCRIPT_DIR}/../../dbt" && pwd)"

PROFILES_DIR="$(mktemp -d)"
trap 'rm -rf "${PROFILES_DIR}"' EXIT

if [[ "${CLICKHOUSE_PROTOCOL}" == "https" ]]; then
  SECURE=true
else
  SECURE=false
fi

cat > "${PROFILES_DIR}/profiles.yml" <<EOF
ingestion:
  target: bootstrap
  outputs:
    bootstrap:
      type: clickhouse
      host: ${CLICKHOUSE_HOST}
      port: ${CLICKHOUSE_PORT}
      schema: silver
      user: ${CLICKHOUSE_USER}
      password: "{{ env_var('CLICKHOUSE_PASSWORD') }}"
      secure: ${SECURE}
      send_receive_timeout: 1500
      query_limit: 0
      connect_timeout: 30
EOF

cd "${DBT_DIR}"
dbt run --profiles-dir "${PROFILES_DIR}" "$@"
