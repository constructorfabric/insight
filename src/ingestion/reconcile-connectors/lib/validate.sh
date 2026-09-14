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
  # The Secret is named by the caller, which knows WHICH instance it is
  # validating. Resolving it here would validate whichever one the API listed
  # first and pass a connector whose other instance is missing a field.
  local secret_name="${4:-}"
  : "${secret_name:?valsec_check_secret: secret_name (arg 4) is required — pass the one from the instance plan}"
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
