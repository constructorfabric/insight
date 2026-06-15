#!/usr/bin/env bash
# log.sh — structured JSON logger (stdout).
# Sourceable; NO top-level CLI.
#
# Every line is one JSON object on stdout, picked up by the cluster's log
# collector (Alloy → Loki; `level` is promoted to a Loki label). This
# replaces the previous PVC file logger: workflow pods are GC'd minutes
# after completion, so stdout+collector is the only durable destination.
#
# Public surface (signatures unchanged from the file-logger era):
#   log_init                 # records run start time (for duration_ms)
#   log_line LEVEL MSG ...   # one JSON line if MSG non-empty; NOOP otherwise
#   log_event EVENT MSG [EXTRA_JSON]
#                            # lifecycle event line (reconcile.started, …)
#   log_run_summary CHANGES ERRS
#                            # ALWAYS emits reconcile.completed — even on a
#                            # no-op run. The old quiet-run policy (KEY
#                            # DECISION #6) is deliberately dropped for the
#                            # completion event: a missing reconcile.completed
#                            # in Loki now MEANS the loop did not run, which
#                            # is exactly what the absence-alert watches.
#   log_close                # NOOP (kept for call-site compatibility)
#
# RECONCILE_RUN_ID (the Argo workflow pod name, injected by the chart) is
# stamped on every line so one tick's lines group together in Loki.

# NOTE: this file is sourced; no top-level `set -euo pipefail`.

RECONCILE_T0=""

log_init() {
  RECONCILE_T0="$(date +%s)"
}

_log_json() {
  # _log_json LEVEL EVENT MSG EXTRA_JSON — single emission point.
  local level="$1" event="$2" msg="$3" extra="${4:-{\}}"
  jq -cn \
    --arg ts "$(date -u +%FT%TZ)" \
    --arg level "$(printf '%s' "${level}" | tr '[:upper:]' '[:lower:]')" \
    --arg event "${event}" \
    --arg msg "${msg}" \
    --arg run_id "${RECONCILE_RUN_ID:-}" \
    --argjson extra "${extra}" \
    '{ts: $ts, level: $level, component: "reconcile"}
     + (if $event  != "" then {event: $event}   else {} end)
     + (if $msg    != "" then {msg: $msg}       else {} end)
     + (if $run_id != "" then {run_id: $run_id} else {} end)
     + $extra'
}

# @cpt-begin:cpt-insightspec-algo-reconcile-write-log-line-on-change:p1
log_line() {
  local level="$1"; shift
  local msg="$*"
  if [[ -z "$msg" ]]; then
    return 0   # quiet-run safety: empty message → noop
  fi
  _log_json "${level}" "" "${msg}" "{}"
}
# @cpt-end:cpt-insightspec-algo-reconcile-write-log-line-on-change:p1

log_event() {
  local event="${1:?log_event: event name required}"
  local msg="${2:-}"
  local extra="${3:-{\}}"
  _log_json INFO "${event}" "${msg}" "${extra}"
}

log_run_summary() {
  local changes="${1:-0}"
  local errs="${2:-0}"
  local dur=0
  [[ -n "${RECONCILE_T0}" ]] && dur=$(( ($(date +%s) - RECONCILE_T0) * 1000 ))
  local status="success"
  [[ "${errs}" -gt 0 ]] && status="failed"
  _log_json "$([[ "${errs}" -gt 0 ]] && echo ERROR || echo INFO)" \
    "reconcile.completed" \
    "reconcile finished: ${changes} change(s), ${errs} error(s)" \
    "{\"status\":\"${status}\",\"changes\":${changes},\"errors\":${errs},\"duration_ms\":${dur}}"
}

log_close() {
  :
}
