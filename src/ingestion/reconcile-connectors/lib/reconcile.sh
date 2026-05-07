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

# NOTE: this file is sourced; no top-level `set -euo pipefail`.

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
  if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
    # @cpt-begin:cpt-insightspec-algo-reconcile-cascade-delete-cronworkflow:p1:inst-cd-dry-run-guard
    log_line WARN "would cascade-delete ${connector}: secret missing (dry-run)"
    # @cpt-end:cpt-insightspec-algo-reconcile-cascade-delete-cronworkflow:p1:inst-cd-dry-run-guard
    return 0
  fi
  local tenant
  tenant="$(reconcile_compute_tenant "${connector}")"
  local workspace_id
  workspace_id="${INSIGHT_AIRBYTE_WORKSPACE_ID:?INSIGHT_AIRBYTE_WORKSPACE_ID must be set (Airbyte workspace UUID for Insight connectors)}"

  # Find all sources whose name starts with the connector slug and delete them.
  # ab_delete_source also cascades connections in newer Airbyte; we make it
  # explicit for safety.
  local sources_json
  sources_json="$(ab_list_sources "${workspace_id}")"
  local connections_json
  connections_json="$(ab_list_connections "${workspace_id}")"

  # Delete connections bound to connector's sources (by name prefix).
  # RECONCILE_DRY_RUN guard at top of reconcile_cascade_delete short-circuits.
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
  # RECONCILE_DRY_RUN guard at top of reconcile_cascade_delete short-circuits.
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
# reconcile_definitions <connector_name> <target_version> <type> <connector_dir>
# diff-definition-version algorithm. Idempotent.
#
# For nocode connectors: drives the builder_projects publish/update flow.
#   - If no definition exists -> create builder project + publish manifest.
#   - If definition exists but builder project doesn't (orphan) -> delete
#     definition and recreate via builder + publish.
#   - If definition + builder both exist and version drifts ->
#     update_active_manifest.
#
# For cdk connectors: existing behaviour — image-tag drift via
# ab_set_definition_image_tag; `cdk-build.sh` handles initial creation.
# ---------------------------------------------------------------------------
reconcile_definitions() {
  local connector_name="$1" target_version="$2" type="$3" connector_dir="${4:-}"
  local definition_id current_value action manifest_path
  local rc=0

  manifest_path="${CONNECTORS_DIR}/${connector_dir}/connector.yaml"

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-none
  local workspace_id
  workspace_id="${INSIGHT_AIRBYTE_WORKSPACE_ID}"
  local defs_json
  defs_json="$(ab_list_definitions "${workspace_id}")"
  # custom is True: Insight namespace separation per ADR-0009.
  definition_id="$(printf '%s' "${defs_json}" | python3 -c '
import sys, json
target = sys.argv[1]
for d in json.load(sys.stdin):
    if d.get("name") == target and d.get("custom") is True:
        print(d.get("sourceDefinitionId", "")); break
' "${connector_name}")"

  if [[ -z "${definition_id}" ]]; then
    if [[ "${type}" == "nocode" ]]; then
      if [[ ! -f "${manifest_path}" ]]; then
        reconcile__log WARN "${connector_name}" \
          "type=nocode but no connector.yaml at ${manifest_path} — skip"
        printf '%s\n' "noop"
        return 0
      fi
      if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
        reconcile__log CHANGE "${connector_name}" \
          "would_call ab_builder_create_with_manifest + ab_builder_publish (first publish)"
        printf '%s\t%s\n' "republish" ""
        return 0
      fi
      local builder_id new_def_id
      if ! builder_id="$(ab_builder_create_with_manifest \
            "${workspace_id}" "${connector_name}" "${manifest_path}")"; then
        reconcile__log ERROR "${connector_name}" "ab_builder_create_with_manifest failed"
        return 1
      fi
      if [[ -z "${builder_id}" ]]; then
        reconcile__log ERROR "${connector_name}" "ab_builder_create_with_manifest: empty builderProjectId"
        return 1
      fi
      if ! new_def_id="$(ab_builder_publish \
            "${workspace_id}" "${builder_id}" "${connector_name}" \
            "${target_version}" "${manifest_path}")"; then
        reconcile__log ERROR "${connector_name}" "ab_builder_publish failed"
        return 1
      fi
      reconcile__log CHANGE "${connector_name}" \
        "first publish: builder=${builder_id} definition=${new_def_id}"
      _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      printf '%s\t%s\n' "republish" "${new_def_id}"
      return 0
    fi
    # @cpt-begin:cpt-insightspec-algo-reconcile-create-cdk-definition:p1
    # @cpt-flow:cpt-insightspec-flow-reconcile-publish-cdk-definition:p1
    # type=cdk first-publish path (per ADR-0011): register pre-built image as
    # custom source_definition. Reconcile never runs `docker build`.
    : "${IMAGE_REGISTRY:?IMAGE_REGISTRY must be set (e.g. ghcr.io/cyberfabric — derives CDK source dockerRepository per ADR-0011)}"
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      reconcile__log CHANGE "${connector_name}" \
        "would_call ab_create_custom_cdk_definition repo=${IMAGE_REGISTRY}/source-${connector_name}-insight tag=${target_version}"
      printf '%s\n' "republish"
      return 0
    fi
    local docker_repo new_def_id
    docker_repo="${IMAGE_REGISTRY}/source-${connector_name}-insight"
    # RECONCILE_DRY_RUN guarded above (would_call branch returns early).
    if ! new_def_id="$(ab_create_custom_cdk_definition \
                       "${workspace_id}" "${connector_name}" \
                       "${docker_repo}" "${target_version}")"; then
      # RECONCILE_DRY_RUN guarded above; this is the error path of the live call.
      reconcile__log ERROR "${connector_name}" "ab_create_custom_cdk_definition failed"
      return 1
    fi
    reconcile__log CHANGE "${connector_name}" \
      "first-publish CDK: ${docker_repo}:${target_version} -> definition=${new_def_id}"
    _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
    printf '%s\t%s\n' "republish" "${new_def_id}"
    return 0
    # @cpt-end:cpt-insightspec-algo-reconcile-create-cdk-definition:p1
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-none

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-mismatch
  if [[ "${type}" == "nocode" ]]; then
    if ! current_value="$(ab_get_definition_description "${definition_id}")"; then
      reconcile__log ERROR "${connector_name}" "ab_get_definition_description failed"
      return 1
    fi
    if [[ "${current_value}" == "${target_version}" ]]; then
      action="noop"
      _RECONCILE_NOOP=$((_RECONCILE_NOOP + 1))
    else
      action="republish"
      if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
        reconcile__log CHANGE "${connector_name}" \
          "would_call ab_builder_update_active_manifest ${definition_id} ${target_version}"
      else
        if [[ ! -f "${manifest_path}" ]]; then
          reconcile__log ERROR "${connector_name}" \
            "version drift but no connector.yaml at ${manifest_path}"
          return 1
        fi
        local builder_id
        builder_id="$(ab_builder_find_by_definition "${workspace_id}" "${definition_id}")"
        if [[ -z "${builder_id}" ]]; then
          # Orphan: definition with no builder project (legacy / imported
          # state). DO NOT delete — that would cascade-break linked sources
          # and connections. Operators must run the migrate-orphan helper
          # which preserves state. See tools/migrate-orphan-definition.sh.
          reconcile__log WARN "${connector_name}" \
            "ORPHAN definition ${definition_id} has no linked builder project. Version drift NOT propagated. Run \`bash src/ingestion/reconcile-connectors/tools/migrate-orphan-definition.sh ${connector_name}\` to safely recreate (state-preserving)."
          printf 'noop\n'
          return 0
        else
          if ! ab_builder_update_active_manifest \
                "${workspace_id}" "${builder_id}" "${target_version}" "${manifest_path}" >/dev/null; then
            reconcile__log ERROR "${connector_name}" "ab_builder_update_active_manifest failed"
            return 1
          fi
          reconcile__log CHANGE "${connector_name}" \
            "definition.nocode updated from ${current_value} to ${target_version}"
        fi
        _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      fi
    fi
  else
    # type=cdk
    local def_json
    if ! def_json="$(ab_get_definition "${definition_id}")"; then
      reconcile__log ERROR "${connector_name}" "ab_get_definition failed"
      return 1
    fi
    current_value="$(printf '%s' "${def_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("dockerImageTag",""))')"
    if [[ "${current_value}" == "${target_version}" ]]; then
      action="noop"
      _RECONCILE_NOOP=$((_RECONCILE_NOOP + 1))
    else
      action="republish"
      if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
        reconcile__log CHANGE "${connector_name}" \
          "would_call ab_set_definition_image_tag ${definition_id} ${target_version}"
      else
        if ! ab_set_definition_image_tag "${definition_id}" "${target_version}" >/dev/null; then
          reconcile__log ERROR "${connector_name}" "ab_set_definition_image_tag failed"
          return 1
        fi
        reconcile__log CHANGE "${connector_name}" \
          "definition.cdk updated from ${current_value} to ${target_version}"
        _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      fi
    fi
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-mismatch

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-return-noop
  printf '%s\t%s\n' "${action}" "${definition_id}"
  return "${rc}"
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
  workspace_id="${INSIGHT_AIRBYTE_WORKSPACE_ID:?INSIGHT_AIRBYTE_WORKSPACE_ID must be set (Airbyte workspace UUID for Insight connectors)}"
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
  workspace_id="${INSIGHT_AIRBYTE_WORKSPACE_ID:?INSIGHT_AIRBYTE_WORKSPACE_ID must be set (Airbyte workspace UUID for Insight connectors)}"
  connections_json="$(ab_list_connections "${workspace_id}")"
  filtered="$(printf '%s' "${connections_json}" \
    | python3 "${_RECONCILE_PY_DIR}/select_connections_by_source.py" "${source_id}")"
  if [[ -z "${filtered}" ]]; then
    # Bootstrap path: source exists but has no connection yet (clean cluster
    # / first run). Create one with discovered schema, append-only sync mode,
    # cron schedule, and reconcile tags. Caller treats this as data-affecting.
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      reconcile__log CHANGE "${connector_name}" \
        "would_call ab_create_connection source=${source_id} (first-time bootstrap)"
      printf 'created\t\n'
      return 0
    fi
    local destination_id
    destination_id="${RECONCILE_DESTINATION_ID:-}"  # RULE-DEFAULTS-OK: explicit empty check below — fail-fast
    if [[ -z "${destination_id}" ]]; then
      reconcile__log ERROR "${connector_name}" \
        "RECONCILE_DESTINATION_ID env not set — cannot bootstrap connection on source ${source_id}"
      return 1
    fi
    local discover_json sync_catalog
    if ! discover_json="$(ab_discover_schema "${source_id}")"; then
      reconcile__log ERROR "${connector_name}" \
        "ab_discover_schema failed for source ${source_id}"
      return 1
    fi
    if ! sync_catalog="$(printf '%s' "${discover_json}" \
          | python3 "${_RECONCILE_PY_DIR}/normalize_catalog_to_append.py")"; then
      reconcile__log ERROR "${connector_name}" \
        "normalize_catalog_to_append failed for source ${source_id}"
      return 1
    fi
    local cron_str schedule_json
    cron_str="$(reconcile_compute_schedule "${connector_name}")"
    schedule_json="$(python3 "${_RECONCILE_PY_DIR}/build_schedule_json.py" "${cron_str}")"
    local tags_json
    tags_json="$(python3 -c 'import sys, json; print(json.dumps(["insight", f"cfg-hash:{sys.argv[1]}"]))' "${secret_cfg_hash}")"
    local conn_name
    conn_name="$(reconcile_compute_connection_name "${connector_name}")"
    local new_conn_json new_conn_id
    # RECONCILE_DRY_RUN guarded by short-circuit at top of bootstrap branch.
    if ! new_conn_json="$(ab_create_connection "${workspace_id}" "${source_id}" \
              "${destination_id}" "${conn_name}" "${schedule_json}" \
              "${tags_json}" "${sync_catalog}")"; then
      reconcile__log ERROR "${connector_name}" \
        "ab_create_connection failed for source ${source_id}"
      return 1
    fi
    new_conn_id="$(printf '%s' "${new_conn_json}" \
      | python3 -c 'import sys,json;print(json.load(sys.stdin).get("connectionId",""))')"
    reconcile__log CHANGE "${connector_name}" \
      "first-create connection: ${new_conn_id}"
    _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
    printf 'created\t%s\n' "${new_conn_id}"
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

  workspace_id="${INSIGHT_AIRBYTE_WORKSPACE_ID:?INSIGHT_AIRBYTE_WORKSPACE_ID must be set (Airbyte workspace UUID for Insight connectors)}"

  # Defensive dry-run guard: callers (reconcile_sources) already short-circuit
  # before calling us, but enforce here too per dod-reconcile-dry-run-non-destructive.
  if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
    reconcile__log CHANGE "${source_name}" \
      "would_call reconcile_recreate_with_state source=${source_id}"
    return 0
  fi

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
  # RECONCILE_DRY_RUN guarded at top of reconcile_recreate_with_state.
  ab_delete_source "${source_id}" >/dev/null
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-delete

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-create
  local new_source_json new_source_id
  # RECONCILE_DRY_RUN guarded at top of reconcile_recreate_with_state.
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
  # RECONCILE_DRY_RUN guarded at top of reconcile_recreate_with_state.
  new_conn_json="$(ab_create_connection "${workspace_id}" "${new_source_id}" \
                    "${destination_id}" "${source_name}-conn" "${schedule_json}" \
                    "${tags_json}")"
  new_connection_id="$(printf '%s' "${new_conn_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("connectionId",""))')"
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-create

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-import
  if [[ -n "${state_json}" && -n "${new_connection_id}" ]]; then
    # RECONCILE_DRY_RUN guarded at top of reconcile_recreate_with_state.
    ab_create_or_update_state "${new_connection_id}" "${state_json}" >/dev/null
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-import

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-tag
  if [[ -n "${new_connection_id}" ]]; then
    # RECONCILE_DRY_RUN guarded at top of reconcile_recreate_with_state.
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
  workspace_id="${INSIGHT_AIRBYTE_WORKSPACE_ID:?INSIGHT_AIRBYTE_WORKSPACE_ID must be set (Airbyte workspace UUID for Insight connectors)}"
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
# ---------------------------------------------------------------------------
# _reconcile_one_connector <name> <connector_dir> <version> <type> \
#                          <opt_dry_run> <opt_no_sync_trigger> <opt_connector>
# Per-connector body extracted from the main loop so a single connector's
# failure can't kill the whole reconcile run. We deliberately do NOT enable
# `set -e` here — failures bubble up through explicit `if ! ...; then`
# branches and are reported via return codes.
# Returns 0 on success, non-zero on any per-layer failure.
# ---------------------------------------------------------------------------
_reconcile_one_connector() {
  local name="$1" connector_dir="$2" version="$3" type="$4"
  local opt_dry_run="$5" opt_no_sync_trigger="$6" opt_connector="$7"
  set +e  # explicit per-call error handling below

  if [[ -n "${opt_connector}" && "${name}" != "${opt_connector}" ]]; then
    _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
    return 0
  fi

  # Missing Secret -> cascade-delete chain (per ADR-0007 / KEY DECISION #7).
  if valsec_secret_missing_p "${name}"; then
    if ! reconcile_cascade_delete "${name}"; then
      return 1
    fi
    _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
    return 0
  fi

  # Invalid Secret -> WARN + skip (per ADR-0007 / KEY DECISION #7).
  local missing_field=""
  if ! missing_field="$(valsec_check_secret "${name}" "${INSIGHT_NAMESPACE}" "${connector_dir}" 2>/dev/null)"; then
    log_line WARN "skip ${name}: missing field ${missing_field:-unknown}"
    _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
    return 0
  fi

  local secret_name
  if ! secret_name="$(disc_match_descriptor_to_secret "${name}")"; then
    reconcile__log WARN "${name}" "no labelled secret in K8s — skipping"
    _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
    return 0
  fi

  local cfg_hash secret_data_json
  cfg_hash="$(disc_compute_cfg_hash "${secret_name}")"
  secret_data_json="$(kubectl -n "${INSIGHT_NAMESPACE}" get secret "${secret_name}" \
    -o json 2>/dev/null \
    | python3 "${_RECONCILE_PY_DIR}/extract_secret_data.py")"

  local data_changed=0

  # Layer 1 — definition
  local def_result def_id def_action
  if ! def_result="$(reconcile_definitions "${name}" "${version}" "${type}" "${connector_dir}")"; then
    log_line ERROR "definition layer failed for ${name}"
    return 1
  fi
  def_id="$(printf '%s' "${def_result}" | tail -1 | cut -f2)"
  if [[ -z "${def_id}" ]]; then
    reconcile__log WARN "${name}" "definition not yet present — skipping source/connection layers this run"
    _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
    return 0
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
    return 1
  fi
  src_id="$(printf '%s' "${src_result}" | tail -1 | cut -f2)"
  src_action="$(printf '%s' "${src_result}" | tail -1 | cut -f1)"
  if [[ -z "${src_id}" ]]; then
    reconcile__log WARN "${name}" "no source_id after layer 2"
    return 0
  fi
  # Source create/update/recreate is data-affecting per ADR-0008.
  [[ "${src_action}" != "noop" ]] && data_changed=1

  # Layer 3 — connection tags (tag-only: NOT data-affecting per ADR-0008).
  # Bootstrap path (first-time create) IS data-affecting and bumps
  # data_changed so a sync trigger fires.
  local conn_result conn_action
  conn_result="$(reconcile_connections "${name}" "${src_id}" "${cfg_hash}")"
  conn_action="$(printf '%s' "${conn_result}" | tail -1 | cut -f1)"
  [[ "${conn_action}" == "created" ]] && data_changed=1

  # CronWorkflow apply (idempotent — kubectl apply no-op when YAML unchanged).
  local conn_name schedule tenant rc=0
  conn_name="$(reconcile_compute_connection_name "${name}")"
  schedule="$(reconcile_compute_schedule "${name}")"
  tenant="$(reconcile_compute_tenant "${name}")"
  if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
    log_line INFO "would_call argo_apply_cronworkflow ${name} (dry-run)"
  elif ! argo_apply_cronworkflow "${name}" "${conn_name}" "${schedule}" "${tenant}" >/dev/null 2>&1; then
    log_line ERROR "argo_apply_cronworkflow failed for ${name}"
    rc=1
  fi

  # Sync-trigger only on data-affecting changes (per ADR-0008 / KEY DECISION #2).
  if [[ "${data_changed}" -eq 1 && "${opt_no_sync_trigger}" -ne 1 ]]; then
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      log_line INFO "would_call argo_submit_sync_trigger ${name} (dry-run)"
    elif argo_submit_sync_trigger "${name}" "${conn_name}" "${tenant}" >/dev/null 2>&1; then
      log_line INFO "submitted sync trigger for ${name}"
    else
      log_line ERROR "argo_submit_sync_trigger failed for ${name}"
      rc=1
    fi
  fi
  # silence unused-arg shellcheck warning
  : "${opt_dry_run}"
  return "${rc}"
}

reconcile_run() {
  : "${INSIGHT_AIRBYTE_WORKSPACE_ID:?INSIGHT_AIRBYTE_WORKSPACE_ID must be set (the Airbyte workspace UUID where Insight connectors are managed; see chart values ingestion.reconcile.airbyteWorkspaceId)}"
  : "${IMAGE_REGISTRY:?IMAGE_REGISTRY must be set (e.g. ghcr.io/cyberfabric; derives CDK source dockerRepository per ADR-0011; chart values ingestion.reconcile.imageRegistry)}"
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
    if ! _reconcile_one_connector "${name}" "${connector_dir}" "${version}" "${type}" \
         "${opt_dry_run}" "${opt_no_sync_trigger}" "${opt_connector}"; then
      log_line ERROR "reconcile: connector ${name} failed (continuing with next)"
      _RECONCILE_FAILED=$((_RECONCILE_FAILED + 1))
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
