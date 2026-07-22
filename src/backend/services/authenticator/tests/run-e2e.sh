#!/usr/bin/env bash
# End-to-end login-loop smoke test for the authenticator (nginx+auth step 04).
#
# Spins up the minimal stack — Redis (docker), fakeidp + authenticator (local
# release binaries) — and runs the ignored `e2e_login_loop` integration test:
#   /auth/login -> fakeidp /authorize -> /auth/callback (cookie) ->
#   /internal/authz (JWT verified against JWKS) -> /auth/me -> /auth/logout ->
#   /internal/authz returns 401.
#
# Everything runs on localhost, so no IdP-URL rewriting is needed. Usage:
#   src/backend/services/authenticator/tests/run-e2e.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE/../../.."   # -> src/backend (the cargo workspace root)

AUTH_PORT=8083
TOKEN_PORT=8093
IDP_PORT=8084
IDENTITY_PORT=8092
REDIS_CT=authenticator-e2e-redis
pids=()

cleanup() {
  set +e
  for p in "${pids[@]:-}"; do kill "$p" 2>/dev/null; done
  docker rm -f "$REDIS_CT" >/dev/null 2>&1
  [[ -n "${KEYS_DIR:-}" ]] && rm -rf "$KEYS_DIR"
  [[ -n "${SVC_KEYS_DIR:-}" ]] && rm -rf "$SVC_KEYS_DIR"
}
trap cleanup EXIT

echo "==> dev ES256 signing key"
# ec_param_enc:named_curve: LibreSSL (macOS) otherwise emits explicit EC params
# the authenticator's p256 loader rejects.
KEYS_DIR="$(mktemp -d)"
# ES256 gateway signing key (§9.6). Named-curve P-256 (see the LibreSSL note
# above). The service-token client key below is also EC.
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -pkeyopt ec_param_enc:named_curve -out "$KEYS_DIR/current.pem"

echo "==> dev service-token keypair (testclient) — generated, never committed"
# The registry (config/insight.yaml) references public_key_paths: [testclient.pub.pem]
# resolved against public_key_dir; the client signs assertions with the private half.
SVC_KEYS_DIR="$(mktemp -d)"
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -pkeyopt ec_param_enc:named_curve -out "$SVC_KEYS_DIR/testclient.key.pem"
openssl pkey -in "$SVC_KEYS_DIR/testclient.key.pem" -pubout -out "$SVC_KEYS_DIR/testclient.pub.pem"

echo "==> Redis"
docker rm -f "$REDIS_CT" >/dev/null 2>&1 || true
docker run -d --name "$REDIS_CT" -p 6399:6379 redis:7-alpine >/dev/null

echo "==> build fakeidp + authenticator"
cargo build --release --bin fakeidp --bin authenticator

# Wait for an HTTP endpoint to answer, or fail loudly.
wait_ready() { # name url
  for _ in $(seq 1 30); do
    curl -fsS -o /dev/null "$2" && return 0
    sleep 1
  done
  echo "ERROR: $1 did not become ready ($2)" >&2
  return 1
}

echo "==> fakeidp :$IDP_PORT"
# Short IdP token TTL so the background refresher (step 10.4) cycles within
# seconds instead of minutes; see the matching margin/tick overrides below.
FAKEIDP_ISSUER="http://localhost:$IDP_PORT" FAKEIDP_BIND="0.0.0.0:$IDP_PORT" \
  FAKEIDP_DEFAULT_AUD=insight-authenticator \
  FAKEIDP_BACKCHANNEL_URL="http://localhost:$AUTH_PORT/auth/oidc/back-channel-logout" \
  FAKEIDP_TOKEN_TTL=15 \
  ./target/release/fakeidp >/tmp/authenticator-e2e-fakeidp.log 2>&1 &
pids+=($!)
wait_ready fakeidp "http://localhost:$IDP_PORT/.well-known/openid-configuration"

echo "==> identity stub :$IDENTITY_PORT (resolves any email to a person)"
python3 "$HERE/identity-stub.py" "127.0.0.1:$IDENTITY_PORT" >/tmp/authenticator-e2e-identity.log 2>&1 &
pids+=($!)
wait_ready identity-stub "http://localhost:$IDENTITY_PORT/internal/persons/by-email/probe@example.com"

echo "==> authenticator :$AUTH_PORT"
APP__gears__authenticator__config__redis_url=redis://localhost:6399 \
APP__gears__authenticator__config__signing_keys_path="$KEYS_DIR" \
APP__gears__authenticator__config__identity_url="http://localhost:$IDENTITY_PORT" \
APP__gears__authenticator__config__gateway_issuer=http://localhost:8080 \
APP__gears__authenticator__config__idp__issuer_url="http://localhost:$IDP_PORT" \
APP__gears__authenticator__config__idp__client_id=insight-authenticator \
APP__gears__authenticator__config__redirect_uri="http://localhost:$AUTH_PORT/auth/callback" \
APP__gears__authenticator__config__service_tokens__public_key_dir="$SVC_KEYS_DIR" \
APP__gears__authenticator__config__idp__refresh_safety_margin_seconds=10 \
APP__gears__authenticator__config__idp__refresh_due_jitter_seconds=1 \
APP__gears__authenticator__config__idp__refresher_tick_seconds=1 \
  ./target/release/authenticator -c services/authenticator/config/insight.yaml run \
  >/tmp/authenticator-e2e-auth.log 2>&1 &
pids+=($!)

echo "==> wait for authenticator readiness"
if ! wait_ready authenticator "http://localhost:$AUTH_PORT/.well-known/jwks.json"; then
  tail -20 /tmp/authenticator-e2e-auth.log >&2 || true
  exit 1
fi

echo "==> run the login loop"
AUTH_BASE="http://localhost:$AUTH_PORT" E2E_USER=dev@company.nonpresent \
  cargo test -p authenticator --test e2e_login_loop -- --ignored --nocapture

echo "==> run the refresh rotation-with-grace loop (step 10.1)"
AUTH_BASE="http://localhost:$AUTH_PORT" E2E_USER=dev@company.nonpresent \
  cargo test -p authenticator --test e2e_refresh -- --ignored --nocapture

echo "==> run the session-management loop (step 10.2)"
AUTH_BASE="http://localhost:$AUTH_PORT" E2E_USER=dev@company.nonpresent \
  cargo test -p authenticator --test e2e_sessions -- --ignored --nocapture

echo "==> run the back-channel logout loop (step 10.3)"
AUTH_BASE="http://localhost:$AUTH_PORT" FAKEIDP_PUBLIC="http://localhost:$IDP_PORT" \
  E2E_USER=dev@company.nonpresent \
  cargo test -p authenticator --test e2e_backchannel -- --ignored --nocapture

echo "==> run the layer-2 rate-limit loop (step 10.6)"
AUTH_BASE="http://localhost:$AUTH_PORT" E2E_USER=dev@company.nonpresent \
  cargo test -p authenticator --test e2e_ratelimit -- --ignored --nocapture

echo "==> run the IdP background-refresher loop (step 10.4: outage + invalid_grant)"
AUTH_BASE="http://localhost:$AUTH_PORT" FAKEIDP_PUBLIC="http://localhost:$IDP_PORT" \
  cargo test -p authenticator --test e2e_refresher -- --ignored --nocapture

echo "==> run the service-token loop (step 06)"
# The token listener binds 8093 (config service_tokens.token_bind_addr); the dev
# `testclient` registry entry resolves public_key_paths against the generated
# SVC_KEYS_DIR set above, and the client signs with the matching private key.
AUTH_BASE="http://localhost:$AUTH_PORT" \
  TOKEN_ENDPOINT="http://localhost:$TOKEN_PORT/internal/token" \
  SVC_KEY="$SVC_KEYS_DIR/testclient.key.pem" \
  cargo test -p authenticator --test e2e_service_token -- --ignored --nocapture

echo "==> PASS"
