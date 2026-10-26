#!/usr/bin/env bash
# Convert a standalone installation to cluster mode on the rig's replicated
# warehouse, along docs/operations/CLICKHOUSE-CLUSTER-CONVERSION.md. The first
# replica alone plays the standalone node: the deploy hook, a connector's
# bronze, a dbt build and both services reach it directly with clusterMode
# off. Then the cluster-mode deploy over those databases must be refused;
# every database is converted on that node with convert-database-to-cluster.sh
# — twice, the second run finding nothing to do — and the cluster-mode deploy,
# a second sync, a dbt build and the services run through the proxy. Last,
# the standalone copies are dropped and every node is asserted against the
# counts the standalone installation held.
#
#   RIG_TOPOLOGY=replicated  convert.sh   (the only topology it runs on)
#   RIG_DBT_VENV, RIG_SERVICES_BIN, RIG_KEEP as run.sh
#
# Needs what run.sh needs.
set -euo pipefail

RIG_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=SCRIPTDIR/warehouse.sh
source "${RIG_DIR}/warehouse.sh"
# shellcheck source=SCRIPTDIR/creators.sh
source "${RIG_DIR}/creators.sh"
if [[ "${TOPOLOGY}" != replicated ]]; then
  echo "convert.sh converts onto the replicated warehouse; RIG_TOPOLOGY is '${TOPOLOGY}'" >&2
  exit 2
fi

CLUSTER_NODES="${RIG_NODES}"
CLUSTER_PROXY="${RIG_PROXY}"
CLUSTER_NAME="${CLICKHOUSE_CLUSTER_NAME}"
FIRST_NODE="${RIG_NODES%%,*}"
# INVARIANT: create-databases.sh DATABASES, in the order the views between
# them read: a view is validated against what it reads when it is created.
OWNED_DATABASES=(staging silver identity config product_usage ingestion_history insight_datasets "${CLICKHOUSE_DATABASE}" presentation)

teardown() {
  stop_v3_core
}

# The first replica alone, as the standalone installation the conversion
# starts from: every creator reaches it directly and emits standalone DDL.
as_standalone_install() {
  export CLICKHOUSE_CLUSTER_MODE=false CLICKHOUSE_CLUSTER_NAME=
  export RIG_TOPOLOGY=standalone RIG_NODES="${FIRST_NODE}" RIG_PROXY="${FIRST_NODE}" CLICKHOUSE_URL="${FIRST_NODE}"
  DESTINATION_HOST=ch1
}

as_cluster() {
  export CLICKHOUSE_CLUSTER_MODE=true CLICKHOUSE_CLUSTER_NAME="${CLUSTER_NAME}"
  export RIG_TOPOLOGY=replicated RIG_NODES="${CLUSTER_NODES}" RIG_PROXY="${CLUSTER_PROXY}" CLICKHOUSE_URL="${CLUSTER_PROXY}"
  DESTINATION_HOST=proxy
}

# The runbook's conversion: the script against the node holding the
# standalone data, with the topology the cluster-mode deploy will declare.
convert() {
  local log="$1"
  shift
  CLICKHOUSE_URL="${FIRST_NODE}" CLICKHOUSE_CLUSTER_MODE=true CLICKHOUSE_CLUSTER_NAME="${CLUSTER_NAME}" \
    "${SCRIPTS}/convert-database-to-cluster.sh" "$@" 2>&1 | tee "${log}" | sed 's/^/  /'
}

bronze_databases() {
  ch_on_node "${FIRST_NODE}" "SELECT name FROM system.databases WHERE startsWith(name, 'bronze_') ORDER BY name"
}

# A second run over converted databases moves nothing: each one is reported
# as already converted, and the relation count stands.
converted_nothing() {
  local log="$1" expected="$2" found
  found="$(grep -c 'nothing to do' "${log}" || true)"
  if [[ "${found}" != "${expected}" ]]; then
    echo "  the second conversion found work in $(( expected - found )) of ${expected} database(s):" >&2
    grep -v 'nothing to do' "${log}" | tail -n 20 >&2
    return 1
  fi
  echo "  ${found} database(s) already converted, nothing moved"
}

# The persons are published before the snapshot, the dataset after it: the
# dataset is dropped on the cluster, and the snapshot is what the re-run is
# held to.
services_standalone() {
  persons_sync 1
  check snapshot --out "${WORKDIR}/counts.json"
  start_v3_core 1 || { stop_v3_core; return 1; }
  if ! dataset_declared 1; then
    stop_v3_core
    grep -iE 'error|warn' "${WORKDIR}/v3-core-1.log" | tail -n 20 >&2
    return 1
  fi
  stop_v3_core
}

# The persons sync swaps behind the watermark the converted table carries; the
# dataset declared standalone is read whole from the converted table on every
# node and dropped; another lives and dies on the cluster.
services_converted() {
  persons_sync 2
  start_v3_core 2 || { stop_v3_core; return 1; }
  if ! dataset_dropped 1 || ! dataset_lifecycle 2; then
    stop_v3_core
    grep -iE 'error|warn' "${WORKDIR}/v3-core-2.log" | tail -n 20 >&2
    return 1
  fi
  stop_v3_core
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

as_standalone_install
step "standalone installation on ch1: deploy hook"
deploy_hook "${WORKDIR}/hook-standalone.log"
step "standalone installation on ch1: bronze through destination-clickhouse, first sync"
write_bronze 1
step "standalone installation on ch1: dbt build"
dbt_build "${WORKDIR}/dbt-run-1"
step "standalone installation on ch1: the services"
services_standalone

as_cluster
step "deploy hook with clickhouse.clusterMode turned on over the standalone databases is refused"
refused_before_ddl "${WORKDIR}/refused-unconverted.log" "exists with engine 'Atomic'" "over the unconverted databases"
step "converting the owned databases on ch1"
convert "${WORKDIR}/convert-owned.log" "${OWNED_DATABASES[@]}"
step "converting the bronze databases on ch1"
BRONZE_DATABASES=()
while IFS= read -r name; do
  [[ -n "${name}" ]] && BRONZE_DATABASES+=("${name}")
done < <(bronze_databases)
convert "${WORKDIR}/convert-bronze.log" "${BRONZE_DATABASES[@]}"
step "converting again finds nothing to do"
convert "${WORKDIR}/convert-again.log" "${OWNED_DATABASES[@]}" "${BRONZE_DATABASES[@]}"
converted_nothing "${WORKDIR}/convert-again.log" $(( ${#OWNED_DATABASES[@]} + ${#BRONZE_DATABASES[@]} ))
step "every node holds the converted warehouse"
check nodes

step "deploy hook, cluster mode"
deploy_hook "${WORKDIR}/hook-cluster.log"
step "bronze through destination-clickhouse, second sync: ${ADDED_COLUMN} added"
write_bronze 2
step "dbt build on the cluster"
dbt_build "${WORKDIR}/dbt-run-2"
step "the services on the cluster"
services_converted
step "dropping the standalone copies"
for name in "${OWNED_DATABASES[@]}" "${BRONZE_DATABASES[@]}"; do
  ch_on_node "${FIRST_NODE}" "DROP DATABASE standalone_${name}"
done
echo "  $(( ${#OWNED_DATABASES[@]} + ${#BRONZE_DATABASES[@]} )) standalone copies dropped"
step "asserting every node"
check assert --baseline "${WORKDIR}/counts.json" --probe-rows "${PROBE_ROWS}" --users presentation \
  --run-results "${WORKDIR}/dbt-run-2/run_results.json" --columns "bronze_github.commits:${ADDED_COLUMN}"
FINISHED=1
step "OK"
