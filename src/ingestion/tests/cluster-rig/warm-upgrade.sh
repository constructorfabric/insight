#!/usr/bin/env bash
# Upgrade a warm installation on the rig's warehouse: the warm-upgrade
# simulation — the base commit's deploy, bronze and sync, the older relation
# shapes an installation carries, then this tree's deploy hook and first sync
# (.github/workflows/scripts/warm-upgrade-sim.sh) — against the topology
# RIG_TOPOLOGY names, and every node compared afterwards.
#
#   RIG_TOPOLOGY=replicated BASE_SHA=<commit>  warm-upgrade.sh
#   RIG_DBT_VENV=<venv>                        reuse a venv with the pinned dbt
#   RIG_KEEP=1                                 leave the containers up
#
# Needs Docker, jq, yq, curl and a python3.12 (or the venv). The base commit
# must be cluster-aware where the topology is replicated: its own deploy hook
# and connectors create the warm state.
set -euo pipefail

RIG_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=SCRIPTDIR/warehouse.sh
source "${RIG_DIR}/warehouse.sh"
REPO_ROOT="$(cd -- "${RIG_DIR}/../../../.." && pwd)"
: "${BASE_SHA:?BASE_SHA must be set (the commit playing the installed release)}"

step "starting the warehouse"
warehouse_up
dbt_venv

step "warm upgrade from $(git -C "${REPO_ROOT}" rev-parse --short "${BASE_SHA}") onto the ${TOPOLOGY} warehouse"
BASE_SHA="${BASE_SHA}" \
BRONZE_CLICKHOUSE_URL="http://${DESTINATION_HOST}:8123" \
DOCKER_NETWORK="${PROJECT}_default" \
  bash "${REPO_ROOT}/.github/workflows/scripts/warm-upgrade-sim.sh" 2>&1 | tee "${WORKDIR}/warm-upgrade.log"

step "asserting every node"
check nodes
FINISHED=1
step "OK"
