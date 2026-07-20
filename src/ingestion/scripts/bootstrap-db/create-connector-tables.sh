#!/usr/bin/env bash
set -euo pipefail

CONNECTOR_DIR="${1:?usage: create-connector-tables.sh <connector-dir> <config.json>}"
CONFIG_JSON="${2:?usage: create-connector-tables.sh <connector-dir> <config.json>}"

: "${CLICKHOUSE_HOST:?CLICKHOUSE_HOST must be set}"
: "${CLICKHOUSE_PORT:?CLICKHOUSE_PORT must be set}"
: "${CLICKHOUSE_PROTOCOL:?CLICKHOUSE_PROTOCOL must be set (http or https)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"
: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set}"
: "${DESTINATION_CLICKHOUSE_IMAGE:?DESTINATION_CLICKHOUSE_IMAGE must be set}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONNECTOR_DIR="$(cd "${CONNECTOR_DIR}" && pwd)"
DESCRIPTOR="${CONNECTOR_DIR}/descriptor.yaml"

NAME="$(yq -r '.name' "${DESCRIPTOR}")"
CONNECTOR_TYPE="$(yq -r '.type // "nocode"' "${DESCRIPTOR}")"
NAMESPACE="$(yq -r '.connection.namespace' "${DESCRIPTOR}")"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "${WORKDIR}"' EXIT
cp "${CONFIG_JSON}" "${WORKDIR}/config.json"

echo "[${NAME}] discover"
if [[ "${CONNECTOR_TYPE}" == "cdk" ]]; then
  SOURCE_IMAGE="$(yq -r '.images.cdk.image' "${DESCRIPTOR}")"
  docker run --rm -v "${WORKDIR}:/work:ro" "${SOURCE_IMAGE}" \
    discover --config /work/config.json \
    > "${WORKDIR}/discover.jsonl" \
    || { tail -n 3 "${WORKDIR}/discover.jsonl" >&2; exit 1; }
else
  : "${SOURCE_DECLARATIVE_MANIFEST_IMAGE:?SOURCE_DECLARATIVE_MANIFEST_IMAGE must be set}"
  docker run --rm -v "${WORKDIR}:/work:ro" -v "${CONNECTOR_DIR}:/manifest:ro" \
    "${SOURCE_DECLARATIVE_MANIFEST_IMAGE}" \
    discover --config /work/config.json --manifest-path /manifest/connector.yaml \
    > "${WORKDIR}/discover.jsonl" \
    || { tail -n 3 "${WORKDIR}/discover.jsonl" >&2; exit 1; }
fi

jq -Rc 'fromjson? | select(.type == "CATALOG") | .catalog' "${WORKDIR}/discover.jsonl" \
  | tail -n 1 > "${WORKDIR}/catalog.json"
[[ -s "${WORKDIR}/catalog.json" ]] || { echo "[${NAME}] no CATALOG message in discover output" >&2; exit 1; }

jq --arg ns "${NAMESPACE}" '{streams: [.streams[] | {
    stream: {
      name: .name,
      namespace: $ns,
      json_schema: .json_schema,
      supported_sync_modes: (.supported_sync_modes // ["full_refresh"])
    },
    sync_mode: "full_refresh",
    destination_sync_mode: "append",
    generation_id: 1,
    minimum_generation_id: 0,
    sync_id: 1
  }]}' "${WORKDIR}/catalog.json" > "${WORKDIR}/configured_catalog.json"

jq -c --arg ns "${NAMESPACE}" '.streams[] | {
    type: "TRACE",
    trace: {
      type: "STREAM_STATUS",
      emitted_at: 1,
      stream_status: {
        stream_descriptor: {name: .name, namespace: $ns},
        status: "COMPLETE"
      }
    }
  }' "${WORKDIR}/catalog.json" > "${WORKDIR}/traces.jsonl"

jq -n '{
    host: env.CLICKHOUSE_HOST,
    port: env.CLICKHOUSE_PORT,
    protocol: env.CLICKHOUSE_PROTOCOL,
    database: env.CLICKHOUSE_DATABASE,
    username: env.CLICKHOUSE_USER,
    password: env.CLICKHOUSE_PASSWORD,
    enable_json: false
  }' > "${WORKDIR}/destination_config.json"

echo "[${NAME}] create tables in ${NAMESPACE}"
docker run --rm -i -v "${WORKDIR}:/work:ro" "${DESTINATION_CLICKHOUSE_IMAGE}" \
  write --config /work/destination_config.json --catalog /work/configured_catalog.json \
  < "${WORKDIR}/traces.jsonl" \
  > "${WORKDIR}/write.jsonl" \
  || { tail -n 5 "${WORKDIR}/write.jsonl" >&2; exit 1; }

MODEL="${NAME//-/_}__bronze_promoted"
if [[ -f "${CONNECTOR_DIR}/dbt/${MODEL}.sql" ]]; then
  echo "[${NAME}] promote bronze to ReplacingMergeTree (${MODEL})"
  "${SCRIPT_DIR}/run-dbt.sh" --select "${MODEL}"
else
  echo "[${NAME}] no ${MODEL}.sql model, skipping promotion"
fi
