#!/usr/bin/env bash

_CONNECTOR_SCHEDULES_PY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../python" && pwd)"
_CONNECTOR_SCHEDULES_READBACK_SCRIPT="${_CONNECTOR_SCHEDULES_PY_DIR}/read_schedule_cronworkflow.py"

connector_schedules_read() {
  if ! _CONNECTOR_SCHEDULES_RESULT="$(python3 "${_CONNECTOR_SCHEDULES_PY_DIR}/read_connector_schedules.py" 2>/dev/null)"; then
    _CONNECTOR_SCHEDULES_RESULT='{"status":"unavailable","reason":"schedule client failed"}'
  fi
  _CONNECTOR_SCHEDULES_STATUS="$(jq -r '.status' <<<"${_CONNECTOR_SCHEDULES_RESULT}" 2>/dev/null)" || _CONNECTOR_SCHEDULES_STATUS=""
  case "${_CONNECTOR_SCHEDULES_STATUS}" in
    ok|unavailable) ;;
    *)
      _CONNECTOR_SCHEDULES_STATUS=unavailable
      _CONNECTOR_SCHEDULES_RESULT='{"status":"unavailable","reason":"invalid schedule client result"}'
      ;;
  esac
  if ! connector_schedules_writable; then
    local reason
    reason="$(jq -r '.reason' <<<"${_CONNECTOR_SCHEDULES_RESULT}")"
    log_line ERROR "connector schedules unavailable: ${reason}; CronWorkflow schedules were left unchanged"
  fi
}

connector_schedules_writable() {
  [[ "${_CONNECTOR_SCHEDULES_STATUS-}" != "unavailable" ]]
}

connector_schedules_lookup() {
  [[ "${_CONNECTOR_SCHEDULES_STATUS-}" == "ok" ]] || return 0
  jq -r --arg connector "$1" --arg source_id "$2" \
    '.schedules[] | select(.connector == $connector and .source_id == $source_id and (.crons | length) > 0) | "api", .crons[]' \
    <<<"${_CONNECTOR_SCHEDULES_RESULT}"
}

connector_schedules_acknowledge() {
  [[ "${_CONNECTOR_SCHEDULES_STATUS-}" == "ok" ]] || return 0
  local connector="$1" source_id="$2" tenant="$3" schedule="$4" origin="$5" deployment="$6" apply_rc="$7"
  local name observed='{}' error=null plan
  name=""
  if [[ "${apply_rc}" -ne 0 ]]; then
    error='"schedule apply failed"'
  else
    name="$(argo_cron_workflow_name "${connector}" "${tenant}" "${source_id}")" || return 1
    if ! observed="$(python3 "${_CONNECTOR_SCHEDULES_READBACK_SCRIPT}" "${INSIGHT_NAMESPACE}" "${name}")"; then
      error='"schedule readback failed"'
      observed='{}'
    fi
  fi
  plan="$(jq -c --arg connector "${connector}" --arg source_id "${source_id}" \
    --arg name "${name}" --arg namespace "${INSIGHT_NAMESPACE}" --arg tenant "${tenant}" \
    --arg schedule "${schedule}" --arg origin "${origin}" --arg deployment "${deployment}" \
    '. as $snapshot |
      {connector:$connector, source_id:$source_id, name:$name, namespace:$namespace, tenant:$tenant,
       crons:($schedule | split("\n")), origin:$origin,
       deployment_crons:($deployment | split("\n") | .[1:]),
       deployment_origin:($deployment | split("\n") | .[0]),
       revision:([$snapshot.schedules[] | select(.connector==$connector and .source_id==$source_id) | .revision][0] // 0)}' <<<"${_CONNECTOR_SCHEDULES_RESULT}")" || return 1
  jq -c --argjson plan "${plan}" --argjson error "${error}" \
    '{plan:$plan,error:$error,observed:.}' <<<"${observed}" \
    | python3 "${_CONNECTOR_SCHEDULES_PY_DIR}/acknowledge_connector_schedule.py"
}
