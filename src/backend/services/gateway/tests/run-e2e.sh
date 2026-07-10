#!/usr/bin/env bash
# End-to-end test for the nginx gateway (NGINX_BFF.md §D; gateway DESIGN 3.9-3.12).
#
# Brings up the real authenticator + fakeidp (steps 03-04) behind the OpenResty
# gateway, with a stub identity resolver and an echo upstream, then runs the five
# §D scenarios from an on-network driver. Container lifecycle (stop/start for the
# fail-closed scenarios) is orchestrated here; the assertions live in assert.py.
#
# Usage: src/backend/services/gateway/tests/run-e2e.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"
BACKEND="$(cd ../../.. && pwd)"
COMPOSE=(docker compose -f docker-compose.e2e.yml)
GW_PORT="${GW_E2E_PORT:-18080}"

cleanup() {
  set +e
  "${COMPOSE[@]}" logs --no-color > /tmp/gw-e2e.log 2>&1
  "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1
  rm -rf "$HERE/keys" "$HERE/work" "$HERE/nginx.conf"
}
trap cleanup EXIT

echo "==> dev ES256 signing key"
mkdir -p "$HERE/keys" "$HERE/work"
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$HERE/keys/current.pem"
chmod 644 "$HERE/keys/current.pem"   # readable by the container's uid 1000

echo "==> generate the e2e nginx.conf (compose names, docker resolver)"
( cd "$BACKEND" && cargo run -q -p routegen -- \
    --routes services/gateway/tests/routes.e2e.yaml \
    --authenticator-url http://authenticator:8083 \
    --front-url http://echo:9090 \
    --resolver 127.0.0.11 \
    -o services/gateway/tests/nginx.conf )

echo "==> build + start the stack"
"${COMPOSE[@]}" up -d --build redis identity-stub fakeidp authenticator echo gateway

wait_http() { # url
  for _ in $(seq 1 60); do
    if curl -fsS -o /dev/null "$1"; then return 0; fi
    sleep 1
  done
  echo "ERROR: not ready: $1" >&2
  "${COMPOSE[@]}" logs --no-color --tail 30 >&2
  return 1
}

wait_status() { # url cookie expected
  local code
  for _ in $(seq 1 30); do
    code=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: $2" "$1" || true)
    [ "$code" = "$3" ] && return 0
    sleep 1
  done
  echo "ERROR: $1 never returned $3 (last=$code)" >&2
  return 1
}

echo "==> wait for gateway + authenticator readiness"
wait_http "http://localhost:${GW_PORT}/healthz"
wait_http "http://localhost:${GW_PORT}/.well-known/jwks.json"

# --no-deps: `run` would otherwise restart the driver's transitive dependencies
# (gateway -> authenticator, echo), reviving the very container a phase just
# killed. The stack is already up; the driver must not touch it.
driver() { "${COMPOSE[@]}" run --rm --no-deps driver "$1"; }

echo "==> scenario 1/2/5: hygiene, login, routing"
driver core

echo "==> scenario 3/4: authenticator down -> cache serves hits, cold cookie 503"
"${COMPOSE[@]}" kill authenticator >/dev/null   # abrupt: no graceful drain
# Wait until the fail-closed state is actually reached (a cold cookie -> 503)
# before asserting, so we do not race the authenticator's shutdown.
wait_status "http://localhost:${GW_PORT}/api/v1/analytics/x" "__Host-sid=cold-poll" 503
driver authdown
"${COMPOSE[@]}" start authenticator >/dev/null
wait_http "http://localhost:${GW_PORT}/.well-known/jwks.json"

echo "==> scenario 3: logout revocation within the exchange-cache window"
sleep 4   # > authz_cache_max_age_seconds (3) so the cached entry expires
driver revoked

echo "==> scenario 4: dead upstream -> 502"
"${COMPOSE[@]}" kill echo >/dev/null
driver upstreamdown
"${COMPOSE[@]}" start echo >/dev/null

echo "==> PASS: all gateway e2e scenarios green"
