#!/usr/bin/env bash
# Deploy onto the rig's warehouse, sync a connector's bronze into it, heal
# what a warm install carries, build the connector's models, run the two
# services that write the warehouse themselves — then sync, deploy, build and
# run once more, and assert what every node holds. On a cluster, then
# plant one plain bronze table on one replica and assert the deploy hook
# refuses it by name.
#
#   RIG_TOPOLOGY=standalone|replicated  run.sh
#   RIG_DBT_VENV=<venv>                 reuse a venv with the pinned dbt
#   RIG_SERVICES_BIN=<dir>              reuse built identity-resolution + insight-v3-core
#   RIG_KEEP=1                          leave the containers up
#
# Needs Docker, jq, yq, curl, a python3.12 (or the venv) and cargo (or the
# binaries). Topologies and assertions: README.md.
set -euo pipefail

RIG_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SCRIPTS="$(cd -- "${RIG_DIR}/../../scripts" && pwd)"
CONNECTORS="$(cd -- "${RIG_DIR}/../../connectors" && pwd)"
TOPOLOGY="${RIG_TOPOLOGY:-replicated}"
PROBE_ROWS=200
# The connector whose models the leg builds and whose streams carry rows, plus
# the one whose bronze that chain crosses into (github__account_emails reads
# the directory roster).
BRONZE_CONNECTORS=(github github-directory)
# The connector every bronze heal is guarded on; its bronze stands only while
# the heals are judged.
HEALED_CONNECTOR=jira
DBT_SELECT="tag:github+ tag:sync:always"
# The column the second sync adds to bronze_github.commits.
ADDED_COLUMN="$(python3 -c "import sys; sys.path.insert(0, '${RIG_DIR}'); import github_records; print(github_records.ADDED_COLUMN)")"

# shellcheck source=SCRIPTDIR/../../scripts/bootstrap-db/pins.env
source "${SCRIPTS}/bootstrap-db/pins.env"
# shellcheck source=SCRIPTDIR/../../scripts/bootstrap-db/lib/connector-catalog.sh
source "${SCRIPTS}/bootstrap-db/lib/connector-catalog.sh"
export CLICKHOUSE_SERVER_IMAGE SOURCE_DECLARATIVE_MANIFEST_IMAGE
export CLICKHOUSE_KEEPER_IMAGE="${CLICKHOUSE_SERVER_IMAGE/clickhouse-server/clickhouse-keeper}"

export CLICKHOUSE_USER=insight CLICKHOUSE_PASSWORD=insight CLICKHOUSE_DATABASE=insight
export CLICKHOUSE_PRESENTATION_PASSWORD=presentation-rig
export RIG_USER_PASSWORD_PRESENTATION="${CLICKHOUSE_PRESENTATION_PASSWORD}"
export RIG_TOPOLOGY="${TOPOLOGY}"

case "${TOPOLOGY}" in
  replicated)
    export CLICKHOUSE_CLUSTER_MODE=true CLICKHOUSE_CLUSTER_NAME=rig
    export RIG_NODES=http://127.0.0.1:18124,http://127.0.0.1:18125
    export RIG_PROXY=http://127.0.0.1:18123
    DESTINATION_HOST=proxy
    ;;
  standalone)
    export CLICKHOUSE_CLUSTER_MODE=false CLICKHOUSE_CLUSTER_NAME=
    export RIG_NODES=http://127.0.0.1:18126
    export RIG_PROXY="${RIG_NODES}"
    DESTINATION_HOST=single
    ;;
  *)
    echo "run.sh: RIG_TOPOLOGY must be replicated or standalone, not '${TOPOLOGY}'" >&2
    exit 2
    ;;
esac
export CLICKHOUSE_URL="${RIG_PROXY}"

PROJECT="insight-cluster-rig-${TOPOLOGY}"
COMPOSE=(docker compose -p "${PROJECT}" --project-directory "${RIG_DIR}" -f "${RIG_DIR}/compose.yaml" --profile "${TOPOLOGY}")
WORKDIR="$(mktemp -d "${RUNNER_TEMP:-/tmp}/cluster-rig.XXXXXXXX")"
STARTED_AT="${SECONDS}"

# shellcheck source=SCRIPTDIR/services.sh
source "${RIG_DIR}/services.sh"

cleanup() {
  local status=$?
  # SAFETY: a shell error (an unbound variable, say) reaches this trap with
  # status 0 on some bash builds; only the last step may end the run green.
  if [[ "${status}" -eq 0 && "${FINISHED:-0}" != 1 ]]; then
    status=1
  fi
  stop_v3_core
  if [[ "${status}" -ne 0 ]]; then
    "${COMPOSE[@]}" logs --tail 80 >&2 || true
  fi
  if [[ "${RIG_KEEP:-0}" != "1" ]]; then
    "${COMPOSE[@]}" down -v -t 5 >/dev/null 2>&1 || true
  fi
  rm -rf "${WORKDIR}"
  exit "${status}"
}
trap cleanup EXIT

step() {
  echo "=== ${TOPOLOGY}: $* (+$(( SECONDS - STARTED_AT ))s) ==="
}

dbt_venv() {
  if [[ -n "${RIG_DBT_VENV:-}" ]]; then
    VENV="${RIG_DBT_VENV}"
    return
  fi
  # INVARIANT: the pinned dbt does not run on interpreters newer than 3.12.
  local python
  python="$(command -v python3.12 || command -v python3.11 || command -v python3)"
  "${python}" -m venv "${WORKDIR}/venv"
  "${WORKDIR}/venv/bin/pip" install --quiet \
    "dbt-core==${DBT_CORE_VERSION}" "dbt-clickhouse==${DBT_CLICKHOUSE_VERSION}" pyyaml
  VENV="${WORKDIR}/venv"
}

# The hook's output goes to the log named, and to the terminal.
deploy_hook() {
  bash "${SCRIPTS}/apply-ch-migrations.sh" 2>&1 | tee "$1"
}

# What the jira heals print when they find work, and only then.
JIRA_HEAL_LINES=('rebuilding on the issue id' 'filling from the issue streams' 'source(s) as a')

healed_once() {
  local log="$1" line
  for line in "${JIRA_HEAL_LINES[@]}"; do
    if ! grep -qF "${line}" "${log}"; then
      echo "  the deploy hook did not report '${line}' over the planted jira bronze" >&2
      return 1
    fi
  done
  echo "  $(grep -cF -f <(printf '%s\n' "${JIRA_HEAL_LINES[@]}") "${log}") jira heal(s) did their work"
}

healed_nothing() {
  local log="$1" line
  for line in "${JIRA_HEAL_LINES[@]}"; do
    if grep -qF "${line}" "${log}"; then
      echo "  the deploy hook healed jira bronze again: $(grep -F "${line}" "${log}" | head -n 1)" >&2
      return 1
    fi
  done
  echo "  no jira heal found work left"
}

# Discover every stream the connectors declare, once; each sync writes them.
discover_streams() {
  local config="${SCRIPTS}/bootstrap-db/connectors-config.yaml" connector work
  for connector in "${BRONZE_CONNECTORS[@]}" "${HEALED_CONNECTOR}"; do
    work="${WORKDIR}/streams/${connector}"
    mkdir -p "${work}"
    connector_config_json "${config}" "${connector}" > "${work}/source.json"
    build_configured_catalog "${CONNECTORS}/$(connector_path "${config}" "${connector}")" \
      "${work}/source.json" "${work}" > /dev/null
  done
}

# One destination write, as reconcile configures it: <dest>/input.jsonl into
# the streams of <dest>/configured_catalog.json.
destination_write() {
  local dest="$1"
  RECONCILE_DEST_CLICKHOUSE_HOST="${DESTINATION_HOST}" \
  RECONCILE_DEST_CLICKHOUSE_PORT=8123 \
  RECONCILE_DEST_CLICKHOUSE_DATABASE="${CLICKHOUSE_DATABASE}" \
  RECONCILE_DEST_CLICKHOUSE_USERNAME="${CLICKHOUSE_USER}" \
  RECONCILE_DEST_CLICKHOUSE_PASSWORD="${CLICKHOUSE_PASSWORD}" \
  RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE="${CLICKHOUSE_CLUSTER_MODE}" \
  RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME="${CLICKHOUSE_CLUSTER_NAME}" \
    python3 "${SCRIPTS}/../reconcile-connectors/python/compose_destination_config.py" > "${dest}/config.json"

  # WORKAROUND: the destination image starts only as its own uid, so its inputs
  # are world-readable; they hold only the throwaway rig's password.
  chmod 0755 "${dest}"
  chmod 0644 "${dest}/config.json" "${dest}/configured_catalog.json"
  echo "  $(jq '.streams | length' "${dest}/configured_catalog.json") stream(s), $(grep -c '"RECORD"' "${dest}/input.jsonl" || true) record(s) through ${DESTINATION_CLICKHOUSE_IMAGE}"
  docker run --rm -i --network "${PROJECT}_default" -v "${dest}:/work:ro" \
    "${DESTINATION_CLICKHOUSE_IMAGE}" \
    write --config /work/config.json --catalog /work/configured_catalog.json \
    < "${dest}/input.jsonl" > "${dest}/write.jsonl" \
    || { tail -n 5 "${dest}/write.jsonl" >&2; return 1; }
}

# Bronze as a sync leaves it, all in one write: the probe stream with rows,
# the github streams with the records github_records.py emits, and every other
# stream the connectors declare, empty. The second sync delivers the same
# records again, extracted later, with ADDED_COLUMN on the commits.
write_bronze() {
  local round="$1" catalogs connector work
  local dest="${WORKDIR}/destination-${round}"
  mkdir -p "${dest}"
  python3 "${RIG_DIR}/probe_stream.py" "${dest}" "${PROBE_ROWS}"
  python3 "${RIG_DIR}/github_records.py" "${dest}/github-records.jsonl" "${round}"
  catalogs=("${dest}/catalog.json")
  for connector in "${BRONZE_CONNECTORS[@]}"; do
    work="${WORKDIR}/streams/${connector}"
    # A stream's records come before the trace that says it is done.
    if [[ "${connector}" == github ]]; then
      cat "${dest}/github-records.jsonl" >> "${dest}/input.jsonl"
    fi
    cat "${work}/traces.jsonl" >> "${dest}/input.jsonl"
    catalogs+=("${work}/configured_catalog.json")
  done
  jq -s --arg column "${ADDED_COLUMN}" --argjson round "${round}" '
    {streams: map(.streams) | add}
    | if $round > 1 then
        .streams |= map(if .stream.namespace == "bronze_github" and .stream.name == "commits"
                        then .stream.json_schema.properties[$column] = {type: ["integer", "null"]} else . end)
      else . end' "${catalogs[@]}" > "${dest}/configured_catalog.json"
  destination_write "${dest}"
}

# The healed connector's bronze as its first sync leaves it: every stream, empty.
write_healed_bronze() {
  local dest="${WORKDIR}/destination-${HEALED_CONNECTOR}" work="${WORKDIR}/streams/${HEALED_CONNECTOR}"
  mkdir -p "${dest}"
  cp "${work}/traces.jsonl" "${dest}/input.jsonl"
  cp "${work}/configured_catalog.json" "${dest}/configured_catalog.json"
  destination_write "${dest}"
}

# The Argo dbt-run step's shape: the profile from dbt_profiles.py, the
# connector's selector, the unbuildable models excluded. Leaves
# <target>/run_results.json for check.py.
dbt_build() {
  local target="$1" profiles="${WORKDIR}/profiles" log="$1.log" unbuildable excluded=()
  mkdir -p "${profiles}"
  python3 "${SCRIPTS}/dbt_profiles.py" --target migrate --profiles-dir "${profiles}" --correlated-subqueries
  (
    cd "${SCRIPTS}/../dbt"
    unbuildable="$(dbt --quiet run-operation list_unbuildable_models \
      --profiles-dir "${profiles}" --target-path "${target}" --log-path "${WORKDIR}/dbt-logs")"
    read -r -a excluded <<< "${unbuildable}"
    echo "  dbt run --select ${DBT_SELECT}, excluding ${#excluded[@]} unbuildable model(s)"
    # shellcheck disable=SC2086
    if ! dbt --no-use-colors run --profiles-dir "${profiles}" --target-path "${target}" --log-path "${WORKDIR}/dbt-logs" \
        --select ${DBT_SELECT} ${excluded[@]+--exclude "${excluded[@]}"} > "${log}" 2>&1; then
      grep -E -A 8 '(ERROR|Error|Completed with)' "${log}" | tail -n 60 >&2
      return 1
    fi
    grep -E '(WARN|Finished running|Done\.)' "${log}" | sed 's/^/  /'
  )
  jq -e '.results | length > 0' "${target}/run_results.json" > /dev/null \
    || { echo "  ${DBT_SELECT} built nothing" >&2; return 1; }
}

check() {
  "${VENV}/bin/python" "${RIG_DIR}/check.py" "$@"
}

# One statement against one node directly, bypassing the proxy.
ch_on_node() {
  local node="$1" sql="$2"
  curl -sS --fail-with-body \
    -H "X-ClickHouse-User: ${CLICKHOUSE_USER}" \
    -H @<(printf 'X-ClickHouse-Key: %s' "${CLICKHOUSE_PASSWORD}") \
    --data-binary "${sql}" "${node%/}/"
}

# A bronze table from before clusterMode was turned on: plain engine, on one
# replica, which the destination's IF NOT EXISTS never converts (#3899). The
# deploy hook must refuse it by name before it creates anything, whichever
# replica the proxy hands its probe to; with the table gone it deploys again.
refuse_plain_bronze_table() {
  local node="${RIG_NODES%%,*}" table="bronze_cluster_rig.pre_flip" log="${WORKDIR}/refused.log"
  ch_on_node "${node}" "CREATE TABLE ${table} (id UInt8) ENGINE = ReplacingMergeTree ORDER BY id"

  if deploy_hook "${log}" > /dev/null; then
    echo "  the deploy hook ran over the plain ${table}" >&2
    return 1
  fi
  if ! grep -q "${table}  ReplacingMergeTree" "${log}"; then
    echo "  the deploy hook failed without naming ${table}:" >&2
    tail -n 30 "${log}" >&2
    return 1
  fi
  if grep -q "=== Creating databases" "${log}"; then
    echo "  the deploy hook created databases before refusing ${table}" >&2
    return 1
  fi
  echo "  refused: $(grep -m1 "${table}" "${log}" | sed 's/^ *//')"

  ch_on_node "${node}" "DROP TABLE ${table}"
  deploy_hook "${log}" > /dev/null || { tail -n 30 "${log}" >&2; return 1; }
  echo "  deployed again once the table was gone"
}

jira_warm_state() {
  "${VENV}/bin/python" "${RIG_DIR}/jira_warm_state.py" "$@"
}

step "starting the warehouse"
"${COMPOSE[@]}" down -v -t 5 >/dev/null 2>&1 || true
if ! "${COMPOSE[@]}" up -d --wait --quiet-pull > "${WORKDIR}/up.log" 2>&1; then
  cat "${WORKDIR}/up.log" >&2
  exit 1
fi

dbt_venv
export PATH="${VENV}/bin:${PATH}"
step "the services' binaries"
services_build
step "discovering the connectors' streams"
discover_streams

step "deploy hook, first run"
deploy_hook "${WORKDIR}/hook-1.log"
step "the jira bronze a warm install carries"
write_healed_bronze
jira_warm_state plant
step "deploy hook over the warm jira bronze: the heals"
deploy_hook "${WORKDIR}/hook-jira-1.log"
healed_once "${WORKDIR}/hook-jira-1.log"
jira_warm_state assert
step "deploy hook again: nothing left to heal"
deploy_hook "${WORKDIR}/hook-jira-2.log"
healed_nothing "${WORKDIR}/hook-jira-2.log"
jira_warm_state drop
step "bronze through destination-clickhouse, first sync"
write_bronze 1
step "services, first run"
services_run 1
step "dbt build, first run"
dbt_build "${WORKDIR}/dbt-run-1"
check snapshot --out "${WORKDIR}/counts.json"
step "bronze through destination-clickhouse, second sync: ${ADDED_COLUMN} added"
write_bronze 2
step "deploy hook, second run"
deploy_hook "${WORKDIR}/hook-2.log"
step "dbt build, second run"
dbt_build "${WORKDIR}/dbt-run-2"
step "services, second run"
services_run 2
step "asserting every node"
check assert --baseline "${WORKDIR}/counts.json" --probe-rows "${PROBE_ROWS}" --users presentation \
  --run-results "${WORKDIR}/dbt-run-2/run_results.json" --columns "bronze_github.commits:${ADDED_COLUMN}"
if [[ "${TOPOLOGY}" == replicated ]]; then
  step "a plain bronze table from before the flip is refused"
  refuse_plain_bronze_table
fi
FINISHED=1
step "OK"
