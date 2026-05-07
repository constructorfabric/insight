#!/usr/bin/env bash
# valsec_* — connector secret validation (sourceable; NO top-level CLI)
# NOTE: this file is sourced; no top-level `set -euo pipefail`.

: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"
: "${CONNECTORS_DIR:?CONNECTORS_DIR must be set, typically src/ingestion/connectors}"

VALSEC_SCRIPT_DIR="$( cd "$(dirname "${BASH_SOURCE[0]}")" && pwd )"
VALSEC_PY_DIR="$( cd "${VALSEC_SCRIPT_DIR}/../python" && pwd )"

# shellcheck source=./discover.sh
source "${VALSEC_SCRIPT_DIR}/discover.sh"

# valsec_check_secret <connector_name> [namespace] [connector_dir]
# Returns 0 if Secret valid, 2 if invalid (prints first missing field on stdout).
# Per ADR-0007: lookup K8s Secret by annotation insight.cyberfabric.com/connector
# (real name pattern is insight-${connector}-${source_id}). Direct
# `kubectl get secret ${connector_slug}` is forbidden.
valsec_check_secret() {
  local connector="$1"
  local namespace="${2:-${INSIGHT_NAMESPACE}}"
  local connector_dir="${3:-${connector}}"
  local secret_name
  if ! secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null)"; then
    return 2
  fi
  if [[ -z "${secret_name}" ]]; then
    return 2
  fi
  local stringdata_file
  stringdata_file="$(mktemp -t insight-reconcile.XXXXXX)"
  trap "rm -f '${stringdata_file}'" RETURN
  kubectl -n "${namespace}" get secret "${secret_name}" -o json \
    | python3 "${VALSEC_PY_DIR}/extract_secret_data.py" \
    > "${stringdata_file}"
  # `connector_dir` is already a full path emitted by disc_load_descriptors
  # (e.g. "src/ingestion/connectors/collaboration/m365") — do NOT prepend
  # CONNECTORS_DIR or you get a double prefix and the descriptor is missing.
  python3 "${VALSEC_PY_DIR}/validate_secret.py" \
    --descriptor "${connector_dir}/descriptor.yaml" \
    --secret-stringdata "${stringdata_file}"
}

# valsec_secret_missing_p <connector_name> [namespace]
# Returns 0 if Secret entirely missing (cascade-delete trigger), 1 otherwise.
# Per ADR-0007: lookup by annotation insight.cyberfabric.com/connector;
# never by `kubectl get secret ${connector_slug}` directly.
valsec_secret_missing_p() {
  local connector="$1"
  local namespace="${2:-${INSIGHT_NAMESPACE}}"
  local secret_name
  if ! secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null)"; then
    return 0
  fi
  [[ -z "${secret_name}" ]]
}
