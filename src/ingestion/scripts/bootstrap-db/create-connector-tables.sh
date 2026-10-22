#!/usr/bin/env bash
# Create one connector's bronze tables on a ClickHouse this host can reach, by
# handing its discovered catalogue to destination-clickhouse — the one creator
# of that layer.
#
# The catalogue itself is built by lib/connector-catalog.sh, which needs no
# ClickHouse; a cluster with no Docker applies the same catalogue through a Job
# instead (tools/seed/seed-stand.sh).
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

# Optional. The destination reaches CLICKHOUSE_HOST from inside a container, so a
# ClickHouse addressable only on a docker network — a compose stand, whose published
# port is the host's and not the container's — needs this container on that network.
DESTINATION_NETWORK=()
if [[ -n "${DOCKER_NETWORK:-}" ]]; then
  DESTINATION_NETWORK=(--network "${DOCKER_NETWORK}")
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/connector-catalog.sh"
# The one composer of the destination's connectionConfiguration: reconcile's
# Airbyte destination, the seed Job and this script hand the destination the
# same topology keys, or a cluster gets a plain bronze from one of them.
COMPOSE_DESTINATION_CONFIG="${SCRIPT_DIR}/../../reconcile-connectors/python/compose_destination_config.py"

WORKDIR="$(allocate_connector_workdir)"
trap 'rm -rf "${WORKDIR}"' EXIT

NAMESPACE="$(build_configured_catalog "${CONNECTOR_DIR}" "${CONFIG_JSON}" "${WORKDIR}")"

RECONCILE_DEST_CLICKHOUSE_HOST="${CLICKHOUSE_HOST}" \
RECONCILE_DEST_CLICKHOUSE_PORT="${CLICKHOUSE_PORT}" \
RECONCILE_DEST_CLICKHOUSE_PROTOCOL="${CLICKHOUSE_PROTOCOL}" \
RECONCILE_DEST_CLICKHOUSE_DATABASE="${CLICKHOUSE_DATABASE}" \
RECONCILE_DEST_CLICKHOUSE_USERNAME="${CLICKHOUSE_USER}" \
RECONCILE_DEST_CLICKHOUSE_PASSWORD="${CLICKHOUSE_PASSWORD}" \
RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE="${CLICKHOUSE_CLUSTER_MODE:-false}" \
RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME="${CLICKHOUSE_CLUSTER_NAME:-}" \
  python3 "${COMPOSE_DESTINATION_CONFIG}" > "${WORKDIR}/destination_config.json"

echo "create tables in ${NAMESPACE}"
# World-readable is deliberate: destination-clickhouse does NOT start under a
# foreign uid (its entrypoint sources /airbyte/base.sh, readable only by its
# baked-in user), so its two inputs are made readable to it instead. They carry
# no connector secrets — only the ClickHouse password.
chmod 0644 "${WORKDIR}/destination_config.json" "${WORKDIR}/configured_catalog.json"
docker run --rm -i ${DESTINATION_NETWORK[@]+"${DESTINATION_NETWORK[@]}"} \
  -v "${WORKDIR}:/work:ro" "${DESTINATION_CLICKHOUSE_IMAGE}" \
  write --config /work/destination_config.json --catalog /work/configured_catalog.json \
  < "${WORKDIR}/traces.jsonl" \
  > "${WORKDIR}/write.jsonl" \
  || { tail -n 5 "${WORKDIR}/write.jsonl" >&2; exit 1; }
