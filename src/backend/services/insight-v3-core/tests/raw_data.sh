#!/usr/bin/env bash
set -euo pipefail

service_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
backend_dir="$(cd "$service_dir/../.." && pwd)"
clickhouse_url="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_URL:-${INTEGRATION_TESTS_CLICKHOUSE_URL:-http://127.0.0.1:18123}}"
clickhouse_database="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_DATABASE:-${INTEGRATION_TESTS_CLICKHOUSE_DATABASE:-insight}}"
datasets_database="${INSIGHT_V3_CORE_TEST_DATASETS_DATABASE:-insight_datasets}"
clickhouse_user="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_USER:-${INTEGRATION_TESTS_CLICKHOUSE_USER:-}}"
clickhouse_password="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_PASSWORD:-${INTEGRATION_TESTS_CLICKHOUSE_PASSWORD:-}}"
port="${INSIGHT_V3_CORE_TEST_PORT:-18086}"
token="${INSIGHT_V3_CORE_TEST_TOKEN:-synthetic-test-token-0123456789abcdef}"
dataset_name="synthetic_events_$$"
log_file="$(mktemp)"
pid=""
identity_port="${INSIGHT_V3_CORE_TEST_IDENTITY_PORT:-18099}"
identity_pid=""
dataset_declared="false"

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
  if [[ "$dataset_declared" == "true" ]]; then
    curl --silent --output /dev/null --max-time 10 \
      --request DELETE \
      --header "authorization: Bearer live-test-administrator" \
      "http://127.0.0.1:$port/v1/datasets/$dataset_name" || true
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

# Declaring a dataset is admin-only and the role is read from identity, which
# by design refuses rather than permits when it cannot be reached. The stub is
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
  "APP__gears__insight_v3_core__config__datasets_database=$datasets_database"
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

# A dataset's records are kept apart from the warehouse the rest of Insight
# reads, in a database this service owns.
datasets_query() {
  curl "${clickhouse_curl[@]}" \
    --data-binary "$1" \
    "$clickhouse_url/?database=$datasets_database"
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

declaration='{"title":"Synthetic events","fields":[{"name":"kind","path":"kind","type":"string"}]}'

status="$(curl --silent --connect-timeout 2 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  --request PUT \
  --header 'content-type: application/json' \
  --data-binary "$declaration" \
  "http://127.0.0.1:$port/v1/datasets/$dataset_name")"
expect_equal "403" "$status" "declaring a dataset with no caller"

# The ingest token admits a record, never a declaration: the two credentials
# answer for different things.
status="$(curl --silent --connect-timeout 2 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  --request PUT \
  --header 'content-type: application/json' \
  --header "x-insight-token: $token" \
  --data-binary "$declaration" \
  "http://127.0.0.1:$port/v1/datasets/$dataset_name")"
expect_equal "403" "$status" "declaring a dataset with the ingest token alone"

# Twice: declaring a name that stands replaces its declaration where it lies,
# so the records and the table they are in are left alone.
for _ in 1 2; do
  status="$(curl --silent --connect-timeout 2 --max-time 10 \
    --output /dev/null --write-out '%{http_code}' \
    --request PUT \
    --header 'content-type: application/json' \
    --header "authorization: Bearer live-test-administrator" \
    --data-binary "$declaration" \
    "http://127.0.0.1:$port/v1/datasets/$dataset_name")"
  expect_equal "200" "$status" "declaring a dataset as an administrator"
  dataset_declared="true"
done

# The table is named for the attempt that made it, so the name is discovered
# rather than assumed - and there is exactly one, which is what proves the
# replacement above made no second table.
physical_table="$(datasets_query "SELECT name FROM system.tables
WHERE database = currentDatabase() AND name LIKE 'ds_${dataset_name}_%'
ORDER BY name
FORMAT TSVRaw")"
expect_equal "1" "$(printf '%s\n' "$physical_table" | grep -c .)" "one table per dataset"

schema="$(datasets_query "SELECT name, type
FROM system.columns
WHERE database = currentDatabase() AND table = '$physical_table'
ORDER BY position
FORMAT TSVRaw")"
expect_equal \
  $'id\tUUID\ntable_name\tString\nraw_data\tString\nreceived_at\tDateTime64(3, \'UTC\')' \
  "$schema" \
  "the declared dataset's table"

raw_values=(
  '{"nested":[1,true,null]}'
  '[1,2,3]'
  '"scalar"'
  '42'
)
request_body="{\"dataset\":\"$dataset_name\",\"raw_data\":${raw_values[0]}}"
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

# A record only lands in a dataset somebody declared.
status="$(curl --silent --connect-timeout 2 --max-time 10 \
  --output /dev/null --write-out '%{http_code}' \
  --header 'content-type: application/json' \
  --header "x-insight-token: $token" \
  --data-binary "{\"dataset\":\"nobody_declared_this\",\"raw_data\":{}}" \
  "http://127.0.0.1:$port/v1/raw-data")"
expect_equal "404" "$status" "raw-data insertion into a dataset nobody declared"

before="$(datasets_query "SELECT count() FROM $physical_table")"
expect_equal "0" "$before" "a new dataset holds no records"

for raw_value in "${raw_values[@]}"; do
  request_body="{\"dataset\":\"$dataset_name\",\"raw_data\":$raw_value}"
  status="$(curl --silent --connect-timeout 2 --max-time 10 \
    --output /dev/null --write-out '%{http_code}' \
    --header 'content-type: application/json' \
    --header "x-insight-token: $token" \
    --data-binary "$request_body" \
    "http://127.0.0.1:$port/v1/raw-data")"
  expect_equal "204" "$status" "authenticated raw-data insertion"
done

stored="$(datasets_query "SELECT
  count(),
  countIf(raw_data = '{\"nested\":[1,true,null]}'),
  countIf(raw_data = '[1,2,3]'),
  countIf(raw_data = '\"scalar\"'),
  countIf(raw_data = '42')
FROM $physical_table
WHERE table_name = '$dataset_name'")"
expect_equal $'4\t1\t1\t1\t1' "$stored" "the records the dataset holds"

# What the dataset's page reads: the same records, through the API.
preview="$(curl --fail --silent --connect-timeout 2 --max-time 10 \
  --header "authorization: Bearer live-test-administrator" \
  "http://127.0.0.1:$port/v1/datasets/$dataset_name/records")"
expect_equal "4" "$(printf '%s' "$preview" | jq '.records | length')" "the preview reads them back"
