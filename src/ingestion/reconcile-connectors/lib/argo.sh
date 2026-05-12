#!/usr/bin/env bash
# argo.sh — Argo CronWorkflow + Workflow CRUD helpers.
# Sourceable; NO top-level CLI.
#
# Public surface:
#   argo_render_cronworkflow CONNECTOR CONNECTION_NAME SCHEDULE TENANT \
#                            CONNECTOR_DIR INSIGHT_SOURCE_ID
#   argo_apply_cronworkflow  CONNECTOR CONNECTION_NAME SCHEDULE TENANT \
#                            CONNECTOR_DIR INSIGHT_SOURCE_ID
#   argo_delete_cronworkflow CONNECTOR TENANT
#   argo_submit_sync_trigger CONNECTOR CONNECTION_NAME TENANT \
#                            CONNECTOR_DIR INSIGHT_SOURCE_ID
#   argo_resolve_connection_id_by_name CONNECTION_NAME
#
# Depends on: lib/env.sh (env_load), lib/airbyte.sh (ab_workspace_id,
# ab_list_connections), python/render_cronworkflow.py,
# python/render_sync_trigger.py, python/filter_connection_by_name.py.

# NOTE: this file is sourced; no top-level `set -euo pipefail` (leaks into
# interactive shells and breaks PROMPT_COMMAND on unset vars).

ARGO_SCRIPT_DIR="$( cd "$(dirname "${BASH_SOURCE[0]}")" && pwd )"
ARGO_PY_DIR="$( cd "${ARGO_SCRIPT_DIR}/../python" && pwd )"
ARGO_TPL_DIR="$( cd "${ARGO_SCRIPT_DIR}/../templates" && pwd )"

# @cpt-begin:cpt-insightspec-algo-reconcile-render-cron-workflow:p1
argo_render_cronworkflow() {
  local connector="$1" connection_name="$2" schedule="$3" tenant="$4"
  local connector_dir="$5" insight_source_id="$6"
  python3 "${ARGO_PY_DIR}/render_cronworkflow.py" \
    --connector "$connector" \
    --connection-name "$connection_name" \
    --schedule "$schedule" \
    --tenant "$tenant" \
    --connector-dir "$connector_dir" \
    --insight-source-id "$insight_source_id" \
    --tpl "${ARGO_TPL_DIR}/cron-workflow.yaml.tpl"
}
# @cpt-end:cpt-insightspec-algo-reconcile-render-cron-workflow:p1

argo_apply_cronworkflow() {
  local connector="$1" connection_name="$2" schedule="$3" tenant="$4"
  local connector_dir="$5" insight_source_id="$6"
  local rendered apply_out
  rendered="$(argo_render_cronworkflow "$connector" "$connection_name" \
                "$schedule" "$tenant" "$connector_dir" "$insight_source_id")" || return 1
  if ! apply_out="$(printf '%s' "$rendered" | kubectl apply -f - 2>&1)"; then
    printf '%s: kubectl apply failed: %s\n' \
      "$connector" "$apply_out" >&2
    return 1
  fi
  printf '%s\n' "$apply_out"
}

argo_delete_cronworkflow() {
  local connector="$1" tenant="$2"
  local name="${connector}-${tenant}-sync"
  local del_out
  # Pin the namespace explicitly. The rendered CronWorkflow lives in
  # `metadata.namespace: ${INSIGHT_NAMESPACE}`; kubectl without `-n`
  # falls back to the current context's default, and `--ignore-not-found`
  # then silently no-ops when contexts disagree — leaving the orphan in
  # place while the reconcile cascade believes it cleaned up.
  if ! del_out="$(kubectl -n "${INSIGHT_NAMESPACE}" \
        delete cronworkflow.argoproj.io/"${name}" --ignore-not-found 2>&1)"; then
    printf '%s: kubectl delete failed: %s\n' \
      "$name" "$del_out" >&2
    return 1
  fi
  printf '%s\n' "$del_out"
}

# @cpt-begin:cpt-insightspec-algo-reconcile-render-sync-trigger:p1
argo_submit_sync_trigger() {
  local connector="$1" connection_name="$2" tenant="$3"
  local connector_dir="$4" insight_source_id="$5"
  local rendered create_out
  rendered="$(python3 "${ARGO_PY_DIR}/render_sync_trigger.py" \
    --connector "$connector" \
    --connection-name "$connection_name" \
    --tenant "$tenant" \
    --connector-dir "$connector_dir" \
    --insight-source-id "$insight_source_id" \
    --tpl "${ARGO_TPL_DIR}/sync-trigger.yaml.tpl")" || return 1
  if ! create_out="$(printf '%s' "$rendered" | kubectl create -f - 2>&1)"; then
    printf '%s: kubectl create failed: %s\n' \
      "$connector" "$create_out" >&2
    return 1
  fi
  printf '%s\n' "$create_out"
}
# @cpt-end:cpt-insightspec-algo-reconcile-render-sync-trigger:p1

# @cpt-begin:cpt-insightspec-algo-reconcile-resolve-connection-by-name:p1
argo_resolve_connection_id_by_name() {
  local connection_name="$1"
  local workspace_id
  workspace_id="$(ab_workspace_id)"
  local list_json
  list_json="$(ab_list_connections "$workspace_id")"
  local matches
  matches="$(printf '%s' "$list_json" \
    | python3 "${ARGO_PY_DIR}/filter_connection_by_name.py" --name "$connection_name")"
  local count
  count="$(printf '%s' "$matches" | grep -c . || true)"
  if [[ "$count" -eq 0 ]]; then
    printf 'ERROR: connection name not found\n' >&2
    return 1
  fi
  if [[ "$count" -gt 1 ]]; then
    printf 'ERROR: ambiguous connection name (%s matches)\n' "$count" >&2
    return 1
  fi
  printf '%s' "$matches"
}
# @cpt-end:cpt-insightspec-algo-reconcile-resolve-connection-by-name:p1
