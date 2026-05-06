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

set -euo pipefail

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
# adopt_match_definition <definition_id> <version> <type>
# Idempotent: write descriptor.version to the right field per type. The
# underlying ab_* helpers are themselves idempotent at the API level
# (Airbyte returns 200 with no change when the value already matches).
# ---------------------------------------------------------------------------
adopt_match_definition() {
  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-anno-def
  local definition_id="$1"
  local version="$2"
  local type="$3"
  case "${type}" in
    cdk)
      ab_set_definition_image_tag "${definition_id}" "${version}" >/dev/null
      ;;
    nocode|*)
      ab_set_definition_description "${definition_id}" "${version}" >/dev/null
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
adopt_run() {
  _ADOPT_ADOPTED=0
  _ADOPT_SKIPPED=0
  _ADOPT_WARNINGS=0
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
  while IFS=$'\t' read -r name connector_dir version type; do
    [[ -n "${name}" ]] || continue
    if [[ -n "${opt_connector}" && "${name}" != "${opt_connector}" ]]; then
      _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
      continue
    fi

    # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-match
    local secret_name
    if ! secret_name="$(disc_match_descriptor_to_secret "${name}")"; then
      # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-skip
      adopt_warn_orphan "${name}" "no labelled secret found in K8s"
      _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
      # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-skip
      continue
    fi
    # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-match

    local cfg_hash
    cfg_hash="$(disc_compute_cfg_hash "${secret_name}")"

    # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-if-matched
    # Find ALL definitions with this name. Legacy clusters often have
    # duplicates (multiple publish events left orphaned definitions
    # alongside the active one); existing sources may reference any of
    # them. Annotate every duplicate so the active one always carries
    # the correct version, and search sources across the full set.
    local definition_ids_json
    definition_ids_json="$(printf '%s' "${definitions_json}" \
      | python3 "${_ADOPT_LIB_DIR}/../python/extract_definition_ids.py" "${name}")"
    local def_count
    def_count="$(printf '%s' "${definition_ids_json}" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')"
    if [[ "${def_count}" -eq 0 ]]; then
      adopt_warn_orphan "${name}" "no matching source_definition in Airbyte"
      _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
      continue
    fi

    while IFS= read -r definition_id; do
      [[ -n "${definition_id}" ]] || continue
      if [[ "${dry_run}" -eq 1 ]]; then
        printf 'would_call adopt_match_definition %s version=%s type=%s\n' \
          "${definition_id}" "${version}" "${type}"
      else
        adopt_match_definition "${definition_id}" "${version}" "${type}"
      fi
    done < <(printf '%s' "${definition_ids_json}" \
      | python3 -c 'import sys,json
for x in json.load(sys.stdin): print(x)')

    # Find connections whose source.sourceDefinitionId is in the set.
    local matching_connections
    matching_connections="$(python3 \
      "${_ADOPT_LIB_DIR}/../python/match_connections_to_definitions.py" \
      "${sources_json}" "${connections_json}" "${definition_ids_json}")"
    if [[ -z "${matching_connections}" ]]; then
      adopt_warn_orphan "${name}" "no connection found for any of ${def_count} matching definition(s)"
      _ADOPT_SKIPPED=$((_ADOPT_SKIPPED + 1))
      continue
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
        adopt_tag_connection "${connection_id}" "${cfg_hash}" "${existing_tags_json}"
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
      if argo_apply_cronworkflow "${name}" "${conn_name}" "${schedule}" "${tenant}" >/dev/null 2>&1; then
        log_line INFO "first-adopt: created CronWorkflow ${name}-${tenant}-sync"
      else
        log_line ERROR "first-adopt: argo_apply_cronworkflow failed for ${name}"
      fi
    fi
    # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-if-matched
  done <<<"${descriptors_tsv}"
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-loop

  # @cpt-begin:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-return
  printf 'adopt summary: adopted=%d skipped=%d warnings=%d (dry_run=%d) — connector_dir scanned\n' \
    "${_ADOPT_ADOPTED}" "${_ADOPT_SKIPPED}" "${_ADOPT_WARNINGS}" "${dry_run}"
  : "${connector_dir:=}"  # silence unused-warning when no descriptors found
  # @cpt-end:cpt-insightspec-flow-reconcile-run-adopt-v2:p1:inst-ad-return
}
