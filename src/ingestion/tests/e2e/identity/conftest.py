"""Fixtures for the identity contract suite (`identity/test_*.py`, #1753).

The service under test is whatever `lib.identity.spawn` yields — the .NET
identity service today, the Rust identity-resolution port after the cutover,
or an external deployment via E2E_IDENTITY_URL. Tests never know which.

Caller identity matters here (unlike the analytics suite): the JWT `sub` claim
IS the caller's person_id, and the seeded dataset gives each fixture client a
distinct vantage point — `api` is alice (admin, sees her whole subtree),
`bob_api` is bob (non-admin, one explicit grant), `service_api` is a
service principal (the authenticator's S2S shape).
"""

from __future__ import annotations

import pytest

from lib import identity as identity_lib
from lib import identity_seed
from lib.config import SessionConfig

pytestmark = pytest.mark.identity


@pytest.fixture(scope="session")
def identity_svc(compose_stack: SessionConfig):
    """Provision the identity DB, boot the service, seed the fixture dataset."""
    with identity_lib.spawn(compose_stack) as svc:
        identity_seed.seed(compose_stack)
        yield svc


@pytest.fixture
def api(identity_svc):
    """Recording client authenticated as ALICE (tenant admin, subtree root)."""
    with identity_svc.client(sub=str(identity_seed.ALICE)) as c:
        yield c


@pytest.fixture
def bob_api(identity_svc):
    """Recording client authenticated as BOB (non-admin; sees own subtree +
    the explicit grant on HIDDEN)."""
    with identity_svc.client(sub=str(identity_seed.BOB)) as c:
        yield c


@pytest.fixture
def service_api(identity_svc):
    """Recording client with a SERVICE-principal token (sub_type=service) —
    the shape the authenticator uses for /internal/* lookups."""
    import httpx

    from lib import api_coverage

    token = identity_svc.auth.mint(
        str(identity_seed.OTHER_TENANT),  # tenant-agnostic endpoints; any real UUID
        sub=str(identity_seed.ALICE),
        sub_type="service",
        roles="service",
    )
    with httpx.Client(
        base_url=identity_svc.base_url,
        timeout=30.0,
        headers={"Authorization": f"Bearer {token}"},
        event_hooks={"response": [api_coverage.record_identity_response]},
    ) as c:
        yield c


@pytest.fixture
def anon_api(identity_svc):
    """Recording client with NO Authorization header (401 cases)."""
    import httpx

    from lib import api_coverage

    with httpx.Client(
        base_url=identity_svc.base_url,
        timeout=30.0,
        event_hooks={"response": [api_coverage.record_identity_response]},
    ) as c:
        yield c
