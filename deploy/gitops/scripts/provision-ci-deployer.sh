#!/usr/bin/env bash
# Thin wrapper: the RBAC objects and mutation logic live in the gitops
# Makefile. Procedure: deploy/gitops/environments/test-stand/credentials-runbook.md.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GITOPS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
  cat <<'USAGE'
provision-ci-deployer.sh MODE [ENV]

MODE (required):
  provision   apply RBAC, wait for the token, assemble a kubeconfig, verify
  verify      re-run the can-i scope matrix, without re-provisioning
  revoke      delete the ServiceAccount, RBAC and token Secret

ENV (optional, default: test-stand): the gitops environment.

Knobs (env vars forwarded to make): KUBE_CTX, CI_DEPLOYER_OUT,
CI_DEPLOYER_TOKEN_WAIT_S, CI_DEPLOYER_CA_FILE. See `make -C deploy/gitops help`.
USAGE
}

[ $# -ge 1 ] || { usage >&2; exit 1; }
case "$1" in
-h | --help) usage; exit 0 ;;
esac

MODE="$1"
# RULE-DEFAULTS-OK: today's only committed manifest lives under test-stand.
ENV_NAME="test-stand"
[ $# -ge 2 ] && ENV_NAME="$2"

case "$MODE" in
provision) TARGET="provision-ci" ;;
verify) TARGET="verify-ci-credential" ;;
revoke) TARGET="revoke-ci" ;;
*)
  usage >&2
  printf 'ERROR: unknown MODE: %s\n' "$MODE" >&2
  exit 1
  ;;
esac

exec make -C "$GITOPS_DIR" "$TARGET" "ENV=$ENV_NAME"
