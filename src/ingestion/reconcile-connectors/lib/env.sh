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

env_log_target_resolve() {
  if env_in_cluster_p; then
    echo "/var/log/insight/reconcile-$(date -u +%Y-%m-%d).log"
  else
    local base="${XDG_STATE_HOME:-$HOME/.local/state}/insight"  # RULE-DEFAULTS-OK: XDG Base Directory Spec defines exactly this fallback
    mkdir -p "$base"
    echo "$base/reconcile-$(date -u +%Y-%m-%d).log"
  fi
}
