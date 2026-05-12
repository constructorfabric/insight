#!/usr/bin/env bash
#
# Top-level installer: Airbyte → Argo Workflows → Insight.
#
# A UX wrapper: "one command installs the whole stack" for the user,
# three sequential Helm releases in different namespaces underneath.
#
# Idempotent: safe to re-run.
#
# Usage:
#   ./deploy/scripts/install.sh
#
# Environment:
#   SKIP_AIRBYTE=1   — Airbyte is already installed or managed separately
#   SKIP_ARGO=1      — Argo Workflows is already installed
#   SKIP_INSIGHT=1   — install only infra (skip the umbrella)
#
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

log() { printf '\033[32m[install]\033[0m %s\n' "$*"; }

if [[ "${SKIP_AIRBYTE:-0}" == "1" ]]; then
  log "SKIP_AIRBYTE=1 → skipping Airbyte"
else
  log "Step 1/3: Airbyte"
  "$ROOT_DIR/install-airbyte.sh"
fi

if [[ "${SKIP_ARGO:-0}" == "1" ]]; then
  log "SKIP_ARGO=1 → skipping Argo Workflows"
else
  log "Step 2/3: Argo Workflows"
  "$ROOT_DIR/install-argo.sh"
fi

if [[ "${SKIP_INSIGHT:-0}" == "1" ]]; then
  log "SKIP_INSIGHT=1 → skipping Insight"
else
  log "Step 3/3: Insight"
  "$ROOT_DIR/install-insight.sh"
fi

NS="${INSIGHT_NAMESPACE:-insight}"
cat <<EOF

══════════════════════════════════════════════════════════════════════════
   All done.

   Airbyte UI:
     kubectl -n $NS port-forward svc/airbyte-airbyte-webapp-svc 8080:80

   Insight UI:
     kubectl -n $NS port-forward svc/insight-frontend 8081:80

   Open http://localhost:8081 in your browser.
══════════════════════════════════════════════════════════════════════════

EOF
