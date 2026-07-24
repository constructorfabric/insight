"""Contract: GET /v1/persons/{email} — the DEPRECATED person lookup.

Still served (and still in the spec universe) while any legacy caller might
exist; POST /v1/profiles is the successor. The response carries the
deprecation headers pointing at it. KNOWN DIVERGENCE candidate: the Rust
port dropped this endpoint (no remaining callers) — when the cutover lands,
the spec universe changes with it and this module goes away.
"""

from __future__ import annotations

import pytest

from lib import identity_seed as seed

pytestmark = pytest.mark.identity


@pytest.fixture(autouse=True)
def _legacy_dotnet_only(identity_svc) -> None:
    """Capability of the EXPLICIT implementation selection — never probed
    from runtime behavior (a probe would turn a real .NET 404 regression into
    a skip, and would land the fallback 404 in the coverage ledger as fake
    proof the route exists). Skips BEFORE any HTTP request."""
    if not identity_svc.supports_deprecated_person_lookup:
        pytest.skip(
            "GET /v1/persons/{email} is .NET-only (approved removal in the "
            "Rust successor; zero callers)"
        )


def test_deprecated_person_lookup_200(api) -> None:
    r = api.get(f"/v1/persons/{seed.BOB_EMAIL}")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    p = r.json()
    assert p["person_id"] == str(seed.BOB)
    assert p["email"] == seed.BOB_EMAIL
    # Successor pointer per RFC 8594-style deprecation headers.
    assert r.headers.get("deprecation"), dict(r.headers)


def test_deprecated_person_lookup_404_unknown(api) -> None:
    r = api.get(f"/v1/persons/{seed.UNKNOWN_EMAIL}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"
