#!/usr/bin/env bash

_TRANSFORM_PLAN_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

transform_plan_run() {
  local dry_run="${1:?transform_plan_run requires the dry-run flag}"
  local args=()
  if [[ "${dry_run}" == "1" ]]; then
    args+=(--dry-run)
  fi

  if ! PYTHONPATH="${_TRANSFORM_PLAN_LIB_DIR}/../python:${_TRANSFORM_PLAN_LIB_DIR}/../../scripts" \
      python3 -m transform_plan "${args[@]}"; then
    log_line ERROR "transform planner failed; reconciliation is unaffected"
  fi
  return 0
}
