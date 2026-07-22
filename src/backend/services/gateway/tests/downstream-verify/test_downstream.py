"""Downstream-verification e2e -- the NGINX_BFF §D scenarios.

The R1 rule as code: every downstream service verifies the gateway JWT itself,
fail-closed, no disable knob. A request without a valid gateway JWT gets 401 no
matter how it arrived. The gateway JWT carries a single signed `tenant_id` (the
sole tenant authority); a token missing it is rejected. Run: run-e2e.sh (or
`pytest` from this dir).
"""

from __future__ import annotations

import base64
import json
import time
import uuid

from conftest import ANALYTICS_DIRECT, GW, IDENTITY_DIRECT, mint_gateway_jwt, mint_service_token

# Analytics is addressed directly under the gateway prefix; strip_prefix turns
# `/api/analytics/v1/metrics` into the service's own `/v1/metrics`.
ANALYTICS_VIA_GW = f"{GW}/api/analytics/v1/metrics"
ANALYTICS_LIST_DIRECT = f"{ANALYTICS_DIRECT}/v1/metrics"
IDENTITY_LOOKUP_DIRECT = f"{IDENTITY_DIRECT}/v1/persons/nobody@example.com"

TENANT_DEV = "00000000-df51-5b42-9538-d2b56b7ee953"  # the dev user's tenant
GATEWAY_ISSUER = "https://authn-tls:8443"  # = authenticator gateway_issuer


def _claims(bearer_jwt: str) -> dict:
    payload = bearer_jwt.split(".")[1]
    payload += "=" * (-len(payload) % 4)
    return json.loads(base64.urlsafe_b64decode(payload))


# ── Scenario 1: login -> cookie -> GET /api/analytics/... -> 200 ──────────────


def test_login_then_analytics_returns_200(client):
    sid = client.login()
    status, _, _ = client.request(ANALYTICS_VIA_GW, headers={"Cookie": f"__Host-sid={sid}"})
    assert status == 200, f"authenticated analytics call got {status}"


# ── Scenario 2: direct to the service port without a JWT -> 401 (R1 proof) ────


def test_analytics_direct_without_jwt_is_401(client):
    status, _, _ = client.request(ANALYTICS_LIST_DIRECT)
    assert status == 401, f"analytics direct/no-JWT got {status}"


def test_identity_direct_without_jwt_is_401(client):
    # The other downstream service verifies too — fail-closed with no JWT.
    status, _, _ = client.request(IDENTITY_LOOKUP_DIRECT)
    assert status == 401, f"identity direct/no-JWT got {status}"


# ── Scenario 3: a validly-signed token missing `tenant_id` -> 401 ─────────────


def test_analytics_valid_signature_missing_tenant_is_401(client):
    # Forge a token signed by the real gateway key (correct kid) but WITHOUT a
    # `tenant_id` claim: the signature/iss/aud all check out, yet the plugin
    # requires `subject_tenant_id` and rejects it — "no tenant, no auth", no Nil
    # fallback.
    now = int(time.time())
    token = mint_gateway_jwt(
        {
            "sub": str(uuid.uuid4()),
            "roles": "analyst",  # space-delimited (OAuth scope) shape
            "sub_type": "user",
            "iss": GATEWAY_ISSUER,
            "aud": "internal-services",
            "iat": now,
            "exp": now + 300,
            "jti": str(uuid.uuid4()),
        }
    )
    status, _, _ = client.request(ANALYTICS_LIST_DIRECT, headers={"Authorization": f"Bearer {token}"})
    assert status == 401, f"token missing tenant_id got {status}, expected 401"


# ── Scenario 4: service token -> accepted, carries the service subject ────────


def test_service_token_accepted_with_service_role(client):
    token = mint_service_token(TENANT_DEV)
    claims = _claims(token)
    # Service-to-service calls go direct (the browser gateway would replace the
    # Authorization header). The token itself carries the identity analytics sees.
    assert claims["sub_type"] == "service", "service token must carry sub_type=service"
    assert "service" in claims["roles"].split(), "service token must carry the service role"
    assert uuid.UUID(claims["sub"]), "service token sub must be a UUID (not service:<name>)"
    assert claims["tenant_id"] == TENANT_DEV, "service token must carry the requested tenant"
    status, _, _ = client.request(ANALYTICS_LIST_DIRECT, headers={"Authorization": f"Bearer {token}"})
    assert status == 200, f"analytics rejected the service token: {status}"


# ── Scenario 5: a request reaching analytics without a valid gateway JWT -> 401 ─


def test_gateway_misconfig_or_forged_token_is_401(client):
    # Models a gateway route accidentally shipped without `auth_request`: the
    # request reaches analytics carrying only what the browser sent (a session
    # cookie and/or a stray bearer), never a gateway-minted JWT. Analytics
    # fail-closes -> 401 (an availability bug, never a breach). Recorded as a CI
    # regression: the downstream check is the real security boundary.
    status, _, _ = client.request(
        ANALYTICS_LIST_DIRECT,
        headers={"Cookie": "__Host-sid=whatever", "Authorization": "Bearer forged.not-a.gateway-jwt"},
    )
    assert status == 401, f"forged/browser token reached analytics as {status}, expected 401"
