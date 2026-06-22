#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# lib/cdk-build.sh — Build a CDK connector Docker image and register/update
# its Airbyte source definition.
#
# Replaces state.sh lookups with ab_* API calls.
# Uses lib/log.sh for progress lines; quiet on no-op.
#
# As a library: source it and call cdk_build <connector_path> [--push]
# As a script:  ./lib/cdk-build.sh <connector_path> [--push]
#
# Env:
#   CLUSTER_NAME     Kind cluster name for local-dev load (default: insight)
#
# Image reference:
#   The full Docker image reference is read verbatim from
#   descriptor.images.cdk.image per ADR-0016
#   (e.g. `ghcr.io/constructorfabric/source-foo-insight:v1.2.3` for push, or
#   `source-foo-insight:dev` for local-only). No registry derivation.
#
# Function naming: cdk_*
# ---------------------------------------------------------------------------

# NOTE: when sourced, do NOT enable `set -euo pipefail` (leaks into caller's
# shell). Strict mode is enabled below in the self-run entry-point guard.

_CDK_LIB_DIR="$( cd "$(dirname "${BASH_SOURCE[0]}")" && pwd )"
_CDK_PY_DIR="$( cd "${_CDK_LIB_DIR}/../python" && pwd )"

# shellcheck source=./env.sh
source "${_CDK_LIB_DIR}/env.sh"
# shellcheck source=./log.sh
source "${_CDK_LIB_DIR}/log.sh"
# shellcheck source=./airbyte.sh
source "${_CDK_LIB_DIR}/airbyte.sh"

# ---------------------------------------------------------------------------
# cdk_find_definition_id <workspace_id> <connector_name>
# Searches the workspace's source definitions for one whose name matches
# <connector_name>. Prints the sourceDefinitionId; empty string if not found.
# ---------------------------------------------------------------------------
cdk_find_definition_id() {
  local workspace_id="$1"
  local connector_name="$2"
  ab_list_definitions "${workspace_id}" | python3 -c '
import sys, json
target = sys.argv[1]
for d in json.load(sys.stdin):
    # custom is True: Insight namespace separation per ADR-0009.
    if d.get("name") == target and d.get("custom") is True:
        print(d.get("sourceDefinitionId", "")); break
' "${connector_name}"
}

# ---------------------------------------------------------------------------
# cdk_create_definition <workspace_id> <connector_name> <docker_repo> <image_tag>
# POSTs to /api/v1/source_definitions/create_custom via ab__curl.
# Prints the new sourceDefinitionId on stdout.
# ---------------------------------------------------------------------------
cdk_create_definition() {
  local workspace_id="$1"
  local connector_name="$2"
  local docker_repo="$3"
  local image_tag="$4"
  local body def_id
  body="$(python3 "${_CDK_PY_DIR}/create_cdk_definition_payload.py" \
    "${workspace_id}" "${connector_name}" "${docker_repo}" "${image_tag}")"
  def_id="$(ab__curl POST /api/v1/source_definitions/create_custom "${body}" \
    | python3 -c 'import sys,json;print(json.load(sys.stdin).get("sourceDefinitionId",""))')"
  if [[ -z "${def_id}" ]]; then
    printf 'cdk_create_definition: API returned no sourceDefinitionId for %s\n' \
      "${connector_name}" >&2
    return 1
  fi
  printf '%s' "${def_id}"
}

# ---------------------------------------------------------------------------
# cdk_register_definition <connector_name> <docker_repo> <image_tag>
# Idempotent: updates the image tag on an existing definition or creates a
# new custom definition. Prints the final sourceDefinitionId.
# ---------------------------------------------------------------------------
cdk_register_definition() {
  local connector_name="$1"
  local docker_repo="$2"
  local image_tag="$3"
  local workspace_id existing_def_id def_id
  workspace_id="$(ab_workspace_id)"
  existing_def_id="$(cdk_find_definition_id "${workspace_id}" "${connector_name}" || true)"

  if [[ -n "${existing_def_id}" ]]; then
    ab_set_definition_image_tag "${existing_def_id}" "${image_tag}" >/dev/null
    log_line INFO "CDK definition updated: ${connector_name} → ${existing_def_id} (tag=${image_tag})"
    printf '%s' "${existing_def_id}"
    return 0
  fi

  def_id="$(cdk_create_definition "${workspace_id}" "${connector_name}" "${docker_repo}" "${image_tag}")"
  log_line INFO "CDK definition created: ${connector_name} → ${def_id} (tag=${image_tag})"
  printf '%s' "${def_id}"
}

# ---------------------------------------------------------------------------
# cdk_build <connector_path> [--push]
# Full CDK build: Docker build → push/Kind-load → Airbyte definition register.
# connector_path is relative to the project root (e.g. git/github).
# ---------------------------------------------------------------------------
cdk_build() {
  local connector="${1:?cdk_build requires connector_path (e.g. git/github)}"
  local push=0
  local arg
  for arg in "${@:2}"; do
    case "${arg}" in
      --push) push=1 ;;
    esac
  done

  local connector_dir="connectors/${connector}"
  local descriptor="${connector_dir}/descriptor.yaml"
  local dockerfile="${connector_dir}/Dockerfile"

  [[ -f "${descriptor}" ]] || {
    printf 'ERROR: no descriptor at %s\n' "${descriptor}" >&2; return 1
  }
  [[ -f "${dockerfile}" ]] || {
    printf 'ERROR: no Dockerfile at %s\n' "${dockerfile}" >&2; return 1
  }

  local connector_name connector_type
  connector_name=$(yq -r '.name' "${descriptor}")
  connector_type=$(yq -r '.type' "${descriptor}")

  if [[ "${connector_type}" != "cdk" ]]; then
    printf 'ERROR: %s is type %s, not cdk. Use reconcile for nocode connectors.\n' \
      "${connector_name}" "${connector_type}" >&2
    return 1
  fi

  local image
  image="$(python3 "${_CDK_PY_DIR}/parse_descriptor.py" \
    --descriptor "${descriptor}" --field images.cdk.image 2>/dev/null || true)"
  if [[ -z "${image}" ]]; then
    printf 'ERROR: descriptor.images.cdk.image is required for type=cdk (got empty) at %s\n' \
      "${descriptor}" >&2
    return 1
  fi
  local docker_repo docker_tag
  IFS=$'\t' read -r docker_repo docker_tag \
    < <(python3 "${_CDK_PY_DIR}/split_docker_image_ref.py" "${image}")

  printf '=== Building CDK connector: %s ===\n' "${connector_name}"
  printf '  Image: %s\n' "${image}"

  printf '  Building Docker image...\n'
  docker build -t "${image}" -f "${dockerfile}" "${connector_dir}"

  local cluster_name="${CLUSTER_NAME:-insight}"  # RULE-DEFAULTS-OK: matches umbrella release name; only used for Kind local-load
  if [[ "${push}" -eq 1 ]]; then
    printf '  Pushing to registry...\n'
    docker push "${image}"
  elif command -v kind &>/dev/null && kind get clusters 2>/dev/null | grep -q "^${cluster_name}$"; then
    printf "  Loading into Kind cluster '%s' (local dev)...\n" "${cluster_name}"
    kind load docker-image "${image}" --name "${cluster_name}"
  fi

  local def_id
  def_id="$(cdk_register_definition "${connector_name}" "${docker_repo}" "${docker_tag}")"

  printf '\n=== Done: %s ===\n' "${connector_name}"
  printf '  Image:      %s\n' "${image}"
  printf '  Definition: %s\n' "${def_id:-unknown}"
  printf '\n  Next: run reconcile-connectors to wire the source and connection.\n'
}

# ---------------------------------------------------------------------------
# Self-run entry point — preserves original CLI surface.
# ---------------------------------------------------------------------------
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  set -euo pipefail
  # Resolve the ingestion root (connectors/ lives there): lib/ is two levels
  # below src/ingestion/.
  cd "${_CDK_LIB_DIR}/../.."
  cdk_build "$@"
fi
