#!/usr/bin/env bash
# Deploy onto the rig's warehouse, sync a connector's bronze into it, heal
# what a warm install carries, build the connector's models, run the two
# services that write the warehouse themselves — then sync, deploy, build and
# run once more. Then the deploys the warehouse must refuse: the topology
# declared the wrong way round, a bronze table from before the flip and, on a
# cluster, a replica that is down. Last, assert what every node holds.
#
# Every deploy hook runs as the chart's clickhouse-migrate Job renders it for
# this warehouse: the Job's command under the Job's env, so `clusterMode` is
# exercised from the chart value down, not by calling the script with the
# right variables.
#
#   RIG_TOPOLOGY=standalone|replicated  run.sh
#   RIG_DBT_VENV=<venv>                 reuse a venv with the pinned dbt
#   RIG_SERVICES_BIN=<dir>              reuse built identity-resolution + insight-v3-core
#   RIG_KEEP=1                          leave the containers up
#
# Needs Docker, helm, jq, yq, curl, a python3.12 (or the venv) and cargo (or
# the binaries). Topologies and assertions: README.md.
set -euo pipefail

RIG_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=SCRIPTDIR/warehouse.sh
source "${RIG_DIR}/warehouse.sh"
CONNECTORS="$(cd -- "${RIG_DIR}/../../connectors" && pwd)"
CHART="$(cd -- "${RIG_DIR}/../../../../charts/insight" && pwd)"
# The scripts root the chart's Job names, which is the toolbox image's; this
# tree's scripts stand in for it.
TOOLBOX_SCRIPTS=/ingestion/scripts
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

# shellcheck source=SCRIPTDIR/../../scripts/bootstrap-db/lib/connector-catalog.sh
source "${SCRIPTS}/bootstrap-db/lib/connector-catalog.sh"
# For CH_DDL_TASK_TIMEOUT_S, the ceiling a coordinated statement answers within.
# shellcheck source=SCRIPTDIR/../../scripts/lib/ch-exec.sh
source "${SCRIPTS}/lib/ch-exec.sh"
# shellcheck source=SCRIPTDIR/services.sh
source "${RIG_DIR}/services.sh"

teardown() {
  stop_v3_core
}

# The clickhouse-migrate Job as the chart renders it for this warehouse: the
# topology the way an operator sets it, plus whatever a leg overrides.
render_migrate_job() {
  helm template rig "${CHART}" --values "${CHART}/tests/values.yaml" \
    --set clickhouse.host=127.0.0.1 --set "clickhouse.port=${RIG_PROXY##*:}" \
    --set "clickhouse.database=${CLICKHOUSE_DATABASE}" --set "clickhouse.username=${CLICKHOUSE_USER}" \
    --set "clickhouse.clusterMode=${CLICKHOUSE_CLUSTER_MODE}" --set "clickhouse.clusterName=${CLICKHOUSE_CLUSTER_NAME}" \
    "$@" --show-only templates/clickhouse-migrate-job.yaml
}

# What the rig holds under the Secret keys the Job reads; a key it lacks is
# left unset, which the Job marks optional or not.
rig_secret() {
  case "$1" in
    clickhouse-password) printf '%s' "${CLICKHOUSE_PASSWORD}" ;;
    clickhouse-presentation-password) printf '%s' "${CLICKHOUSE_PRESENTATION_PASSWORD}" ;;
    *) return 1 ;;
  esac
}

# The deploy hook as the chart's Job runs it: its command, under its env and
# nothing else of this shell's — the chart's spelling of the topology is what
# reaches the script. Output goes to the log named, and to the terminal.
# Helm overrides after the log reach the render.
deploy_hook() {
  local log="$1" job line name key optional secret job_env=() command=()
  shift
  job="$(render_migrate_job "$@" 2> "${log}")" || { cat "${log}" >&2; return 1; }
  while IFS= read -r line; do
    case "${line}" in
      "value "*)
        job_env+=("${line#value }") ;;
      "secret "*)
        read -r name key optional <<< "${line#secret }"
        if secret="$(rig_secret "${key}")"; then
          job_env+=("${name}=${secret}")
        elif [[ "${optional}" != true ]]; then
          echo "  the Job reads Secret key ${key} for ${name}, which the rig does not hold" >&2
          return 1
        fi ;;
    esac
  done < <(yq -r '.spec.template.spec.containers[0].env[]
    | if .valueFrom then "secret " + .name + " " + .valueFrom.secretKeyRef.key + " " + ((.valueFrom.secretKeyRef.optional // false) | tostring)
      else "value " + .name + "=" + ((.value // "") | tostring) end' <<< "${job}")
  while IFS= read -r line; do
    command+=("${line//${TOOLBOX_SCRIPTS}/${SCRIPTS}}")
  done < <(yq -r '.spec.template.spec.containers[0] | .command + .args | .[]' <<< "${job}")
  if [[ "${command[*]}" != *apply-ch-migrations.sh* ]]; then
    echo "  the Job's command no longer runs apply-ch-migrations.sh: ${command[*]}" >&2
    return 1
  fi
  env -i PATH="${PATH}" HOME="${HOME}" "${job_env[@]}" "${command[@]}" 2>&1 | tee "${log}"
}

# A deploy the warehouse must refuse: the hook fails, says why in the words
# its guard documents, and has created nothing. Helm overrides after the three
# arguments reach the render.
refused_before_ddl() {
  local log="$1" reason="$2" what="$3"
  shift 3
  if deploy_hook "${log}" "$@" > /dev/null; then
    echo "  the deploy hook ran ${what}" >&2
    return 1
  fi
  if ! grep -qF "${reason}" "${log}"; then
    echo "  the deploy hook failed ${what} without saying '${reason}':" >&2
    tail -n 30 "${log}" >&2
    return 1
  fi
  if grep -q "=== Creating databases" "${log}"; then
    echo "  the deploy hook created databases before refusing ${what}" >&2
    return 1
  fi
  echo "  refused: $(grep -m1 -F "${reason}" "${log}" | sed 's/^ *//')"
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

# The flag turned on over a lone node: the chart hands the hook clusterMode
# with a cluster name, and the server lists no such cluster.
refuse_cluster_flag() {
  refused_before_ddl "${WORKDIR}/refused-cluster-flag.log" "lists no such" \
    "with clickhouse.clusterMode on against the lone node" \
    --set clickhouse.clusterMode=true --set clickhouse.clusterName=rig
}

# The flag turned off over Replicated databases: every creator would emit
# standalone DDL into them.
refuse_standalone_flag() {
  refused_before_ddl "${WORKDIR}/refused-standalone-flag.log" "is Replicated, but clickhouse.clusterMode is" \
    "with clickhouse.clusterMode off against the Replicated databases" \
    --set clickhouse.clusterMode=false
}

# A bronze table from before clusterMode was turned on: plain engine, on one
# replica, which the destination's IF NOT EXISTS never converts (#3899). The
# deploy hook must refuse it by name, whichever replica the proxy hands its
# probe to; with the table gone it deploys again.
refuse_plain_bronze_table() {
  local node="${RIG_NODES%%,*}" table="bronze_cluster_rig.pre_flip" log="${WORKDIR}/refused-plain-bronze.log"
  ch_on_node "${node}" "CREATE TABLE ${table} (id UInt8) ENGINE = ReplacingMergeTree ORDER BY id"
  refused_before_ddl "${log}" "${table}  ReplacingMergeTree" "over the plain ${table}"
  ch_on_node "${node}" "DROP TABLE ${table}"
  deploy_hook "${WORKDIR}/hook-after-plain-bronze.log" > /dev/null || { tail -n 30 "${WORKDIR}/hook-after-plain-bronze.log" >&2; return 1; }
  echo "  deployed again once the table was gone"
}

# Which replica answers each of N requests through the proxy, one per line.
proxy_answers() {
  local i
  for i in $(seq 1 "$1"); do
    ch_on_node "${RIG_PROXY}" "SELECT hostName()" 2>/dev/null || echo "-"
  done
}

# Wait until the proxy's answers to N requests are exactly the replicas named.
proxy_serves() {
  local expected="$1" n="$2" i answers
  for i in $(seq 1 30); do
    answers="$(proxy_answers "${n}" | sort -u | paste -sd, -)"
    [[ "${answers}" == "${expected}" ]] && return 0
    sleep 1
  done
  echo "  the proxy still answers from ${answers}, expected ${expected}" >&2
  return 1
}

# Every relation one node holds, in a form two reads can be compared by.
relations_on() {
  ch_on_node "$1" "SELECT database, name, engine, uuid FROM system.tables
    WHERE database NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema') ORDER BY 1, 2"
}

# A replica down while the deploy runs. The hook must refuse before any DDL —
# the bronze sweep reads every replica and a missing one fails it closed — and
# within the ceiling a coordinated statement waits, leaving the live replica
# as it was; with the replica back, it deploys again.
deploy_with_replica_down() {
  local live="${RIG_NODES%%,*}" log="${WORKDIR}/refused-replica-down.log" before after started elapsed
  "${COMPOSE[@]}" stop -t 10 ch2 > /dev/null 2>&1
  proxy_serves ch1 10
  before="$(relations_on "${live}")"

  started="${SECONDS}"
  refused_before_ddl "${log}" "could not read system.tables on every replica" "with ch2 down"
  elapsed=$(( SECONDS - started ))
  if (( elapsed >= CH_DDL_TASK_TIMEOUT_S )); then
    echo "  the deploy hook took ${elapsed}s to refuse, not under the ${CH_DDL_TASK_TIMEOUT_S}s a coordinated statement waits" >&2
    return 1
  fi
  after="$(relations_on "${live}")"
  if [[ "${before}" != "${after}" ]]; then
    echo "  ch1's relations changed while ch2 was down:" >&2
    diff <(printf '%s\n' "${before}") <(printf '%s\n' "${after}") >&2 || true
    return 1
  fi
  echo "  refused in ${elapsed}s, ch1 unchanged"

  "${COMPOSE[@]}" up -d --wait ch2 > /dev/null 2>&1
  proxy_serves ch1,ch2 20
  # ch1 may still hold the address ch2 had; the sweep needs every replica.
  local seen=0
  for _ in $(seq 1 60); do
    seen="$(ch_on_node "${live}" "SELECT count() FROM clusterAllReplicas('${CLICKHOUSE_CLUSTER_NAME}', system.one) SETTINGS skip_unavailable_shards = 0" 2>/dev/null)" || seen=0
    [[ "${seen}" == 2 ]] && break
    sleep 1
  done
  if [[ "${seen}" != 2 ]]; then
    echo "  ch1 still reaches ${seen:-0} of 2 replicas a minute after ch2 came back" >&2
    return 1
  fi
  deploy_hook "${WORKDIR}/hook-after-replica-down.log" > /dev/null || { tail -n 30 "${WORKDIR}/hook-after-replica-down.log" >&2; return 1; }
  echo "  deployed again once ch2 was back"
}

jira_warm_state() {
  "${VENV}/bin/python" "${RIG_DIR}/jira_warm_state.py" "$@"
}

step "starting the warehouse"
warehouse_up
dbt_venv
export PATH="${VENV}/bin:${PATH}"
step "the chart's subcharts"
helm dependency update "${CHART}" > "${WORKDIR}/helm-deps.log" 2>&1 || { cat "${WORKDIR}/helm-deps.log" >&2; exit 1; }
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
if [[ "${TOPOLOGY}" == replicated ]]; then
  step "deploy hook with clickhouse.clusterMode turned off is refused"
  refuse_standalone_flag
  step "a plain bronze table from before the flip is refused"
  refuse_plain_bronze_table
  step "deploy hook with ch2 down is refused, and deploys once ch2 is back"
  deploy_with_replica_down
else
  step "deploy hook with clickhouse.clusterMode turned on is refused"
  refuse_cluster_flag
fi
step "asserting every node"
check assert --baseline "${WORKDIR}/counts.json" --probe-rows "${PROBE_ROWS}" --users presentation \
  --run-results "${WORKDIR}/dbt-run-2/run_results.json" --columns "bronze_github.commits:${ADDED_COLUMN}"
FINISHED=1
step "OK"
