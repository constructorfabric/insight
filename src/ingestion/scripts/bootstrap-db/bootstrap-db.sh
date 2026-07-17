#!/usr/bin/env bash
set -euo pipefail

CONFIG_FILE="${1:?usage: bootstrap-db.sh <connectors-config.yaml>}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ -f "${SCRIPT_DIR}/.env" ]]; then
  set -a
  source "${SCRIPT_DIR}/.env"
  set +a
fi

: "${CLICKHOUSE_HOST:?CLICKHOUSE_HOST must be set}"
: "${CLICKHOUSE_PORT:?CLICKHOUSE_PORT must be set}"
: "${CLICKHOUSE_PROTOCOL:?CLICKHOUSE_PROTOCOL must be set (http or https)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"
: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set}"

echo "=== Creating connector tables ==="
"${SCRIPT_DIR}/seed-connectors.sh" "${CONFIG_FILE}"

echo "=== Running all dbt models ==="
"${SCRIPT_DIR}/run-dbt.sh" || echo "dbt run finished with errors, continuing" >&2

echo "=== Applying ClickHouse migrations ==="
export CLICKHOUSE_URL="${CLICKHOUSE_PROTOCOL}://${CLICKHOUSE_HOST}:${CLICKHOUSE_PORT}"
bash "${SCRIPT_DIR}/../apply-ch-migrations.sh"
