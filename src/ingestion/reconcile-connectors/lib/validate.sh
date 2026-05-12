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
  # connector_dir is the FULL path emitted by disc_load_descriptors
  # (e.g. "src/ingestion/connectors/collaboration/m365"); the bare
  # connector slug as a fallback would silently look up
  # `${connector}/descriptor.yaml` relative to the cron pod's cwd and
  # always miss. Require the caller to pass the resolved path.
  local connector_dir="${3:-}"
  : "${connector_dir:?valsec_check_secret: connector_dir (arg 3) is required — pass the full path from disc_load_descriptors}"
  local secret_name
  if ! secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null)"; then
    return 2
  fi
  if [[ -z "${secret_name}" ]]; then
    return 2
  fi
  local stringdata_file rc
  stringdata_file="$(mktemp -t insight-reconcile.XXXXXX)" || return 2
  kubectl -n "${namespace}" get secret "${secret_name}" -o json \
    | python3 "${VALSEC_PY_DIR}/extract_secret_data.py" \
    > "${stringdata_file}"
  # `connector_dir` is already a full path emitted by disc_load_descriptors
  # (e.g. "src/ingestion/connectors/collaboration/m365") — do NOT prepend
  # CONNECTORS_DIR or you get a double prefix and the descriptor is missing.
  python3 "${VALSEC_PY_DIR}/validate_secret.py" \
    --descriptor "${connector_dir}/descriptor.yaml" \
    --secret-stringdata "${stringdata_file}"
  rc=$?
  # Sourced libraries MUST NOT install `trap … RETURN` (it would
  # clobber the caller's traps and fire on every later function return).
  # Clean up explicitly here.
  rm -f "${stringdata_file}"
  return ${rc}
}

# valsec_secret_missing_p <connector_name> [namespace]
# Returns:
#   0  Secret entirely missing → caller may cascade-delete.
#   1  Secret exists.
#   2  kubectl/API failure (transient). Caller MUST NOT treat as
#      "missing" — destructive cascade-delete must skip this iteration.
# Per ADR-0007: lookup by annotation insight.cyberfabric.com/connector;
# never by `kubectl get secret ${connector_slug}` directly.
valsec_secret_missing_p() {
  local connector="$1"
  local namespace="${2:-${INSIGHT_NAMESPACE}}"
  local secret_name rc
  secret_name="$(disc_match_descriptor_to_secret "${connector}" "${namespace}" 2>/dev/null)"
  rc=$?
  case ${rc} in
    0)
      # match returned; consider the Secret missing only if name is empty.
      [[ -z "${secret_name}" ]]
      ;;
    1)
      # genuine "no match" from disc_match — Secret is really missing.
      return 0
      ;;
    *)
      # 2 (kubectl/API transient) or anything unexpected: do NOT report
      # missing; let the caller skip the cascade-delete branch.
      return 2
      ;;
  esac
}
