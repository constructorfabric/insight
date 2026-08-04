#!/usr/bin/env bash
# ADR-0003 broker PoC (#2194) — headless end-to-end proof.
#
# Drives the full brokered login with no shortcut at any step:
#   GET /auth/login                      authenticator starts code+PKCE, 302 to the BROKER realm
#   broker authorize                     auto-redirects (identity-provider-redirector) to the upstream realm
#   upstream login form                  real username/password POST
#   upstream -> broker/.../endpoint      code returns to the broker, first-broker-login runs,
#                                        the hardcoded-attribute mapper stamps tenant_id
#   broker -> /auth/callback             the UNCHANGED authenticator exchanges the broker's code,
#                                        resolves the person via Identity, sets __Host-sid
#   GET /internal/authz                  the gateway auth_request target returns X-Gateway-Jwt
#
# PASS = the gateway JWT carries a single string `tenant_id`.
#
# usage: verify.sh [gateway-base] [email] [password]
set -euo pipefail

BASE="${1:-http://localhost:8080}"
ENV_FILE="${ENV_FILE:-$(cd "$(dirname "$0")/../../../.." && pwd)/.env.compose}"
DEFAULT_EMAIL="$(grep -E '^DEV_USER_EMAIL=' "$ENV_FILE" 2>/dev/null | tail -1 | cut -d= -f2- || true)"
EMAIL="${2:-${DEFAULT_EMAIL:?no email given and DEV_USER_EMAIL not in $ENV_FILE}}"
PASS="${3:-${POC_UPSTREAM_USER_PASSWORD:-insight-dev}}"
EXPECTED_TENANT="$(grep -E '^TENANT_DEFAULT_ID=' "$ENV_FILE" 2>/dev/null | tail -1 | cut -d= -f2- || true)"
EXPECTED_TENANT="${EXPECTED_TENANT:-00000000-df51-5b42-9538-d2b56b7ee953}"

JAR="$(mktemp)"; trap 'rm -f "$JAR"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
location_of() { tr -d '\r' | awk 'tolower($1)=="location:"{print $2}'; }

# 1. the authenticator must send the browser to the BROKER realm
AUTHZ_URL=$(curl -sS -o /dev/null -D - "$BASE/auth/login" | location_of)
[[ -n "$AUTHZ_URL" ]] || fail "/auth/login did not redirect (stack not up?)"
case "$AUTHZ_URL" in
  */realms/insight-broker/*) echo "ok: login starts at the broker realm" ;;
  *) fail "authorize URL is not the broker realm: $AUTHZ_URL (run poc-up.sh first)" ;;
esac

# 2. follow redirects to the upstream login form; the chain must traverse the
#    brokering hop (single-IdP auto-redirect, no broker login page)
URL="$AUTHZ_URL" PAGE="" SAW_UPSTREAM_AUTHORIZE=false
for _ in 1 2 3 4 5 6; do
  HDRS="$(mktemp)"
  PAGE=$(curl -sS -c "$JAR" -b "$JAR" -D "$HDRS" "$URL")
  CODE=$(awk 'NR==1{print $2}' "$HDRS"); LOC=$(location_of < "$HDRS"); rm -f "$HDRS"
  [[ "$CODE" == 200 ]] && break
  [[ -n "$LOC" ]] || fail "HTTP $CODE with no Location at $URL"
  case "$LOC" in */realms/poc-upstream/protocol/openid-connect/auth*) SAW_UPSTREAM_AUTHORIZE=true ;; esac
  URL="$LOC"
done
$SAW_UPSTREAM_AUTHORIZE || fail "broker did not auto-redirect to the upstream IdP"
echo "ok: broker auto-redirected to the upstream authorize endpoint"

ACTION=$(printf '%s' "$PAGE" | grep -o 'action="[^"]*login-actions/authenticate[^"]*"' | head -1 \
  | sed 's/^action="//; s/"$//; s/\&amp;/\&/g')
[[ -n "$ACTION" ]] || fail "no upstream login form at $URL"
case "$ACTION" in
  */realms/poc-upstream/*) echo "ok: credentials go to the UPSTREAM realm's form" ;;
  *) fail "login form is not the upstream realm's: $ACTION" ;;
esac

# 3. submit credentials; walk redirects until the code lands at /auth/callback.
#    The chain must pass through the broker's endpoint (the brokered exchange).
NEXT=$(curl -sS -c "$JAR" -b "$JAR" -o /dev/null -D - \
  --data-urlencode "username=$EMAIL" --data-urlencode "password=$PASS" \
  "$ACTION" | location_of)
[[ -n "$NEXT" ]] || fail "upstream rejected the credentials for $EMAIL"
SAW_BROKER_ENDPOINT=false
for _ in 1 2 3 4 5 6 7 8; do
  case "$NEXT" in
    */realms/insight-broker/broker/poc-upstream/endpoint*) SAW_BROKER_ENDPOINT=true ;;
    */auth/callback\?*) break ;;
  esac
  NEXT=$(curl -sS -c "$JAR" -b "$JAR" -o /dev/null -D - "$NEXT" | location_of)
  [[ -n "$NEXT" ]] || fail "redirect chain ended before /auth/callback"
done
$SAW_BROKER_ENDPOINT || fail "the code never traversed the broker endpoint"
echo "ok: upstream code returned via the broker endpoint, broker code issued to the authenticator"

# 4. deliver the code at the gateway origin; capture __Host-sid by hand (it is
#    Secure, so curl stores but will not replay it over plain http)
SID=$(curl -sS -c "$JAR" -b "$JAR" -o /dev/null -D - "$BASE/${NEXT#*://*/}" | tr -d '\r' \
  | awk 'tolower($1)=="set-cookie:"{print $2}' | grep '^__Host-sid=' | cut -d';' -f1 | cut -d= -f2)
[[ -n "$SID" ]] || fail "the authenticator's callback set no __Host-sid"
echo "ok: unchanged authenticator exchanged the broker's code and opened a session"

# 5. session works through the gateway
ME=$(curl -sS -H "Cookie: __Host-sid=$SID" "$BASE/auth/me")
printf 'ok: /auth/me -> %s\n' "$ME"

# 6. the minted gateway JWT carries the single string tenant_id
JWT=$(curl -sS -o /dev/null -D - -H "Cookie: __Host-sid=$SID" "http://localhost:8083/internal/authz" \
  | tr -d '\r' | awk 'tolower($1)=="x-gateway-jwt:"{print $3}')
[[ -n "$JWT" ]] || fail "/internal/authz returned no X-Gateway-Jwt"
printf '%s' "$JWT" | cut -d. -f2 | EXPECTED_TENANT="$EXPECTED_TENANT" python3 -c '
import base64, json, os, sys
raw = sys.stdin.read().strip()
claims = json.loads(base64.urlsafe_b64decode(raw + "=" * (-len(raw) % 4)))
print("gateway JWT claims:", json.dumps(claims, sort_keys=True))
tid = claims.get("tenant_id")
assert isinstance(tid, str) and tid, f"tenant_id must be a single string, got {tid!r}"
expected = os.environ["EXPECTED_TENANT"]
assert tid == expected, f"tenant_id {tid!r} != expected {expected!r}"
print(f"PASS: gateway JWT carries the single string tenant_id = {tid}")
'
