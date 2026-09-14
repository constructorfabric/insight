#!/usr/bin/env bash
# argo.sh — Argo CronWorkflow + Workflow CRUD helpers.
# Sourceable; NO top-level CLI.
#
# Public surface:
#   argo_cron_workflow_name              CONNECTOR TENANT SOURCE_ID
#   argo_assert_distinct_cron_names      CONNECTOR TENANT SOURCE_ID...
#   argo_cron_workflow_name_full_tenant  CONNECTOR TENANT
#   argo_render_cronworkflow CONNECTOR CONNECTION_NAME SCHEDULE TENANT \
#                            INSIGHT_SOURCE_ID DBT_SELECT ENRICH_IMAGE
#   argo_apply_cronworkflow  CONNECTOR CONNECTION_NAME SCHEDULE TENANT \
#                            INSIGHT_SOURCE_ID DBT_SELECT ENRICH_IMAGE
#   argo_delete_cronworkflow CONNECTOR TENANT SOURCE_ID
#   argo_submit_sync_trigger CONNECTOR CONNECTION_NAME TENANT \
#                            INSIGHT_SOURCE_ID DBT_SELECT ENRICH_IMAGE [BUMP_KIND]
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

# WORKAROUND: Argo rejects a CronWorkflow whose name exceeds 52 characters — it
# appends `-<10-digit unix timestamp>` per scheduled Workflow, which must fit a
# 63-character DNS label. The controller rejects it only after a successful
# apply, so the object exists, carries a SpecError condition and never
# schedules. A 36-character UUID tenant would leave 10 for the connector slug.
ARGO_CRON_TENANT_CHARS=8
ARGO_CRON_NAME_MAX_CHARS=52

# INVARIANT: a DNS-1123 name segment ends alphanumeric; truncation can leave a
# separator at the tail.
_argo_tenant_prefix() {
  local tenant_prefix="${1:0:${ARGO_CRON_TENANT_CHARS}}"
  while [[ "$tenant_prefix" == *[-.] ]]; do
    tenant_prefix="${tenant_prefix%?}"
  done
  printf '%s' "$tenant_prefix"
}

# The instance's own label inside its connector's name.
#
# A source id conventionally repeats the connector it belongs to
# (`claude-team-invoices-main`), and the name it goes into already carries that
# — a repeat costs the budget twice and reads as a stutter. A source id that
# does NOT start with the connector's name is used whole: it is outside the
# convention, and shortening it by some other rule would be inventing one.
argo_instance_label() {
  local connector="$1" source_id="$2"
  printf '%s' "${source_id#"${connector}-"}"
}

argo_cron_workflow_name() {
  local connector="$1" tenant="$2" source_id="$3"
  if [[ -z "${source_id}" ]]; then
    printf '%s: a CronWorkflow name needs the instance it schedules; got an empty source id.\n' \
      "$connector" >&2
    return 1
  fi
  local tenant_prefix label
  tenant_prefix="$(_argo_tenant_prefix "$tenant")"
  label="$(argo_instance_label "$connector" "$source_id")"

  local name="${connector}-${label}-${tenant_prefix}-sync"
  if (( ${#name} > ARGO_CRON_NAME_MAX_CHARS )); then
    printf 'CronWorkflow name %s is %s characters; Argo accepts at most %s. Shorten the connector slug or the source id.\n' \
      "$name" "${#name}" "${ARGO_CRON_NAME_MAX_CHARS}" >&2
    return 1
  fi
  printf '%s' "$name"
}

# INVARIANT: two instances of one connector must never render one name.
#
# The label drops a repeated connector prefix, so two source ids differing only
# by that prefix collapse onto each other. Applied, the second would replace the
# first's schedule under the same object — the connector would look scheduled
# while one of its instances silently stopped syncing. Checked across a
# connector's whole instance set before anything is applied.
argo_assert_distinct_cron_names() {
  local connector="$1" tenant="$2"
  shift 2
  local source_id name
  local -A claimed_by=()
  for source_id in "$@"; do
    name="$(argo_cron_workflow_name "$connector" "$tenant" "$source_id")" || return 1
    if [[ -n "${claimed_by[$name]:-}" ]]; then
      printf '%s: source ids %s and %s both name their CronWorkflow %s; one would replace the other. Give one a source id that does not collapse onto the other.\n' \
        "$connector" "${claimed_by[$name]}" "$source_id" "$name" >&2
      return 1
    fi
    claimed_by[$name]="$source_id"
  done
}

# An installation may still hold a CronWorkflow named with the whole tenant id.
# It schedules independently of the bounded-name object, so both existing would
# sync the connector twice per window.
argo_cron_workflow_name_full_tenant() {
  local connector="$1" tenant="$2"
  printf '%s-%s-sync' "$connector" "$tenant"
}

# And one named before the instance label joined the name. Same hazard: it
# schedules on its own, so an install that keeps both syncs twice per window.
argo_cron_workflow_name_without_instance() {
  local connector="$1" tenant="$2"
  printf '%s-%s-sync' "$connector" "$(_argo_tenant_prefix "$tenant")"
}

# The names an earlier release of this loop could hold for the same instance,
# one per line and never the one in use now.
#
# Emitted without repeats: the two shapes coincide when the tenant is no longer
# than the bound, and deleting one name twice would read in the log as two
# objects removed.
_argo_superseded_names() {
  local connector="$1" tenant="$2" current="$3"
  local candidate
  local -A emitted=()
  for candidate in \
    "$(argo_cron_workflow_name_full_tenant "$connector" "$tenant")" \
    "$(argo_cron_workflow_name_without_instance "$connector" "$tenant")"; do
    [[ "${candidate}" != "${current}" ]] || continue
    [[ -z "${emitted[$candidate]:-}" ]] || continue
    emitted[$candidate]=1
    printf '%s\n' "${candidate}"
  done
}

_argo_delete_cronworkflow_named() {
  local name="$1"
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

# @cpt-begin:cpt-insightspec-algo-reconcile-render-cron-workflow:p1
argo_render_cronworkflow() {
  local connector="$1" connection_name="$2" schedule="$3" tenant="$4"
  local insight_source_id="$5" dbt_select="$6" enrich_image="$7"
  local cron_name
  cron_name="$(argo_cron_workflow_name "$connector" "$tenant" "$insight_source_id")" || return 1
  python3 "${ARGO_PY_DIR}/render_cronworkflow.py" \
    --connector "$connector" \
    --connection-name "$connection_name" \
    --schedule "$schedule" \
    --tenant "$tenant" \
    --cron-name "$cron_name" \
    --insight-source-id "$insight_source_id" \
    --dbt-select "$dbt_select" \
    --enrich-image "$enrich_image" \
    --tpl "${ARGO_TPL_DIR}/cron-workflow.yaml.tpl"
}
# @cpt-end:cpt-insightspec-algo-reconcile-render-cron-workflow:p1

argo_apply_cronworkflow() {
  local connector="$1" connection_name="$2" schedule="$3" tenant="$4"
  local insight_source_id="$5" dbt_select="$6" enrich_image="$7"

  # INVARIANT: resolved before anything is rendered or applied. Argo rejects an
  # over-cap name only AFTER a successful apply, leaving an object that carries
  # a SpecError and never schedules — so the length has to stop the apply, not
  # merely the render.
  local current_name
  current_name="$(argo_cron_workflow_name "$connector" "$tenant" "$insight_source_id")" || return 1

  local rendered apply_out
  rendered="$(argo_render_cronworkflow "$connector" "$connection_name" \
                "$schedule" "$tenant" "$insight_source_id" \
                "$dbt_select" "$enrich_image")" || return 1
  if ! apply_out="$(printf '%s' "$rendered" | kubectl apply -f - 2>&1)"; then
    printf '%s: kubectl apply failed: %s\n' \
      "$connector" "$apply_out" >&2
    return 1
  fi
  printf '%s\n' "$apply_out"

  # INVARIANT: exit 2 means the apply succeeded but a superseded CronWorkflow
  # was not removed — both schedule the sync until it is gone.
  local superseded
  while IFS= read -r superseded; do
    [[ -n "${superseded}" ]] || continue
    _argo_delete_cronworkflow_named "${superseded}" >/dev/null || return 2
  done < <(_argo_superseded_names "$connector" "$tenant" "${current_name}")
}

argo_delete_cronworkflow() {
  local connector="$1" tenant="$2" source_id="$3"
  local current_name superseded rc=0

  # A name over the cap was never applied, so there is nothing to remove under
  # it — removal still has to clear the superseded objects below.
  current_name="$(argo_cron_workflow_name "$connector" "$tenant" "$source_id")" || current_name=""

  while IFS= read -r superseded; do
    [[ -n "${superseded}" ]] || continue
    _argo_delete_cronworkflow_named "${superseded}" || rc=1
  done < <(_argo_superseded_names "$connector" "$tenant" "${current_name}")

  [[ -n "${current_name}" ]] || return "$rc"
  _argo_delete_cronworkflow_named "${current_name}" || rc=1
  return "$rc"
}

# Only this instance's own schedule. Its siblings keep theirs, and the shapes
# that name no instance are left where they are: they are shared, and removing
# one instance is not a reason to touch what the others may still be using.
argo_delete_instance_cronworkflow() {
  local connector="$1" tenant="$2" source_id="$3"
  local name
  name="$(argo_cron_workflow_name "$connector" "$tenant" "$source_id")" || return 1
  _argo_delete_cronworkflow_named "${name}"
}

# Only the shapes that name no instance — for a connector with no source left to
# read an instance id out of, which is every connector that was never installed
# here and every one whose last instance has just gone.
argo_delete_superseded_cronworkflows() {
  local connector="$1" tenant="$2"
  local name rc=0
  while IFS= read -r name; do
    [[ -n "${name}" ]] || continue
    _argo_delete_cronworkflow_named "${name}" || rc=1
  done < <(_argo_superseded_names "$connector" "$tenant" "")
  return "$rc"
}

# @cpt-begin:cpt-insightspec-algo-reconcile-render-sync-trigger:p1
argo_submit_sync_trigger() {
  local connector="$1" connection_name="$2" tenant="$3"
  local insight_source_id="$4" dbt_select="$5" enrich_image="$6"
  local bump_kind="${7:-none}"
  local rendered create_out
  rendered="$(python3 "${ARGO_PY_DIR}/render_sync_trigger.py" \
    --connector "$connector" \
    --connection-name "$connection_name" \
    --tenant "$tenant" \
    --insight-source-id "$insight_source_id" \
    --dbt-select "$dbt_select" \
    --enrich-image "$enrich_image" \
    --bump-kind "$bump_kind" \
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
