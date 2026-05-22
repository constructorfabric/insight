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
# shellcheck source=./connector-naming.sh
# Provides reconcile_compute_{connection_name,schedule,tenant} used by
# both reconcile.sh and adopt.sh. Sourced before adopt.sh so the helpers
# are resolvable regardless of which entry point loads which file first.
source "${_RECONCILE_LIB_DIR}/connector-naming.sh"
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
  printf '%-7s %s: %s\n' \
    "${level}" "${connector}" "${message}" >&2
  # Mirror to the audit file-log so per-connector CHANGE/INFO/WARN events
  # are not stderr-only. log_line is a noop on empty msg and on level
  # filtering; safe to call unconditionally.
  log_line "${level}" "${connector}: ${message}"
}

# reconcile_compute_{connection_name,schedule,tenant} have moved to
# lib/connector-naming.sh so adopt.sh can call them without depending on
# the order in which reconcile.sh and adopt.sh source each other.

# ---------------------------------------------------------------------------
# reconcile_resolve_destination_id <log_subject>
# Resolves (or creates) the Airbyte destination Bronze sink owned by
# reconcile. Strategy:
#   1. If RECONCILE_DESTINATION_ID env is set (legacy / explicit override),
#      use it verbatim.
#   2. Otherwise, look up an existing destination by name
#      RECONCILE_DESTINATION_NAME (default `clickhouse-bronze`); if absent,
#      create one with definition Clickhouse and config from
#      RECONCILE_DEST_CLICKHOUSE_* env (host/port/db/user/password).
# Caches the resolved id in _RECONCILE_DESTINATION_ID for the run.
# Echoes the destinationId on stdout; returns non-zero on failure.
# ---------------------------------------------------------------------------
reconcile_resolve_destination_id() {
  local subject="$1"
  if [[ -n "${_RECONCILE_DESTINATION_ID:-}" ]]; then
    printf '%s' "${_RECONCILE_DESTINATION_ID}"
    return 0
  fi
  if [[ -n "${RECONCILE_DESTINATION_ID:-}" ]]; then
    _RECONCILE_DESTINATION_ID="${RECONCILE_DESTINATION_ID}"
    printf '%s' "${_RECONCILE_DESTINATION_ID}"
    return 0
  fi
  local dest_name="${RECONCILE_DESTINATION_NAME:-clickhouse-bronze}"  # RULE-DEFAULTS-OK: project-fixed name, not operator-tunable
  local def_id
  if ! def_id="$(ab_destination_definition_id_by_name Clickhouse 2>/dev/null)"; then
    reconcile__log ERROR "${subject}" \
      "Airbyte does not register a Clickhouse destination definition in this workspace — cannot bootstrap Bronze sink"
    return 1
  fi

  # Build connection config from env. Required for fresh-cluster bootstrap;
  # caller (Helm chart reconcile-cron.yaml) injects them from chart values
  # + insight-db-creds secret.
  : "${RECONCILE_DEST_CLICKHOUSE_HOST:?RECONCILE_DEST_CLICKHOUSE_HOST must be set (the in-cluster ClickHouse host for the Bronze destination)}"
  : "${RECONCILE_DEST_CLICKHOUSE_PORT:?RECONCILE_DEST_CLICKHOUSE_PORT must be set}"
  : "${RECONCILE_DEST_CLICKHOUSE_DATABASE:?RECONCILE_DEST_CLICKHOUSE_DATABASE must be set}"
  : "${RECONCILE_DEST_CLICKHOUSE_USERNAME:?RECONCILE_DEST_CLICKHOUSE_USERNAME must be set}"
  : "${RECONCILE_DEST_CLICKHOUSE_PASSWORD:?RECONCILE_DEST_CLICKHOUSE_PASSWORD must be set}"
  local config_json
  config_json="$(python3 -c '
import os, json
print(json.dumps({
  "host":     os.environ["RECONCILE_DEST_CLICKHOUSE_HOST"],
  "port":     int(os.environ["RECONCILE_DEST_CLICKHOUSE_PORT"]),
  "database": os.environ["RECONCILE_DEST_CLICKHOUSE_DATABASE"],
  "username": os.environ["RECONCILE_DEST_CLICKHOUSE_USERNAME"],
  "password": os.environ["RECONCILE_DEST_CLICKHOUSE_PASSWORD"],
  "ssl":      False,
  "schema":   "default",
}))
')"

  local dest_id
  if ! dest_id="$(ab_ensure_destination "${dest_name}" "${def_id}" "${config_json}")"; then
    reconcile__log ERROR "${subject}" "ab_ensure_destination failed for ${dest_name}"
    return 1
  fi
  if [[ -z "${dest_id}" ]]; then
    reconcile__log ERROR "${subject}" "ab_ensure_destination returned empty id for ${dest_name}"
    return 1
  fi
  _RECONCILE_DESTINATION_ID="${dest_id}"
  printf '%s' "${dest_id}"
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
    log_line WARN "would remove ${connector} from Airbyte — its Secret was deleted in Kubernetes"
    # @cpt-end:cpt-insightspec-algo-reconcile-cascade-delete-cronworkflow:p1:inst-cd-dry-run-guard
    return 0
  fi
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
  log_line WARN "${connector}: Secret was deleted in Kubernetes — removed connector from Airbyte"
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
# reconcile_definitions <connector_name> <target_version> <type> <connector_dir> [<cdk_image>]
# diff-definition-version algorithm. Idempotent.
#
# Per ADR-0015: target_version is strict semver MAJOR.MINOR.PATCH.
# Validation is delegated to python/classify_bump.py — non-semver target
# fails fast with exit 2 here (operator typo); legacy non-semver values on
# the Airbyte side are classified as `migration` (no full-refresh).
#
# For nocode connectors: drives the builder_projects publish/update flow.
#   - If no definition exists -> create builder project + publish manifest.
#   - If definition exists but builder project doesn't (orphan) -> delete
#     definition and recreate via builder + publish.
#   - If definition + builder both exist and version drifts ->
#     update_active_manifest.
#
# For cdk connectors: image drift via ab_set_definition_image_tag, driven by
# descriptor.images.cdk.image (a full Docker image reference; NOT descriptor.version).
# The reference is split via python/split_docker_image_ref.py into
# dockerRepository + dockerImageTag (digest or tag). When the image field is
# empty for type=cdk, WARN+skip until the image is published.
#
# Output: TSV `<action>\t<bump_kind>\t<definition_id>` on stdout where
#   action     ∈ {republish, noop}
#   bump_kind  ∈ {none, patch, minor, major, migration}
# ---------------------------------------------------------------------------
reconcile_definitions() {
  local connector_name="$1" target_version="$2" type="$3" connector_dir="${4:-}" cdk_image="${5:-}"
  local definition_id current_value action manifest_path bump_kind
  local rc=0

  # connector_dir is already a full path emitted by disc_load_descriptors
  # (e.g. "src/ingestion/connectors/collaboration/m365") — do NOT prepend
  # CONNECTORS_DIR or the path doubles up.
  manifest_path="${connector_dir}/connector.yaml"

  # Type=cdk requires cdk_image (full Docker reference). When absent,
  # WARN+skip — image not yet published. See FEATURE DoD
  # cpt-insightspec-dod-reconcile-cdk-image-required.
  if [[ "${type}" == "cdk" && -z "${cdk_image}" ]]; then
    reconcile__log WARN "${connector_name}" \
      "connector is cdk type but no image set in descriptor — skipping until image is published"
    printf 'noop\tnone\t\n'
    return 0
  fi

  # Per ADR-0015: descriptor.version is validated as strict semver only
  # when an actual diff is detected (the comparison below calls
  # classify_bump.py). Legacy values such as "2026.05.04" or "1.0" pass
  # through unchanged on the noop path so the migration to semver can
  # happen one connector at a time, on whatever cadence the operator
  # chooses, without a fleet-wide hard cutoff.
  bump_kind="none"

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-none
  local workspace_id
  workspace_id="$(ab_workspace_id)"
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
          "connector is nocode type but no manifest file at ${manifest_path} — skipping"
        printf 'noop\tnone\t\n'
        return 0
      fi
      if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
        reconcile__log CHANGE "${connector_name}" \
          "would publish connector for the first time (no definition exists yet)"
        # First-time publish is treated as bump_kind=major per ADR-0015 §Bump
        # kinds: the connector has no existing state to preserve, so its first
        # sync is effectively a full-refresh from cursor zero.
        # Pseudo def_id so downstream layers stay informative in dry-run.
        # Real run reaches the live calls below and returns the real UUID.
        printf 'republish\tmajor\tDRY-RUN-PENDING-NOCODE\n'
        return 0
      fi
      local builder_id new_def_id
      if ! builder_id="$(ab_builder_create_with_manifest \
            "${workspace_id}" "${connector_name}" "${manifest_path}")"; then
        reconcile__log ERROR "${connector_name}" "failed to create connector builder project"
        return 1
      fi
      if [[ -z "${builder_id}" ]]; then
        reconcile__log ERROR "${connector_name}" "Airbyte returned empty builder project id"
        return 1
      fi
      if ! new_def_id="$(ab_builder_publish \
            "${workspace_id}" "${builder_id}" "${connector_name}" \
            "${target_version}" "${manifest_path}")"; then
        reconcile__log ERROR "${connector_name}" "failed to publish connector definition"
        return 1
      fi
      reconcile__log CHANGE "${connector_name}" \
        "published for the first time: builder project ${builder_id}, definition ${new_def_id}"
      _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      printf 'republish\tmajor\t%s\n' "${new_def_id}"
      return 0
    fi
    # @cpt-begin:cpt-insightspec-algo-reconcile-create-cdk-definition:p1
    # @cpt-flow:cpt-insightspec-flow-reconcile-publish-cdk-definition:p1
    # type=cdk first-publish path (per ADR-0016, supersedes ADR-0011): register
    # pre-built image as custom source_definition. Reconcile never runs
    # `docker build`. The full image reference comes verbatim from
    # descriptor.images.cdk.image and is split into dockerRepository +
    # dockerImageTag via split_docker_image_ref.py.
    local docker_repo docker_tag
    IFS=$'\t' read -r docker_repo docker_tag \
      < <(python3 "${_RECONCILE_PY_DIR}/split_docker_image_ref.py" "${cdk_image}")
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      reconcile__log CHANGE "${connector_name}" \
        "would register cdk image ${docker_repo}:${docker_tag} as new definition"
      # CDK first-publish: per ADR-0015 §Bump-kind storage scope, CDK bump
      # classification is deferred — first publish emits bump_kind=patch so
      # the downstream re-discover runs without triggering full-refresh.
      # Pseudo def_id so downstream layers stay informative in dry-run.
      printf 'republish\tpatch\tDRY-RUN-PENDING-CDK\n'
      return 0
    fi
    local new_def_id
    # RECONCILE_DRY_RUN guarded above (would_call branch returns early).
    if ! new_def_id="$(ab_create_custom_cdk_definition \
                       "${workspace_id}" "${connector_name}" \
                       "${docker_repo}" "${docker_tag}")"; then
      # RECONCILE_DRY_RUN guarded above; this is the error path of the live call.
      reconcile__log ERROR "${connector_name}" "failed to register cdk image as new definition"
      return 1
    fi
    reconcile__log CHANGE "${connector_name}" \
      "registered cdk image ${docker_repo}:${docker_tag} as definition ${new_def_id}"
    _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
    printf 'republish\tpatch\t%s\n' "${new_def_id}"
    return 0
    # @cpt-end:cpt-insightspec-algo-reconcile-create-cdk-definition:p1
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-none

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-mismatch
  if [[ "${type}" == "nocode" ]]; then
    if ! current_value="$(ab_get_definition_description "${definition_id}")"; then
      reconcile__log ERROR "${connector_name}" "failed to read current connector version from Airbyte"
      return 1
    fi
    if [[ "${current_value}" == "${target_version}" ]]; then
      action="noop"
      bump_kind="none"
      _RECONCILE_NOOP=$((_RECONCILE_NOOP + 1))
    else
      action="republish"
      # Per ADR-0015: classify the diff for the caller (re-discover catalog
      # always; dispatch full-refresh on major only). `current_value` may be
      # a legacy non-semver string (e.g. "2026.05.04") — classify_bump.py
      # returns "migration" in that case (no full-refresh).
      if ! bump_kind="$(python3 "${_RECONCILE_PY_DIR}/classify_bump.py" \
            "${target_version}" "${current_value}" 2>/dev/null)"; then
        reconcile__log ERROR "${connector_name}" \
          "classify_bump rejected target '${target_version}' (must be strict semver per ADR-0015)"
        return 1
      fi
      if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
        reconcile__log CHANGE "${connector_name}" \
          "would update connector definition to version ${target_version} (bump_kind=${bump_kind})"
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
          printf 'noop\tnone\t%s\n' "${definition_id}"
          return 0
        else
          if ! ab_builder_update_active_manifest \
                "${workspace_id}" "${definition_id}" "${target_version}" "${manifest_path}" >/dev/null; then
            reconcile__log ERROR "${connector_name}" "failed to publish connector definition"
            return 1
          fi
          reconcile__log CHANGE "${connector_name}" \
            "connector definition version: ${current_value} → ${target_version} (bump_kind=${bump_kind})"
        fi
        _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      fi
    fi
  else
    # type=cdk
    local def_json
    if ! def_json="$(ab_get_definition "${definition_id}")"; then
      reconcile__log ERROR "${connector_name}" "failed to read current connector definition from Airbyte"
      return 1
    fi
    local current_repo current_tag
    current_repo="$(printf '%s' "${def_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("dockerRepository",""))')"
    current_tag="$(printf '%s' "${def_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("dockerImageTag",""))')"
    local desc_repo desc_tag
    IFS=$'\t' read -r desc_repo desc_tag \
      < <(python3 "${_RECONCILE_PY_DIR}/split_docker_image_ref.py" "${cdk_image}")

    if [[ "${current_repo}" != "${desc_repo}" ]]; then
      reconcile__log WARN "${connector_name}" \
        "cdk image repository changed (${current_repo} → ${desc_repo}); manual recreate-with-state needed — skipping for now"
      printf 'noop\tnone\t%s\n' "${definition_id}"
      return 0
    fi
    current_value="${current_tag}"
    if [[ "${current_tag}" == "${desc_tag}" ]]; then
      action="noop"
      bump_kind="none"
      _RECONCILE_NOOP=$((_RECONCILE_NOOP + 1))
    else
      action="republish"
      # Per ADR-0015 §Bump-kind storage scope: CDK image bumps emit
      # bump_kind=patch so re-discover runs without full-refresh. Operators
      # who need a CDK full-refresh dispatch it explicitly.
      bump_kind="patch"
      if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
        reconcile__log CHANGE "${connector_name}" \
          "would update connector definition to version ${desc_tag}"
      else
        if ! ab_set_definition_image_tag "${definition_id}" "${desc_tag}" >/dev/null; then
          reconcile__log ERROR "${connector_name}" "failed to update cdk image tag in Airbyte"
          return 1
        fi
        reconcile__log CHANGE "${connector_name}" \
          "cdk image tag: ${current_tag} → ${desc_tag}"
        _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      fi
    fi
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-if-mismatch

  # @cpt-begin:cpt-insightspec-algo-reconcile-diff-definition-version:p1:inst-ddv-return-noop
  printf '%s\t%s\t%s\n' "${action}" "${bump_kind}" "${definition_id}"
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
        "would create source ${expected_source_name}"
    else
      local created
      created="$(ab_create_source "${workspace_id}" "${definition_id}" \
                  "${expected_source_name}" "${target_cfg_json}")"
      source_id="$(printf '%s' "${created}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("sourceId",""))')"
      reconcile__log CHANGE "${connector_name}" "source ${source_id} created"
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
  case "${change_class}" in
    breaking)
      action="recreate"
      if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
        reconcile__log CHANGE "${connector_name}" \
          "would recreate source ${source_id} (config change is breaking — state preserved across recreate)"
      else
        local recreate_result new_src_id
        if ! recreate_result="$(reconcile_recreate_with_state "" "${source_id}" "${definition_id}" \
              "${expected_source_name}" "${target_cfg_json}" "${secret_cfg_hash}" \
              "${connector_name}")"; then
          reconcile__log ERROR "${connector_name}" \
            "reconcile_recreate_with_state failed for source ${source_id}"
          return 1
        fi
        # recreate_result last line is `<new_source_id>\t<new_connection_id>`.
        new_src_id="$(printf '%s' "${recreate_result}" | tail -1 | awk -F'\t' '{print $1}')"
        if [[ -n "${new_src_id}" ]]; then
          # Use the NEW source id for the rest of the layer; the old one
          # was deleted inside reconcile_recreate_with_state.
          source_id="${new_src_id}"
        fi
        _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      fi
      ;;
    non-breaking)
  # @cpt-end:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-if-stale-def
      # @cpt-begin:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-return-update
      action="update"
      if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
        reconcile__log CHANGE "${connector_name}" \
          "would update source ${source_id} with new credentials"
      else
        ab_update_source "${source_id}" "${target_cfg_json}" \
          "${expected_source_name}" >/dev/null
        reconcile__log INFO "${connector_name}" "source ${source_id} updated"
        _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      fi
      # @cpt-end:cpt-insightspec-algo-reconcile-diff-source-config:p1:inst-dsc-return-update
      ;;
    noop|*)
      action="noop"
      _RECONCILE_NOOP=$((_RECONCILE_NOOP + 1))
      ;;
  esac
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
    # Bootstrap path: source exists but has no connection yet (clean cluster
    # / first run). Create one with discovered schema, append-only sync mode,
    # manual schedule (Argo CronWorkflow is the sole scheduler — see
    # templates/cron-workflow.yaml.tpl), and reconcile tags. Caller treats
    # this as data-affecting.
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      reconcile__log CHANGE "${connector_name}" \
        "source ${source_id} has no connection yet — will create one"
      printf 'created\t\n'
      return 0
    fi
    local destination_id
    if ! destination_id="$(reconcile_resolve_destination_id "${connector_name}")"; then
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
    # Airbyte connection is created with scheduleType=manual; Argo
    # CronWorkflow drives sync timing (reconcile_compute_schedule feeds the
    # CronWorkflow render in _reconcile_one_connector). Without this,
    # Airbyte's Temporal scheduler would fire syncs on its own cron in
    # parallel with Argo, landing Bronze rows without running dbt.
    local schedule_json='{"scheduleType":"manual"}'
    local tag_names_json tags_json
    # cfg-hash truncated to first 12 hex chars: Airbyte caps tag name at 30,
    # the prefix `cfg-hash:` is 9, full sha256 (64) blows the limit. 12 chars
    # = 48 bits of entropy — plenty to detect drift on this small key set.
    tag_names_json="$(python3 -c 'import sys, json; print(json.dumps(["insight", f"cfg-hash:{sys.argv[1][:12]}"]))' "${secret_cfg_hash}")"
    # Airbyte v1 schemas require Tag objects (tagId/workspaceId/name/color);
    # ab_resolve_tags creates any missing tags in the workspace and echoes
    # the resolved Tag-object array.
    tags_json="$(ab_resolve_tags "${workspace_id}" "${tag_names_json}")"
    local conn_name
    conn_name="$(reconcile_compute_connection_name "${connector_name}")"
    local new_conn_json new_conn_id
    # RECONCILE_DRY_RUN guarded by short-circuit at top of bootstrap branch.
    # Per-connector ClickHouse schema: bronze_<connector>. Reconcile owns
    # this convention so dbt downstream can target predictable schemas;
    # without it, all connectors' streams collide in destination.database.
    local namespace_format="bronze_${connector_name}"
    if ! new_conn_json="$(ab_create_connection "${workspace_id}" "${source_id}" \
              "${destination_id}" "${conn_name}" "${schedule_json}" \
              "${tags_json}" "${sync_catalog}" "${namespace_format}")"; then
      reconcile__log ERROR "${connector_name}" \
        "ab_create_connection failed for source ${source_id}"
      return 1
    fi
    new_conn_id="$(printf '%s' "${new_conn_json}" \
      | python3 -c 'import sys,json;print(json.load(sys.stdin).get("connectionId",""))')"
    reconcile__log CHANGE "${connector_name}" \
      "connection ${new_conn_id} created"
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
          "would tag connection ${connection_id} as managed by Insight (cfg-hash ${secret_cfg_hash})"
      else
        adopt_tag_connection "${connection_id}" "${secret_cfg_hash}" "${existing_tags_json}"
        reconcile__log CHANGE "${connector_name}" \
          "connection ${connection_id} tags updated"
        _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      fi
      # Caller's `tail -1 | cut -f1` reads this to decide whether to fire
      # a sync trigger. Emit the action so cfg-hash rotations are seen.
      printf 'patch_tags\t%s\n' "${connection_id}"
    else
      _RECONCILE_NOOP=$((_RECONCILE_NOOP + 1))
      printf 'noop\t%s\n' "${connection_id}"
    fi
    # @cpt-end:cpt-insightspec-algo-reconcile-diff-connection-tags:p2:inst-dct-if-drift
  done <<<"${filtered}"
}

# ---------------------------------------------------------------------------
# reconcile_refresh_catalog <connector_name> <source_id> <connection_id>
# Per ADR-0015 / cpt-insightspec-algo-reconcile-refresh-catalog-on-republish:
# called whenever the definition was republished and a connection already
# exists. Re-discovers the source schema, normalizes append-only with
# every stream and field selected, then POSTs /connections/update to PATCH
# the sync_catalog in place. State (per-stream cursors) survives the
# update because Airbyte keys state on (connectionId, streamName), not on
# catalog shape.
# Returns 0 on success or noop (dry-run / connection_id empty), 1 on
# discover or update failure.
# ---------------------------------------------------------------------------
reconcile_refresh_catalog() {
  local connector_name="$1" source_id="$2" connection_id="$3"
  if [[ -z "${connection_id}" ]]; then
    # Bootstrap path: caller will have already created the connection with
    # a freshly-discovered catalog. Nothing to refresh.
    return 0
  fi
  if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
    reconcile__log CHANGE "${connector_name}" \
      "would refresh sync_catalog on connection ${connection_id} (re-discover; new streams/fields auto-enabled)"
    return 0
  fi
  local discover_json sync_catalog
  if ! discover_json="$(ab_discover_schema "${source_id}")"; then
    reconcile__log ERROR "${connector_name}" \
      "ab_discover_schema failed during catalog refresh for source ${source_id}"
    return 1
  fi
  if ! sync_catalog="$(printf '%s' "${discover_json}" \
        | python3 "${_RECONCILE_PY_DIR}/normalize_catalog_to_append.py")"; then
    reconcile__log ERROR "${connector_name}" \
      "normalize_catalog_to_append failed during catalog refresh for source ${source_id}"
    return 1
  fi
  if ! ab_update_connection_sync_catalog "${connection_id}" "${sync_catalog}" >/dev/null; then
    reconcile__log ERROR "${connector_name}" \
      "ab_update_connection_sync_catalog failed for connection ${connection_id}"
    return 1
  fi
  reconcile__log CHANGE "${connector_name}" \
    "sync_catalog refreshed on connection ${connection_id} (new streams/fields auto-enabled)"
  return 0
}

# ---------------------------------------------------------------------------
# reconcile_recreate_with_state <connection_id> <source_id> <definition_id> \
#                               <source_name> <target_cfg_json> <cfg_hash> \
#                               <connector_name>
# Decision #5: state_export → delete → create_source → create_connection
# → state_import. If <connection_id> empty, the function looks up the
# connection bound to <source_id> first. <connector_name> is the
# descriptor slug (e.g. `github-v2`) and drives the connection's
# bronze_<connector> namespace; passed explicitly because parsing it
# out of source_name breaks for slugs containing `-`.
# ---------------------------------------------------------------------------
reconcile_recreate_with_state() {
  local connection_id="$1" source_id="$2" definition_id="$3"
  local source_name="$4" target_cfg_json="$5" cfg_hash="$6"
  local connector_name="${7:?reconcile_recreate_with_state: connector_name (arg 7) is required for bronze_<connector> namespace}"
  local workspace_id

  workspace_id="$(ab_workspace_id)"

  # Defensive dry-run guard: callers (reconcile_sources) already short-circuit
  # before calling us, but enforce here too per dod-reconcile-dry-run-non-destructive.
  if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
    reconcile__log CHANGE "${source_name}" \
      "would recreate source ${source_id} (config change is breaking — state preserved across recreate)"
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
  local state_json="" state_backup=""
  if [[ -n "${connection_id}" ]]; then
    if ! state_json="$(ab_get_state "${connection_id}")"; then
      reconcile__log ERROR "${source_name}" "state export failed — aborting recreate"
      return 1
    fi
    # Persist the exported state to a 0600 tempfile *before* destructive
    # ab_delete_source so an operator can re-import via the legacy
    # /api/v1/state/create_or_update endpoint if a later step in this
    # function fails and the in-memory state_json is lost.
    state_backup="$(mktemp -t insight-state.XXXXXX)" \
      && chmod 600 "${state_backup}" \
      && printf '%s' "${state_json}" > "${state_backup}" \
      || { reconcile__log ERROR "${source_name}" "state backup tempfile failed — aborting"; return 1; }
    reconcile__log INFO "${source_name}" "state backup: ${state_backup}"
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
  if ! destination_id="$(reconcile_resolve_destination_id "${source_name}")"; then
    return 1
  fi
  # Airbyte connection re-created with scheduleType=manual; Argo
  # CronWorkflow is the sole scheduler (see bootstrap branch of
  # reconcile_connections for rationale).
  local schedule_json='{"scheduleType":"manual"}'
  local tag_names_json tags_json
  # cfg-hash truncated to 12 hex (Airbyte tag-name max is 30; 'cfg-hash:'+12 = 21).
  tag_names_json="$(python3 -c 'import sys, json; print(json.dumps(["insight", f"cfg-hash:{sys.argv[1][:12]}"]))' "${cfg_hash}")"
  # Airbyte v1 schemas require Tag objects on connection create/patch.
  tags_json="$(ab_resolve_tags "${workspace_id}" "${tag_names_json}")"
  local new_conn_json new_connection_id
  local namespace_format="bronze_${connector_name}"
  # RECONCILE_DRY_RUN guarded at top of reconcile_recreate_with_state.
  new_conn_json="$(ab_create_connection "${workspace_id}" "${new_source_id}" \
                    "${destination_id}" "${source_name}-conn" "${schedule_json}" \
                    "${tags_json}" "" "${namespace_format}")"
  new_connection_id="$(printf '%s' "${new_conn_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("connectionId",""))')"
  # @cpt-end:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-create

  # @cpt-begin:cpt-insightspec-algo-reconcile-export-import-state-on-recreate:p1:inst-eisor-import
  if [[ -n "${state_json}" && -n "${new_connection_id}" ]]; then
    # RECONCILE_DRY_RUN guarded at top of reconcile_recreate_with_state.
    # state restore failure on a fresh recreate means the new connection
    # will resync from cursor zero; surface the error so an operator can
    # re-import from the state_backup tempfile instead of silently
    # losing cursors.
    if ! ab_create_or_update_state "${new_connection_id}" "${state_json}" >/dev/null; then
      reconcile__log ERROR "${source_name}" \
        "state restore failed for new connection ${new_connection_id} — recover from ${state_backup}"
      return 1
    fi
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
    "recreated: new source ${new_source_id}, new connection ${new_connection_id} (state preserved)"
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
    reconcile__log INFO "gc" "skipped (--no-gc set)"
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
        "would garbage-collect orphan connection ${conn_id} and source ${src_id}"
    else
      # connection deletes cascade in newer Airbyte but we delete source
      # explicitly to be safe (Airbyte private API).
      ab_delete_source "${src_id}" >/dev/null
      reconcile__log CHANGE "${conn_name}" \
        "garbage-collected orphan connection ${conn_id} and source ${src_id}"
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
# _reconcile_one_connector <name> <connector_dir> <version> <type> <cdk_image> \
#                          <enrich_image> <dbt_select> <opt_dry_run> \
#                          <opt_no_sync_trigger> <opt_connector>
# Per-connector body extracted from the main loop so a single connector's
# failure can't kill the whole reconcile run. We deliberately do NOT enable
# `set -e` here — failures bubble up through explicit `if ! ...; then`
# branches and are reported via return codes.
# Returns 0 on success, non-zero on any per-layer failure.
# ---------------------------------------------------------------------------
_reconcile_one_connector() {
  local name="$1" connector_dir="$2" version="$3" type="$4" cdk_image="$5" enrich_image="$6" dbt_select="$7"
  local opt_dry_run="$8" opt_no_sync_trigger="$9" opt_connector="${10}"
  set +e  # explicit per-call error handling below

  if [[ -n "${opt_connector}" && "${name}" != "${opt_connector}" ]]; then
    _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
    return 0
  fi

  # Missing Secret -> cascade-delete chain (per ADR-0007 / KEY DECISION #7).
  # Distinguish exit codes: 0=missing, 1=exists, 2=transient API failure.
  # Only act on 0 — treating 2 as "missing" would cascade-delete prod
  # sources on a flaky kubectl/RBAC blip.
  local _missing_rc
  valsec_secret_missing_p "${name}"
  _missing_rc=$?
  case ${_missing_rc} in
    0)
      if ! reconcile_cascade_delete "${name}"; then
        return 1
      fi
      _RECONCILE_CHANGED=$((_RECONCILE_CHANGED + 1))
      return 0
      ;;
    2)
      log_line WARN "${name}: secret lookup failed (transient API error) — skipping this run, will retry next tick"
      _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
      return 0
      ;;
  esac
  # rc=1 → secret exists, fall through to layer reconciliation.

  # Invalid Secret -> WARN + skip (per ADR-0007 / KEY DECISION #7).
  local missing_field=""
  if ! missing_field="$(valsec_check_secret "${name}" "${INSIGHT_NAMESPACE}" "${connector_dir}" 2>/dev/null)"; then
    log_line WARN "${name}: required field \"${missing_field:-unknown}\" missing in Secret — skipping"
    _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
    return 0
  fi

  local secret_name
  if ! secret_name="$(disc_match_descriptor_to_secret "${name}")"; then
    reconcile__log WARN "${name}" "no Secret found in Kubernetes for this connector — skipping"
    _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
    return 0
  fi

  local cfg_hash secret_data_json
  cfg_hash="$(disc_compute_cfg_hash "${secret_name}")"
  secret_data_json="$(kubectl -n "${INSIGHT_NAMESPACE}" get secret "${secret_name}" \
    -o json 2>/dev/null \
    | python3 "${_RECONCILE_PY_DIR}/extract_secret_data.py")"

  local data_changed=0
  local rc=0

  # Layer 1 — definition
  local def_result def_id def_action def_bump_kind
  if ! def_result="$(reconcile_definitions "${name}" "${version}" "${type}" "${connector_dir}" "${cdk_image}")"; then
    log_line ERROR "${name}: failed to reconcile connector definition"
    return 1
  fi
  # TSV `action\tbump_kind\tdef_id` per ADR-0015. `awk -F'\t'` (not `cut`)
  # so a missing-tab line collapses to empty in the trailing fields rather
  # than mirroring field 1 into 2 and 3 (defeats the empty-def_id guard
  # below).
  def_action="$(printf '%s' "${def_result}" | tail -1 | awk -F'\t' '{print $1}')"
  def_bump_kind="$(printf '%s' "${def_result}" | tail -1 | awk -F'\t' '{print $2}')"
  def_id="$(printf '%s' "${def_result}" | tail -1 | awk -F'\t' '{print $3}')"
  [[ -n "${def_bump_kind}" ]] || def_bump_kind="none"
  if [[ -z "${def_id}" ]]; then
    reconcile__log WARN "${name}" "definition not ready — skipping source and connection setup"
    _RECONCILE_SKIPPED=$((_RECONCILE_SKIPPED + 1))
    return 0
  fi
  [[ "${def_action}" == "republish" ]] && data_changed=1

  # Layer 2 — source
  local tenant_id="${INSIGHT_TENANT_ID:-}"
  local source_id_label
  source_id_label="$(kubectl -n "${INSIGHT_NAMESPACE}" get secret "${secret_name}" \
    -o jsonpath='{.metadata.annotations.insight\.cyberfabric\.com/source-id}' 2>/dev/null || true)"
  [[ -n "${source_id_label}" ]] || source_id_label="main"
  local expected_source_name="${name}-${source_id_label}-${tenant_id}"

  # Inject Insight platform identity fields into the source config the
  # K8s Secret only carries connector-specific credentials; the manifest
  # spec also requires `insight_tenant_id` (from reconcile's tenant
  # config) and `insight_source_id` (from the secret's
  # `insight.cyberfabric.com/source-id` annotation). We add them here so
  # the operator never has to duplicate identity into the secret payload.
  local source_cfg_json
  source_cfg_json="$(INSIGHT_TENANT_ID_VAL="${tenant_id}" \
                     INSIGHT_SOURCE_ID_VAL="${source_id_label}" \
    python3 -c '
import sys, os, json
d = json.loads(sys.stdin.read() or "{}") or {}
d["insight_tenant_id"] = os.environ["INSIGHT_TENANT_ID_VAL"]
d["insight_source_id"] = os.environ["INSIGHT_SOURCE_ID_VAL"]
print(json.dumps(d))
' <<<"${secret_data_json}")"

  local src_result src_id src_action
  if ! src_result="$(reconcile_sources "${name}" "${source_cfg_json}" "${cfg_hash}" \
                "${def_id}" "${expected_source_name}")"; then
    log_line ERROR "${name}: failed to reconcile source"
    return 1
  fi
  # `awk -F'\t'` (not `cut -f2`) for the same reason as the def_* parsing
  # above — a single-field fallback line (e.g. `fail\n` from a missing
  # TSV row) would otherwise propagate as both src_action AND src_id and
  # bypass the emptiness guard below.
  src_action="$(printf '%s' "${src_result}" | tail -1 | awk -F'\t' '{print $1}')"
  src_id="$(printf '%s' "${src_result}" | tail -1 | awk -F'\t' '{print $2}')"
  if [[ -z "${src_id}" ]]; then
    reconcile__log WARN "${name}" "source not yet created (will be on real run) — skipping connection setup"
    return 0
  fi
  # Source create/update/recreate is data-affecting per ADR-0008.
  [[ "${src_action}" != "noop" ]] && data_changed=1

  # Layer 3 — connection tags. Two outcomes are data-affecting and trigger
  # a sync afterwards:
  #   1. `created` — first-time bootstrap of the connection.
  #   2. `patch_tags` — cfg-hash drift detected, i.e. the K8s Secret rotated.
  #      Per ADR-0008 the layer is "tag-only" in the sense that we do not
  #      recreate the connection, but a credential rotation is still a
  #      genuine reason to re-sync (the new credentials may scope to a
  #      different account / dataset).
  local conn_result conn_action conn_id
  if ! conn_result="$(reconcile_connections "${name}" "${src_id}" "${cfg_hash}")"; then
    log_line ERROR "${name}: failed to reconcile connection"
    _RECONCILE_FAILED=$((_RECONCILE_FAILED + 1))
    return 1
  fi
  conn_action="$(printf '%s' "${conn_result}" | tail -1 | awk -F'\t' '{print $1}')"
  conn_id="$(printf '%s' "${conn_result}" | tail -1 | awk -F'\t' '{print $2}')"
  case "${conn_action}" in
    created)
      data_changed=1
      ;;
    patch_tags)
      # cfg-hash drift = K8s Secret rotated. classify_change cannot tell
      # because Airbyte returns secrets masked (`********`) on /sources/list,
      # so the source diff was a false-noop. The cfg-hash tag is the
      # canonical rotation signal; push the new config now so the sync we
      # are about to trigger uses fresh credentials.
      if ab_update_source "${src_id}" "${source_cfg_json}" \
            "${expected_source_name}" >/dev/null; then
        reconcile__log INFO "${name}" \
          "rotated source ${src_id} credentials (cfg-hash drift)"
      else
        reconcile__log ERROR "${name}" \
          "ab_update_source failed during rotation for ${src_id}"
        rc=1
      fi
      data_changed=1
      ;;
  esac

  # Per ADR-0015: every version bump that resulted in a republish (any
  # bump_kind != none) refreshes the connection's sync_catalog so new
  # streams and fields advertised by the connector land in bronze on the
  # next sync. Bootstrap path (conn_action == created) already discovered
  # the catalog as part of ab_create_connection, so skip there.
  if [[ "${def_action}" == "republish" && "${conn_action}" != "created" ]]; then
    if ! reconcile_refresh_catalog "${name}" "${src_id}" "${conn_id}"; then
      rc=1
    fi
  fi

  # CronWorkflow apply (idempotent — kubectl apply no-op when YAML unchanged).
  local conn_name schedule tenant
  conn_name="$(reconcile_compute_connection_name "${name}")"
  schedule="$(reconcile_compute_schedule "${name}")"
  tenant="$(reconcile_compute_tenant "${name}")"
  if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
    log_line INFO "${name}: would create/update Argo CronWorkflow"
  elif ! argo_apply_cronworkflow "${name}" "${conn_name}" "${schedule}" "${tenant}" \
                                  "${source_id_label}" "${dbt_select}" \
                                  "${enrich_image}" >/dev/null 2>&1; then
    log_line ERROR "${name}: failed to create/update Argo CronWorkflow"
    rc=1
  fi

  # Sync-trigger only on data-affecting changes (per ADR-0008 / KEY DECISION #2).
  # Per ADR-0015: bump_kind=major dispatches a one-shot dbt --full-refresh
  # on the auto-triggered sync only. Scoped to this connector's
  # descriptor.dbt_select — no cross-connector cascade.
  if [[ "${data_changed}" -eq 1 && "${opt_no_sync_trigger}" -ne 1 ]]; then
    if [[ "${RECONCILE_DRY_RUN:-0}" -eq 1 ]]; then  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
      if [[ "${def_bump_kind}" == "major" ]]; then
        log_line INFO "${name}: would trigger a one-shot sync with dbt --full-refresh (bump_kind=major)"
      else
        log_line INFO "${name}: would trigger a one-shot sync (bump_kind=${def_bump_kind})"
      fi
    elif argo_submit_sync_trigger "${name}" "${conn_name}" "${tenant}" \
                                   "${source_id_label}" "${dbt_select}" \
                                   "${enrich_image}" "${def_bump_kind}" >/dev/null 2>&1; then
      if [[ "${def_bump_kind}" == "major" ]]; then
        log_line INFO "${name}: triggered a one-shot sync with dbt --full-refresh (bump_kind=major)"
      else
        log_line INFO "${name}: triggered a one-shot sync"
      fi
    else
      log_line ERROR "${name}: failed to trigger sync"
      rc=1
    fi
  fi
  # silence unused-arg shellcheck warning
  : "${opt_dry_run}"
  return "${rc}"
}

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

  while IFS=$'\t' read -r name connector_dir version type cdk_image enrich_image dbt_select; do
    [[ -n "${name}" ]] || continue
    if ! _reconcile_one_connector "${name}" "${connector_dir}" "${version}" "${type}" "${cdk_image}" "${enrich_image}" "${dbt_select}" \
         "${opt_dry_run}" "${opt_no_sync_trigger}" "${opt_connector}"; then
      log_line ERROR "${name}: reconcile failed (continuing with next)"
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
