#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
source "$SCRIPT_DIR/../lib/ch-exec.sh"

if [[ -z "${CLICKHOUSE_MCP_PASSWORD:-}" ]]; then
  echo "  NOTE: CLICKHOUSE_MCP_PASSWORD unset; skipping MCP access"
  exit 0
fi

case "${CLICKHOUSE_MCP_PASSWORD}" in
  *"'"* | *"\\"* | *";"*)
    echo "  ERROR: CLICKHOUSE_MCP_PASSWORD must not contain quotes, backslashes, or semicolons" >&2
    exit 1
    ;;
esac

if ! printf 'CREATE ROLE IF NOT EXISTS insight_mcp_ro' | _ch_http_query >/dev/null 2>&1; then
  echo "  ERROR: ClickHouse admin lacks access_management; cannot provision MCP access" >&2
  exit 1
fi

run_ch < "$SCRIPT_DIR/mcp-role.sql"
printf 'GRANT SELECT ON `%s`.* TO insight_mcp_ro' "${CLICKHOUSE_DATABASE}" | _ch_http_query >/dev/null

run_ch <<SQL
CREATE USER IF NOT EXISTS insight_mcp IDENTIFIED BY '${CLICKHOUSE_MCP_PASSWORD}' SETTINGS
  readonly = 1,
  max_execution_time = 30,
  max_threads = 2,
  max_memory_usage = 536870912,
  max_result_rows = 5000,
  max_result_bytes = 5242880,
  result_overflow_mode = 'throw',
  max_rows_to_read = 1000000,
  max_bytes_to_read = 268435456;
ALTER USER insight_mcp IDENTIFIED BY '${CLICKHOUSE_MCP_PASSWORD}' SETTINGS
  readonly = 1,
  max_execution_time = 30,
  max_threads = 2,
  max_memory_usage = 536870912,
  max_result_rows = 5000,
  max_result_bytes = 5242880,
  result_overflow_mode = 'throw',
  max_rows_to_read = 1000000,
  max_bytes_to_read = 268435456;
GRANT insight_mcp_ro TO insight_mcp;
ALTER USER insight_mcp DEFAULT ROLE insight_mcp_ro;
SQL
echo "  insight_mcp read-only user ready"
