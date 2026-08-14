"""`GET /v1/persons` — the operator's person search over current observed values.

    GET /v1/persons   200 by email fragment · 400 no terms · 403 without the grant

Display ordering and truncation are proven in the service's own live tests,
where the journal can be staged to force them.

The picker behind bind/merge: an operator holding an account's email fragment
must find the person it currently belongs to, tenant-wide and WITHOUT the
visibility filter — the seeded operator is deliberately outside the org chart,
so a visibility-filtered search would answer them nobody, ever. What is worth
proving on the deployed path is exactly that pairing: the admin row opens a
tenant-wide view (`test_the_operator_finds_a_person_...`), and without the row
the surface refuses rather than shrinking to an empty result
(`test_without_the_admin_row_search_refuses`) — an empty 200 would read as
"person does not exist" to the UI, which then offers a wrong detach.

Match semantics (superseded values stop matching; a value two persons claim
returns both) are proven per-row in the service's own live tests, where the
journal can be staged; the stand's seeded journal is single-claim throughout.

The 401 half is in `test_gateway.py`, swept over every operation.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, identity_path

from ..schemas import PersonListResponse

PERSONS = identity_path("/v1/persons")


@pytest.mark.security
def test_an_unauthenticated_caller_never_reaches_any_of_this(api_client: ApiClient) -> None:
    """Proven per operation by `test_gateway.py`; spot-checked here so this
    module carries its own reason for using a session at all."""
    response = api_client.get(PERSONS, params={"q": "anything"})
    assert response.status_code == 401, f"{response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
def test_the_operator_finds_a_person_by_an_email_fragment(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """Tenant-wide search, not visibility-scoped: the operator sees nobody in
    `/v1/subchart` (`test_admin.py` keeps that true) yet must find any person
    here, or the picker is useless to the one persona allowed to use it.
    """
    lead = stand_manifest.fixture("dev_lead")
    fragment = lead.email.split("@")[0]

    response = admin_operator_session.client.get(PERSONS, params={"q": fragment})

    assert response.status_code == 200, f"{response.status_code} {response.text[:300]}"
    listing = response.parse(PersonListResponse)
    matches = [str(item.person_id) for item in listing.items]
    assert lead.uuid in matches, f"{fragment!r} did not find {lead.email}: {matches}"
    assert listing.next_cursor is None

    found = next(item for item in listing.items if str(item.person_id) == lead.uuid)
    assert found.email == lead.email, "the card carries the current email"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_search_without_terms_is_a_client_error(
    admin_operator_session: PersonaSession,
) -> None:
    """`q` is required: an admin surface must never dump the whole tenant
    because a UI raced its debounce and sent an empty query."""
    for params in (None, {"q": " "}):
        response = admin_operator_session.client.get(PERSONS, params=params)
        assert response.status_code == 400, (
            f"q={params!r} answered {response.status_code}: {response.text[:300]}"
        )


@pytest.mark.requires_seed("ceo")
@pytest.mark.security
def test_without_the_admin_row_search_refuses(
    realm_admin_session: PersonaSession,
) -> None:
    """403, not an empty 200 — the CEO holds the realm role and no
    `person_roles` row, and a silent empty listing would read to the UI as
    "no such person" rather than "you may not search"."""
    response = realm_admin_session.client.get(PERSONS, params={"q": "anything"})
    assert response.status_code == 403, f"{response.status_code} {response.text[:300]}"
