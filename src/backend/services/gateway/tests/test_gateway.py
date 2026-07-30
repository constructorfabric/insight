"""Gateway e2e -- the five NGINX_BFF step-05 scenarios, top-to-bottom.

Run: src/backend/services/gateway/tests/run-e2e.sh  (or `pytest` from this dir).
The compose stack + fail-closed lifecycle live in conftest.py.
"""

from __future__ import annotations

import base64
import json
import re
import time

from conftest import AUTHENTICATOR, AUTHZ_CACHE_MAX_AGE, GW

ROUTES = ["/api/analytics", "/api/identity"]
UUID_V7 = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")

UNAUTHENTICATED_TYPE = "gts://gts.cf.core.errors.err.v1~cf.core.err.unauthenticated.v1~"
SERVICE_UNAVAILABLE_TYPE = "gts://gts.cf.core.errors.err.v1~cf.core.err.service_unavailable.v1~"


def _b64url_json(segment):
    segment += "=" * (-len(segment) % 4)
    return json.loads(base64.urlsafe_b64decode(segment))


# ── Scenario 1: no cookie -> 401 on every route, canonical problem body ───────


def test_no_cookie_401_on_every_route(client):
    for route in ROUTES:
        status, headers, _ = client.request(f"{GW}{route}/x")
        assert status == 401, f"{route}: got {status}"
        assert "www-authenticate" in headers, route


def test_401_body_is_canonical_problem(client):
    _, headers, body = client.request(f"{GW}/api/analytics/x")
    assert "problem+json" in headers.get("content-type", "")
    prob = json.loads(body)
    assert prob["type"] == UNAUTHENTICATED_TYPE
    assert prob["title"] == "Unauthenticated"
    assert prob["status"] == 401


# ── Scenario 2: login -> JWT injected, session cookie stripped, R3 hygiene ────


def test_login_injects_verifiable_jwt_and_applies_hygiene(client):
    sid = client.login()
    resp = client.request(
        f"{GW}/api/analytics/data",
        headers={
            "Cookie": f"__Host-sid={sid}; keep=1",
            "Authorization": "Bearer FORGED",
            "X-Correlation-Id": "forged-corr",
        },
    )
    status, _, body = resp
    assert status == 200, f"got {status}"
    echoed = json.loads(body)["headers"]

    auth = echoed.get("authorization", "")
    assert auth.startswith("Bearer ") and auth != "Bearer FORGED", auth
    header, claims = auth.split(" ", 1)[1].split(".")[:2]
    assert _b64url_json(header)["alg"] == "ES256"
    assert "sub" in _b64url_json(claims)

    assert "__Host-sid" not in echoed.get("cookie", "")
    assert "keep=1" in echoed.get("cookie", "")

    corr = echoed.get("x-correlation-id", "")
    assert corr and corr != "forged-corr"
    assert UUID_V7.match(corr), corr


def test_correlation_ids_are_unique_per_request(client):
    sid = client.login()
    seen = set()
    for _ in range(3):
        _, _, body = client.request(f"{GW}/api/analytics/data", headers={"Cookie": f"__Host-sid={sid}"})
        seen.add(json.loads(body)["headers"].get("x-correlation-id"))
    assert len(seen) == 3, seen


def test_jwks_served_by_authenticator_not_gateway(client):
    # JWKS is public and served directly by the authenticator (the key issuer).
    status, _, body = client.request(f"{AUTHENTICATOR}/.well-known/jwks.json")
    assert status == 200, f"got {status}"
    assert "keys" in json.loads(body)


# ── Scenario 5: SPA passthrough + internal surface ────────────────────────────


def test_spa_passthrough_on_root(client):
    status, _, body = client.request(f"{GW}/")
    assert status == 200 and json.loads(body)["path"] == "/"


def test_internal_and_unmatched_api_are_404(client):
    assert client.request(f"{GW}/internal/anything")[0] == 404
    assert client.request(f"{GW}/api/v1/nope")[0] == 404


# ── Scenario 3/4: authenticator down -> cache serves hits, cold cookie 503 ────


class TestAuthenticatorDown:
    def test_cached_cookie_still_served(self, client, session_sid, authenticator_down):
        status, _, _ = client.request(f"{GW}/api/analytics/x", headers={"Cookie": f"__Host-sid={session_sid}"})
        assert status == 200, f"got {status}"

    def test_cold_cookie_fails_closed(self, client, authenticator_down):
        status, headers, body = client.request(
            f"{GW}/api/analytics/x", headers={"Cookie": "__Host-sid=cold-never-seen"}
        )
        assert status == 503, f"got {status}"
        assert "retry-after" in headers
        prob = json.loads(body)
        assert prob["type"] == SERVICE_UNAVAILABLE_TYPE
        assert prob["context"]["retry_after_seconds"] == 5


# ── Scenario 3: revocation takes effect within the cache window ────────────────


def test_revocation_within_cache_window(client, session_sid):
    logout = client.request(f"{GW}/auth/logout", headers={"Cookie": f"__Host-sid={session_sid}"}, method="POST")
    assert logout[0] == 200, f"logout got {logout[0]}"
    # Revocation reaches the gateway once the cached exchange expires (<= max-age).
    deadline = time.monotonic() + AUTHZ_CACHE_MAX_AGE + 5
    status = None
    while time.monotonic() < deadline:
        status, _, _ = client.request(f"{GW}/api/analytics/x", headers={"Cookie": f"__Host-sid={session_sid}"})
        if status == 401:
            break
        time.sleep(1)
    assert status == 401, f"revoked session still {status} after {AUTHZ_CACHE_MAX_AGE + 5}s"


# ── Scenario 4: dead upstream -> 502 ──────────────────────────────────────────


def test_dead_upstream_5xx(client, echo_down):
    sid = client.login()  # exchange succeeds (authenticator up); echo is down
    status, _, _ = client.request(f"{GW}/api/analytics/x", headers={"Cookie": f"__Host-sid={sid}"})
    # Connection refused -> 502; unroutable (killed container) -> 504 at the
    # bounded proxy_connect_timeout. Either is the fail-fast dead-upstream path.
    assert status in (502, 504), f"got {status}"
