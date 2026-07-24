"""Contract: GET /internal/persons/by-email/{email} — the login-bootstrap S2S
lookup. Restricted to SERVICE principals (JWT sub_type=service); the tenant is
deliberately ignored (at login the tenant is not yet known)."""

from __future__ import annotations

import pytest

from lib import identity_seed as seed

pytestmark = pytest.mark.identity


def test_by_email_200_service_token(service_api) -> None:
    """The resolved person comes back as a source-descriptor quadruple
    (insight_source_type='person', insight_source_id=<person uuid>) — the
    shape the authenticator's IdentityPersonResolver consumes."""
    r = service_api.get(f"/internal/persons/by-email/{seed.ALICE_EMAIL}")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    body = r.json()
    assert body["insight_source_type"] == "person", body
    assert body["insight_source_id"] == str(seed.ALICE), body
    assert body["value"] == seed.ALICE_EMAIL, body
    assert body["value_type"] == "email", body


def test_by_email_404_unknown(service_api) -> None:
    r = service_api.get(f"/internal/persons/by-email/{seed.UNKNOWN_EMAIL}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_by_email_403_user_token(api) -> None:
    """A regular user principal must not reach the S2S surface."""
    r = api.get(f"/internal/persons/by-email/{seed.ALICE_EMAIL}")
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_by_email_401_unauthenticated(anon_api) -> None:
    assert anon_api.get(f"/internal/persons/by-email/{seed.ALICE_EMAIL}").status_code == 401


def test_health_endpoints_public(anon_api) -> None:
    """/health + /healthz answer 200 with no auth (probe surface)."""
    assert anon_api.get("/health").status_code == 200
    assert anon_api.get("/healthz").status_code == 200
