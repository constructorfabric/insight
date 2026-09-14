#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Copy the mover's account of every sync into the connector sync ledger.
#
# Runs at the end of a reconcile tick, which already authenticates to the mover
# and knows which connectors it manages. This file is the gatherer: it resolves
# the connector -> connection map, mints the token, and hands the work to
# `python3 -m sweep` on stdin. The planning and the writing live there; the
# sweep logs its own structured JSON lines straight to stderr.
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
_SWEEP_SCRIPTS_DIR="$(cd "${_SWEEP_LIB_DIR}/../../scripts" && pwd)"

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
  # The sweep writes structured JSON lines to stderr itself, so stderr flows
  # to the pod log untouched — re-wrapping would nest JSON in JSON.
  local status=0
  printf '%s' "${work}" \
    | AIRBYTE_TOKEN="${token}" \
      PYTHONPATH="${_SWEEP_PY_DIR}:${_SWEEP_SCRIPTS_DIR}" \
      python3 -m sweep \
    || status=$?

  if (( status != 0 )); then
    log_line WARN "sweep: this tick recorded nothing or only part of it (exit ${status}); reconciliation is unaffected"
  fi
  return 0
}

# ---------------------------------------------------------------------------
# sweep__build_work <tick_id> <connections_json>
# Emits the JSON the sweep reads on stdin:
#   {"tick_id": "...",
#    "connectors":  [{"name": ..., "tenant_id": ..., "source_id": ...,
#                     "connection_id": ...}],
#    "descriptors": [{"name": ..., "namespace": ...}]}
#
# The identity travels with the name because one connector can be installed
# more than once, and the ledger tells those instances apart by it.
#
# `descriptors` covers every connector this build ships, installed or not: the
# ledger retains history after a Secret is removed, and the relation a
# descriptor names is where that connector's own record of which instance it
# was still lives.
#
# Configured means what it means to the reconcile loop: a descriptor WITH a
# Kubernetes Secret. Descriptors are every connector the product ships, and a
# descriptor without a Secret is not installed here — reconcile cascade-deletes
# its source and connection rather than driving it — so reporting the descriptor
# set would fill the page with connectors this install does not have.
#
# A connector the mover has no connection for yet is reported WITHOUT a
# connection id rather than dropped. It is configured — which is the first thing
# the page answers — and simply has nothing to read yet; dropping it would make
# "configured and never ran" a state the page could not show.
# ---------------------------------------------------------------------------
sweep__build_work() {
  local tick_id="$1"
  local connections="$2"

  # SAFETY: one read of the desired state, and a read that failed records
  # nothing. Asking per connector lets an API blip answer "not configured" for
  # one of them, and the read surface takes a sealed snapshot as authoritative
  # — the connector would render as removed rather than as unread.
  local plan_tsv
  if ! plan_tsv="$(disc_load_instances)"; then
    log_line WARN "sweep: cannot read the connector Secrets; recording nothing this tick"
    return 1
  fi

  local entries=() shipped=()
  local name connector_dir version type cdk_image enrich_image dbt_select ns_format
  local source_id secret_name cfg_hash conn_name conn_id tenant
  local seen_descriptor=""
  # Re-delimited on US for the same reason reconcile_run does it: TAB is
  # IFS-whitespace, so the plan's empty fields would collapse and shift.
  while IFS=$'\037' read -r name connector_dir version type cdk_image enrich_image dbt_select \
        ns_format source_id secret_name cfg_hash; do
    [[ -n "${name}" ]] || continue
    # Read into deliberately; the snapshot needs the identity and nothing else.
    : "${connector_dir}${version}${type}${cdk_image}${enrich_image}${dbt_select}${cfg_hash}"

    # Every descriptor, installed or not: history outlives a Secret, and the
    # only place an uninstalled connector's own recorded identity survives is
    # the relation this names.
    if [[ "${seen_descriptor}" != *"|${name}|"* ]]; then
      seen_descriptor+="|${name}|"
      shipped+=("$(jq -cn --arg n "${name}" --arg ns "${ns_format}" \
        '{name: $n, namespace: $ns}')")
    fi

    # A descriptor no Secret names is not installed here, and reconcile agrees:
    # it deletes such a connector's source rather than driving it.
    [[ -n "${secret_name}" ]] || continue
    tenant="$(reconcile_compute_tenant "${name}")"
    conn_name="$(reconcile_compute_connection_name "${name}" "${source_id}")"
    # Not piped into `head`: without `pipefail` that hides a nonzero exit from
    # the filter, and a failed lookup would read as "no connection yet" — which
    # would seal a snapshot claiming a connector the mover was never asked
    # about. Take the first line after the status has been checked.
    if ! conn_id="$(printf '%s' "${connections}" \
      | python3 "${_SWEEP_PY_DIR}/filter_connection_by_name.py" --name "${conn_name}")"; then
      log_line WARN "sweep: cannot resolve a connection for ${name}; recording nothing this tick"
      return 1
    fi
    conn_id="${conn_id%%$'\n'*}"
    if [[ -n "${conn_id}" ]]; then
      entries+=("$(jq -cn --arg n "${name}" --arg t "${tenant}" \
        --arg s "${source_id}" --arg c "${conn_id}" \
        '{name: $n, tenant_id: $t, source_id: $s, connection_id: $c}')")
    else
      entries+=("$(jq -cn --arg n "${name}" --arg t "${tenant}" \
        --arg s "${source_id}" \
        '{name: $n, tenant_id: $t, source_id: $s}')")
    fi
  done < <(printf '%s\n' "${plan_tsv}" | tr '\t' '\037')

  # `jq -s` slurps the stream into an array, so the objects never have to be
  # joined by hand — no separator to get wrong and no `IFS` to reassign.
  local connectors_json='[]' descriptors_json='[]'
  if (( ${#entries[@]} > 0 )); then
    connectors_json="$(printf '%s\n' "${entries[@]}" | jq -sc '.')"
  fi
  if (( ${#shipped[@]} > 0 )); then
    descriptors_json="$(printf '%s\n' "${shipped[@]}" | jq -sc '.')"
  fi
  jq -cn --arg t "${tick_id}" --argjson c "${connectors_json}" --argjson d "${descriptors_json}" \
    '{tick_id: $t, connectors: $c, descriptors: $d}'
}
