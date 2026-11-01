#!/usr/bin/env bash
# start-clickhouse.sh <container-name> — bring up one throwaway ClickHouse for
# a CI lane and wait until it answers queries.
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
# Data lives on a tmpfs, not the runner disk: every lane starts from an empty
# server and throws it away, so nothing needs to survive the container, and
# the runner's root volume is the standard tier. CLICKHOUSE_TMPFS_SIZE caps
# the data dir (a lane that outgrows it fails with "No space left on device";
# db-footprint.sh prints how close it came). CLICKHOUSE_TMPFS_SIZE=0 puts the
# data back on an anonymous volume.
#
# Memory: ClickHouse sizes itself from the RAM it can see, which for an
# unbounded container is the whole machine it shares with the rest of the
# job. CLICKHOUSE_MEM_LIMIT is the server's budget; raise it for a lane that
# reports MEMORY_LIMIT_EXCEEDED. tmpfs pages are charged to the container's
# cgroup as well, so the cgroup limit is the budget plus the tmpfs size, and
# the server's own cap is pinned to 80% of the budget (the same ratio
# deploy/compose/clickhouse-memory.xml applies) instead of 80% of the
# inflated cgroup limit, which would let the kernel kill the server as its
# data dir fills.
#
# Called once per phase: the lane deliberately builds each phase on a virgin
# cluster (see the workflow header).
set -euo pipefail

NAME="${1:?usage: start-clickhouse.sh <container-name>}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

: "${CLICKHOUSE_SERVER_IMAGE:?CLICKHOUSE_SERVER_IMAGE must be set (from pins.env)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"
: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set}"

MEM_LIMIT="${CLICKHOUSE_MEM_LIMIT:-3g}"
TMPFS_SIZE="${CLICKHOUSE_TMPFS_SIZE:-2g}"

to_bytes() {
  # The sizes Docker's --memory and --tmpfs accept: bytes, or <n>m / <n>g.
  case "$1" in
    *[0-9][gG]) echo $(( ${1%?} * 1024 * 1024 * 1024 )) ;;
    *[0-9][mM]) echo $(( ${1%?} * 1024 * 1024 )) ;;
    *[0-9][kK]) echo $(( ${1%?} * 1024 )) ;;
    *[0-9][bB]) echo "${1%?}" ;;
    *[0-9]) echo "$1" ;;
    *) echo "::error::size '$1' must be bytes or <n>k, <n>m, <n>g" >&2; exit 1 ;;
  esac
}

cgroup_limit="${MEM_LIMIT}"
storage=()
if [ "${TMPFS_SIZE}" != "0" ]; then
  cgroup_limit="$(( $(to_bytes "${MEM_LIMIT}") + $(to_bytes "${TMPFS_SIZE}") ))"
  cap_dir="$(mktemp -d)"
  printf '<clickhouse><max_server_memory_usage>%s</max_server_memory_usage></clickhouse>\n' \
    "$(( $(to_bytes "${MEM_LIMIT}") * 8 / 10 ))" > "${cap_dir}/memory-cap.xml"
  storage=(
    --tmpfs "/var/lib/clickhouse:rw,size=${TMPFS_SIZE}"
    -v "${cap_dir}/memory-cap.xml:/etc/clickhouse-server/config.d/memory-cap.xml:ro"
  )
fi

docker run -d --name "${NAME}" -p 8123:8123 \
  --memory "${cgroup_limit}" \
  "${storage[@]}" \
  -v "${REPO_ROOT}/deploy/compose/clickhouse-memory.xml:/etc/clickhouse-server/config.d/memory.xml:ro" \
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
    echo -n "${NAME}: ready, memory budget "
    curl -sS --fail-with-body "http://localhost:8123/" \
      -H "X-ClickHouse-User: ${CLICKHOUSE_USER}" \
      -H "X-ClickHouse-Key: ${CLICKHOUSE_PASSWORD}" \
      --data-binary "SELECT formatReadableSize(toUInt64(value)) FROM system.server_settings WHERE name = 'max_server_memory_usage'" \
      || echo "unknown"
    exit 0
  fi
  sleep 1
done

echo "::error title=ClickHouse did not start::${NAME} never answered SELECT 1 within 60s"
docker logs --tail 100 "${NAME}" || true
exit 1
