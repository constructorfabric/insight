#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# @cpt:cpt-insightspec-featstatus-reconcile — diff + apply engine
# @cpt-flow:cpt-insightspec-flow-reconcile-run-reconcile-v2:p1
# @cpt-algo:cpt-insightspec-algo-reconcile-diff-definition-version:p1
# @cpt-algo:cpt-insightspec-algo-reconcile-diff-source-config:p1
# @cpt-algo:cpt-insightspec-algo-reconcile-diff-connection-tags:p2
# @cpt-algo:cpt-insightspec-algo-reconcile-gc-orphans:p2
# @cpt-algo:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1
#
# Per-layer reconcile: definitions → sources → connections → optional GC.
# Driven by descriptor.yaml + K8s Secrets (desired state) and Airbyte
# (actual state). All mutations are idempotent. Recreate is rare and
# preserves stream cursors via state export/import (Decision #5).
# Sourced — never executed standalone.
#
# Function naming: `reconcile_*`; lowercase.
# ---------------------------------------------------------------------------

set -euo pipefail

: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"
: "${CONNECTORS_DIR:?CONNECTORS_DIR must be set, typically src/ingestion/connectors}"

_RECONCILE_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_RECONCILE_PY_DIR="$(cd "${_RECONCILE_LIB_DIR}/../python" && pwd)"

# shellcheck source=./airbyte.sh
source "${_RECONCILE_LIB_DIR}/airbyte.sh"
# shellcheck source=./discover.sh
source "${_RECONCILE_LIB_DIR}/discover.sh"
# shellcheck source=./adopt.sh
source "${_RECONCILE_LIB_DIR}/adopt.sh"
# shellcheck source=./argo.sh
source "${_RECONCILE_LIB_DIR}/argo.sh"
# shellcheck source=./log.sh
source "${_RECONCILE_LIB_DIR}/log.sh"
# shellcheck source=./validate.sh
source "${_RECONCILE_LIB_DIR}/validate.sh"

# Counters reset per reconcile_run.
_RECONCILE_CHANGED=0
_RECONCILE_NOOP=0
_RECONCILE_FAILED=0
_RECONCILE_SKIPPED=0

# ---------------------------------------------------------------------------
# reconcile__log <level> <connector> <message>
# Single-line structured log to stderr (level is INFO|WARN|ERROR|CHANGE).
# Never includes secret values.
# ---------------------------------------------------------------------------
reconcile__log() {
  local level="$1" connector="$2" message="$3"
  printf '%-6s [reconcile] connector=%s %s\n' \
    "${level}" "${connector}" "${message}" >&2
}

# ---------------------------------------------------------------------------
# reconcile_compute_connection_name <connector_name>
# Derives the Airbyte connection name for a connector: pattern
#   {connector}-{source_id_label}-{tenant_id}-conn
# matching the name used when the connection was created.
# ---------------------------------------------------------------------------
reconcile_compute_connection_name() {
  local connector="$1"
  local namespace="${INSIGHT_NAMESPACE}"
  local secret_name
  secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null || true)"
  if [[ -z "${secret_name}" ]]; then
    printf '%s-main-%s-conn' "${connector}" "${INSIGHT_TENANT_ID:-}"
    return 0
  fi
  local source_id_label
  source_id_label="$(kubectl -n "${namespace}" get secret "${secret_name}" \
    -o jsonpath='{.metadata.annotations.insight\.cyberfabric\.com/source-id}' \
    2>/dev/null || true)"
  [[ -n "${source_id_label}" ]] || source_id_label="main"
  printf '%s-%s-%s-conn' "${connector}" "${source_id_label}" "${INSIGHT_TENANT_ID:-}"
}

# ---------------------------------------------------------------------------
# reconcile_compute_schedule <connector_name>
# Schedule precedence: Secret annotation > descriptor.yaml.schedule > default.
# ---------------------------------------------------------------------------
reconcile_compute_schedule() {
  local connector="$1"
  local namespace="${INSIGHT_NAMESPACE}"
  local secret_name schedule
  secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null || true)"
  if [[ -n "${secret_name}" ]]; then
    schedule="$(kubectl -n "${namespace}" get secret "${secret_name}" \
      -o jsonpath='{.metadata.annotations.insight\.cyberfabric\.com/schedule}' \
      2>/dev/null || true)"
    [[ -n "${schedule}" ]] && { printf '%s' "${schedule}"; return 0; }
  fi
  schedule="$(python3 "${_RECONCILE_PY_DIR}/parse_descriptor.py" \
    --descriptor "${CONNECTORS_DIR}/${connector}/descriptor.yaml" \
    --field schedule 2>/dev/null || true)"
  [[ -n "${schedule}" ]] && { printf '%s' "${schedule}"; return 0; }
  printf '0 0 * * *'
}

# ---------------------------------------------------------------------------
# reconcile_compute_tenant <connector_name>
# Resolves tenant slug: env INSIGHT_TENANT_ID > Secret metadata > "default".
# ---------------------------------------------------------------------------
reconcile_compute_tenant() {
  local connector="$1"
  [[ -n "${INSIGHT_TENANT_ID:-}" ]] && { printf '%s' "${INSIGHT_TENANT_ID}"; return 0; }
  local namespace="${INSIGHT_NAMESPACE}"
  local secret_name
  secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null || true)"
  if [[ -z "${secret_name}" ]]; then
    printf 'default'
    return 0
  fi
  local secret_file
  secret_file="$(mktemp -t insight-reconcile.XXXXXX)"
  trap "rm -f '${secret_file}'" RETURN
  kubectl -n "${namespace}" get secret "${secret_name}" -o json > "${secret_file}" 2>/dev/null || true
  python3 "${_RECONCILE_PY_DIR}/resolve_tenant.py" \
    --secret-json "${secret_file}" 2>/dev/null || printf 'default'
}

# ---------------------------------------------------------------------------
# reconcile_cascade_delete <connector_name>
# Deletes all Airbyte connections + sources + definition (if orphaned) and
# the per-connector Argo CronWorkflow. Called when the Secret is missing.
# ---------------------------------------------------------------------------
# @cpt-begin:cpt-insightspec-algo-reconcile-cascade-delete-cronworkflow:p1
reconcile_cascade_delete() {
  local connector="$1"
  local tenant
  tenant="$(reconcile_compute_tenant "${connector}")"
  local workspace_id
  workspace_id="$(ab_workspace_id)"

  # Find all sources whose name starts with the connector slug and delete them.
  # ab_delete_source also cascades connections in newer Airbyte; we make it
  # explicit for safety.
  local sources_json
  sources_json="$(ab_list_sources "${workspace_id}")"
  local connections_json
  connections_json="$(ab_list_connections "${workspace_id}")"

  # Delete connections bound to connector's sources (by name prefix).
  while IFS= read -r conn_id; do
    [[ -n "${conn_id}" ]] || continue
    ab_delete_source "${conn_id}" >/dev/null 2>&1 || true
  done < <(printf '%s' "${sources_json}" \
    | python3 -c '
import json, sys
target = sys.argv[1]
for s in json.load(sys.stdin):
    n = s.get("name", "")
    if n == target or n.startswith(f"{target}-"):
        print(s.get("sourceId", ""))
' "${connector}" 2>/dev/null || true)

  # Delete the per-connector CronWorkflow.
  argo_delete_cronworkflow "${connector}" "${tenant}" 2>/dev/null || true
  log_line WARN "cascade-delete ${connector}: secret missing"
}
# @cpt-end:cpt-insightspec-algo-reconcile-cascade-delete-cronworkflow:p1

# ---------------------------------------------------------------------------
# reconcile_classify_change <current_cfg_json> <target_cfg_json>
# Heuristic: any change in fields that re-tenant the source (host, db,
# schema, account, workspace, organization, repository, stream slice) is
# breaking. Credential rotations / interval tweaks are non-breaking.
# Echoes "breaking" or "non-breaking".
# ---------------------------------------------------------------------------
reconcile_classify_change() {
  local current_json="$1" target_json="$2"
  python3 "${_RECONCILE_PY_DIR}/classify_change.py" \
    "${current_json}" "${target_json}"
}

# ---------------------------------------------------------------------------
# reconcile_definitions <connector_name> <target_version> <type>
# diff-definition-version algorithm. Idempotent.
# ---------------------------------------------------------------------------
reconcile_definitions() {
  local connector_name="$1" target_version="$2" type="$3"
  local definition_id current_value action

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-none
  local workspace_id
  workspace_id="$(ab_workspace_id)"
  local defs_json
  defs_json="$(ab_list_definitions "${workspace_id}")"
  definition_id="$(printf '%s' "${defs_json}" | python3 -c '
import sys, json
target = sys.argv[1]
for d in json.load(sys.stdin):
    if d.get("name") == target:
        print(d.get("sourceDefinitionId", "")); break
' "${connector_name}")"
  if [[ -z "${definition_id}" ]]; then
    reconcile__log INFO "${connector_name}" \
      "no definition present — caller should publish first (action=republish)"
    printf '%s\n' "republish"
    return 0
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-none

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-mismatch
  local def_json
  def_json="$(ab_get_definition "${definition_id}")"
  if [[ "${type}" == "cdk" ]]; then
    current_value="$(printf '%s' "${def_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("dockerImageTag",""))')"
  else
    current_value="$(printf '%s' "${def_json}" | python3 -c 'import sys,json;d=json.load(sys.stdin);print((d.get("declarativeManifest") or {}).get("description",""))')"
  fi
  if [[ "${current_value}" == "${target_version}" ]]; then
    action="noop"
    _RECONCILE_NOOP=$((_RECONCILE_NOOP + 1))
  else
    action="republish"
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      reconcile__log CHANGE "${connector_name}" \
        "would_call ${type}_set_definition_${type} ${definition_id} ${target_version}"
    else
      adopt_match_definition "${definition_id}" "${target_version}" "${type}"
      reconcile__log CHANGE "${connector_name}" \
        "definition.${type} updated from ${current_value} to ${target_version}"
      _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
    fi
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-mismatch

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-return-noop
  printf '%s\t%s\n' "${action}" "${definition_id}"
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-return-noop
}

# ---------------------------------------------------------------------------
# reconcile_sources <connector_name> <target_cfg_json> <secret_cfg_hash> \
#                   <definition_id> <expected_source_name>
# diff-source-config algorithm. Returns TSV "action\tsource_id" on stdout.
# Action one of: create | update | recreate | noop.
# ---------------------------------------------------------------------------
reconcile_sources() {
  local connector_name="$1" target_cfg_json="$2" secret_cfg_hash="$3"
  local definition_id="$4" expected_source_name="$5"
  local workspace_id sources_json source_id current_cfg_json action change_class

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-name
  workspace_id="$(ab_workspace_id)"
  sources_json="$(ab_list_sources "${workspace_id}")"
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-name

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-if-none
  source_id="$(printf '%s' "${sources_json}" | python3 -c '
import sys, json
target = sys.argv[1]
for s in json.load(sys.stdin):
    if s.get("name") == target:
        print(s.get("sourceId", "")); break
' "${expected_source_name}")"
  if [[ -z "${source_id}" ]]; then
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      reconcile__log CHANGE "${connector_name}" \
        "would_call ab_create_source ${expected_source_name}"
    else
      local created
      created="$(ab_create_source "${workspace_id}" "${definition_id}" \
                  "${expected_source_name}" "${target_cfg_json}")"
      source_id="$(printf '%s' "${created}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("sourceId",""))')"
      reconcile__log CHANGE "${connector_name}" "source created: ${source_id}"
      _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
    fi
    printf 'create\t%s\n' "${source_id}"
    return 0
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-if-none

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-if-stale-def
  current_cfg_json="$(printf '%s' "${sources_json}" \
    | python3 "${_RECONCILE_PY_DIR}/select_source_config_by_name.py" \
        "${expected_source_name}")"
  change_class="$(reconcile_classify_change "${current_cfg_json}" "${target_cfg_json}")"
  if [[ "${change_class}" == "breaking" ]]; then
    action="recreate"
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      reconcile__log CHANGE "${connector_name}" \
        "would_call reconcile_recreate_with_state source=${source_id} (breaking)"
    else
      reconcile_recreate_with_state "" "${source_id}" "${definition_id}" \
        "${expected_source_name}" "${target_cfg_json}" "${secret_cfg_hash}"
      _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
    fi
  else
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-if-stale-def
    # @cpt-begin:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-return-update
    action="update"
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      reconcile__log CHANGE "${connector_name}" \
        "would_call ab_update_source ${source_id}"
    else
      ab_update_source "${source_id}" "${target_cfg_json}" \
        "${expected_source_name}" >/dev/null
      reconcile__log INFO "${connector_name}" "source updated: ${source_id}"
      _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
    fi
    # @cpt-end:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-return-update
  fi
  printf '%s\t%s\n' "${action}" "${source_id}"
}

# ---------------------------------------------------------------------------
# reconcile_connections <connector_name> <source_id> <secret_cfg_hash>
# diff-connection-tags algorithm. PATCHes connection tags so the set
# contains `insight` and a single `cfg-hash:<hash>` entry. Idempotent.
# Tag-only changes do NOT set data_changed (per ADR-0008).
# ---------------------------------------------------------------------------
reconcile_connections() {
  local connector_name="$1" source_id="$2" secret_cfg_hash="$3"
  local workspace_id connections_json filtered

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-connection-tags:p2:inst-dct-find-tag
  workspace_id="$(ab_workspace_id)"
  connections_json="$(ab_list_connections "${workspace_id}")"
  filtered="$(printf '%s' "${connections_json}" \
    | python3 "${_RECONCILE_PY_DIR}/select_connections_by_source.py" "${source_id}")"
  if [[ -z "${filtered}" ]]; then
    reconcile__log WARN "${connector_name}" \
      "no connection on source ${source_id} (caller should create one)"
    return 0
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-connection-tags:p2:inst-dct-find-tag

  while IFS= read -r conn_line; do
    [[ -n "${conn_line}" ]] || continue
    local connection_id existing_tags_json desired_action
    connection_id="$(printf '%s' "${conn_line}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["connectionId"])')"
    existing_tags_json="$(printf '%s' "${conn_line}" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin).get("tags",[])))')"

    # @cpt-begin:cpt-insightspec-algo-reconcile-diff-connection-tags:p2:inst-dct-if-drift
    desired_action="$(python3 "${_RECONCILE_PY_DIR}/tag_drift_check.py" \
      "${existing_tags_json}" "${secret_cfg_hash}")"
    if [[ "${desired_action}" == "patch_tags" ]]; then
      if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
        reconcile__log CHANGE "${connector_name}" \
          "would_call adopt_tag_connection ${connection_id} cfg_hash=${secret_cfg_hash}"
      else
        adopt_tag_connection "${connection_id}" "${secret_cfg_hash}" "${existing_tags_json}"
        reconcile__log CHANGE "${connector_name}" \
          "connection ${connection_id} tags patched"
        _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      fi
    else
      _RECONCILE_NOOP=$((_RECONCILE_NOOP + 1))
    fi
    # @cpt-end:cpt-insightspec-algo-reconcile-diff-connection-tags:p2:inst-dct-if-drift
  done <<<"${filtered}"
}

# ---------------------------------------------------------------------------
# reconcile_recreate_with_state <connection_id> <source_id> <definition_id> \
#                               <source_name> <target_cfg_json> <cfg_hash>
# Decision #5: state_export → delete → create_source → create_connection
# → state_import. If <connection_id> empty, the function looks up the
# connection bound to <source_id> first.
# ---------------------------------------------------------------------------
reconcile_recreate_with_state() {
  local connection_id="$1" source_id="$2" definition_id="$3"
  local source_name="$4" target_cfg_json="$5" cfg_hash="$6"
  local workspace_id

  workspace_id="$(ab_workspace_id)"

  # If caller didn't supply, find the (single) connection for this source.
  if [[ -z "${connection_id}" ]]; then
    local conns
    conns="$(ab_list_connections "${workspace_id}")"
    connection_id="$(printf '%s' "${conns}" \
      | python3 "${_RECONCILE_PY_DIR}/select_connection_by_source.py" "${source_id}")"
  fi

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-try
  local state_json=""
  if [[ -n "${connection_id}" ]]; then
    if ! state_json="$(ab_get_state "${connection_id}")"; then
      reconcile__log ERROR "${source_name}" "state export failed — aborting recreate"
      return 1
    fi
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-try

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-delete
  ab_delete_source "${source_id}" >/dev/null
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-delete

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-create
  local new_source_json new_source_id
  new_source_json="$(ab_create_source "${workspace_id}" "${definition_id}" \
                      "${source_name}" "${target_cfg_json}")"
  new_source_id="$(printf '%s' "${new_source_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("sourceId",""))')"

  local destination_id
  destination_id="${RECONCILE_DESTINATION_ID:-}"  # RULE-DEFAULTS-OK: explicit empty check below
  if [[ -z "${destination_id}" ]]; then
    reconcile__log ERROR "${source_name}" \
      "RECONCILE_DESTINATION_ID env not set — cannot create new connection"
    return 1
  fi
  : "${RECONCILE_DEFAULT_SCHEDULE_JSON:?Set RECONCILE_DEFAULT_SCHEDULE_JSON (e.g. '{\"scheduleType\":\"manual\"}' or cron form)}"
  local schedule_json="${RECONCILE_DEFAULT_SCHEDULE_JSON}"
  local tags_json
  tags_json="$(python3 -c 'import sys, json; print(json.dumps(["insight", f"cfg-hash:{sys.argv[1]}"]))' "${cfg_hash}")"
  local new_conn_json new_connection_id
  new_conn_json="$(ab_create_connection "${workspace_id}" "${new_source_id}" \
                    "${destination_id}" "${source_name}-conn" "${schedule_json}" \
                    "${tags_json}")"
  new_connection_id="$(printf '%s' "${new_conn_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("connectionId",""))')"
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-create

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-import
  if [[ -n "${state_json}" && -n "${new_connection_id}" ]]; then
    ab_create_or_update_state "${new_connection_id}" "${state_json}" >/dev/null
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-import

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-tag
  if [[ -n "${new_connection_id}" ]]; then
    ab_patch_connection_tags "${new_connection_id}" "${tags_json}" >/dev/null
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-tag

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-return
  reconcile__log CHANGE "${source_name}" \
    "recreate complete: new_source=${new_source_id} new_connection=${new_connection_id}"
  printf '%s\t%s\n' "${new_source_id}" "${new_connection_id}"
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-return
}

# ---------------------------------------------------------------------------
# reconcile_gc_orphans
# Delete connections + sources tagged `insight` whose connector descriptor
# no longer exists on disk. Skipped entirely when --no-gc was passed by
# the caller (reconcile_run sets RECONCILE_NO_GC=1 in that case). DoD:
# cpt-insightspec-dod-reconcile-gc-protected-by-no-gc-flag
# ---------------------------------------------------------------------------
reconcile_gc_orphans() {
  # @cpt-begin:cpt-insightspec-algo-reconcile-gc-orphans:p2:inst-gc-conn-loop
  if [[ "${RECONCILE_NO_GC:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
    reconcile__log INFO "_gc" "skipped (--no-gc set)"
    return 0
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-gc-orphans:p2:inst-gc-conn-loop

  local workspace_id descriptors_tsv known_names
  workspace_id="$(ab_workspace_id)"
  descriptors_tsv="$(disc_load_descriptors)"
  known_names="$(printf '%s\n' "${descriptors_tsv}" \
    | python3 "${_RECONCILE_PY_DIR}/extract_descriptor_names.py")"

  local connections_json sources_json
  connections_json="$(ab_list_connections "${workspace_id}")"
  sources_json="$(ab_list_sources "${workspace_id}")"

  # @cpt-begin:cpt-insightspec-algo-reconcile-gc-orphans:p2:inst-gc-conn-orphan
  local orphan_lines
  orphan_lines="$(python3 "${_RECONCILE_PY_DIR}/find_orphan_connections.py" \
    "${known_names}" "${sources_json}" "${connections_json}")"
  # @cpt-end:cpt-insightspec-algo-reconcile-gc-orphans:p2:inst-gc-conn-orphan

  # @cpt-begin:cpt-insightspec-algo-reconcile-gc-orphans:p2:inst-gc-src-loop
  while IFS=$'\t' read -r conn_id src_id conn_name; do
    [[ -n "${conn_id}" ]] || continue
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      reconcile__log CHANGE "${conn_name}" \
        "would_gc connection=${conn_id} source=${src_id}"
    else
      # connection deletes cascade in newer Airbyte but we delete source
      # explicitly to be safe (Airbyte private API).
      ab_delete_source "${src_id}" >/dev/null
      reconcile__log CHANGE "${conn_name}" \
        "gc deleted connection=${conn_id} source=${src_id}"
      _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
    fi
  done <<<"${orphan_lines}"
  # @cpt-end:cpt-insightspec-algo-reconcile-gc-orphans:p2:inst-gc-src-loop
}

# ---------------------------------------------------------------------------
# reconcile_dry_run [args...]
# Read-only diff: sets RECONCILE_DRY_RUN=1 and delegates to reconcile_run.
# ---------------------------------------------------------------------------
reconcile_dry_run() {
  RECONCILE_DRY_RUN=1 reconcile_run "$@"
}

# ---------------------------------------------------------------------------
# reconcile_run [opt_dry_run [opt_no_sync_trigger [opt_no_gc [opt_connector]]]]
# Top-level orchestrator. Iterates descriptors, validates secrets, calls
# layered reconcilers (definition, source, connection), applies Argo
# CronWorkflow (idempotent), submits sync-trigger on data-affecting changes,
# then runs optional GC. Returns 0 on success, 2 if any layer logged ERROR.
# ---------------------------------------------------------------------------
reconcile_run() {
  local opt_dry_run="${1:-0}"
  local opt_no_sync_trigger="${2:-0}"
  local opt_no_gc="${3:-0}"
  local opt_connector="${4:-}"

  [[ "${opt_dry_run}" -eq 1 ]] && export RECONCILE_DRY_RUN=1
  [[ "${opt_no_gc}" -eq 1 ]]   && export RECONCILE_NO_GC=1

  _RECONCILE_CHANGED=0
  _RECONCILE_NOOP=0
  _RECONCILE_FAILED=0
  _RECONCILE_SKIPPED=0

  log_init

  local descriptors_tsv
  descriptors_tsv="$(disc_load_descriptors)"

  while IFS=$'\t' read -r name connector_dir version type; do
    [[ -n "${name}" ]] || continue
    if [[ -n "${opt_connector}" && "${name}" != "${opt_connector}" ]]; then
      _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
      continue
    fi

    # Missing Secret → cascade-delete chain (per ADR-0007 / KEY DECISION #7).
    if valsec_secret_missing_p "${name}"; then
      if ! reconcile_cascade_delete "${name}"; then
        _RECONCILE_FAILED=$((_RECONCILE_FAILED + 1))
      fi
      _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      continue
    fi

    # Invalid Secret → WARN + skip (per ADR-0007 / KEY DECISION #7).
    local missing_field=""
    if ! missing_field="$(valsec_check_secret "${name}" 2>/dev/null)"; then
      log_line WARN "skip ${name}: missing field ${missing_field:-unknown}"
      _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
      continue
    fi

    local secret_name
    if ! secret_name="$(disc_match_descriptor_to_secret "${name}")"; then
      reconcile__log WARN "${name}" "no labelled secret in K8s — skipping"
      _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
      continue
    fi

    local cfg_hash secret_data_json
    cfg_hash="$(disc_compute_cfg_hash "${secret_name}")"
    secret_data_json="$(kubectl -n "${INSIGHT_NAMESPACE}" get secret "${secret_name}" \
      -o json 2>/dev/null \
      | python3 "${_RECONCILE_PY_DIR}/extract_secret_data.py")"

    local data_changed=0

    # Layer 1 — definition
    local def_result def_id def_action
    if ! def_result="$(reconcile_definitions "${name}" "${version}" "${type}")"; then
      log_line ERROR "definition layer failed for ${name}"
      _RECONCILE_FAILED=$((_RECONCILE_FAILED + 1))
      continue
    fi
    def_id="$(printf '%s' "${def_result}" | tail -1 | cut -f2)"
    if [[ -z "${def_id}" ]]; then
      reconcile__log WARN "${name}" "definition not yet present — skipping source/connection layers this run"
      _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
      continue
    fi
    def_action="$(printf '%s' "${def_result}" | tail -1 | cut -f1)"
    [[ "${def_action}" == "republish" ]] && data_changed=1

    # Layer 2 — source
    local tenant_id="${INSIGHT_TENANT_ID:-}"
    local source_id_label
    source_id_label="$(kubectl -n "${INSIGHT_NAMESPACE}" get secret "${secret_name}" \
      -o jsonpath='{.metadata.annotations.insight\.cyberfabric\.com/source-id}' 2>/dev/null || true)"
    [[ -n "${source_id_label}" ]] || source_id_label="main"
    local expected_source_name="${name}-${source_id_label}-${tenant_id}"
    local src_result src_id src_action
    if ! src_result="$(reconcile_sources "${name}" "${secret_data_json}" "${cfg_hash}" \
                  "${def_id}" "${expected_source_name}")"; then
      log_line ERROR "source layer failed for ${name}"
      _RECONCILE_FAILED=$((_RECONCILE_FAILED + 1))
      continue
    fi
    src_id="$(printf '%s' "${src_result}" | tail -1 | cut -f2)"
    src_action="$(printf '%s' "${src_result}" | tail -1 | cut -f1)"
    [[ -n "${src_id}" ]] || { reconcile__log WARN "${name}" "no source_id after layer 2"; continue; }
    # Source create/update/recreate is data-affecting per ADR-0008.
    [[ "${src_action}" != "noop" ]] && data_changed=1

    # Layer 3 — connection tags (tag-only: NOT data-affecting per ADR-0008).
    reconcile_connections "${name}" "${src_id}" "${cfg_hash}"

    # CronWorkflow apply (idempotent — kubectl apply no-op when YAML unchanged).
    local conn_name schedule tenant
    conn_name="$(reconcile_compute_connection_name "${name}")"
    schedule="$(reconcile_compute_schedule "${name}")"
    tenant="$(reconcile_compute_tenant "${name}")"
    if ! argo_apply_cronworkflow "${name}" "${conn_name}" "${schedule}" "${tenant}" >/dev/null 2>&1; then
      log_line ERROR "argo_apply_cronworkflow failed for ${name}"
      _RECONCILE_FAILED=$((_RECONCILE_FAILED + 1))
    fi

    # Sync-trigger only on data-affecting changes (per ADR-0008 / KEY DECISION #2).
    if [[ "${data_changed}" -eq 1 && "${opt_no_sync_trigger}" -ne 1 ]]; then
      if argo_submit_sync_trigger "${name}" "${conn_name}" "${tenant}" >/dev/null 2>&1; then
        log_line INFO "submitted sync trigger for ${name}"
      else
        log_line ERROR "argo_submit_sync_trigger failed for ${name}"
        _RECONCILE_FAILED=$((_RECONCILE_FAILED + 1))
      fi
    fi
  done <<<"${descriptors_tsv}"
  # shellcheck disable=SC2034
  : "${connector_dir:=}"  # silence unused-variable warning when no descriptors

  # Layer 4 — GC (skipped when --no-gc).
  reconcile_gc_orphans

  log_run_summary "${_RECONCILE_CHANGED}" "${_RECONCILE_FAILED}"
  log_close
  return $(( _RECONCILE_FAILED > 0 ? 2 : 0 ))
}
