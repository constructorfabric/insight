#!/usr/bin/env bash

_CONNECTOR_SCHEDULES_PY_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../python" && pwd)"

connector_schedules_read() {
  if ! _CONNECTOR_SCHEDULES_RESULT="$(python3 "${_CONNECTOR_SCHEDULES_PY_DIR}/read_connector_schedules.py" 2>/dev/null)"; then
    _CONNECTOR_SCHEDULES_RESULT='{"status":"unavailable","reason":"schedule client failed"}'
  fi
  _CONNECTOR_SCHEDULES_STATUS="$(jq -r '.status' <<<"${_CONNECTOR_SCHEDULES_RESULT}" 2>/dev/null)" || _CONNECTOR_SCHEDULES_STATUS=""
  case "${_CONNECTOR_SCHEDULES_STATUS}" in
    ok|off|unavailable) ;;
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
    '.schedules[] | select(.connector == $connector and .source_id == $source_id) | "api", .crons[]' \
    <<<"${_CONNECTOR_SCHEDULES_RESULT}"
}
