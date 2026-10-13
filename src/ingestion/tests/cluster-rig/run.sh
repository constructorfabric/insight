#!/usr/bin/env bash
# Deploy onto the rig's warehouse twice and assert what every node holds.
#
#   RIG_TOPOLOGY=standalone|replicated  run.sh
#   RIG_DBT_VENV=<venv>                 reuse a venv with the pinned dbt
#   RIG_KEEP=1                          leave the containers up
#
# What each topology deploys, and why the replicated leg expects the full hook
# to fail: README.md.
set -euo pipefail

RIG_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SCRIPTS="$(cd -- "${RIG_DIR}/../../scripts" && pwd)"
TOPOLOGY="${RIG_TOPOLOGY:-replicated}"
PROBE_ROWS=200

# shellcheck source=SCRIPTDIR/../../scripts/bootstrap-db/pins.env
source "${SCRIPTS}/bootstrap-db/pins.env"
export CLICKHOUSE_SERVER_IMAGE
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

cleanup() {
  local status=$?
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

deploy_hook() {
  bash "${SCRIPTS}/apply-ch-migrations.sh"
}

create_databases() {
  bash "${SCRIPTS}/create-databases.sh"
}

dbt_ddl_hooks() {
  local profiles="${WORKDIR}/profiles"
  mkdir -p "${profiles}"
  python3 "${SCRIPTS}/dbt_profiles.py" --target migrate --profiles-dir "${profiles}"
  local macro
  for macro in create_task_config_tables create_ai_config_tables create_identity_persons; do
    echo "  dbt run-operation ${macro}"
    (cd "${SCRIPTS}/../dbt" && dbt run-operation "${macro}" --profiles-dir "${profiles}" --quiet)
  done
}

write_bronze() {
  local dest="${WORKDIR}/destination"
  mkdir -p "${dest}"
  RECONCILE_DEST_CLICKHOUSE_HOST="${DESTINATION_HOST}" \
  RECONCILE_DEST_CLICKHOUSE_PORT=8123 \
  RECONCILE_DEST_CLICKHOUSE_DATABASE="${CLICKHOUSE_DATABASE}" \
  RECONCILE_DEST_CLICKHOUSE_USERNAME="${CLICKHOUSE_USER}" \
  RECONCILE_DEST_CLICKHOUSE_PASSWORD="${CLICKHOUSE_PASSWORD}" \
  RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE="${CLICKHOUSE_CLUSTER_MODE}" \
  RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME="${CLICKHOUSE_CLUSTER_NAME}" \
    python3 "${SCRIPTS}/../reconcile-connectors/python/compose_destination_config.py" > "${dest}/config.json"

  python3 "${RIG_DIR}/probe_stream.py" "${dest}" "${PROBE_ROWS}"

  # WORKAROUND: the destination image starts only as its own uid, so its inputs
  # are world-readable; they hold only the throwaway rig's password.
  chmod 0755 "${dest}"
  chmod 0644 "${dest}"/*
  docker run --rm -i --network "${PROJECT}_default" -v "${dest}:/work:ro" \
    "${DESTINATION_CLICKHOUSE_IMAGE}" \
    write --config /work/config.json --catalog /work/catalog.json \
    < "${dest}/input.jsonl" > "${dest}/write.jsonl" \
    || { tail -n 5 "${dest}/write.jsonl" >&2; return 1; }
}

check() {
  "${VENV}/bin/python" "${RIG_DIR}/check.py" "$@"
}

echo "=== ${TOPOLOGY}: starting the warehouse ==="
"${COMPOSE[@]}" down -v -t 5 >/dev/null 2>&1 || true
if ! "${COMPOSE[@]}" up -d --wait --quiet-pull > "${WORKDIR}/up.log" 2>&1; then
  cat "${WORKDIR}/up.log" >&2
  exit 1
fi

dbt_venv
export PATH="${VENV}/bin:${PATH}"

if [[ "${TOPOLOGY}" == "standalone" ]]; then
  echo "=== Deploy hook, first run ==="
  deploy_hook
  echo "=== Bronze through destination-clickhouse ==="
  write_bronze
  check snapshot --out "${WORKDIR}/counts.json"
  echo "=== Deploy hook, second run ==="
  deploy_hook
  check assert --baseline "${WORKDIR}/counts.json" --probe-rows "${PROBE_ROWS}" --users presentation
  echo "=== standalone: OK ==="
  exit 0
fi

echo "=== Cluster-ready creators, first run ==="
create_databases
dbt_ddl_hooks
echo "=== Bronze through destination-clickhouse ==="
write_bronze
check snapshot --out "${WORKDIR}/counts.json"
echo "=== Cluster-ready creators, second run ==="
create_databases
dbt_ddl_hooks
check assert --baseline "${WORKDIR}/counts.json" --probe-rows "${PROBE_ROWS}" --users presentation

echo "=== Deploy hook through the proxy (expected to fail until T11/T12/T23 land) ==="
if deploy_hook > "${WORKDIR}/hook.log" 2>&1; then
  echo "::error title=cluster rig::The full deploy hook now succeeds on the replicated rig. Make the replicated leg strict: run deploy_hook twice there, as the standalone leg does, and delete this expected-failure step."
  exit 1
fi
echo "  failed, as expected; its last lines:"
{ grep -v '^{' "${WORKDIR}/hook.log" || true; } | tail -n 4 | sed 's/^/    /'
check gaps --expect-failure-from "${WORKDIR}/hook.log"
echo "=== replicated: OK (cluster-ready creators strict; full hook still expected to fail) ==="
