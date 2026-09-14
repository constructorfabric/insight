#!/usr/bin/env bash
set -euo pipefail

service_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
backend_dir="$(cd "$service_dir/../.." && pwd)"
clickhouse_url="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_URL:-${INTEGRATION_TESTS_CLICKHOUSE_URL:-http://127.0.0.1:18123}}"
clickhouse_database="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_DATABASE:-${INTEGRATION_TESTS_CLICKHOUSE_DATABASE:-insight}}"
clickhouse_user="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_USER:-${INTEGRATION_TESTS_CLICKHOUSE_USER:-}}"
clickhouse_password="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_PASSWORD:-${INTEGRATION_TESTS_CLICKHOUSE_PASSWORD:-}}"
port="${INSIGHT_V3_CORE_TEST_PORT:-18086}"
token="${INSIGHT_V3_CORE_TEST_TOKEN:-synthetic-test-token-0123456789abcdef}"
table_name="synthetic_events_$$"
log_file="$(mktemp)"
pid=""
identity_port="${INSIGHT_V3_CORE_TEST_IDENTITY_PORT:-18099}"
identity_pid=""
table_created="false"

cleanup() {
  status=$?
  trap - EXIT
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  if [[ -n "$identity_pid" ]]; then
    kill "$identity_pid" 2>/dev/null || true
    wait "$identity_pid" 2>/dev/null || true
  fi
  if [[ "$table_created" == "true" ]]; then
    clickhouse_query "DROP TABLE IF EXISTS $table_name" >/dev/null 2>&1 || true
  fi
  if [[ "$status" != "0" ]] && [[ -s "$log_file" ]]; then
    cat "$log_file" >&2
  fi
  rm -f "$log_file"
  exit "$status"
}
trap cleanup EXIT

# The definition store and the identity service the config demands at startup.
# CI hands us a MariaDB (see scripts/ci/components.py); a local run falls back
# to the compose stand's, and identity is only dialed by a request that needs a
# person resolved — no live service is required to boot.
database_url="${INTEGRATION_TESTS_MARIADB_URL:-mysql://insight:insight@127.0.0.1:3306/insight_v3}"
identity_url="${INTEGRATION_TESTS_IDENTITY_URL:-http://127.0.0.1:$identity_port}"

# Creating a table is admin-only and the role is read from identity, which by
# design refuses rather than permits when it cannot be reached. The stub is
# that service for the length of this test.
python3 "$service_dir/tests/identity-stub.py" "$identity_port" &
identity_pid=$!
for _ in $(seq 1 50); do
  if curl --silent --output /dev/null --max-time 1 \
    --header "authorization: Bearer probe" \
    "http://127.0.0.1:$identity_port/v1/me"; then
    break
  fi
  sleep 0.1
done

app_env=(
  env
  "APP__gears__insight_v3_core__config__clickhouse_url=$clickhouse_url"
  "APP__gears__insight_v3_core__config__clickhouse_database=$clickhouse_database"
  "APP__gears__insight_v3_core__config__ingest_token=$token"
  "APP__gears__insight_v3_core__config__database_url=$database_url"
  "APP__gears__insight_v3_core__config__identity_url=$identity_url"
)
clickhouse_curl=(--fail --silent --show-error --connect-timeout 2 --max-time 10)

if [[ -n "$clickhouse_user" || -n "$clickhouse_password" ]]; then
  app_env+=(
    "APP__gears__insight_v3_core__config__clickhouse_user=$clickhouse_user"
    "APP__gears__insight_v3_core__config__clickhouse_password=$clickhouse_password"
  )
  clickhouse_curl+=(--user "$clickhouse_user:$clickhouse_password")
fi

clickhouse_query() {
  curl "${clickhouse_curl[@]}" \
    --data-binary "$1" \
    "$clickhouse_url/?database=$clickhouse_database"
}

expect_equal() {
  local expected="$1"
  local actual="$2"
  local context="$3"

  if [[ "$actual" == "$expected" ]]; then
    return 0
  fi

  printf '%s: expected %q, got %q\n' "$context" "$expected" "$actual" >&2
  return 1
}

for _ in 1 2; do
  "${app_env[@]}" cargo run --quiet --manifest-path "$backend_dir/Cargo.toml" \
    --package insight-v3-core -- \
    --config "$service_dir/config/insight.yaml" migrate
done

"${app_env[@]}" \
  "APP__gears__api_gateway__config__bind_addr=127.0.0.1:$port" \
  "$backend_dir/target/debug/insight-v3-core" \
  --config "$service_dir/config/insight.yaml" run >"$log_file" 2>&1 &
pid=$!

ready="false"
for _ in {1..240}; do
  if ! kill -0 "$pid" 2>/dev/null; then
    cat "$log_file"
    exit 1
  fi

  if curl --fail --silent --connect-timeout 2 --max-time 5 \
    "http://127.0.0.1:$port/health" >/dev/null; then
    ready="true"
    break
  fi
  sleep 0.25
done
if [[ "$ready" != "true" ]]; then
  cat "$log_file"
  exit 1
fi

status="$(curl --silent --connect-timeout 2 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  --request PUT \
  "http://127.0.0.1:$port/v1/tables/$table_name")"
expect_equal "401" "$status" "table creation without a token"

# Creating a table passes two gates, not one: the instance token admits the
# request, and the caller must hold the admin role. Ingest itself keeps the
# token alone, which the POSTs below still assert.
status="$(curl --silent --connect-timeout 2 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  --request PUT \
  --header "x-insight-token: $token" \
  "http://127.0.0.1:$port/v1/tables/$table_name")"
expect_equal "403" "$status" "table creation with the token but no caller"

for _ in 1 2; do
  status="$(curl --silent --connect-timeout 2 --max-time 10 \
    --output /dev/null --write-out '%{http_code}' \
    --request PUT \
    --header "x-insight-token: $token" \
    --header "authorization: Bearer live-test-administrator" \
    "http://127.0.0.1:$port/v1/tables/$table_name")"
  expect_equal "204" "$status" "authenticated table creation"
  table_created="true"
done

schema="$(clickhouse_query "SELECT name, type
FROM system.columns
WHERE database = currentDatabase() AND table = '$table_name'
ORDER BY position
FORMAT TSVRaw")"
expect_equal \
  $'id\tUUID\ntable_name\tString\nraw_data\tString\nreceived_at\tDateTime64(3, \'UTC\')' \
  "$schema" \
  "created table schema"

raw_values=(
  '{"nested":[1,true,null]}'
  '[1,2,3]'
  '"scalar"'
  '42'
)
request_body="{\"table\":\"$table_name\",\"raw_data\":${raw_values[0]}}"
status="$(curl --silent --connect-timeout 2 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  --header 'content-type: application/json' \
  --data-binary "$request_body" \
  "http://127.0.0.1:$port/v1/raw-data")"
expect_equal "401" "$status" "raw-data insertion without a token"

status="$(curl --silent --connect-timeout 2 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  --header 'content-type: application/json' \
  --header 'x-insight-token: incorrect-token-0123456789abcdef' \
  --data-binary "$request_body" \
  "http://127.0.0.1:$port/v1/raw-data")"
expect_equal "401" "$status" "raw-data insertion with an incorrect token"

before="$(clickhouse_query "SELECT count() FROM $table_name")"
expect_equal "0" "$before" "new table row count"

for raw_value in "${raw_values[@]}"; do
  request_body="{\"table\":\"$table_name\",\"raw_data\":$raw_value}"
  status="$(curl --silent --connect-timeout 2 --max-time 10 \
    --output /dev/null --write-out '%{http_code}' \
    --header 'content-type: application/json' \
    --header "x-insight-token: $token" \
    --data-binary "$request_body" \
    "http://127.0.0.1:$port/v1/raw-data")"
  expect_equal "204" "$status" "authenticated raw-data insertion"
done

stored="$(clickhouse_query "SELECT
  count(),
  countIf(raw_data = '{\"nested\":[1,true,null]}'),
  countIf(raw_data = '[1,2,3]'),
  countIf(raw_data = '\"scalar\"'),
  countIf(raw_data = '42')
FROM $table_name
WHERE table_name = '$table_name'")"
expect_equal $'4\t1\t1\t1\t1' "$stored" "stored raw-data rows"
