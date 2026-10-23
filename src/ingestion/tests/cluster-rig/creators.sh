#!/usr/bin/env bash
# Sourced by run.sh and convert.sh: the creators of warehouse relations the
# rig drives, each the way the installation runs it — the chart's
# clickhouse-migrate Job, reduced to its command under its env; the pinned
# destination-clickhouse, configured by reconcile's composer; dbt, with the
# profile the Argo dbt-run step writes — and the connectors whose streams they
# carry. The creators read the CLICKHOUSE_* / RIG_* env warehouse.sh exports
# at the time they run, so a leg that changes topology mid-way re-exports
# them and calls the same functions.
#
# Expects warehouse.sh's RIG_DIR, SCRIPTS, WORKDIR, PROJECT, RIG_PROXY and
# CLICKHOUSE_*. Exposes CHART, PROBE_ROWS, ADDED_COLUMN, DBT_SELECT and:
#   deploy_hook <log> [helm --set ...]   — the deploy hook, as the Job runs it
#   refused_before_ddl <log> <reason> <what> [--set ...]
#                                        — a deploy the warehouse must refuse
#   discover_streams                     — every stream the connectors declare
#   write_bronze <round>                 — one sync's bronze through the destination
#   write_healed_bronze                  — the healed connector's bronze, empty
#   dbt_build <target>                   — the connector's models, as Argo builds them
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
