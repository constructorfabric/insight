#!/usr/bin/env bash
# env_* — environment variable resolution helpers (sourceable; NO top-level CLI)
# NOTE: this file is sourced; no top-level `set -euo pipefail`.

env_load() {
  : "${AIRBYTE_URL:?must be set}"
  : "${INSIGHT_NAMESPACE:=insight}"
  : "${INSIGHT_RECONCILE_TOKEN_TTL:=600}"
  export AIRBYTE_URL INSIGHT_NAMESPACE INSIGHT_RECONCILE_TOKEN_TTL
}

env_in_cluster_p() {
  test -f /var/run/secrets/kubernetes.io/serviceaccount/token
}

# env_log_target_resolve was removed together with the PVC file logger —
# lib/log.sh now emits JSON to stdout (the log collector is the durable
# destination), so there is no file target to resolve.
