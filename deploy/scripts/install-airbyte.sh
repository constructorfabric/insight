#!/usr/bin/env bash
#
# Install/upgrade Airbyte as a standalone Helm release.
#
# Idempotent: re-running does `helm upgrade` with the same values.
#
# All Insight components (Airbyte, Argo Workflows, the umbrella) live in
# the SAME namespace — see deploy/README.md for the single-namespace model.
# Multiple Insight instances on the same cluster use different namespaces;
# each is self-contained.
#
# Environment overrides:
#   INSIGHT_NAMESPACE  (default: insight) — shared by all components
#   AIRBYTE_RELEASE    (default: airbyte)
#   AIRBYTE_VERSION    (default: 1.8.5)
#   AIRBYTE_VALUES     (default: deploy/airbyte/values.yaml)
#   EXTRA_VALUES_FILE  additional -f values.yaml (for prod overrides)
#
# Usage:
#   ./deploy/scripts/install-airbyte.sh
#   EXTRA_VALUES_FILE=deploy/airbyte/values-prod.yaml ./deploy/scripts/install-airbyte.sh
#
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

NAMESPACE="${INSIGHT_NAMESPACE:-insight}"
RELEASE="${AIRBYTE_RELEASE:-airbyte}"
VERSION="${AIRBYTE_VERSION:-1.8.5}"
VALUES="${AIRBYTE_VALUES:-deploy/airbyte/values.yaml}"
EXTRA="${EXTRA_VALUES_FILE:-}"

log() { printf '\033[36m[install-airbyte]\033[0m %s\n' "$*"; }
die() { printf '\033[31m[install-airbyte] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# ─── Prerequisites ─────────────────────────────────────────────────────
command -v helm      >/dev/null || die "helm not found"
command -v kubectl   >/dev/null || die "kubectl not found"
[[ -f "$VALUES" ]]                || die "values file not found: $VALUES"
[[ -z "$EXTRA" || -f "$EXTRA" ]]  || die "extra values file not found: $EXTRA"

log "Cluster: $(kubectl config current-context)"
log "Namespace: $NAMESPACE · Release: $RELEASE · Chart: airbyte/airbyte@$VERSION"

# ─── Repo ──────────────────────────────────────────────────────────────
if ! helm repo list 2>/dev/null | grep -q '^airbyte\s'; then
  log "Adding Airbyte helm repo"
  helm repo add airbyte https://airbytehq.github.io/helm-charts
fi
helm repo update airbyte >/dev/null

# ─── Install / upgrade ─────────────────────────────────────────────────
VALUES_ARGS=(-f "$VALUES")
[[ -n "$EXTRA" ]] && VALUES_ARGS+=(-f "$EXTRA")

log "Running helm upgrade --install"
helm upgrade --install "$RELEASE" airbyte/airbyte \
  --namespace "$NAMESPACE" --create-namespace \
  --version "$VERSION" \
  "${VALUES_ARGS[@]}" \
  --wait --timeout 15m

# ─── JWT secret ────────────────────────────────────────────────────────
# airbyte-auth-secrets / jwt-signature-secret is created by the Airbyte
# chart in the release namespace. Since Insight components live in the
# SAME namespace, no cross-namespace mirror is needed — the ingestion
# WorkflowTemplates reference the secret directly.
if kubectl -n "$NAMESPACE" get secret airbyte-auth-secrets >/dev/null 2>&1; then
  log "Verified: $NAMESPACE/airbyte-auth-secrets (Insight workflows will use it)"
else
  log "WARNING: airbyte-auth-secrets not yet present in $NAMESPACE."
  log "         It is created by the Airbyte chart on first boot; rerun"
  log "         this script after Airbyte finishes starting."
fi

# ─── Initial setup wizard ──────────────────────────────────────────────
# Airbyte 1.5.x simple-auth shows a setup wizard on first UI visit that
# can leave the instance in a half-initialised state if the user clicks
# away. Complete it via API so UI login works out of the box.
#
# Idempotent: if setup is already complete, the second call is a no-op.
: "${AIRBYTE_SETUP_EMAIL:?ERROR: AIRBYTE_SETUP_EMAIL is required — first-install setup wizard creates the admin user with this address}"
: "${AIRBYTE_SETUP_ORG:?ERROR: AIRBYTE_SETUP_ORG is required — first-install setup wizard names the workspace organization with this value}"
SETUP_EMAIL="$AIRBYTE_SETUP_EMAIL"
SETUP_ORG="$AIRBYTE_SETUP_ORG"

if kubectl -n "$NAMESPACE" get secret airbyte-auth-secrets >/dev/null 2>&1; then
  log "Completing initial setup via API (email=$SETUP_EMAIL, org=$SETUP_ORG)"

  # Pick a free local port dynamically — hardcoded 18001 collides with
  # any other tool the developer may have on that port.
  PF_LOCAL_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("",0)); print(s.getsockname()[1])')

  # Temporary port-forward with a trap — under `set -euo pipefail`, any
  # non-zero exit below would otherwise leak the PF process.
  kubectl -n "$NAMESPACE" port-forward svc/"$RELEASE"-airbyte-server-svc "$PF_LOCAL_PORT:8001" >/dev/null 2>&1 &
  PF_PID=$!
  # shellcheck disable=SC2064
  trap "kill $PF_PID 2>/dev/null || true" EXIT INT TERM

  # Wait up to 20s for server API.
  for _ in $(seq 1 20); do
    curl -sf -o /dev/null "http://localhost:$PF_LOCAL_PORT/api/v1/instance_configuration" && break
    sleep 1
  done

  CID=$(kubectl -n "$NAMESPACE" get secret airbyte-auth-secrets -o jsonpath='{.data.instance-admin-client-id}' | base64 -d)
  CSEC=$(kubectl -n "$NAMESPACE" get secret airbyte-auth-secrets -o jsonpath='{.data.instance-admin-client-secret}' | base64 -d)
  TOKEN=$(curl -sf -X POST "http://localhost:$PF_LOCAL_PORT/api/v1/applications/token" \
    -H "Content-Type: application/json" \
    -d "{\"client_id\":\"$CID\",\"client_secret\":\"$CSEC\"}" -m 10 \
    | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])" 2>/dev/null || true)

  if [[ -n "$TOKEN" ]]; then
    curl -sf -X POST "http://localhost:$PF_LOCAL_PORT/api/v1/instance_configuration/setup" \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer $TOKEN" \
      -d "{\"email\":\"$SETUP_EMAIL\",\"organizationName\":\"$SETUP_ORG\",\"initialSetupComplete\":true,\"anonymousDataCollection\":false,\"news\":false,\"securityUpdates\":false}" \
      -m 10 -o /dev/null && log "Initial setup complete." || log "Initial setup call failed (may already be set up)."
  else
    log "Could not mint access token; skipping setup wizard (complete via UI)."
  fi
  # Cleanup is handled by the EXIT trap above.
  kill "$PF_PID" 2>/dev/null || true
  trap - EXIT INT TERM
fi

# ─── Summary ───────────────────────────────────────────────────────────
cat <<EOF

✓ Airbyte installed.

Verify:
  kubectl -n $NAMESPACE get pods
  kubectl -n $NAMESPACE port-forward svc/$RELEASE-airbyte-webapp-svc 8080:80
  # then open http://localhost:8080

API reachable at:
  http://$RELEASE-airbyte-server-svc.$NAMESPACE.svc.cluster.local:8001

Insight will use JWT secret: $NAMESPACE/airbyte-auth-secrets (key: jwt-signature-secret)

Next step:
  INSIGHT_NAMESPACE=$NAMESPACE ./deploy/scripts/install-insight.sh

EOF
