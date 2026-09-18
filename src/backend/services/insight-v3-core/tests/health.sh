#!/usr/bin/env bash
set -euo pipefail

service_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
backend_dir="$(cd "$service_dir/../.." && pwd)"
port="${INSIGHT_V3_CORE_TEST_PORT:-18086}"
clickhouse_url="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_URL:-${INTEGRATION_TESTS_CLICKHOUSE_URL:-http://127.0.0.1:18123}}"
log_file="$(mktemp)"
pid=""

# Invoked by the EXIT trap below.
# shellcheck disable=SC2329
cleanup() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -f "$log_file"
}
trap cleanup EXIT

# The definition store and the identity service the config demands at startup.
# CI hands us a MariaDB (see scripts/ci/components.py); a local run falls back
# to the compose stand's, and identity is only dialed by a request that needs a
# person resolved — no live service is required to boot.
database_url="${INTEGRATION_TESTS_MARIADB_URL:-mysql://insight:insight@127.0.0.1:3306/insight_v3}"
identity_url="${INTEGRATION_TESTS_IDENTITY_URL:-http://identity.invalid}"

healthcheck() {
  local status
  status="$(curl --connect-timeout 2 --max-time 5 --silent \
    --output /dev/null --write-out '%{http_code}' "$1")" || return
  [[ "$status" == "200" ]]
}

APP__gears__api_gateway__config__bind_addr="127.0.0.1:$port" \
  APP__gears__insight_v3_core__config__clickhouse_url="$clickhouse_url" \
  APP__gears__insight_v3_core__config__clickhouse_database="insight" \
  APP__gears__insight_v3_core__config__ingest_token="health-test-token-0123456789abcdefghi" \
  APP__gears__insight_v3_core__config__database_url="$database_url" \
  APP__gears__insight_v3_core__config__identity_url="$identity_url" \
  cargo run --quiet --manifest-path "$backend_dir/Cargo.toml" \
  --package insight-v3-core -- \
  --config "$service_dir/config/insight.yaml" run >"$log_file" 2>&1 &
pid=$!

for _ in {1..240}; do
  if ! kill -0 "$pid" 2>/dev/null; then
    cat "$log_file"
    exit 1
  fi

  if healthcheck "http://127.0.0.1:$port/health" && \
     healthcheck "http://127.0.0.1:$port/healthz"; then
    exit 0
  fi

  sleep 0.25
done

cat "$log_file"
exit 1
