#!/usr/bin/env bash
# Sourced by run.sh and warm-upgrade.sh: the rig's warehouse for one topology —
# its compose project, the env every creator reads for it, the nodes a check
# reaches directly — and the venv the checks run in.
#
# Reads RIG_TOPOLOGY (replicated by default), RIG_DBT_VENV and RIG_KEEP; needs
# RIG_DIR. Exposes TOPOLOGY, SCRIPTS, PROJECT, COMPOSE, WORKDIR, RIG_NODES,
# RIG_PROXY, DESTINATION_HOST and the CLICKHOUSE_* the hook reads, plus:
#   step            — a timestamped heading
#   warehouse_up    — the compose project, waited for
#   dbt_venv        — VENV with the pinned dbt (and check.py's driver) and pytest
#   check           — check.py in that venv
#   ch_on_node      — one statement against one node, bypassing the proxy
#
# The EXIT trap tears the warehouse down unless RIG_KEEP=1, after `teardown`
# where the caller defines one, and ends the run red unless FINISHED=1: a shell
# error (an unbound variable, say) reaches the trap with status 0 on some bash
# builds, so only the caller's last step may end a run green.

: "${RIG_DIR:?warehouse.sh expects RIG_DIR}"
SCRIPTS="$(cd -- "${RIG_DIR}/../../scripts" && pwd)"
TOPOLOGY="${RIG_TOPOLOGY:-replicated}"

# shellcheck source=SCRIPTDIR/../../scripts/bootstrap-db/pins.env
source "${SCRIPTS}/bootstrap-db/pins.env"
export CLICKHOUSE_SERVER_IMAGE SOURCE_DECLARATIVE_MANIFEST_IMAGE
export CLICKHOUSE_KEEPER_IMAGE="${CLICKHOUSE_SERVER_IMAGE/clickhouse-server/clickhouse-keeper}"

export CLICKHOUSE_USER=insight CLICKHOUSE_PASSWORD=insight CLICKHOUSE_DATABASE=insight
export CLICKHOUSE_PRESENTATION_PASSWORD=presentation-rig
export RIG_USER_PASSWORD_PRESENTATION="${CLICKHOUSE_PRESENTATION_PASSWORD}"
export RIG_TOPOLOGY="${TOPOLOGY}"

# shellcheck disable=SC2034  # DESTINATION_HOST is read by the scripts that source this
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
    echo "$(basename -- "$0"): RIG_TOPOLOGY must be replicated or standalone, not '${TOPOLOGY}'" >&2
    exit 2
    ;;
esac
export CLICKHOUSE_URL="${RIG_PROXY}"

PROJECT="insight-cluster-rig-${TOPOLOGY}"
COMPOSE=(docker compose -p "${PROJECT}" --project-directory "${RIG_DIR}" -f "${RIG_DIR}/compose.yaml" --profile "${TOPOLOGY}")
WORKDIR="$(mktemp -d "${RUNNER_TEMP:-/tmp}/cluster-rig.XXXXXXXX")"
STARTED_AT="${SECONDS}"

cleanup() {
  local status=$?
  if [[ "${status}" -eq 0 && "${FINISHED:-0}" != 1 ]]; then
    status=1
  fi
  if declare -F teardown > /dev/null; then
    teardown
  fi
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

warehouse_up() {
  "${COMPOSE[@]}" down -v -t 5 >/dev/null 2>&1 || true
  if ! "${COMPOSE[@]}" up -d --wait --quiet-pull > "${WORKDIR}/up.log" 2>&1; then
    cat "${WORKDIR}/up.log" >&2
    return 1
  fi
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
    "dbt-core==${DBT_CORE_VERSION}" "dbt-clickhouse==${DBT_CLICKHOUSE_VERSION}" pyyaml pytest
  VENV="${WORKDIR}/venv"
}

check() {
  "${VENV}/bin/python" "${RIG_DIR}/check.py" "$@"
}

ch_on_node() {
  local node="$1" sql="$2"
  curl -sS --fail-with-body \
    -H "X-ClickHouse-User: ${CLICKHOUSE_USER}" \
    -H @<(printf 'X-ClickHouse-Key: %s' "${CLICKHOUSE_PASSWORD}") \
    --data-binary "${sql}" "${node%/}/"
}
