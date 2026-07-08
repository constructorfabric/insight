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

cd "$(dirname "$0")/../../../.."   # -> src/backend
KEYS="$(cd ../../deploy/compose/authenticator-dev-keys && pwd)"

AUTH_PORT=8083
IDP_PORT=8084
REDIS_CT=authenticator-e2e-redis
pids=()

cleanup() {
  set +e
  for p in "${pids[@]:-}"; do kill "$p" 2>/dev/null; done
  docker rm -f "$REDIS_CT" >/dev/null 2>&1
}
trap cleanup EXIT

echo "==> Redis"
docker rm -f "$REDIS_CT" >/dev/null 2>&1 || true
docker run -d --name "$REDIS_CT" -p 6399:6379 redis:7-alpine >/dev/null

echo "==> build fakeidp + authenticator"
cargo build --release --bin fakeidp --bin authenticator

echo "==> fakeidp :$IDP_PORT"
FAKEIDP_ISSUER="http://localhost:$IDP_PORT" FAKEIDP_BIND="0.0.0.0:$IDP_PORT" \
  FAKEIDP_DEFAULT_AUD=insight-authenticator \
  ./target/release/fakeidp >/tmp/authenticator-e2e-fakeidp.log 2>&1 &
pids+=($!)

echo "==> authenticator :$AUTH_PORT"
APP__gears__authenticator__config__redis_url=redis://localhost:6399 \
APP__gears__authenticator__config__signing_keys_path="$KEYS" \
APP__gears__authenticator__config__identity_url= \
APP__gears__authenticator__config__gateway_issuer=http://localhost:8080 \
APP__gears__authenticator__config__idp__issuer_url="http://localhost:$IDP_PORT" \
APP__gears__authenticator__config__idp__client_id=insight-authenticator \
APP__gears__authenticator__config__redirect_uri="http://localhost:$AUTH_PORT/auth/callback" \
  ./target/release/authenticator -c services/authenticator/config/insight.yaml run \
  >/tmp/authenticator-e2e-auth.log 2>&1 &
pids+=($!)

echo "==> wait for readiness"
for _ in $(seq 1 30); do
  if curl -fsS -o /dev/null "http://localhost:$AUTH_PORT/.well-known/jwks.json"; then break; fi
  sleep 1
done

echo "==> run the login loop"
AUTH_BASE="http://localhost:$AUTH_PORT" E2E_USER=dev@company.nonpresent \
  cargo test -p authenticator --test e2e_login_loop -- --ignored --nocapture

echo "==> PASS"
