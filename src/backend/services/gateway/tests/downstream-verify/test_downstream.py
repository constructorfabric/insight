from __future__ import annotations

import base64
import json
import time
import uuid

from conftest import ANALYTICS_DIRECT, GW, IDENTITY_DIRECT, mint_gateway_jwt, mint_service_token

ANALYTICS_VIA_GW = f"{GW}/api/analytics/v1/metric-definitions"
ANALYTICS_LIST_DIRECT = f"{ANALYTICS_DIRECT}/v1/metric-definitions"
IDENTITY_LIST_DIRECT = f"{IDENTITY_DIRECT}/v1/roles"

TENANT_DEV = "00000000-df51-5b42-9538-d2b56b7ee953"
GATEWAY_ISSUER = "https://authn-tls:8443"


def _claims(bearer_jwt: str) -> dict:
    payload = bearer_jwt.split(".")[1]
    payload += "=" * (-len(payload) % 4)
    return json.loads(base64.urlsafe_b64decode(payload))


def test_login_then_analytics_returns_200(client):
    sid = client.login()
    status, _, _ = client.request(ANALYTICS_VIA_GW, headers={"Cookie": f"__Host-sid={sid}"})
    assert status == 200, f"authenticated analytics call got {status}"


def test_analytics_direct_without_jwt_is_401(client):
    status, _, _ = client.request(ANALYTICS_LIST_DIRECT)
    assert status == 401, f"analytics direct/no-JWT got {status}"


def test_identity_direct_without_jwt_is_401(client):
    status, _, _ = client.request(IDENTITY_LIST_DIRECT)
    assert status == 401, f"identity direct/no-JWT got {status}"


def test_analytics_valid_signature_missing_tenant_is_401(client):
    now = int(time.time())
    token = mint_gateway_jwt(
        {
            "sub": str(uuid.uuid4()),
            "roles": "analyst",
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


def test_service_token_accepted_with_service_role(client):
    token = mint_service_token(TENANT_DEV)
    claims = _claims(token)
    assert claims["sub_type"] == "service", "service token must carry sub_type=service"
    assert "service" in claims["roles"].split(), "service token must carry the service role"
    assert uuid.UUID(claims["sub"]), "service token sub must be a UUID (not service:<name>)"
    assert claims["tenant_id"] == TENANT_DEV, "service token must carry the requested tenant"
    status, _, _ = client.request(ANALYTICS_LIST_DIRECT, headers={"Authorization": f"Bearer {token}"})
    assert status == 200, f"analytics rejected the service token: {status}"


def test_gateway_misconfig_or_forged_token_is_401(client):
    status, _, _ = client.request(
        ANALYTICS_LIST_DIRECT,
        headers={"Cookie": "__Host-sid=whatever", "Authorization": "Bearer forged.not-a.gateway-jwt"},
    )
    assert status == 401, f"forged/browser token reached analytics as {status}, expected 401"
