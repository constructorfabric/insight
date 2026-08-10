#!/usr/bin/env bash
# End-to-end login-loop smoke test for the authenticator (nginx+auth step 04).
#
# Spins up the minimal stack — Redis + Keycloak (docker), authenticator (local
# release binaries) — and runs the ignored `e2e_*` integration suites:
#   /auth/login -> Keycloak login form -> /auth/callback (cookie) ->
#   /internal/authz (JWT verified against JWKS) -> /auth/me -> /auth/logout ->
#   /internal/authz returns 401.
#
# The IdP is a real Keycloak importing the generated compose realm
# (`insight-seed-realm`, from src/ingestion/tools/seed) with the rig's overlay
# on top
# (tests/kc-realm-overlay.py: test users, fast token lifespan, back-channel
# registration, and a second realm for the host-keyed issuer map). IdP-side
# events the suites need to provoke (logout, revocation, outage) are driven
# through the Keycloak admin API and `docker pause` (tests/common/kc.rs).
#
# Everything runs on localhost, so no IdP-URL rewriting is needed. Usage:
#   src/backend/services/authenticator/tests/run-e2e.sh
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$HERE/../../../../.." && pwd)"   # the repo root
cd "$HERE/../../.."   # -> src/backend (the cargo workspace root)

# Endpoint-coverage ledger: tests/common/mod.rs records every test-client
# request against the authenticator into this file (merged across the serial
# cargo test invocations below). The endpoint coverage gate consumes it:
#   python3 src/ingestion/tests/e2e/lib/api_coverage.py --suite authenticator \
#     --observed "$E2E_COVERAGE_LEDGER" \
#     --spec docs/components/backend/authenticator/openapi.json
# Reset it up front so dead coverage from a previous run can't survive.
export E2E_COVERAGE_LEDGER="${E2E_COVERAGE_LEDGER:-$HERE/.artifacts/observed_authenticator_endpoints.json}"
rm -f "$E2E_COVERAGE_LEDGER"

# Env-overridable so the rig can run next to a compose stack, which holds
# several of the defaults (8083/8085/8093).
AUTH_PORT="${AUTH_PORT:-8083}"
TOKEN_PORT="${TOKEN_PORT:-8093}"
AUTH2_PORT="${AUTH2_PORT:-8085}"
TOKEN2_PORT="${TOKEN2_PORT:-8095}"
AUTH3_PORT="${AUTH3_PORT:-8087}"
TOKEN3_PORT="${TOKEN3_PORT:-8097}"
KC_PORT="${KC_PORT:-8084}"
IDENTITY_PORT="${IDENTITY_PORT:-8092}"
REDIS_CT=authenticator-e2e-redis
KC_CT=authenticator-e2e-keycloak
# Same image the compose stack pins for its realm (docker-compose.yml).
KC_IMAGE=quay.io/keycloak/keycloak:26.4
KC_REALM=insight
KC_REALM_B=insight-b
KC_ADMIN_USER=admin
KC_ADMIN_PASSWORD=admin
E2E_USER=dev@company.nonpresent
# The realm generator lives in the seed package and is run through uv, which
# resolves and installs it on first use — the same way dev-compose.sh and the
# gitops `keycloak-realm` target invoke it.
SEED_DIR="$ROOT_DIR/src/ingestion/tools/seed"
# Every realm user carries a tenant claim and the generator requires one rather
# than defaulting to a stand's. Which tenant is immaterial here: the rig
# resolves people by email (idp.external_id_claim=email), so this only has to
# be named, and it is the value the compose stack uses.
KC_TENANT_ID=00000000-df51-5b42-9538-d2b56b7ee953
pids=()

cleanup() {
  set +e
  for p in "${pids[@]:-}"; do kill "$p" 2>/dev/null; done
  # A failed e2e_refresher can leave the container paused; a paused container
  # cannot be force-removed.
  docker unpause "$KC_CT" >/dev/null 2>&1
  docker rm -f "$KC_CT" "$REDIS_CT" >/dev/null 2>&1
  [[ -n "${KEYS_DIR:-}" ]] && rm -rf "$KEYS_DIR"
  [[ -n "${SVC_KEYS_DIR:-}" ]] && rm -rf "$SVC_KEYS_DIR"
  [[ -n "${AUTH_CONFIG:-}" ]] && rm -f "$AUTH_CONFIG"
  [[ -n "${AUTH2_CONFIG:-}" ]] && rm -f "$AUTH2_CONFIG"
  [[ -n "${AUTH3_CONFIG:-}" ]] && rm -f "$AUTH3_CONFIG"
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

echo "==> generate the Keycloak import set (roster realm + e2e overlay)"
# Inside the repo (not mktemp) so docker file-sharing covers it on macOS;
# .artifacts/ is gitignored.
KC_IMPORT_DIR="$HERE/.artifacts/keycloak-import"
rm -rf "$KC_IMPORT_DIR" && mkdir -p "$KC_IMPORT_DIR"
# The redirect URIs are the three authenticator instances below; the compose
# defaults would deregister them (--authenticator-redirect REPLACES, not
# appends).
command -v uv >/dev/null 2>&1 || {
  echo "uv is required to generate the realm — https://docs.astral.sh/uv/getting-started/installation/" >&2
  exit 1
}
TENANT_DEFAULT_ID="$KC_TENANT_ID" \
uv run --project "$SEED_DIR" insight-seed-realm \
  --dev-email "$E2E_USER" \
  --authenticator-redirect "http://localhost:$AUTH_PORT/auth/callback" \
  --authenticator-redirect "http://localhost:$AUTH2_PORT/auth/callback" \
  --authenticator-redirect "http://localhost:$AUTH3_PORT/auth/callback" \
  --out "$KC_IMPORT_DIR/realm-insight.generated.json"
# host.docker.internal: the container must reach the authenticator process on
# the docker host (docker run maps it via host-gateway below).
python3 "$HERE/kc-realm-overlay.py" \
  --realm "$KC_IMPORT_DIR/realm-insight.generated.json" \
  --out-dir "$KC_IMPORT_DIR" \
  --second-realm "$KC_REALM_B" \
  --backchannel-url "http://host.docker.internal:$AUTH_PORT/auth/oidc/back-channel-logout" \
  --access-token-lifespan 15
rm -f "$KC_IMPORT_DIR/realm-insight.generated.json"

echo "==> Keycloak :$KC_PORT (realms $KC_REALM + $KC_REALM_B, --import-realm)"
docker rm -f "$KC_CT" >/dev/null 2>&1 || true
# KC_HOSTNAME pins the advertised issuer (and the `iss` of back-channel
# logout tokens) to the host-published origin, so what the authenticator
# discovers is also what admin-triggered logout tokens carry.
docker run -d --name "$KC_CT" -p "$KC_PORT:8080" \
  --add-host=host.docker.internal:host-gateway \
  -e KC_BOOTSTRAP_ADMIN_USERNAME="$KC_ADMIN_USER" \
  -e KC_BOOTSTRAP_ADMIN_PASSWORD="$KC_ADMIN_PASSWORD" \
  -e KC_HOSTNAME="http://localhost:$KC_PORT" \
  -e JAVA_OPTS_APPEND="-Xms256m -Xmx512m" \
  -v "$KC_IMPORT_DIR:/opt/keycloak/data/import:ro" \
  "$KC_IMAGE" start-dev --import-realm >/dev/null

KC_BASE="http://localhost:$KC_PORT"
ISSUER="$KC_BASE/realms/$KC_REALM"
ISSUER_B="$KC_BASE/realms/$KC_REALM_B"

echo "==> build the authenticator"
cargo build --release --bin authenticator

# Wait for an HTTP endpoint to answer, or fail loudly. Tries default to 30.
wait_ready() { # name url [tries]
  for _ in $(seq 1 "${3:-30}"); do
    curl --connect-timeout 2 --max-time 5 -fsS -o /dev/null "$2" && return 0
    sleep 1
  done
  echo "ERROR: $1 did not become ready ($2)" >&2
  return 1
}

echo "==> wait for Keycloak realm import"
# Keycloak start + realm import runs tens of seconds; the realm discovery
# document answers only once its import committed.
if ! wait_ready keycloak "$ISSUER/.well-known/openid-configuration" 120; then
  docker logs --tail 40 "$KC_CT" >&2 || true
  exit 1
fi
wait_ready keycloak-b "$ISSUER_B/.well-known/openid-configuration" 30

echo "==> identity stub :$IDENTITY_PORT (resolves any email/external-id to a person)"
python3 "$HERE/identity-stub.py" "127.0.0.1:$IDENTITY_PORT" >/tmp/authenticator-e2e-identity.log 2>&1 &
pids+=($!)
wait_ready identity-stub "http://localhost:$IDENTITY_PORT/internal/persons/by-external-id?source_type=faketest&external_id=probe"

echo "==> authenticator :$AUTH_PORT"
# The config's own bind addrs are the default 8083/8093; rewrite them like the
# other instances so the port overrides above reach instance #1 too.
AUTH_CONFIG="$(mktemp "${TMPDIR:-/tmp}/authenticator-e2e-cfg.XXXXXX")"
sed -e "s/8083/$AUTH_PORT/g" -e "s/8093/$TOKEN_PORT/g" \
    -e 's#/tmp/authenticator-grpc#/tmp/authenticator-e2e1-grpc#' \
  services/authenticator/config/insight.yaml > "$AUTH_CONFIG"
# override_enabled exercises the `__override` view-as loop (e2e_override); the
# parameter is inert for every other test (nothing else sends it).
# external_id_claim=email: the realm users' `sub` is a Keycloak uuid, and the
# identity stub keys logins and `__override` targets by email so both resolve
# to the same person. The fast refresh lifecycle (margin 10 s, tick 1 s,
# jitter ±1 s) pairs with the realm's 15 s access-token lifespan
# (kc-realm-overlay.py), so a session's IdP tokens refresh every ~5 s.
APP__gears__authenticator__config__override_enabled=true \
APP__gears__authenticator__config__redis_url=redis://localhost:6399 \
APP__gears__authenticator__config__signing_keys_path="$KEYS_DIR" \
APP__gears__authenticator__config__identity_url="http://localhost:$IDENTITY_PORT" \
APP__gears__authenticator__config__gateway_issuer=http://localhost:8080 \
APP__gears__authenticator__config__idp__issuer_url="$ISSUER" \
APP__gears__authenticator__config__idp__client_id=insight-authenticator \
APP__gears__authenticator__config__idp__client_secret=insight-authenticator-dev-secret \
APP__gears__authenticator__config__idp__source_type=faketest \
APP__gears__authenticator__config__idp__external_id_claim=email \
APP__gears__authenticator__config__redirect_uri="http://localhost:$AUTH_PORT/auth/callback" \
APP__gears__authenticator__config__service_tokens__public_key_dir="$SVC_KEYS_DIR" \
APP__gears__authenticator__config__idp__refresh_safety_margin_seconds=10 \
APP__gears__authenticator__config__idp__refresh_due_jitter_seconds=1 \
APP__gears__authenticator__config__idp__refresher_tick_seconds=1 \
  ./target/release/authenticator -c "$AUTH_CONFIG" run \
  >/tmp/authenticator-e2e-auth.log 2>&1 &
pids+=($!)

echo "==> wait for authenticator readiness"
if ! wait_ready authenticator "http://localhost:$AUTH_PORT/.well-known/jwks.json"; then
  tail -20 /tmp/authenticator-e2e-auth.log >&2 || true
  exit 1
fi

echo "==> authenticator #2 :$AUTH2_PORT (override_enabled at its default: false)"
# Same stack (Redis/keys/IdP/identity), own ports + grpc socket. This instance
# proves the `__override` parameter is inert unless an environment opts in.
AUTH2_CONFIG="$(mktemp "${TMPDIR:-/tmp}/authenticator-e2e-cfg2.XXXXXX")"
sed -e "s/8083/$AUTH2_PORT/g" -e "s/8093/$TOKEN2_PORT/g" \
    -e 's#/tmp/authenticator-grpc#/tmp/authenticator-e2e2-grpc#' \
  services/authenticator/config/insight.yaml > "$AUTH2_CONFIG"
APP__gears__authenticator__config__redis_url=redis://localhost:6399 \
APP__gears__authenticator__config__signing_keys_path="$KEYS_DIR" \
APP__gears__authenticator__config__identity_url="http://localhost:$IDENTITY_PORT" \
APP__gears__authenticator__config__gateway_issuer=http://localhost:8080 \
APP__gears__authenticator__config__idp__issuer_url="$ISSUER" \
APP__gears__authenticator__config__idp__client_id=insight-authenticator \
APP__gears__authenticator__config__idp__client_secret=insight-authenticator-dev-secret \
APP__gears__authenticator__config__idp__source_type=faketest \
APP__gears__authenticator__config__idp__external_id_claim=email \
APP__gears__authenticator__config__redirect_uri="http://localhost:$AUTH2_PORT/auth/callback" \
APP__gears__authenticator__config__service_tokens__public_key_dir="$SVC_KEYS_DIR" \
  ./target/release/authenticator -c "$AUTH2_CONFIG" run \
  >/tmp/authenticator-e2e-auth2.log 2>&1 &
pids+=($!)

echo "==> wait for authenticator #2 readiness"
if ! wait_ready authenticator2 "http://localhost:$AUTH2_PORT/.well-known/jwks.json"; then
  tail -20 /tmp/authenticator-e2e-auth2.log >&2 || true
  exit 1
fi

echo "==> authenticator #3 :$AUTH3_PORT (host-keyed issuer map, ADR-0003)"
# Two hosts -> two realms of the one Keycloak; the flat idp client fields stay
# empty (map mode replaces them). The map rides ONE env var as a JSON string —
# exactly the shape a deployment would use.
AUTH3_CONFIG="$(mktemp "${TMPDIR:-/tmp}/authenticator-e2e-cfg3.XXXXXX")"
sed -e "s/8083/$AUTH3_PORT/g" -e "s/8093/$TOKEN3_PORT/g" \
    -e 's#/tmp/authenticator-grpc#/tmp/authenticator-e2e3-grpc#' \
  services/authenticator/config/insight.yaml > "$AUTH3_CONFIG"
APP__gears__authenticator__config__redis_url=redis://localhost:6399 \
APP__gears__authenticator__config__signing_keys_path="$KEYS_DIR" \
APP__gears__authenticator__config__identity_url="http://localhost:$IDENTITY_PORT" \
APP__gears__authenticator__config__gateway_issuer=http://localhost:8080 \
APP__gears__authenticator__config__idp__source_type=faketest \
APP__gears__authenticator__config__idp__external_id_claim=email \
APP__gears__authenticator__config__idp__hosts="{\"tenant-a.example\": {\"issuer_url\": \"$ISSUER\", \"client_id\": \"insight-authenticator\", \"client_secret\": \"insight-authenticator-dev-secret\"}, \"tenant-b.example\": {\"issuer_url\": \"$ISSUER_B\", \"client_id\": \"insight-authenticator\", \"client_secret\": \"insight-authenticator-dev-secret\"}}" \
APP__gears__authenticator__config__redirect_uri="http://localhost:$AUTH3_PORT/auth/callback" \
APP__gears__authenticator__config__service_tokens__public_key_dir="$SVC_KEYS_DIR" \
  ./target/release/authenticator -c "$AUTH3_CONFIG" run \
  >/tmp/authenticator-e2e-auth3.log 2>&1 &
pids+=($!)

echo "==> wait for authenticator #3 readiness"
if ! wait_ready authenticator3 "http://localhost:$AUTH3_PORT/.well-known/jwks.json"; then
  tail -20 /tmp/authenticator-e2e-auth3.log >&2 || true
  exit 1
fi

# Keycloak coordinates for the suites (tests/common/kc.rs): the login form
# password and the admin-API/docker seams for IdP-side events.
export E2E_KC_BASE="$KC_BASE"
export E2E_KC_REALM="$KC_REALM"
export E2E_KC_CONTAINER="$KC_CT"
export E2E_KC_ADMIN_USER="$KC_ADMIN_USER"
export E2E_KC_ADMIN_PASSWORD="$KC_ADMIN_PASSWORD"
export E2E_USER_PASSWORD=insight-dev

echo "==> run the login loop"
AUTH_BASE="http://localhost:$AUTH_PORT" E2E_USER="$E2E_USER" \
  cargo test -p authenticator --test e2e_login_loop -- --ignored --nocapture

echo "==> run the refresh rotation-with-grace loop (step 10.1)"
AUTH_BASE="http://localhost:$AUTH_PORT" E2E_USER="$E2E_USER" \
  cargo test -p authenticator --test e2e_refresh -- --ignored --nocapture

echo "==> run the session-management loop (step 10.2)"
AUTH_BASE="http://localhost:$AUTH_PORT" E2E_USER="$E2E_USER" \
  cargo test -p authenticator --test e2e_sessions -- --ignored --nocapture

echo "==> run the 401 contract for the session-cookie surface"
AUTH_BASE="http://localhost:$AUTH_PORT" \
  cargo test -p authenticator --test e2e_unauthorized -- --ignored --nocapture

echo "==> run the __override view-as loop (#1941)"
# Each test owns a disjoint {impersonator + targets} set of realm users
# (kc-realm-overlay.py), so the suite is safe under cargo's default parallel
# execution — one test's revoke-all can never reach a sibling's session.
AUTH_BASE="http://localhost:$AUTH_PORT" AUTH_BASE_DISABLED="http://localhost:$AUTH2_PORT" \
  cargo test -p authenticator --test e2e_override -- --ignored --nocapture

echo "==> run the host-keyed issuer map loop (ADR-0003)"
AUTH_BASE="http://localhost:$AUTH3_PORT" AUTH_FLAT_BASE="http://localhost:$AUTH_PORT" \
  AUTH3_LOG=/tmp/authenticator-e2e-auth3.log \
  E2E_IDP_ISSUER="$ISSUER" E2E_IDP2_ISSUER="$ISSUER_B" \
  E2E_USER="$E2E_USER" \
  cargo test -p authenticator --test e2e_hostmap -- --ignored --nocapture

echo "==> run the back-channel logout loop (step 10.3)"
AUTH_BASE="http://localhost:$AUTH_PORT" \
  cargo test -p authenticator --test e2e_backchannel -- --ignored --nocapture

echo "==> run the layer-2 rate-limit loop (step 10.6)"
AUTH_BASE="http://localhost:$AUTH_PORT" E2E_USER="$E2E_USER" \
  cargo test -p authenticator --test e2e_ratelimit -- --ignored --nocapture

echo "==> run the IdP background-refresher loop (step 10.4: outage + invalid_grant)"
AUTH_BASE="http://localhost:$AUTH_PORT" \
  cargo test -p authenticator --test e2e_refresher -- --ignored --nocapture

echo "==> run the service-token loop (step 06)"
# The token listener binds 8093 (config service_tokens.token_bind_addr); the dev
# `testclient` registry entry resolves public_key_paths against the generated
# SVC_KEYS_DIR set above, and the client signs with the matching private key.
AUTH_BASE="http://localhost:$AUTH_PORT" \
  TOKEN_ENDPOINT="http://localhost:$TOKEN_PORT/internal/token" \
  SVC_KEY="$SVC_KEYS_DIR/testclient.key.pem" \
  cargo test -p authenticator --test e2e_service_token -- --ignored --nocapture

echo "==> PASS (endpoint-coverage ledger: $E2E_COVERAGE_LEDGER)"
