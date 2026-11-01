#!/usr/bin/env bash
# db-footprint.sh <container-name> <data-dir> — one line on how full a DB
# container's tmpfs data dir got and how close its cgroup came to the memory
# limit, so a lane that fails on "No space left on device" or an OOM kill can
# be resized from the log. Never fails the step: a container that is not
# running has only its exit state to report.
set -uo pipefail

NAME="${1:?usage: db-footprint.sh <container-name> <data-dir>}"
DIR="${2:?usage: db-footprint.sh <container-name> <data-dir>}"

if ! used="$(docker exec "${NAME}" df -h "${DIR}" 2>/dev/null | awk 'NR == 2 { print $3 " of " $2 }')" || [ -z "${used}" ]; then
  state="$(docker inspect -f 'status={{.State.Status}} exit={{.State.ExitCode}} oom-killed={{.State.OOMKilled}}' "${NAME}" 2>/dev/null)"
  echo "${NAME}: not running (${state:-no such container}), no footprint"
  exit 0
fi

# cgroup v2 paths; the runners are Ubuntu 24.04 and ubuntu-latest. A read that
# fails prints as "unknown", never as a number.
iec() {
  local v
  if v="$(docker exec "${NAME}" cat "$1" 2>/dev/null)" && [ -n "${v}" ]; then
    if [ "${v}" = "max" ]; then echo "unlimited"; else numfmt --to=iec "${v}"; fi
  else
    echo "unknown"
  fi
}
echo "${NAME}: ${DIR} ${used} used; cgroup memory peak $(iec /sys/fs/cgroup/memory.peak) of $(iec /sys/fs/cgroup/memory.max)"
