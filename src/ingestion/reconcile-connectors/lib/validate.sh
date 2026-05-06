#!/usr/bin/env bash
# valsec_* — connector secret validation (sourceable; NO top-level CLI)
# NOTE: this file is sourced; no top-level `set -euo pipefail`.

: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"

VALSEC_SCRIPT_DIR="$( cd "$(dirname "${BASH_SOURCE[0]}")" && pwd )"
VALSEC_PY_DIR="$( cd "${VALSEC_SCRIPT_DIR}/../python" && pwd )"

# Returns 0 if Secret valid, 2 if invalid (prints first missing field on stdout).
# Convention: Secret name == connector slug (per ADR-0007).
valsec_check_secret() {
  local connector="$1"
  local namespace="${2:-${INSIGHT_NAMESPACE}}"
  if ! kubectl -n "${namespace}" get secret "${connector}" >/dev/null 2>&1; then
    return 2   # whole Secret missing — caller should also use valsec_secret_missing_p
  fi
  local stringdata_file
  stringdata_file="$(mktemp -t insight-reconcile.XXXXXX)"
  trap "rm -f '${stringdata_file}'" RETURN
  kubectl -n "${namespace}" get secret "${connector}" -o json \
    | python3 "${VALSEC_PY_DIR}/extract_secret_data.py" \
    > "${stringdata_file}"
  python3 "${VALSEC_PY_DIR}/validate_secret.py" \
    --descriptor "connectors/${connector}/descriptor.yaml" \
    --secret-stringdata "${stringdata_file}"
}

# Returns 0 if Secret entirely missing (cascade-delete trigger), 1 otherwise.
valsec_secret_missing_p() {
  local connector="$1"
  local namespace="${2:-${INSIGHT_NAMESPACE}}"
  ! kubectl -n "${namespace}" get secret "${connector}" >/dev/null 2>&1
}
