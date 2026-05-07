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
  printf 'WARN [adopt] connector=%s reason=%s\n' \
    "${connector_name}" "${reason}" >&2
  _ADOPT_WARNINGS=$((_ADOPT_WARNINGS + 1))
}

# ---------------------------------------------------------------------------
# adopt_match_definition <definition_id> <version> <type> \
#                        <connector_name> <connector_dir>
# Idempotent: write descriptor.version to the right field per type. The
# underlying ab_* helpers are themselves idempotent at the API level
# (Airbyte returns 200 with no change when the value already matches).
# nocode arm uses the builder-aware update_active_manifest path (ADR-0010).
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
  case "${type}" in
    cdk)
      ab_set_definition_image_tag "${definition_id}" "${version}" >/dev/null
      ;;
    nocode|*)
      local builder_id manifest_path
      builder_id="$(ab_builder_find_by_definition "${INSIGHT_AIRBYTE_WORKSPACE_ID}" "${definition_id}")"
      if [[ -z "${builder_id}" ]]; then
        adopt_warn_orphan "${connector_name}" \
          "ORPHAN definition ${definition_id} (no builder project) — skipping version sync"
        return 0
      fi
      manifest_path="${CONNECTORS_DIR}/${connector_dir}/connector.yaml"
      if [[ ! -f "${manifest_path}" ]]; then
        adopt_warn_orphan "${connector_name}" \
          "type=nocode but connector.yaml missing at ${manifest_path}"
        return 0
      fi
      ab_builder_update_active_manifest "${INSIGHT_AIRBYTE_WORKSPACE_ID}" \
        "${builder_id}" "${version}" "${manifest_path}" >/dev/null
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
  local tags_json
  tags_json=$(python3 -c '
import sys, json
existing = json.loads(sys.argv[1] or "[]")
cfg_hash = sys.argv[2]
keep = []
for t in existing:
    name = t.get("name", t) if isinstance(t, dict) else t
    if name == "insight" or (isinstance(name, str) and name.startswith("cfg-hash:")):
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
# _adopt_one_connector <name> <connector_dir> <version> <type> \
#                      <dry_run> <opt_connector> <workspace_id> \
#                      <definitions_json> <sources_json> <connections_json>
# Per-connector adopt body extracted so a single connector failure can't
# kill the whole adopt run. `set +e` enforced; failures bubble through
# explicit `if ! ...` branches and the function's return code.
# ---------------------------------------------------------------------------
_adopt_one_connector() {
  local name="$1" connector_dir="$2" version="$3" type="$4"
  local dry_run="$5" opt_connector="$6" workspace_id="$7"
  local definitions_json="$8" sources_json="$9" connections_json="${10}"
  set +e

  if [[ -n "${opt_connector}" && "${name}" != "${opt_connector}" ]]; then
    _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
    return 0
  fi

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-match
  local secret_name
  if ! secret_name="$(disc_match_descriptor_to_secret "${name}")"; then
    # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-skip
    adopt_warn_orphan "${name}" "no labelled secret found in K8s"
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
    adopt_warn_orphan "${name}" "no matching source_definition in Airbyte"
    _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
    return 0
  fi

  while IFS= read -r definition_id; do
    [[ -n "${definition_id}" ]] || continue
    if [[ "${dry_run}" -eq 1 ]]; then
      printf 'would_call adopt_match_definition %s version=%s type=%s connector=%s\n' \
        "${definition_id}" "${version}" "${type}" "${name}"
    else
      if ! adopt_match_definition "${definition_id}" "${version}" "${type}" \
            "${name}" "${connector_dir}"; then
        log_line ERROR "adopt: adopt_match_definition failed for ${name} def=${definition_id}"
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
    adopt_warn_orphan "${name}" "no connection found for any of ${def_count} matching definition(s)"
    _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
    return 0
  fi

  while IFS= read -r conn_line; do
    [[ -n "${conn_line}" ]] || continue
    local connection_id existing_tags_json
    connection_id="$(printf '%s' "${conn_line}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["connectionId"])')"
    existing_tags_json="$(printf '%s' "${conn_line}" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin).get("tags",[])))')"
    if [[ "${dry_run}" -eq 1 ]]; then
      printf 'would_call adopt_tag_connection %s cfg_hash=%s\n' \
        "${connection_id}" "${cfg_hash}"
    else
      if ! adopt_tag_connection "${connection_id}" "${cfg_hash}" "${existing_tags_json}"; then
        log_line ERROR "adopt: adopt_tag_connection failed for ${name} conn=${connection_id}"
        return 1
      fi
    fi
    _ADOPT_ADOPTED=$((_ADOPT_ADOPTED + 1))
  done <<<"${matching_connections}"

  # Apply (or update) the per-connector Argo CronWorkflow.
  if [[ "${dry_run}" -eq 1 ]]; then
    printf 'would_call argo_apply_cronworkflow %s\n' "${name}"
  else
    local conn_name; conn_name="$(reconcile_compute_connection_name "${name}")"
    local schedule;  schedule="$(reconcile_compute_schedule "${name}")"
    local tenant;    tenant="$(reconcile_compute_tenant "${name}")"
    # ADOPT_DRY_RUN guarded above (would_call branch).
    if argo_apply_cronworkflow "${name}" "${conn_name}" "${schedule}" "${tenant}" >/dev/null 2>&1; then
      log_line INFO "first-adopt: created CronWorkflow ${name}-${tenant}-sync"
    else
      # ADOPT_DRY_RUN guarded above (would_call branch).
      log_line ERROR "first-adopt: argo_apply_cronworkflow failed for ${name}"
      return 1
    fi
  fi
  # silence unused-arg shellcheck warnings
  : "${workspace_id}" "${connector_dir}"
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-if-matched
  return 0
}

adopt_run() {
  : "${INSIGHT_AIRBYTE_WORKSPACE_ID:?INSIGHT_AIRBYTE_WORKSPACE_ID must be set (the Airbyte workspace UUID where Insight connectors are managed; see chart values ingestion.reconcile.airbyteWorkspaceId)}"
  _ADOPT_ADOPTED=0
  _ADOPT_SKIPPED=0
  _ADOPT_WARNINGS=0
  _ADOPT_FAILED=0
  local dry_run="${1:-${ADOPT_DRY_RUN:-0}}"  # RULE-DEFAULTS-OK: feature flag — OFF when caller doesn't opt in
  local opt_connector="${2:-}"

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-resolve-env
  local workspace_id="${INSIGHT_AIRBYTE_WORKSPACE_ID}"
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
  while IFS=$'\t' read -r name connector_dir version type; do
    [[ -n "${name}" ]] || continue
    if ! _adopt_one_connector "${name}" "${connector_dir}" "${version}" "${type}" \
         "${dry_run}" "${opt_connector}" "${workspace_id}" \
         "${definitions_json}" "${sources_json}" "${connections_json}"; then
      log_line ERROR "adopt: connector ${name} failed (continuing with next)"
      _ADOPT_FAILED=$((_ADOPT_FAILED + 1))
    fi
  done <<<"${descriptors_tsv}"
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-loop

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-return
  printf 'adopt summary: adopted=%d skipped=%d warnings=%d failed=%d (dry_run=%d) — connector_dir scanned\n' \
    "${_ADOPT_ADOPTED}" "${_ADOPT_SKIPPED}" "${_ADOPT_WARNINGS}" "${_ADOPT_FAILED}" "${dry_run}"
  : "${connector_dir:=}"  # silence unused-warning when no descriptors found
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-return
}
