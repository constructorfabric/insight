#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Copy the mover's account of every sync into the connector sync ledger.
#
# Runs at the end of a reconcile tick, which already authenticates to the mover
# and knows which connectors it manages. This file is the gatherer: it resolves
# the connector -> connection map, mints the token, and hands the work to
# `python3 -m sweep` on stdin. The planning and the writing live there.
#
# INVARIANT: this never fails a tick. Observability is subordinate to the thing
# observed — a sweep that cannot record must not stop connectors from being
# reconciled — so every path returns 0 and says what happened in the log.
#
# Spec: docs/components/backend/analytics/specs/connector-health.
# ---------------------------------------------------------------------------

# NOTE: this file is sourced; no top-level `set -euo pipefail`.

_SWEEP_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_SWEEP_PY_DIR="$(cd "${_SWEEP_LIB_DIR}/../python" && pwd)"

# ---------------------------------------------------------------------------
# sweep_run
# Records this tick's syncs and configured set. Always returns 0.
# ---------------------------------------------------------------------------
sweep_run() {
  if [[ "${RECONCILE_DRY_RUN:-}" == "1" ]]; then
    log_line INFO "sweep: dry run, recording nothing"
    return 0
  fi

  local tick_id
  tick_id="${RECONCILE_RUN_ID:-}"
  if [[ -z "${tick_id}" ]]; then
    # Outside the cluster there is no workflow pod to name the tick after.
    tick_id="local-$(date -u +%Y%m%dT%H%M%SZ)-$$"
  fi

  local workspace_id
  if ! workspace_id="$(ab_workspace_id)"; then
    log_line WARN "sweep: cannot resolve the workspace; recording nothing this tick"
    return 0
  fi

  local connections
  if ! connections="$(ab_list_connections "${workspace_id}")"; then
    log_line WARN "sweep: cannot list connections; recording nothing this tick"
    return 0
  fi

  local work
  if ! work="$(sweep__build_work "${tick_id}" "${connections}")"; then
    log_line WARN "sweep: cannot describe this tick's work; recording nothing"
    return 0
  fi

  local token
  if ! token="$(ab_get_token)"; then
    log_line WARN "sweep: cannot mint a mover token; recording nothing this tick"
    return 0
  fi

  # SAFETY: the token rides the child's environment, never its argv — argv is
  # world-readable inside the pod and the environment is not.
  local output status=0
  output="$(printf '%s' "${work}" \
    | AIRBYTE_TOKEN="${token}" PYTHONPATH="${_SWEEP_PY_DIR}" python3 -m sweep 2>&1)" \
    || status=$?

  while IFS= read -r line; do
    [[ -n "${line}" ]] && log_line INFO "${line}"
  done <<< "${output}"

  if (( status != 0 )); then
    log_line WARN "sweep: this tick recorded nothing or only part of it (exit ${status}); reconciliation is unaffected"
  fi
  return 0
}

# ---------------------------------------------------------------------------
# sweep__build_work <tick_id> <connections_json>
# Emits the JSON the sweep reads on stdin:
#   {"tick_id": "...", "connectors": [{"name": ..., "connection_id": ...}]}
#
# A connector with no connection yet is left out rather than reported with an
# empty id: it is configured but has nothing to read, and the snapshot below
# still records it as managed.
# ---------------------------------------------------------------------------
sweep__build_work() {
  local tick_id="$1"
  local connections="$2"

  local entries=()
  local name rest conn_name conn_id
  # Re-delimited on US for the same reason reconcile_run does it: TAB is
  # IFS-whitespace, so empty descriptor fields would collapse and shift. Only
  # the name is read here; `rest` absorbs the columns this sweep has no use for.
  while IFS=$'\037' read -r name rest; do
    [[ -n "${name}" ]] || continue
    : "${rest}"  # read into it deliberately; nothing here needs the other columns
    conn_name="$(reconcile_compute_connection_name "${name}")"
    conn_id="$(printf '%s' "${connections}" \
      | python3 "${_SWEEP_PY_DIR}/filter_connection_by_name.py" --name "${conn_name}" \
      | head -1)"
    [[ -n "${conn_id}" ]] || continue
    entries+=("$(jq -cn --arg n "${name}" --arg c "${conn_id}" \
      '{name: $n, connection_id: $c}')")
  done < <(disc_load_descriptors | tr '\t' '\037')

  local joined
  joined="$(IFS=,; printf '%s' "${entries[*]:-}")"
  jq -cn --arg t "${tick_id}" --argjson c "[${joined}]" \
    '{tick_id: $t, connectors: $c}'
}
