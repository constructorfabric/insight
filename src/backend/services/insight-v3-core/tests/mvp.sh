#!/usr/bin/env bash
set -euo pipefail

service_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_file="$service_dir/tests/fixtures/events.jsonl"
pulls_fixture_file="$service_dir/tests/fixtures/pull_requests.jsonl"

port="${INSIGHT_V3_CORE_PORT:-8087}"
base_url="http://127.0.0.1:$port"

clickhouse_url="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_URL:-http://127.0.0.1:${CLICKHOUSE_HTTP_PORT:-8123}}"
clickhouse_database="${INSIGHT_V3_CORE_TEST_CLICKHOUSE_DATABASE:-insight}"
clickhouse_user="${CLICKHOUSE_USER:-insight}"
clickhouse_password="${CLICKHOUSE_PASSWORD:-insight-local}"

table_name="mvp_events_$$"
metric_name="commits_per_day_$$"
widget_table_name="commits_table_$$"
widget_graph_name="commits_graph_$$"
dashboard_name="engineering_$$"

pulls_table_name="mvp_pull_requests_$$"
opened_metric_name="prs_opened_$$"
merged_metric_name="prs_merged_$$"
by_repo_metric_name="prs_by_repo_$$"
opened_widget_name="prs_opened_line_$$"
merged_widget_name="prs_merged_stat_$$"
by_repo_widget_name="prs_by_repo_stat_$$"
pulls_dashboard_name="pull_requests_$$"

log_file="$(mktemp)"
body_file="$(mktemp)"
table_created="false"
definitions_created="false"

cleanup() {
  status=$?
  trap - EXIT
  if [[ "$table_created" == "true" ]]; then
    for dropped in "$table_name" "$pulls_table_name"; do
      curl --silent --show-error --connect-timeout 2 --max-time 10 \
        --user "$clickhouse_user:$clickhouse_password" \
        --data-binary "DROP TABLE IF EXISTS $dropped" \
        "$clickhouse_url/?database=$clickhouse_database" >/dev/null 2>&1 || true
    done
  fi
  if [[ "$definitions_created" == "true" ]]; then
    curl --silent --show-error --connect-timeout 2 --max-time 10 \
      --user "$clickhouse_user:$clickhouse_password" \
      --data-binary "ALTER TABLE metrics DELETE WHERE name IN ('$metric_name', '$opened_metric_name', '$merged_metric_name', '$by_repo_metric_name')" \
      "$clickhouse_url/?database=$clickhouse_database" >/dev/null 2>&1 || true
    curl --silent --show-error --connect-timeout 2 --max-time 10 \
      --user "$clickhouse_user:$clickhouse_password" \
      --data-binary "ALTER TABLE widgets DELETE WHERE name IN ('$widget_table_name', '$widget_graph_name', '$opened_widget_name', '$merged_widget_name', '$by_repo_widget_name', '${by_repo_widget_name}_line')" \
      "$clickhouse_url/?database=$clickhouse_database" >/dev/null 2>&1 || true
    curl --silent --show-error --connect-timeout 2 --max-time 10 \
      --user "$clickhouse_user:$clickhouse_password" \
      --data-binary "ALTER TABLE dashboards DELETE WHERE name IN ('$dashboard_name', '$pulls_dashboard_name')" \
      "$clickhouse_url/?database=$clickhouse_database" >/dev/null 2>&1 || true
  fi
  if [[ "$status" != "0" ]] && [[ -s "$log_file" ]]; then
    cat "$log_file" >&2
  fi
  rm -f "$log_file" "$body_file"
  exit "$status"
}
trap cleanup EXIT

step() {
  printf '\n== step %s: %s ==\n' "$1" "$2"
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

http() {
  local method="$1"
  local path="$2"
  shift 2
  curl --silent --show-error --connect-timeout 2 --max-time 10 \
    --request "$method" \
    --output "$body_file" --write-out '%{http_code}' \
    "$@" \
    "$base_url$path"
}

step 0 "read the ingest token and confirm the stack is reachable"

token="${INSIGHT_V3_INGEST_TOKEN:-}"
if [[ -z "$token" ]]; then
  echo "INSIGHT_V3_INGEST_TOKEN is not set. Export it to the value insight-v3-core was started with." >&2
  exit 1
fi
if (( ${#token} < 32 )); then
  echo "INSIGHT_V3_INGEST_TOKEN must be at least 32 bytes (got ${#token})." >&2
  exit 1
fi
echo "ingest token present (${#token} bytes)"

status="$(http GET /health)"
if [[ "$status" != "200" ]]; then
  echo "insight-v3-core is not reachable at $base_url (GET /health returned $status)." >&2
  echo "This script does not start the stack — bring it up yourself first." >&2
  exit 1
fi
echo "stack reachable at $base_url"

step 1 "PUT /v1/tables/$table_name"
status="$(http PUT "/v1/tables/$table_name" --header "X-Insight-Token: $token")"
expect_equal "204" "$status" "table creation"
table_created="true"
echo "-> $status"

step 2 "POST /v1/raw-data for each fixture line"
line_count=0
while IFS= read -r line || [[ -n "$line" ]]; do
  [[ -z "$line" ]] && continue
  line_count=$((line_count + 1))
  request_body="$(jq -c --arg table "$table_name" '{table: $table, raw_data: .}' <<<"$line")"
  status="$(http POST /v1/raw-data \
    --header "X-Insight-Token: $token" \
    --header 'content-type: application/json' \
    --data-binary "$request_body")"
  expect_equal "204" "$status" "raw-data insertion (line $line_count)"
done <"$fixture_file"
expect_equal "30" "$line_count" "fixture line count"
echo "-> ingested $line_count events"

step 3 "PUT /v1/metrics/$metric_name"
metric_body="$(jq -n --arg table "$table_name" '{
  table: $table,
  fields: [
    {json: "day", type: "string", as_name: "day"},
    {json: "lines", type: "int", agg: "sum", as_name: "lines"}
  ],
  group_by: ["day"],
  filters: [],
  limit: 100
}')"
status="$(http PUT "/v1/metrics/$metric_name" \
  --header 'content-type: application/json' \
  --data-binary "$metric_body")"
expect_equal "204" "$status" "metric definition"
definitions_created="true"
echo "-> $status"

step 4 "POST /v1/metrics/$metric_name/run"
status="$(http POST "/v1/metrics/$metric_name/run")"
response_body="$(cat "$body_file")"
expect_equal "200" "$status" "metric run"
columns="$(jq -r '.columns | join(",")' <<<"$response_body")"
expect_equal "day,lines" "$columns" "metric run columns"
row_count="$(jq '.rows | length' <<<"$response_body")"
expect_equal "10" "$row_count" "metric run row count"
day_one_lines="$(jq -r '.rows[] | select(.[0] == "2026-09-01") | .[1]' <<<"$response_body")"
expect_equal "59" "$day_one_lines" "commits_per_day sum for 2026-09-01"
echo "-> $status, $row_count rows, columns=$columns"

step 5 "PUT /v1/widgets/$widget_table_name"
status="$(http PUT "/v1/widgets/$widget_table_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg metric "$metric_name" '{type: "table", metric: $metric, columns: ["day", "lines"]}')")"
expect_equal "204" "$status" "table widget"
echo "-> $status"

step 6 "PUT /v1/widgets/$widget_graph_name"
status="$(http PUT "/v1/widgets/$widget_graph_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg metric "$metric_name" '{type: "line", metric: $metric, x: "day", y: "lines"}')")"
expect_equal "204" "$status" "line widget"
echo "-> $status"

step 7 "PUT /v1/dashboards/$dashboard_name"
status="$(http PUT "/v1/dashboards/$dashboard_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg t "$widget_table_name" --arg g "$widget_graph_name" \
    '{title: "Engineering", widgets: [$t, $g]}')")"
expect_equal "204" "$status" "dashboard definition"
echo "-> $status"

step 8 "GET /v1/dashboards/$dashboard_name"
status="$(http GET "/v1/dashboards/$dashboard_name")"
response_body="$(cat "$body_file")"
expect_equal "200" "$status" "dashboard read"
has_table="$(jq --arg n "$widget_table_name" '.widgets | index($n) != null' <<<"$response_body")"
has_graph="$(jq --arg n "$widget_graph_name" '.widgets | index($n) != null' <<<"$response_body")"
expect_equal "true" "$has_table" "dashboard widgets include $widget_table_name"
expect_equal "true" "$has_graph" "dashboard widgets include $widget_graph_name"
echo "-> $status, widgets=$(jq -c '.widgets' <<<"$response_body")"

step 9 "PUT /v1/tables/$pulls_table_name"
status="$(http PUT "/v1/tables/$pulls_table_name" --header "X-Insight-Token: $token")"
expect_equal "204" "$status" "pull-request table creation"
echo "-> $status"

step 10 "POST /v1/raw-data for each pull request"
pull_count=0
while IFS= read -r line || [[ -n "$line" ]]; do
  [[ -z "$line" ]] && continue
  pull_count=$((pull_count + 1))
  request_body="$(jq -c --arg table "$pulls_table_name" '{table: $table, raw_data: .}' <<<"$line")"
  status="$(http POST /v1/raw-data \
    --header "X-Insight-Token: $token" \
    --header 'content-type: application/json' \
    --data-binary "$request_body")"
  expect_equal "204" "$status" "pull-request insertion (line $pull_count)"
done <"$pulls_fixture_file"
expected_pulls="$(grep -c '[^[:space:]]' "$pulls_fixture_file")"
expect_equal "$expected_pulls" "$pull_count" "pull-request fixture line count"
echo "-> ingested $pull_count pull requests"

step 11 "PUT /v1/metrics/$opened_metric_name — a clock on when a PR was opened"
status="$(http PUT "/v1/metrics/$opened_metric_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg table "$pulls_table_name" '{
    table: $table,
    time: {json: "opened_at"},
    fields: [{json: "pull_request", type: "int", agg: "count", as_name: "opened"}]
  }')")"
expect_equal "204" "$status" "opened metric"
definitions_created="true"
echo "-> $status"

step 12 "PUT /v1/metrics/$merged_metric_name — a second clock on the same rows"
status="$(http PUT "/v1/metrics/$merged_metric_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg table "$pulls_table_name" '{
    table: $table,
    time: {json: "merged_at"},
    fields: [{json: "pull_request", type: "int", agg: "count", as_name: "merged"}]
  }')")"
expect_equal "204" "$status" "merged metric"
echo "-> $status"

step 13 "PUT /v1/metrics/$by_repo_metric_name — no clock at all"
status="$(http PUT "/v1/metrics/$by_repo_metric_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg table "$pulls_table_name" '{
    table: $table,
    fields: [{json: "pull_request", type: "int", agg: "count", as_name: "total"}]
  }')")"
expect_equal "204" "$status" "clockless metric"
echo "-> $status"

step 14 "POST /v1/metrics/$opened_metric_name/run — one bucket a day"
status="$(http POST "/v1/metrics/$opened_metric_name/run" \
  --header 'content-type: application/json' \
  --data-binary '{"range": "P30D"}')"
response_body="$(cat "$body_file")"
expect_equal "200" "$status" "opened over the last 30 days"
columns="$(jq -r '.columns | join(",")' <<<"$response_body")"
expect_equal "bucket,opened" "$columns" "opened columns"
bucket_count="$(jq '.rows | length' <<<"$response_body")"
if (( bucket_count < 2 )); then
  echo "expected several day buckets, got $bucket_count" >&2
  exit 1
fi
echo "-> $status, $bucket_count buckets, columns=$columns"

step 15 "POST /v1/metrics/$merged_metric_name/run — one number for the window"
status="$(http POST "/v1/metrics/$merged_metric_name/run" \
  --header 'content-type: application/json' \
  --data-binary '{"range": "P30D", "bucket": false}')"
response_body="$(cat "$body_file")"
expect_equal "200" "$status" "merged in the last 30 days"
expect_equal "merged" "$(jq -r '.columns | join(",")' <<<"$response_body")" "merged columns"
expect_equal "1" "$(jq '.rows | length' <<<"$response_body")" "merged row count"
undated="$(jq -r '.undated' <<<"$response_body")"
if [[ "$undated" == "null" ]]; then
  echo "a windowed run must report how many rows carry no clock" >&2
  exit 1
fi
echo "-> $status, total=$(jq -r '.rows[0][0]' <<<"$response_body"), never merged=$undated"

step 16 "POST /v1/metrics/$by_repo_metric_name/run — a range it cannot answer"
status="$(http POST "/v1/metrics/$by_repo_metric_name/run" \
  --header 'content-type: application/json' \
  --data-binary '{"range": "P30D"}')"
expect_equal "400" "$status" "a clockless metric refuses a range"
echo "-> $status"

step 17 "PUT the three widgets"
status="$(http PUT "/v1/widgets/$opened_widget_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg metric "$opened_metric_name" \
    '{type: "line", metric: $metric, title: "Pull requests opened", x: "bucket", y: "opened"}')")"
expect_equal "204" "$status" "opened line widget"
status="$(http PUT "/v1/widgets/$merged_widget_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg metric "$merged_metric_name" \
    '{type: "stat", metric: $metric, title: "Merged in this window", value: "merged", label: "Merged"}')")"
expect_equal "204" "$status" "merged stat widget"
status="$(http PUT "/v1/widgets/$by_repo_widget_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg metric "$by_repo_metric_name" \
    '{type: "stat", metric: $metric, title: "Pull requests, all time", value: "total", label: "Total"}')")"
expect_equal "204" "$status" "clockless stat widget"
echo "-> three widgets stored"

step 18 "a clockless line cannot draw the bucket it has no clock for"
status="$(http PUT "/v1/widgets/${by_repo_widget_name}_line" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n --arg metric "$by_repo_metric_name" \
    '{type: "line", metric: $metric, x: "bucket", y: "total"}')")"
expect_equal "400" "$status" "clockless line widget"
echo "-> $status"

step 19 "PUT /v1/dashboards/$pulls_dashboard_name"
status="$(http PUT "/v1/dashboards/$pulls_dashboard_name" \
  --header 'content-type: application/json' \
  --data-binary "$(jq -n \
    --arg line "$opened_widget_name" \
    --arg stat "$merged_widget_name" \
    --arg all "$by_repo_widget_name" \
    '{
      title: "Pull requests",
      time_ranges: ["P7D", "P30D", "P1Y"],
      default_range: "P30D",
      items: [{widget: $line}, {widget: $stat}, {widget: $all}]
    }')")"
expect_equal "204" "$status" "pull-request dashboard"
echo "-> $status"

echo
echo "All 19 steps passed (tables=$table_name, $pulls_table_name)."
