#!/usr/bin/env bash
# start-clickhouse.sh <container-name> — bring up one throwaway ClickHouse for
# the connectors-ddl lane and wait until it answers queries.
#
# Same version production runs (CLICKHOUSE_SERVER_IMAGE from
# scripts/bootstrap-db/pins.env, loaded into the job env), and the same flags the
# local recipe in scripts/bootstrap-db/README.md uses:
#
#   CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 — lets the `insight` admin run
#     CREATE ROLE / CREATE USER / GRANT, which bootstrap-db needs to provision
#     the read-only `presentation_ro` role (#1963/#1964). The official image
#     disables it; both compose stacks and the bitnami prod admin have it.
#
# Called once per phase: the lane deliberately builds each phase on a virgin
# cluster (see the workflow header).
set -euo pipefail

NAME="${1:?usage: start-clickhouse.sh <container-name>}"

: "${CLICKHOUSE_SERVER_IMAGE:?CLICKHOUSE_SERVER_IMAGE must be set (from pins.env)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"
: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set}"

docker run -d --name "${NAME}" -p 8123:8123 \
  -e CLICKHOUSE_USER \
  -e CLICKHOUSE_PASSWORD \
  -e "CLICKHOUSE_DB=${CLICKHOUSE_DATABASE}" \
  -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  "${CLICKHOUSE_SERVER_IMAGE}"

for _ in $(seq 1 60); do
  if curl -sS --fail-with-body "http://localhost:8123/" \
      -H "X-ClickHouse-User: ${CLICKHOUSE_USER}" \
      -H "X-ClickHouse-Key: ${CLICKHOUSE_PASSWORD}" \
      --data-binary 'SELECT 1' >/dev/null 2>&1; then
    echo "${NAME}: ready"
    exit 0
  fi
  sleep 1
done

echo "::error title=ClickHouse did not start::${NAME} never answered SELECT 1 within 60s"
docker logs --tail 100 "${NAME}" || true
exit 1
