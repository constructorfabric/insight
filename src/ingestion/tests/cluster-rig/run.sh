#!/usr/bin/env bash
# Deploy onto the rig's warehouse, sync a connector's bronze into it, heal
# what a warm install carries, run the reconcile sweep against the ledger,
# build the connector's models, run the two services that write the warehouse
# themselves — then sync, deploy, build and run once more. Then the deploys
# the warehouse must refuse: the topology declared the wrong way round, a
# bronze table from before the flip and, on a cluster, a replica that is down.
# Last, assert what every node holds.
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
# the binaries). The creators are creators.sh's, the services services.sh's.
# Topologies and assertions: README.md.
set -euo pipefail

RIG_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=SCRIPTDIR/warehouse.sh
source "${RIG_DIR}/warehouse.sh"
# shellcheck source=SCRIPTDIR/creators.sh
source "${RIG_DIR}/creators.sh"

teardown() {
  stop_v3_core
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
step "the reconcile sweep against the connector sync ledger"
check quorum-boundary --out "${WORKDIR}/sweep-boundary.json"
sweep_ledger "${WORKDIR}/sweep.log"
check sweep --after "${WORKDIR}/sweep-boundary.json"
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
