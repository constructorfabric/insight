#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib/discover.sh"
source "${SCRIPT_DIR}/lib/connector-naming.sh"
source "${SCRIPT_DIR}/lib/argo.sh"
source "${SCRIPT_DIR}/lib/log.sh"

schedule_apply() {
  : "${INSIGHT_TENANT_ID:?INSIGHT_TENANT_ID must be set}"
  local work_dir="$1" instances listed row connector directory version type image dbt_select namespace source_id secret hash
  local deployment resolved origin schedule
  python3 "${SCRIPT_DIR}/python/read_schedule_descriptor.py" --directory "${CONNECTORS_DIR}" || return 1
  connector_schedules_read
  connector_schedules_writable || return 1
  instances="$(disc_load_instances)" || return 1
  listed="$(python3 "${_CONNECTOR_SCHEDULES_READBACK_SCRIPT}" "${INSIGHT_NAMESPACE}" --list)" || return 1
  printf '%s' "${listed}" > "${work_dir}/workflows.json"
  : > "${work_dir}/plans.jsonl"

  while IFS= read -r row; do
    [[ -n "${row}" ]] || continue
    IFS='|' read -r connector directory version type image dbt_select namespace source_id secret hash <<< "${row//$'\t'/|}"
    [[ -n "${secret}" ]] || continue
    deployment="$(reconcile_resolve_deployment_schedule "${connector}" "${secret}")" || return 1
    resolved="$(reconcile_resolve_schedule "${connector}" "${secret}" "${source_id}" "${deployment}")" || return 1
    origin="${resolved%%$'\n'*}"
    schedule="${resolved#*$'\n'}"
    connector_schedules_plan "${connector}" "${source_id}" "${INSIGHT_TENANT_ID}" \
      "${schedule}" "${origin}" "${deployment}" >> "${work_dir}/plans.jsonl" || return 1
  done <<< "${instances}"

  python3 "${SCRIPT_DIR}/python/schedule_apply.py" "${work_dir}/plans.jsonl" "${work_dir}/workflows.json"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  work_dir="$(mktemp -d)"
  trap 'rm -rf "${work_dir}"' EXIT
  schedule_apply "${work_dir}"
fi
