"""`GET /v1/persons` — the operator's person listing over current observed values.

    GET /v1/persons   200 by email fragment · 200 whole roster · 400 stale cursor
                      · 403 without the grant

Display ordering is proven in the service's own live tests, where the journal
can be staged to force it. What the deployed path adds is that paging over it
is sound: a page boundary that repeated or skipped a person would be invisible
in a unit test and ruinous in a console an operator trusts to show everyone.

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


@pytest.mark.requires_seed("admin_operator", "dev_lead")
@pytest.mark.reliability
def test_a_query_without_terms_lists_the_roster(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """An operator reviewing identities has to see who exists, so a blank `q`
    lists the tenant rather than refusing. What used to make refusing right —
    that an admin surface must never dump the whole tenant — is now carried by
    the page: the answer is bounded whether or not anyone typed anything.
    """
    lead = stand_manifest.fixture("dev_lead")

    for params in (None, {"q": " "}):
        response = admin_operator_session.client.get(PERSONS, params=params)
        assert response.status_code == 200, (
            f"q={params!r} answered {response.status_code}: {response.text[:300]}"
        )
        listing = response.parse(PersonListResponse)
        assert listing.items, f"q={params!r} listed nobody"
        assert len(listing.items) <= 20, "the default page is the bound"

    everyone = _walk(admin_operator_session, {}, pages=10)
    assert lead.uuid in everyone, "the roster must reach a seeded person"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_next_page_neither_repeats_nor_skips_a_person(
    admin_operator_session: PersonaSession,
) -> None:
    """The boundary is the whole point of a cursor. Walked one row at a time,
    the pages must partition the roster: a repeat means the operator sees a
    person twice, a skip means somebody is unreachable by browsing."""
    one_at_a_time = _walk(admin_operator_session, {"limit": 1}, pages=6)

    # Without this the test passes on a build that issues no cursor at all: the
    # walk would stop after one row, and one row never repeats itself.
    assert len(one_at_a_time) > 1, (
        "the walk never left the first page — the listing offered no cursor"
    )
    assert len(one_at_a_time) == len(set(one_at_a_time)), (
        f"a person appeared on two pages: {one_at_a_time}"
    )

    whole_page = admin_operator_session.client.get(PERSONS, params={"limit": 6})
    listed = [str(item.person_id) for item in whole_page.parse(PersonListResponse).items]
    assert one_at_a_time == listed[: len(one_at_a_time)], (
        "walking one row at a time must retrace the same order, in the same places"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_cursor_issued_for_another_query_is_refused(
    admin_operator_session: PersonaSession,
) -> None:
    """Resuming a narrowed search where a wider one left off would skip
    everyone before that position — silently, and only for the operator who
    typed one more letter. The service refuses instead."""
    first = admin_operator_session.client.get(PERSONS, params={"limit": 1})
    cursor = first.parse(PersonListResponse).next_cursor
    assert cursor, "a one-row page of the roster must offer a next page"

    resumed = admin_operator_session.client.get(
        PERSONS, params={"limit": 1, "q": "nobody-by-this-name", "cursor": cursor}
    )
    assert resumed.status_code == 400, f"{resumed.status_code} {resumed.text[:300]}"


def _walk(session: PersonaSession, params: dict[str, object], *, pages: int) -> list[str]:
    """Person ids in the order the listing serves them, following the cursor."""
    found: list[str] = []
    cursor: str | None = None
    for _ in range(pages):
        page = session.client.get(
            PERSONS, params={**params, **({"cursor": cursor} if cursor else {})}
        )
        assert page.status_code == 200, f"{page.status_code} {page.text[:300]}"
        listing = page.parse(PersonListResponse)
        found.extend(str(item.person_id) for item in listing.items)
        cursor = listing.next_cursor
        if not cursor:
            break
    return found


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
