#!/usr/bin/env bash
# ADR-0003 broker PoC (#2194) — apply the broker + upstream realms with
# keycloak-config-cli and re-point the UNCHANGED authenticator at the broker.
#
# Prerequisite: the compose stack is up in keycloak mode:
#     ./dev-compose.sh up --auth=keycloak
#
# What this does, on top of that stack:
#   1. Applies realms/*.yaml to the running Keycloak with keycloak-config-cli
#      (idempotent; secrets and host-specific URLs arrive via $(env:...)
#      substitution, never from the YAML itself).
#   2. Recreates ONLY the authenticator container with its issuer pointed at
#      the broker realm (`insight-broker`) instead of the direct realm — the
#      same binary/image, config-surface change only, exactly as ADR-0003
#      prescribes.
#
# Undo: re-run `./dev-compose.sh up --auth=keycloak` (restores the direct
# realm issuer); the PoC realms stay in Keycloak until the container is
# recreated, which is harmless.
set -euo pipefail
cd "$(dirname "$0")/../../../.."   # repo root

CONFIG_CLI_IMAGE="adorsys/keycloak-config-cli:6.5.1-26.1.0"
ENV_FILE="${ENV_FILE:-.env.compose}"
PROJECT="${COMPOSE_PROJECT_NAME:-insight}"

# Same host-IP trick as dev-compose.sh: one issuer URL that the browser AND
# the containers can both reach.
detect_host_ip() {
  if command -v ipconfig >/dev/null 2>&1; then           # macOS
    local ifc
    ifc="$(route -n get default 2>/dev/null | awk '/interface:/{print $2}')"
    if [[ -n "$ifc" ]] && ipconfig getifaddr "$ifc" 2>/dev/null; then return 0; fi
    ipconfig getifaddr en0 2>/dev/null && return 0
    return 1
  fi
  ip route get 1.1.1.1 2>/dev/null \
    | awk '{for (i=1;i<=NF;i++) if ($i=="src") {print $(i+1); exit}}'
}

HOST_IP="$(detect_host_ip)" || { echo "ERROR: cannot detect host IP" >&2; exit 1; }
KC_PUBLIC_BASE="http://${HOST_IP}:8085/kc"

# DEV_USER_EMAIL from the env file: the upstream user must resolve to a seeded
# person or the authenticator refuses the login.
DEV_USER_EMAIL="$(grep -E '^DEV_USER_EMAIL=' "$ENV_FILE" | tail -1 | cut -d= -f2-)"
: "${DEV_USER_EMAIL:?DEV_USER_EMAIL not found in $ENV_FILE}"
TENANT_ID="$(grep -E '^TENANT_DEFAULT_ID=' "$ENV_FILE" | tail -1 | cut -d= -f2-)"
TENANT_ID="${TENANT_ID:-00000000-df51-5b42-9538-d2b56b7ee953}"

# Dev-only PoC credentials (same convention as gen-realm.py's dev secret —
# synthetic, compose-only, never used on a deployed stand).
POC_AUTHENTICATOR_CLIENT_SECRET="${POC_AUTHENTICATOR_CLIENT_SECRET:-poc-broker-authenticator-dev-secret}"
POC_BROKER_CLIENT_SECRET="${POC_BROKER_CLIENT_SECRET:-poc-upstream-broker-dev-secret}"
POC_UPSTREAM_USER_PASSWORD="${POC_UPSTREAM_USER_PASSWORD:-insight-dev}"

echo "=== [1/2] Applying PoC realms with keycloak-config-cli ==="
docker run --rm --network "$PROJECT" \
  -v "$(pwd)/deploy/compose/keycloak/broker-poc/realms:/config:ro" \
  -e KEYCLOAK_URL="http://keycloak:8085/kc" \
  -e KEYCLOAK_USER=admin \
  -e KEYCLOAK_PASSWORD=admin \
  -e KEYCLOAK_AVAILABILITYCHECK_ENABLED=true \
  -e KEYCLOAK_AVAILABILITYCHECK_TIMEOUT=120s \
  -e IMPORT_FILES_LOCATIONS='/config/*.yaml' \
  -e IMPORT_VARSUBSTITUTION_ENABLED=true \
  -e POC_KC_PUBLIC_BASE="$KC_PUBLIC_BASE" \
  -e POC_UPSTREAM_USER_EMAIL="$DEV_USER_EMAIL" \
  -e POC_UPSTREAM_USER_PASSWORD="$POC_UPSTREAM_USER_PASSWORD" \
  -e POC_BROKER_CLIENT_SECRET="$POC_BROKER_CLIENT_SECRET" \
  -e POC_AUTHENTICATOR_CLIENT_SECRET="$POC_AUTHENTICATOR_CLIENT_SECRET" \
  -e POC_TENANT_ID="$TENANT_ID" \
  "$CONFIG_CLI_IMAGE"

echo "=== [2/2] Re-pointing the (unchanged) authenticator at the broker realm ==="
compose_cmd=(docker compose --project-name "$PROJECT" --env-file "$ENV_FILE" -f docker-compose.yml)
[[ -f deploy/compose/override.generated.yml ]] && compose_cmd+=(-f deploy/compose/override.generated.yml)

KEYCLOAK_HOSTNAME="$KC_PUBLIC_BASE" \
AUTHENTICATOR_OIDC_ISSUER="${KC_PUBLIC_BASE}/realms/insight-broker" \
OIDC_CLIENT_ID="insight-authenticator" \
OIDC_CLIENT_SECRET="$POC_AUTHENTICATOR_CLIENT_SECRET" \
  "${compose_cmd[@]}" --profile auth-keycloak up -d --no-deps authenticator

echo
echo "Broker PoC is live:"
echo "  issuer     ${KC_PUBLIC_BASE}/realms/insight-broker  (authenticator now points here)"
echo "  upstream   ${KC_PUBLIC_BASE}/realms/poc-upstream    (login: ${DEV_USER_EMAIL} / ${POC_UPSTREAM_USER_PASSWORD})"
echo "Verify end to end:  deploy/compose/keycloak/broker-poc/verify.sh"
