#!/usr/bin/env bash
# Cut a warm installation over to the canonical tenant UUID, the way
# deploy/TENANT_IDENTITY_UPGRADE.md tells an operator to, and check every step.
#
#   1. Install BASE_SHA the way warm-upgrade-sim.sh does: its deploy hook, the
#      zoom Bronze its connector creates, and its sync of the identity map
#      views, which carry no tenant column. Then deploy this tree
#      (apply-ch-migrations.sh): it builds no model and drops those views, so
#      analytics answers not-ready instead of failing on the column until the
#      rebuild recreates them.
#   2. Hold the state a warm installation holds at that point: Bronze, a
#      history relation, operator config and the sync ledger written under a
#      legacy tenant that is not a UUID, and identity inputs the previous
#      release stamped with a hashed tenant. The history relation is built by
#      this tree's own dbt from that Bronze; the identity inputs model refuses
#      to build over it, because tenant_uuid() refuses a tenant that is not a
#      UUID, which is why dbt stays stopped until the migration.
#   3. `tenant_migration plan` and `apply`: nothing lost, tenant and keys moved.
#   4. `rollback`, then `restore` from the dump: each gives back the warm state
#      exactly.
#   5. Apply again and rebuild with `--full-refresh` the way a sync selects,
#      the history relation in the selection and the `sync:always` views
#      beside it: the identity inputs carry only the canonical tenant, the
#      identity map views are back and key on it, the history relation refuses
#      the flag and keeps every row, and `finalize` leaves no work table behind.
#
# Required env (same contract as apply-ch-migrations.sh):
#   CLICKHOUSE_URL, CLICKHOUSE_USER, CLICKHOUSE_PASSWORD, CLICKHOUSE_DATABASE
#   BASE_SHA  — commit whose tree plays the already-installed release
# Optional env, for the destination-clickhouse container that creates bronze:
#   BRONZE_CLICKHOUSE_URL — the URL that container reaches ClickHouse at
#                           (default CLICKHOUSE_URL; 127.0.0.1 is not it)
# Needs python3 (3.11+), docker, jq, yq v4 and network access to PyPI; each
# phase gets its own venv pinned from its tree's pins.env.
set -euo pipefail

: "${BASE_SHA:?BASE_SHA must be set (the commit playing the installed release)}"
: "${CLICKHOUSE_URL:?}" "${CLICKHOUSE_USER:?}" "${CLICKHOUSE_PASSWORD:?}" "${CLICKHOUSE_DATABASE:?}"

REPO_ROOT="$(git rev-parse --show-toplevel)"
SCRIPTS="$REPO_ROOT/src/ingestion/scripts"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

NEW_TENANT="0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
OLD_TENANT="example-legacy"
SOURCE="zoom-main"
FIXTURES=(bronze_zoom.users staging.zoom__users_snapshot config.task_value_map ingestion_history.sync_events)

# shellcheck source=src/ingestion/scripts/lib/ch-exec.sh
source "$SCRIPTS/lib/ch-exec.sh"

fail() {
  echo "::error title=tenant cutover::$1"
  exit 1
}

query() {
  printf '%s' "$1" | _ch_http_query | tr -d '\n'
}

migration() {
  (cd "$SCRIPTS" && python3 -m tenant_migration "$@")
}

PYTHON_BIN="$(command -v python3.12 || command -v python3.11 || command -v python3)"

dbt_venv_for() {
  local pins="$1" venv="$2"
  "$PYTHON_BIN" -m venv "$venv"
  "$venv/bin/pip" install --quiet \
    "dbt-core==$(grep '^DBT_CORE_VERSION=' "$pins" | cut -d= -f2)" \
    "dbt-clickhouse==$(grep '^DBT_CLICKHOUSE_VERSION=' "$pins" | cut -d= -f2)" pyyaml
}

# The zoom Bronze a first sync leaves: destination-clickhouse applying the
# connector's discovered catalogue, from the given tree's connector and pins.
create_zoom_bronze() {
  local tree="$1" url="${BRONZE_CLICKHOUSE_URL:-$CLICKHOUSE_URL}" protocol hostport port
  protocol="${url%%://*}"
  hostport="${url#*://}"; hostport="${hostport%%/*}"
  if [[ "$hostport" == *:* ]]; then
    port="${hostport##*:}"
  elif [[ "$protocol" == https ]]; then
    port=443
  else
    port=80
  fi
  local pins="$tree/src/ingestion/scripts/bootstrap-db/pins.env"
  CLICKHOUSE_PROTOCOL="$protocol" \
  CLICKHOUSE_HOST="${hostport%%:*}" \
  CLICKHOUSE_PORT="$port" \
  DESTINATION_CLICKHOUSE_IMAGE="$(grep '^DESTINATION_CLICKHOUSE_IMAGE=' "$pins" | cut -d= -f2)" \
  SOURCE_DECLARATIVE_MANIFEST_IMAGE="$(grep '^SOURCE_DECLARATIVE_MANIFEST_IMAGE=' "$pins" | cut -d= -f2)" \
    bash "$tree/src/ingestion/scripts/bootstrap-db/seed-connectors.sh" \
      "$tree/src/ingestion/scripts/bootstrap-db/connectors-config.yaml" zoom
}

# The previous release's sync of the identity map views and their buildable
# ancestors, with the base tree's dbt and profile.
build_base_identity_maps() {
  local tree="$1" path="$WORKDIR/base-venv/bin:$PATH" profiles="$WORKDIR/base-profiles" target="$WORKDIR/base-target"
  local project="$tree/src/ingestion/dbt" unbuildable excluded=()
  mkdir -p "$profiles"
  PATH="$path" python3 "$tree/src/ingestion/scripts/dbt_profiles.py" --target migrate \
    --profiles-dir "$profiles" --correlated-subqueries
  unbuildable="$(cd "$project" && PATH="$path" dbt --quiet run-operation list_unbuildable_models \
    --profiles-dir "$profiles" --target-path "$target" --log-path "$target/logs")"
  read -r -a excluded <<< "$unbuildable"
  (cd "$project" && PATH="$path" dbt --no-use-colors run --profiles-dir "$profiles" \
    --target-path "$target" --log-path "$target/logs" --select +account_assignment +person_map \
    ${excluded[@]+--exclude "${excluded[@]}"})
}

dbt_venv_for "$SCRIPTS/bootstrap-db/pins.env" "$WORKDIR/venv"
export PATH="$WORKDIR/venv/bin:$PATH"

DBT_PROFILES_DIR="$WORKDIR/profiles"
export DBT_PROFILES_DIR
mkdir -p "$DBT_PROFILES_DIR"
python3 "$SCRIPTS/dbt_profiles.py" --target migrate --correlated-subqueries --profiles-dir "$DBT_PROFILES_DIR"

dbt_run() {
  (cd "$REPO_ROOT/src/ingestion/dbt" && dbt run --profiles-dir "$DBT_PROFILES_DIR" "$@")
}

# Rows and a hash over every column: equal only when the table holds exactly
# the same rows.
fingerprint() {
  local relation
  for relation in "${FIXTURES[@]}"; do
    printf '%s=%s;' "$relation" "$(query "SELECT concat(toString(count()), ':', toString(groupBitXor(cityHash64(*)))) FROM $relation")"
  done
}

map_views() {
  query "SELECT arrayStringConcat(groupArray(concat(t.name, ':', toString(has(c.names, 'tenant_id')))), ',') FROM
    (SELECT name FROM system.tables WHERE database = 'identity' AND name IN ('account_assignment', 'person_map') ORDER BY name) AS t
    LEFT JOIN (SELECT table, groupArray(name) AS names FROM system.columns WHERE database = 'identity' GROUP BY table) AS c
    ON c.table = t.name"
}

echo "=== Step 1: install base $(git -C "$REPO_ROOT" rev-parse --short "$BASE_SHA"), then deploy this tree over its identity map views ==="
BASE="$WORKDIR/base"
mkdir -p "$BASE"
git -C "$REPO_ROOT" archive "$BASE_SHA" src/ingestion | tar -x -C "$BASE"
dbt_venv_for "$BASE/src/ingestion/scripts/bootstrap-db/pins.env" "$WORKDIR/base-venv"
PATH="$WORKDIR/base-venv/bin:$PATH" bash "$BASE/src/ingestion/scripts/apply-ch-migrations.sh"
create_zoom_bronze "$BASE"
build_base_identity_maps "$BASE"

[[ "$(map_views)" == "account_assignment:0,person_map:0" ]] \
  || fail "the base sync must leave identity map views without a tenant column — got '$(map_views)'"

bash "$SCRIPTS/apply-ch-migrations.sh"

[[ "$(map_views)" == "" ]] \
  || fail "the deploy must drop identity map views without a tenant column, so analytics answers not-ready until a sync rebuilds them — got '$(map_views)'"

echo "=== Step 2: the warm state, under the legacy tenant '$OLD_TENANT' ==="
# Rows of the zoom users stream in the Bronze its connector created, with the
# columns the snapshot tracks; keys follow the connector's formula.
USER_COLUMNS="unique_key, tenant_id, source_id, id, first_name, last_name, display_name, email, dept, status,
    role_id, timezone, language, phone_number, employee_unique_id, type,
    _airbyte_raw_id, _airbyte_extracted_at, _airbyte_meta, _airbyte_generation_id"
run_ch <<SQL
INSERT INTO bronze_zoom.users ($USER_COLUMNS) VALUES
    ('$OLD_TENANT-$SOURCE-u1', '$OLD_TENANT', '$SOURCE', 'u1', 'Ada', 'Example', 'Ada Example',
     'ada@example.com', 'Eng', 'active', '1', 'UTC', 'en', '', 'E-1', '1', 'raw-1', '2026-01-01 00:00:00', '{}', 0),
    ('$OLD_TENANT-$SOURCE-u2', '$OLD_TENANT', '$SOURCE', 'u2', 'Bo', 'Example', 'Bo Example',
     'bo@example.com', 'Ops', 'active', '1', 'UTC', 'en', '', 'E-2', '1', 'raw-2', '2026-01-01 00:00:00', '{}', 0),
    ('-$SOURCE-u3', NULL, '$SOURCE', 'u3', 'Cy', 'Example', 'Cy Example',
     'cy@example.com', 'Ops', 'active', '1', 'UTC', 'en', '', 'E-3', '1', 'raw-3', '2026-01-01 00:00:00', '{}', 0);
INSERT INTO ingestion_history.sync_events (tick_id, job_id, connector, tenant_id, source_id, event, status)
VALUES ('tick-1', 'job-1', 'zoom', '$OLD_TENANT', '$SOURCE', 'sync', 'succeeded');
SQL

dbt_run --select zoom__users_snapshot zoom__users_fields_history

# The config tables come from dbt's on-run-start, which the deploy no longer runs.
run_ch <<SQL
INSERT INTO config.task_value_map (tenant_id, insight_source_id, data_source, field_id, value_id, valid_from, recorded_at, canonical_value)
VALUES ('$OLD_TENANT', 'jira-main', 'insight_jira', 'status', '10001', '2026-01-01 00:00:00', '2026-01-01 00:00:00', 'done');
SQL

# A second observation of u1, so the history relation holds two versions.
run_ch <<SQL
INSERT INTO bronze_zoom.users ($USER_COLUMNS) VALUES
    ('$OLD_TENANT-$SOURCE-u1', '$OLD_TENANT', '$SOURCE', 'u1', 'Ada', 'Example', 'Ada Example',
     'ada@example.com', 'Research', 'active', '1', 'UTC', 'en', '', 'E-1', '1', 'raw-4', '2026-01-02 00:00:00', '{}', 0);
SQL
dbt_run --select zoom__users_snapshot

if dbt_run --select zoom__identity_inputs >"$WORKDIR/refused.log" 2>&1; then
  fail "zoom__identity_inputs built over Bronze stamped '$OLD_TENANT': tenant_uuid() must refuse a tenant that is not a UUID"
fi
grep -q "CANNOT_PARSE_UUID" "$WORKDIR/refused.log" \
  || fail "zoom__identity_inputs failed over Bronze stamped '$OLD_TENANT', but not on its tenant: $(tail -c 2000 "$WORKDIR/refused.log")"

# What the previous release left in the identity inputs: the tenant hashed. The
# base sync built the relation; this tree's producer refuses the legacy tenant,
# so both are run empty — this tree's shape where it differs — and the relation
# is given that row.
dbt_run --empty --select zoom__identity_inputs identity_inputs
run_ch <<SQL
INSERT INTO identity.identity_inputs (unique_key, insight_tenant_id, insight_source_id, insight_source_type,
    source_account_id, value_type, value, value_field_name, operation_type, _synced_at, _version)
VALUES ('$OLD_TENANT-$SOURCE-zoom-u1-email-UPSERT-1', toUUID(UUIDNumToString(sipHash128('$OLD_TENANT'))),
    toUUID(UUIDNumToString(sipHash128('$SOURCE'))), 'zoom', 'u1', 'email', 'ada@example.com',
    'bronze_zoom.users.email', 'UPSERT', '2026-01-01 00:00:00', 1);
SQL

# Merged up front: a background merge must not change a fingerprint.
run_ch <<<"OPTIMIZE TABLE bronze_zoom.users FINAL"
WARM="$(fingerprint)"
SNAPSHOT_ROWS="$(query "SELECT count() FROM staging.zoom__users_snapshot")"
[[ "$SNAPSHOT_ROWS" == "4" ]] || fail "the warm history relation holds $SNAPSHOT_ROWS rows, expected 4 (three users, u1 twice)"

echo "=== Step 3: plan and apply ==="
migration plan --tenant "$NEW_TENANT" --old-tenant "$OLD_TENANT" | tee "$WORKDIR/plan.txt"
for relation in "${FIXTURES[@]}"; do
  grep -q "^$relation " "$WORKDIR/plan.txt" || fail "the plan does not list $relation"
done
migration apply --tenant "$NEW_TENANT" --old-tenant "$OLD_TENANT" --dump-dir "$WORKDIR/dump-1"

assert_migrated() {
  local got expected="4:0:3|3:3|1:0|1:0|$NEW_TENANT-$SOURCE-u1"
  got="$(query "SELECT concat(
      toString((SELECT count() FROM staging.zoom__users_snapshot)), ':',
      toString((SELECT countIf(tenant_id != '$NEW_TENANT' OR NOT startsWith(unique_key, '$NEW_TENANT-$SOURCE-')) FROM staging.zoom__users_snapshot)), ':',
      toString((SELECT uniqExact(unique_key) FROM staging.zoom__users_snapshot)), '|',
      toString((SELECT count() FROM bronze_zoom.users)), ':',
      toString((SELECT countIf(tenant_id = '$NEW_TENANT' AND startsWith(unique_key, '$NEW_TENANT-$SOURCE-')) FROM bronze_zoom.users FINAL)), '|',
      toString((SELECT count() FROM config.task_value_map)), ':',
      toString((SELECT countIf(tenant_id != '$NEW_TENANT' OR NOT startsWith(unique_key, '$NEW_TENANT:')) FROM config.task_value_map)), '|',
      toString((SELECT count() FROM ingestion_history.sync_events)), ':',
      toString((SELECT countIf(tenant_id != '$NEW_TENANT') FROM ingestion_history.sync_events)), '|',
      (SELECT min(unique_key) FROM bronze_zoom.users WHERE id = 'u1'))")"
  [[ "$got" == "$expected" ]] || fail "after apply expected '$expected' — every row kept, each tenant column and key prefix on $NEW_TENANT, the row without a tenant moved too — but got '$got'"
}
assert_migrated

echo "=== Step 4: rollback and restore give the warm state back ==="
migration rollback
[[ "$(fingerprint)" == "$WARM" ]] || fail "rollback did not put the warm tables back exactly: '$(fingerprint)' != '$WARM'"
migration finalize

migration apply --tenant "$NEW_TENANT" --old-tenant "$OLD_TENANT" --dump-dir "$WORKDIR/dump-2"
migration finalize
migration restore --dump-dir "$WORKDIR/dump-2"
[[ "$(fingerprint)" == "$WARM" ]] || fail "restore did not load the dump back exactly: '$(fingerprint)' != '$WARM'"
migration finalize

echo "=== Step 5: apply, rebuild, finalize ==="
migration apply --tenant "$NEW_TENANT" --old-tenant "$OLD_TENANT" --dump-dir "$WORKDIR/dump-3"
assert_migrated
# Gold is left out: its views read the evidence of every connector class, and
# this warm state synced only one.
dbt_run --full-refresh --select zoom__users_snapshot+ tag:sync:always --exclude tag:gold

[[ "$(map_views)" == "account_assignment:1,person_map:1" ]] \
  || fail "the rebuild must recreate the identity map views keyed by tenant — got '$(map_views)'"

got="$(query "SELECT concat(
    toString(count()), ':',
    toString(countIf(toString(insight_tenant_id) != '$NEW_TENANT')), ':',
    toString(countIf(value = 'ada@example.com')), ':',
    toString(countIf(value = 'cy@example.com')), '|',
    toString((SELECT count() FROM staging.zoom__users_snapshot)))
  FROM identity.identity_inputs FINAL WHERE value_type = 'email'")"
[[ "$got" =~ ^[1-9][0-9]*:0:[1-9][0-9]*:[1-9][0-9]*\|4$ ]] \
  || fail "after the rebuild expected the email identity inputs all under $NEW_TENANT (the hashed-tenant row gone, every user present) and the history relation, which refuses a full refresh, still holding its 4 rows — got '$got' (inputs:foreign:ada:cy|history)"

migration finalize
leftovers="$(query "SELECT count() FROM system.tables WHERE position(name, '__tenant_') > 0")"
[[ "$leftovers" == "0" ]] || fail "finalize left $leftovers work table(s) behind"
migration plan --tenant "$NEW_TENANT" --old-tenant "$OLD_TENANT" | tee "$WORKDIR/replan.txt"
grep -q "^tables changing: 0;" "$WORKDIR/replan.txt" || fail "a second plan still finds rows to move"

echo "=== Tenant cutover OK: nothing lost, keys moved, rollback and restore exact, rebuild on the canonical tenant ==="
