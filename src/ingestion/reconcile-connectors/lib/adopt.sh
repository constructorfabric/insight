#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# @cpt:cpt-insightspec-featstatus-reconcile — adoption pass
# @cpt-flow:cpt-insightspec-flow-reconcile-run-adopt-v2:p1
#
# One-shot adoption that aligns existing Airbyte resources with the
# declarative descriptor + K8s Secret model — annotation only, NO creates,
# NO deletes (Decision #7). Matches definitions to descriptor.yaml by
# `name`, sets definition.description (nocode) or dockerImageTag (CDK)
# to descriptor.version, and patches connection.tags to include `insight`
# and `cfg-hash:<sha256>`. Bad/unlabelled Secrets → WARN + skip
# (Decision #8). Sourced — never executed standalone.
#
# Function naming: `adopt_*`; lowercase.
# ---------------------------------------------------------------------------

# NOTE: this file is sourced; no top-level `set -euo pipefail`.

_ADOPT_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=./airbyte.sh
source "${_ADOPT_LIB_DIR}/airbyte.sh"
# shellcheck source=./discover.sh
source "${_ADOPT_LIB_DIR}/discover.sh"
# shellcheck source=./connector-naming.sh
# Provides reconcile_compute_{connection_name,schedule,tenant}. Sourcing
# here (instead of relying on reconcile.sh having loaded it first) is
# what eliminates the previous circular dependency between adopt.sh
# and reconcile.sh.
source "${_ADOPT_LIB_DIR}/connector-naming.sh"
# shellcheck source=./argo.sh
source "${_ADOPT_LIB_DIR}/argo.sh"
# shellcheck source=./log.sh
source "${_ADOPT_LIB_DIR}/log.sh"

# Counters; reset on each adopt_run.
_ADOPT_ADOPTED=0
_ADOPT_SKIPPED=0
_ADOPT_WARNINGS=0
_ADOPT_FAILED=0

# ---------------------------------------------------------------------------
# adopt_warn_orphan <connector_name> <reason>
# Emit a single structured WARN to stderr and bump the warnings counter.
# Used for unmatched secrets / sources / definitions during adoption.
# ---------------------------------------------------------------------------
adopt_warn_orphan() {
  local connector_name="$1"
  local reason="$2"
  printf 'WARN    %s: %s\n' \
    "${connector_name}" "${reason}" >&2
  _ADOPT_WARNINGS=$((_ADOPT_WARNINGS + 1))
}

# ---------------------------------------------------------------------------
# adopt_match_definition <definition_id> <version> <type> \
#                        <connector_name> <connector_dir> [<cdk_image>]
# Idempotent: align the right Airbyte field per type:
#   - cdk: dockerImageTag <- tag/digest portion of descriptor.images.cdk.image
#     (NOT descriptor.version). When the image field is empty -> WARN+skip
#     (image not yet published; CI's first build will stamp it).
#   - nocode: builder.active_manifest.description <- descriptor.version
#     via update_active_manifest (ADR-0010).
# The underlying ab_* helpers are themselves idempotent at the API level
# (Airbyte returns 200 with no change when the value already matches).
# Definitions without a linked builder project (orphans) are WARN+skip;
# operators run tools/migrate-orphan-definition.sh to recover safely.
# ---------------------------------------------------------------------------
adopt_match_definition() {
  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-anno-def
  local definition_id="$1"
  local version="$2"
  local type="$3"
  local connector_name="${4:-?}"
  local connector_dir="${5:-}"
  local cdk_image="${6:-}"
  case "${type}" in
    cdk)
      if [[ -z "${cdk_image}" ]]; then
        adopt_warn_orphan "${connector_name}" \
          "connector is cdk type but no image set in descriptor — skipping until image is published"
        return 0
      fi
      local _adopt_repo _adopt_tag
      IFS=$'\t' read -r _adopt_repo _adopt_tag \
        < <(python3 "${_ADOPT_LIB_DIR}/../python/split_docker_image_ref.py" "${cdk_image}")
      ab_set_definition_image_tag "${definition_id}" "${_adopt_tag}" >/dev/null
      ;;
    nocode|*)
      local builder_id manifest_path workspace_id
      workspace_id="$(ab_workspace_id)"
      builder_id="$(ab_builder_find_by_definition "${workspace_id}" "${definition_id}")"
      if [[ -z "${builder_id}" ]]; then
        adopt_warn_orphan "${connector_name}" \
          "ORPHAN definition ${definition_id} (no builder project) — skipping version sync"
        return 0
      fi
      # connector_dir is already a full path from disc_load_descriptors —
      # do not prepend CONNECTORS_DIR (would double up).
      manifest_path="${connector_dir}/connector.yaml"
      if [[ ! -f "${manifest_path}" ]]; then
        adopt_warn_orphan "${connector_name}" \
          "connector is nocode type but no manifest file at ${manifest_path} — skipping"
        return 0
      fi
      # ab_builder_update_active_manifest takes source_definition_id (not
      # builder_project_id). builder_id was checked above only to gate
      # orphan detection.
      ab_builder_update_active_manifest "${workspace_id}" \
        "${definition_id}" "${version}" "${manifest_path}" >/dev/null
      ;;
  esac
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-anno-def
}

# ---------------------------------------------------------------------------
# adopt_tag_connection <connection_id> <cfg_hash> <existing_tags_json>
# Build the desired tag set as `[insight, cfg-hash:<sha>]` plus any
# pre-existing tags that aren't `insight` or a previous `cfg-hash:*`,
# then PATCH. Idempotent — second run produces an identical tag list.
# ---------------------------------------------------------------------------
adopt_tag_connection() {
  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-anno-conn
  local connection_id="$1"
  local cfg_hash="$2"
  local existing_tags_json="${3:-[]}"
  local tag_names_json
  # cfg-hash truncated to 12 hex (Airbyte tag-name max is 30 chars).
  tag_names_json=$(python3 -c '
import sys, json
existing = json.loads(sys.argv[1] or "[]")
cfg_hash = sys.argv[2][:12]
keep = []
for t in existing:
    # Airbyte tags are dicts with a "name" key; normalize to a bare
    # string and drop anything that does not yield one (e.g. a
    # malformed tag dict without "name") so the set() dedup below
    # never sees an unhashable dict.
    name = t.get("name") if isinstance(t, dict) else t
    if not isinstance(name, str):
        continue
    if name == "insight" or name.startswith("cfg-hash:"):
        continue
    keep.append(name)
keep.extend(["insight", f"cfg-hash:{cfg_hash}"])
# de-dup preserving order
seen = set(); out = []
for n in keep:
    if n not in seen:
        seen.add(n); out.append(n)
print(json.dumps(out))
' "${existing_tags_json}" "${cfg_hash}")
  # Convert string-name array to Tag objects (Airbyte v1 schema requires
  # tagId/workspaceId/name/color even on PATCH).
  local workspace_id tags_json
  workspace_id="$(ab_workspace_id)"
  tags_json="$(ab_resolve_tags "${workspace_id}" "${tag_names_json}")"
  # ADOPT_DRY_RUN guarded by callers (_adopt_one_connector + reconcile_connections).
  ab_patch_connection_tags "${connection_id}" "${tags_json}" >/dev/null
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-anno-conn
}

# ---------------------------------------------------------------------------
# adopt_run [--dry-run]
# Orchestrates the full adoption pass. Idempotent: a second run on a
# fully-adopted set issues zero state-changing API calls (
# cpt-insightspec-dod-reconcile-adoption-idempotent). Each call site is
# guarded by an `if [[ "${ADOPT_DRY_RUN:-0}" -eq 1 ]]` short-circuit so  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
# callers can pre-set the flag.
# ---------------------------------------------------------------------------
# ---------------------------------------------------------------------------
# _adopt_one_connector <name> <connector_dir> <version> <type> <cdk_image> \
#                      <dry_run> <opt_connector> <workspace_id> \
#                      <definitions_json> <sources_json> <connections_json>
# Per-connector adopt body extracted so a single connector failure can't
# kill the whole adopt run. `set +e` enforced; failures bubble through
# explicit `if ! ...` branches and the function's return code.
# ---------------------------------------------------------------------------
_adopt_one_connector() {
  local name="$1" connector_dir="$2" version="$3" type="$4" cdk_image="$5"
  local enrich_image="$6" dbt_select="$7"
  local dry_run="$8" opt_connector="$9" workspace_id="${10}"
  local definitions_json="${11}" sources_json="${12}" connections_json="${13}"
  set +e

  if [[ -n "${opt_connector}" && "${name}" != "${opt_connector}" ]]; then
    _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
    return 0
  fi

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-match
  local secret_name
  if ! secret_name="$(disc_match_descriptor_to_secret "${name}")"; then
    # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-skip
    adopt_warn_orphan "${name}" "no Secret found in Kubernetes for this connector — skipping"
    _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
    # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-skip
    return 0
  fi
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-match

  local cfg_hash
  cfg_hash="$(disc_compute_cfg_hash "${secret_name}")"

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-if-matched
  local definition_ids_json
  definition_ids_json="$(printf '%s' "${definitions_json}" \
    | python3 "${_ADOPT_LIB_DIR}/../python/extract_definition_ids.py" "${name}")"
  local def_count
  def_count="$(printf '%s' "${definition_ids_json}" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')"
  if [[ "${def_count}" -eq 0 ]]; then
    adopt_warn_orphan "${name}" "no matching connector definition found in Airbyte"
    _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
    return 0
  fi

  while IFS= read -r definition_id; do
    [[ -n "${definition_id}" ]] || continue
    if [[ "${dry_run}" -eq 1 ]]; then
      printf 'CHANGE  %s: would update connector definition %s to version %s\n' \
        "${name}" "${definition_id}" "${version}"
      : "${type}" "${cdk_image}"
    else
      if ! adopt_match_definition "${definition_id}" "${version}" "${type}" \
            "${name}" "${connector_dir}" "${cdk_image}"; then
        log_line ERROR "${name}: failed to update connector definition ${definition_id}"
        return 1
      fi
    fi
  done < <(printf '%s' "${definition_ids_json}" \
    | python3 -c 'import sys,json
for x in json.load(sys.stdin): print(x)')

  local matching_connections
  matching_connections="$(python3 \
    "${_ADOPT_LIB_DIR}/../python/match_connections_to_definitions.py" \
    "${sources_json}" "${connections_json}" "${definition_ids_json}")"
  if [[ -z "${matching_connections}" ]]; then
    adopt_warn_orphan "${name}" "no connection found for any of ${def_count} matching definition(s) — skipping"
    _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
    return 0
  fi

  while IFS= read -r conn_line; do
    [[ -n "${conn_line}" ]] || continue
    local connection_id existing_tags_json
    connection_id="$(printf '%s' "${conn_line}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["connectionId"])')"
    existing_tags_json="$(printf '%s' "${conn_line}" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin).get("tags",[])))')"
    if [[ "${dry_run}" -eq 1 ]]; then
      printf 'CHANGE  %s: would tag connection %s as managed (cfg-hash %s)\n' \
        "${name}" "${connection_id}" "${cfg_hash}"
    else
      if ! adopt_tag_connection "${connection_id}" "${cfg_hash}" "${existing_tags_json}"; then
        log_line ERROR "${name}: failed to tag connection ${connection_id}"
        return 1
      fi
    fi
    _ADOPT_ADOPTED=$((_ADOPT_ADOPTED + 1))
  done <<<"${matching_connections}"

  # Apply (or update) the per-connector Argo CronWorkflow.
  if [[ "${dry_run}" -eq 1 ]]; then
    printf 'CHANGE  %s: would create/update Argo CronWorkflow\n' "${name}"
  else
    local conn_name; conn_name="$(reconcile_compute_connection_name "${name}")"
    local schedule;  schedule="$(reconcile_compute_schedule "${name}")"
    local tenant;    tenant="$(reconcile_compute_tenant "${name}")"
    # source_id_label is needed by the rendered CronWorkflow as
    # `insight_source_id` for ingestion-pipeline. Pull from the
    # connector's Secret annotation (set by apply.sh).
    local source_id_label
    source_id_label="$(kubectl -n "${INSIGHT_NAMESPACE}" get secret "${secret_name}" \
      -o jsonpath='{.metadata.annotations.insight\.cyberfabric\.com/source-id}' 2>/dev/null || true)"
    [[ -n "${source_id_label}" ]] || source_id_label="main"
    # ADOPT_DRY_RUN guarded above (would_call branch).
    if argo_apply_cronworkflow "${name}" "${conn_name}" "${schedule}" "${tenant}" \
                                "${source_id_label}" "${dbt_select}" \
                                "${enrich_image}" >/dev/null 2>&1; then
      log_line INFO "${name}: created Argo CronWorkflow ${name}-${tenant}-sync"
    else
      # ADOPT_DRY_RUN guarded above (would_call branch).
      log_line ERROR "${name}: failed to create/update Argo CronWorkflow"
      return 1
    fi
  fi
  # silence unused-arg shellcheck warning (workspace_id and connector_dir
  # are plumbed for symmetry with reconcile.sh; descriptor-derived
  # parameters now flow via dbt_select / enrich_image arguments).
  : "${workspace_id}" "${connector_dir}"
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-if-matched
  return 0
}

adopt_run() {
  _ADOPT_ADOPTED=0
  _ADOPT_SKIPPED=0
  _ADOPT_WARNINGS=0
  _ADOPT_FAILED=0
  local dry_run="${1:-${ADOPT_DRY_RUN:-0}}"  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
  local opt_connector="${2:-}"

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-resolve-env
  local workspace_id
  workspace_id="$(ab_workspace_id)"
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-resolve-env

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-discover
  local descriptors_tsv
  descriptors_tsv="$(disc_load_descriptors)"
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-discover

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-list-actual
  local definitions_json sources_json connections_json
  definitions_json="$(ab_list_definitions "${workspace_id}")"
  sources_json="$(ab_list_sources "${workspace_id}")"
  connections_json="$(ab_list_connections "${workspace_id}")"
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-list-actual

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-loop
  while IFS=$'\t' read -r name connector_dir version type cdk_image enrich_image dbt_select; do
    [[ -n "${name}" ]] || continue
    if ! _adopt_one_connector "${name}" "${connector_dir}" "${version}" "${type}" "${cdk_image}" \
         "${enrich_image}" "${dbt_select}" \
         "${dry_run}" "${opt_connector}" "${workspace_id}" \
         "${definitions_json}" "${sources_json}" "${connections_json}"; then
      log_line ERROR "${name}: adopt failed (continuing with next)"
      _ADOPT_FAILED=$((_ADOPT_FAILED + 1))
    fi
  done <<<"${descriptors_tsv}"
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-loop

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-return
  printf 'adopt finished: %d adopted, %d skipped, %d warning(s), %d failed\n' \
    "${_ADOPT_ADOPTED}" "${_ADOPT_SKIPPED}" "${_ADOPT_WARNINGS}" "${_ADOPT_FAILED}"
  : "${dry_run}"
  : "${connector_dir:=}"  # silence unused-warning when no descriptors found
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-return
}
