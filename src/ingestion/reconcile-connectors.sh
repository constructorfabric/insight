#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# @cpt:cpt-insightspec-feature-reconcile — single declarative entrypoint
# @cpt-flow:cpt-insightspec-flow-reconcile-run-reconcile:p1
# @cpt-flow:cpt-insightspec-flow-reconcile-run-adopt:p1
# @cpt-flow:cpt-insightspec-flow-reconcile-dry-run:p2
#
# Replaces the legacy fan of scripts (connect.sh, register.sh, cleanup.sh,
# sync-state.sh, reset-connector.sh, update-connectors.sh,
# update-connections.sh) with one CLI:
#
#   reconcile-connectors.sh [adopt|reconcile] [--dry-run]
#                           [--connector <name>] [--no-gc]
#
# Default subcommand is `reconcile`. `adopt` performs a one-shot
# annotation pass on existing legacy resources (no creates / deletes).
# ---------------------------------------------------------------------------

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

# shellcheck source=airbyte-toolkit/lib/airbyte.sh
source "${SCRIPT_DIR}/airbyte-toolkit/lib/airbyte.sh"
# shellcheck source=airbyte-toolkit/lib/discover.sh
source "${SCRIPT_DIR}/airbyte-toolkit/lib/discover.sh"
# shellcheck source=airbyte-toolkit/lib/adopt.sh
source "${SCRIPT_DIR}/airbyte-toolkit/lib/adopt.sh"
# shellcheck source=airbyte-toolkit/lib/reconcile.sh
source "${SCRIPT_DIR}/airbyte-toolkit/lib/reconcile.sh"

# ---------------------------------------------------------------------------
# usage — print CLI help and exit. Called for -h/--help and on bad args.
# ---------------------------------------------------------------------------
usage() {
  cat <<'EOF'
Usage: reconcile-connectors.sh [adopt|reconcile] [OPTIONS]

Subcommands:
  reconcile (default)    Apply descriptor-driven reconcile across all connectors
  adopt                  One-shot annotation pass for legacy resources
                         (no creates / deletes)

Options:
  --dry-run              Print diff report without applying changes
  --connector <name>     Limit reconcile to a single connector
  --no-gc                Skip orphan garbage collection (reconcile only)
  -h, --help             Show this usage and exit 0

Environment:
  INSIGHT_TENANT_ID      Override the cluster ConfigMap tenant_id
  AIRBYTE_URL            Airbyte server base URL
                         (e.g., http://airbyte-server.airbyte.svc:8001)
  AIRBYTE_TOKEN_FILE     Path to bearer token file
                         (default: /var/run/secrets/airbyte/token)
EOF
}

# ---------------------------------------------------------------------------
# resolve_tenant_id — resolve insight tenant id with env-wins policy.
# @cpt-begin:cpt-insightspec-flow-reconcile-run-reconcile:p1:inst-rr-resolve-tenant
# (also covers inst-ad-resolve-tenant for the adopt flow)
# ---------------------------------------------------------------------------
resolve_tenant_id() {
  if [[ -n "${INSIGHT_TENANT_ID:-}" ]]; then
    printf '%s' "${INSIGHT_TENANT_ID}"
    return 0
  fi
  local val
  val="$(kubectl -n data get cm insight-config \
          -o jsonpath='{.data.tenant_id}' 2>/dev/null || true)"
  if [[ -z "${val}" ]]; then
    printf 'resolve_tenant_id: no INSIGHT_TENANT_ID env and no ConfigMap insight-config in ns data\n' >&2
    return 1
  fi
  printf '%s' "${val}"
}
# @cpt-end:cpt-insightspec-flow-reconcile-run-reconcile:p1:inst-rr-resolve-tenant

main() {
  # @cpt-begin:cpt-insightspec-flow-reconcile-run-reconcile:p1:inst-rr-resolve-airbyte-env
  # AIRBYTE_URL / AIRBYTE_TOKEN_FILE are honored by lib/airbyte.sh; nothing
  # to do here besides asserting the URL is non-empty.
  : "${AIRBYTE_URL:?AIRBYTE_URL must be set (e.g. http://airbyte-server:8001)}"
  # @cpt-end:cpt-insightspec-flow-reconcile-run-reconcile:p1:inst-rr-resolve-airbyte-env

  local subcmd="reconcile"
  local dry_run=0
  local connector=""
  local no_gc=0

  while [[ $# -gt 0 ]]; do
    case "$1" in
      adopt|reconcile) subcmd="$1"; shift ;;
      --dry-run)       dry_run=1; shift ;;
      --connector)     connector="${2:?--connector requires NAME}"; shift 2 ;;
      --no-gc)         no_gc=1; shift ;;
      -h|--help)       usage; return 0 ;;
      *)               printf 'unknown arg: %s\n' "$1" >&2; usage >&2; return 64 ;;
    esac
  done

  local tenant_id
  if ! tenant_id="$(resolve_tenant_id)"; then
    return 1
  fi
  export INSIGHT_TENANT_ID="${tenant_id}"

  printf 'tenant=%s subcommand=%s dry-run=%d connector=%s no-gc=%d\n' \
    "${tenant_id}" "${subcmd}" "${dry_run}" "${connector:-<all>}" "${no_gc}" >&2

  # @cpt-begin:cpt-insightspec-flow-reconcile-dry-run:p2:inst-dr-call-flow
  case "${subcmd}" in
    adopt)
      if [[ "${dry_run}" -eq 1 ]]; then
        ADOPT_DRY_RUN=1 adopt_run --dry-run
      else
        adopt_run
      fi
      ;;
    reconcile)
      local args=()
      [[ "${no_gc}" -eq 1 ]] && args+=(--no-gc)
      [[ -n "${connector}" ]] && args+=(--connector "${connector}")
      if [[ "${dry_run}" -eq 1 ]]; then
        reconcile_dry_run "${args[@]}"
      else
        reconcile_run "${args[@]}"
      fi
      ;;
    *)
      printf 'unreachable: bad subcommand %s\n' "${subcmd}" >&2
      return 70
      ;;
  esac
  # @cpt-end:cpt-insightspec-flow-reconcile-dry-run:p2:inst-dr-call-flow
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  main "$@"
fi
