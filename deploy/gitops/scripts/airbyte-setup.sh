#!/usr/bin/env bash
#
# airbyte-setup.sh — post-install setup wizard automation for Airbyte.
#
# Airbyte 1.5.x simple-auth shows a setup wizard on first UI visit that
# can leave the instance in a half-initialised state if the user clicks
# away. This script completes it via API so UI login works on first try.
#
# Idempotent: a re-run on an already-configured instance is a no-op.
#
# Inputs (env vars):
#   NAMESPACE           required — Kubernetes namespace of the Airbyte release
#   AIRBYTE_RELEASE     required — Helm release name (e.g. `airbyte`)
#   AIRBYTE_SETUP_EMAIL required — admin email for the workspace
#   AIRBYTE_SETUP_ORG   required — workspace organisation name
#
set -euo pipefail

: "${NAMESPACE:?NAMESPACE is required}"
: "${AIRBYTE_RELEASE:?AIRBYTE_RELEASE is required}"
: "${AIRBYTE_SETUP_EMAIL:?AIRBYTE_SETUP_EMAIL is required (first-install admin email)}"
: "${AIRBYTE_SETUP_ORG:?AIRBYTE_SETUP_ORG is required (first-install organisation name)}"

log() { printf '\033[36m[airbyte-setup]\033[0m %s\n' "$*"; }

if ! kubectl -n "$NAMESPACE" get secret airbyte-auth-secrets >/dev/null 2>&1; then
  log "WARNING: $NAMESPACE/airbyte-auth-secrets not present yet."
  log "         The Airbyte chart creates it on first boot. Re-run this"
  log "         script after Airbyte finishes starting."
  exit 0
fi

log "Completing initial setup via API (email=$AIRBYTE_SETUP_EMAIL, org=$AIRBYTE_SETUP_ORG)"

# Pick a free local port dynamically — a hardcoded port collides with
# any other tool the developer may have running on it.
PF_LOCAL_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("",0)); print(s.getsockname()[1])')

kubectl -n "$NAMESPACE" port-forward \
  "svc/${AIRBYTE_RELEASE}-airbyte-server-svc" "$PF_LOCAL_PORT:8001" \
  >/dev/null 2>&1 &
PF_PID=$!
# shellcheck disable=SC2064
trap "kill $PF_PID 2>/dev/null || true" EXIT INT TERM

# Wait up to 20s for the server API to come up on the port-forward.
for _ in $(seq 1 20); do
  curl -sf -o /dev/null \
    "http://localhost:$PF_LOCAL_PORT/api/v1/instance_configuration" && break
  sleep 1
done

CID=$(kubectl -n "$NAMESPACE" get secret airbyte-auth-secrets \
  -o jsonpath='{.data.instance-admin-client-id}' | base64 -d)
CSEC=$(kubectl -n "$NAMESPACE" get secret airbyte-auth-secrets \
  -o jsonpath='{.data.instance-admin-client-secret}' | base64 -d)

TOKEN=$(curl -sf -X POST \
  "http://localhost:$PF_LOCAL_PORT/api/v1/applications/token" \
  -H "Content-Type: application/json" \
  -d "{\"client_id\":\"$CID\",\"client_secret\":\"$CSEC\"}" -m 10 \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['access_token'])" \
    2>/dev/null || true)

if [[ -n "$TOKEN" ]]; then
  curl -sf -X POST \
    "http://localhost:$PF_LOCAL_PORT/api/v1/instance_configuration/setup" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $TOKEN" \
    -d "{\"email\":\"$AIRBYTE_SETUP_EMAIL\",\"organizationName\":\"$AIRBYTE_SETUP_ORG\",\"initialSetupComplete\":true,\"anonymousDataCollection\":false,\"news\":false,\"securityUpdates\":false}" \
    -m 10 -o /dev/null \
    && log "Initial setup complete." \
    || log "Initial setup call failed (may already be set up)."
else
  log "Could not mint access token; skipping setup wizard (complete via UI)."
fi

kill "$PF_PID" 2>/dev/null || true
trap - EXIT INT TERM
