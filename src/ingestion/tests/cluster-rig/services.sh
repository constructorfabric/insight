#!/usr/bin/env bash
# Sourced by run.sh: the two services that write ClickHouse themselves, run
# from this tree's binaries against the rig's warehouse through the proxy, and
# what every node must hold afterwards.
#
# identity-resolution's persons sync stages a snapshot of its MariaDB persons
# log, counts it back, guards the published watermark and swaps it live — the
# one caller of insight-clickhouse's quorum pair. insight-v3-core declares a
# dataset into a table of its own, takes records into it from several callers
# at once, pages them back and drops the table with the dataset.
#
# Expects run.sh's RIG_DIR, WORKDIR, COMPOSE, RIG_PROXY, CLICKHOUSE_* and
# check(). RIG_SERVICES_BIN names a directory holding both binaries; otherwise
# they are built from the tree (debug, like ci.yml's live lanes).

BACKEND="$(cd -- "${RIG_DIR}/../../../backend" && pwd)"
SERVICES_BIN="${RIG_SERVICES_BIN:-${CARGO_TARGET_DIR:-${BACKEND}/target}/debug}"
MARIADB_URL="mysql://insight:insight@127.0.0.1:18306"
V3_CORE_PORT=18086
IDENTITY_STUB_PORT=18099
V3_CORE_INGEST_TOKEN="cluster-rig-ingest-token-0123456789abcdef"
RIG_TENANT="a4d11000-0000-4000-8000-00000000c1a0"
RIG_SOURCE="a4d11000-0000-4000-8000-00000000c1a1"
PERSONS=5
DATASET_RECORDS=32
DATASET_WRITERS=8

services_build() {
  if [[ -n "${RIG_SERVICES_BIN:-}" ]]; then
    if [[ ! -x "${SERVICES_BIN}/identity-resolution" || ! -x "${SERVICES_BIN}/insight-v3-core" ]]; then
      echo "  RIG_SERVICES_BIN=${SERVICES_BIN} lacks identity-resolution or insight-v3-core" >&2
      return 1
    fi
    echo "  ${SERVICES_BIN}: identity-resolution + insight-v3-core"
    return
  fi
  echo "  building identity-resolution + insight-v3-core into ${SERVICES_BIN}"
  cargo build --quiet --manifest-path "${BACKEND}/Cargo.toml" -p identity-resolution -p insight-v3-core
}

# Every service reads its warehouse and topology the way the chart hands them
# over: APP__ overrides on its own config file.
identity_env() {
  env \
    "APP__gears__identity_resolution__config__database_url=${MARIADB_URL}/identity" \
    "APP__gears__identity_resolution__config__clickhouse_url=${RIG_PROXY}" \
    "APP__gears__identity_resolution__config__clickhouse_user=${CLICKHOUSE_USER}" \
    "APP__gears__identity_resolution__config__clickhouse_password=${CLICKHOUSE_PASSWORD}" \
    "APP__gears__identity_resolution__config__clickhouse_cluster_mode=${CLICKHOUSE_CLUSTER_MODE}" \
    "APP__gears__identity_resolution__config__clickhouse_cluster_name=${CLICKHOUSE_CLUSTER_NAME}" \
    "APP__gears__identity_resolution__config__tenant_default_id=${RIG_TENANT}" \
    "${SERVICES_BIN}/identity-resolution" -c "${BACKEND}/services/identity-resolution/config/insight.yaml" "$@"
}

# SAFETY: an array, not a function: the server is backgrounded, and `$!` must
# be the binary's own pid to stop it, not a subshell's.
v3_core_env() {
  V3_CORE_ENV=(
    env
    "APP__gears__insight_v3_core__config__database_url=${MARIADB_URL}/insight_v3"
    "APP__gears__insight_v3_core__config__clickhouse_url=${RIG_PROXY}"
    "APP__gears__insight_v3_core__config__clickhouse_database=${CLICKHOUSE_DATABASE}"
    "APP__gears__insight_v3_core__config__clickhouse_user=${CLICKHOUSE_USER}"
    "APP__gears__insight_v3_core__config__clickhouse_password=${CLICKHOUSE_PASSWORD}"
    "APP__gears__insight_v3_core__config__clickhouse_cluster_mode=${CLICKHOUSE_CLUSTER_MODE}"
    "APP__gears__insight_v3_core__config__clickhouse_cluster_name=${CLICKHOUSE_CLUSTER_NAME}"
    "APP__gears__insight_v3_core__config__ingest_token=${V3_CORE_INGEST_TOKEN}"
    "APP__gears__insight_v3_core__config__identity_url=http://127.0.0.1:${IDENTITY_STUB_PORT}"
    "APP__gears__api_gateway__config__bind_addr=127.0.0.1:${V3_CORE_PORT}"
    "${SERVICES_BIN}/insight-v3-core" -c "${BACKEND}/services/insight-v3-core/config/insight.yaml"
  )
}

# The persons log the sync copies: one tenant, PERSONS people with an e-mail each.
seed_persons_log() {
  local values=() joined i
  for ((i = 1; i <= PERSONS; i++)); do
    values+=("('email', 'rig', UNHEX(REPLACE('${RIG_SOURCE}', '-', '')), UNHEX(REPLACE('${RIG_TENANT}', '-', '')), \
      'person${i}@example.com', UNHEX(REPLACE('a4d11000-0000-4000-8000-0000000000$(printf '%02x' "${i}")', '-', '')), \
      UNHEX(REPLACE('a4d11000-0000-4000-8000-0000000000$(printf '%02x' "${i}")', '-', '')), 'cluster-rig')")
  done
  printf -v joined '%s,' "${values[@]}"
  "${COMPOSE[@]}" exec -T mariadb mariadb -uinsight -pinsight identity -e \
    "INSERT INTO persons (value_type, insight_source_type, insight_source_id, insight_tenant_id, value_id, person_id, author_person_id, reason) VALUES ${joined%,}"
}

# Round 1 publishes onto an install without the live table (the sync renames
# its staging into place); round 2 finds the table dbt's hook created and
# swaps behind the watermark guard. Both stage, count back and swap through
# the proxy.
persons_sync() {
  local round="$1" force=()
  local log="${WORKDIR}/persons-sync-${round}.log" boundary="${WORKDIR}/persons-sync-${round}.boundary.json"
  if [[ "${round}" == 1 ]]; then
    identity_env migrate > "${log}" 2>&1 || { tail -n 30 "${log}" >&2; return 1; }
    seed_persons_log
  else
    # The log did not move since the last publish; --force republishes anyway.
    force=(--force)
  fi
  check quorum-boundary --out "${boundary}"
  if ! identity_env sync ${force[@]+"${force[@]}"} > "${log}" 2>&1; then
    echo "  persons sync failed (exit $?):" >&2
    tail -n 30 "${log}" >&2
    return 1
  fi
  echo "  $(tail -n 1 "${log}" | cut -c 1-200)"
  check relation --name identity.identity_persons --rows "${PERSONS}"
  check quorum --after "${boundary}"
}

stop_v3_core() {
  for pid in "${V3_CORE_PID:-}" "${IDENTITY_STUB_PID:-}"; do
    if [[ -n "${pid}" ]]; then
      kill "${pid}" 2>/dev/null || true
      wait "${pid}" 2>/dev/null || true
    fi
  done
  V3_CORE_PID="" IDENTITY_STUB_PID=""
}

start_v3_core() {
  local round="$1" i
  local log="${WORKDIR}/v3-core-${round}.log"
  python3 "${BACKEND}/services/insight-v3-core/tests/identity-stub.py" "${IDENTITY_STUB_PORT}" &
  IDENTITY_STUB_PID=$!
  v3_core_env
  "${V3_CORE_ENV[@]}" migrate > "${log}" 2>&1 || { tail -n 30 "${log}" >&2; return 1; }
  "${V3_CORE_ENV[@]}" run >> "${log}" 2>&1 &
  V3_CORE_PID=$!
  for i in $(seq 1 120); do
    if ! kill -0 "${V3_CORE_PID}" 2>/dev/null; then
      tail -n 30 "${log}" >&2
      return 1
    fi
    if curl -fs --max-time 2 "http://127.0.0.1:${V3_CORE_PORT}/health" > /dev/null; then
      return 0
    fi
    sleep 0.5
  done
  echo "  insight-v3-core did not come up on :${V3_CORE_PORT}" >&2
  tail -n 30 "${log}" >&2
  return 1
}

v3_core_call() {
  local expected="$1" what="$2" status
  shift 2
  status="$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 "$@")"
  if [[ "${status}" != "${expected}" ]]; then
    echo "  ${what}: HTTP ${status}, expected ${expected}" >&2
    return 1
  fi
}

# One dataset per round: declared, filled by DATASET_WRITERS callers at once,
# paged back, found on every node, dropped, gone from every node. The two
# halves stand alone for a leg that changes the warehouse between them.
# INVARIANT: called under `if`, where `set -e` is off: every step returns on
# its own.
dataset_lifecycle() {
  dataset_declared "$1" && dataset_dropped "$1"
}

dataset_declared() {
  local round="$1" url="http://127.0.0.1:${V3_CORE_PORT}" total i statuses
  local name="rig_events_${round}"
  local admin=(--header "authorization: Bearer cluster-rig-administrator")
  v3_core_call 200 "declaring ${name}" --request PUT "${admin[@]}" \
    --header 'content-type: application/json' \
    --data-binary '{"title":"Rig events","fields":[{"name":"n","path":"n","type":"int"}]}' \
    "${url}/v1/datasets/${name}" || return 1

  statuses="${WORKDIR}/dataset-${round}.statuses"
  seq 1 "${DATASET_RECORDS}" | xargs -P "${DATASET_WRITERS}" -I{} \
    curl -s -o /dev/null -w '%{http_code}\n' --max-time 20 \
      --header 'content-type: application/json' --header "x-insight-token: ${V3_CORE_INGEST_TOKEN}" \
      --data-binary "{\"dataset\":\"${name}\",\"raw_data\":{\"n\":{}}}" "${url}/v1/raw-data" > "${statuses}" || return 1
  if [[ "$(grep -c '^204$' "${statuses}")" != "${DATASET_RECORDS}" ]]; then
    echo "  ${DATASET_RECORDS} records posted by ${DATASET_WRITERS} writers, statuses: $(sort "${statuses}" | uniq -c | tr -s ' \n' ' ')" >&2
    return 1
  fi
  # A record is never read back by the request that made it, and the page is
  # served by whichever replica answers: the count catches up, it is not
  # confirmed (insight-clickhouse consistency.rs).
  for i in $(seq 1 40); do
    total="$(curl -fs --max-time 20 "${admin[@]}" "${url}/v1/datasets/${name}/records?limit=${DATASET_RECORDS}" \
      | jq -r 'if (.records | length) == .total then .total else empty end')"
    [[ "${total}" == "${DATASET_RECORDS}" ]] && break
    sleep 0.5
  done
  if [[ "${total}" != "${DATASET_RECORDS}" ]]; then
    echo "  the page of ${name} never reached ${DATASET_RECORDS} records (last: ${total:-none})" >&2
    return 1
  fi
  echo "  ${name}: ${DATASET_RECORDS} records from ${DATASET_WRITERS} writers at once, paged back whole"
}

dataset_dropped() {
  local round="$1" url="http://127.0.0.1:${V3_CORE_PORT}" table
  local name="rig_events_${round}"
  local admin=(--header "authorization: Bearer cluster-rig-administrator")
  table="$(curl -fs "${RIG_NODES%%,*}/?user=${CLICKHOUSE_USER}&password=${CLICKHOUSE_PASSWORD}" \
    --data-binary "SELECT name FROM system.tables WHERE database = 'insight_datasets' AND name LIKE 'ds_${name}_%'")" || return 1
  if [[ "$(printf '%s\n' "${table}" | grep -c .)" != 1 ]]; then
    echo "  ${name}: expected one table, found: ${table:-none}" >&2
    return 1
  fi
  check relation --name "insight_datasets.${table}" --rows "${DATASET_RECORDS}" || return 1

  v3_core_call 204 "dropping ${name}" --request DELETE "${admin[@]}" "${url}/v1/datasets/${name}" || return 1
  check absent --name "insight_datasets.${table}" || return 1
}

services_run() {
  local round="$1"
  local log="${WORKDIR}/v3-core-${round}.log"
  persons_sync "${round}"
  start_v3_core "${round}" || { stop_v3_core; return 1; }
  if ! dataset_lifecycle "${round}"; then
    stop_v3_core
    grep -iE 'error|warn' "${log}" | tail -n 20 >&2
    return 1
  fi
  stop_v3_core
}
