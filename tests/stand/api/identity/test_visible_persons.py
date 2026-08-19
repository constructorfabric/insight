"""`/v1/visible-persons` — what the caller may see, filtered and enumerated.

    POST /v1/visible-persons   200 · 415 wrong-ct
    GET  /v1/visible-persons   200 · 200 paged · 200 searched · 400 bad cursor

The one identity route that answers a question ABOUT visibility rather than
being governed by it, which makes it the sharpest place to state the rule: the
same caller, the same request, and membership decides each email separately.

Two filters run, and the test covers both. Ids resolve to persons within the
caller's TENANT, so somebody in another tenant is not a candidate at all; the
survivors are then narrowed to what the caller can see in the org chart. A
regression in either one leaks a different thing — the first would disclose that
an id exists somewhere in the product, the second who reports to whom — so the
assertion names both an out-of-tenant and an out-of-scope person rather than
treating "not visible" as one bucket.

The tenant half got sharper in #2098: a wildcard grant used to echo the request
back, so a wildcard holder in tenant A could have tenant B's ids confirmed. It
is intersected with the tenant's persons log now, which is exactly what the
other-tenant case below asserts.

The visible/out-of-scope pair is the same one `test_subchart.py` establishes
(`development_ic` in, `sales_ic` out), so the two routes are held to one story
about the seeded org rather than each inventing its own.

The GET answers "who" where the POST answers "which of these", over one visible
set. So the same pair decides both: a person the filter refuses to confirm must
not appear in the enumeration either, which is the case that would catch an
enumeration built on its own rule.
"""

from __future__ import annotations

import uuid

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, identity_path

from ..schemas import VisiblePersons, VisiblePersonsPage

#: `visible_persons.rs::MAX_PERSON_IDS` — one bound parameter per id.
_MAX_PERSON_IDS = 1000

VISIBLE_PERSONS = identity_path("/v1/visible-persons")

#: Nobody holds this. Present so the answer is shown to drop an id it cannot
#: resolve, rather than echoing back whatever it was handed — which is what a
#: wildcard grant did before #2098.
UNKNOWN_PERSON_ID = "01900000-0000-7000-8000-000000000000"


@pytest.mark.requires_seed("dev_lead", "development_ic", "sales_ic", "other_tenant_lead")
@pytest.mark.security
def test_only_the_people_the_caller_may_see_come_back(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """One request, five emails, and each one decided on its own merits.

    Asserting the whole partition rather than a single membership: a route that
    returned everything it was given, or nothing, would satisfy any one-sided
    check, and both are plausible failures for a filter.
    """
    self_ = lead_session.person.uuid
    report = stand_manifest.fixture("development_ic").uuid
    outsider = stand_manifest.fixture("sales_ic").uuid
    other_tenant = stand_manifest.fixture("other_tenant_lead").uuid

    response = lead_session.client.post(
        VISIBLE_PERSONS,
        json_body={"person_ids": [self_, report, outsider, other_tenant, UNKNOWN_PERSON_ID]},
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    visible = {str(person_id) for person_id in response.parse(VisiblePersons).visible}
    assert {self_, report} <= visible, (
        f"a lead cannot see themselves or their own report: {sorted(visible)}"
    )
    assert outsider not in visible, f"{outsider} is outside the lead's org scope"
    assert other_tenant not in visible, (
        f"{other_tenant} belongs to another tenant and is not even a candidate — "
        "returning them would cross the tenant boundary, not merely widen a scope"
    )
    assert UNKNOWN_PERSON_ID not in visible, "an unresolvable id was echoed back"


@pytest.mark.reliability
def test_visible_persons_415_wrong_content_type(api: ApiClient) -> None:
    """A body refused on its media type, not parsed."""
    response = api.post(
        VISIBLE_PERSONS, content='{"person_ids":[]}', headers={"Content-Type": "text/plain"}
    )
    assert response.status_code == 415, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.parametrize(
    ("person_ids", "case"),
    [
        ([], "empty list"),
        ([str(uuid.UUID(int=0))], "only the nil uuid"),
    ],
)
@pytest.mark.reliability
def test_a_request_naming_nobody_is_a_400(api: ApiClient, person_ids: list[str], case: str) -> None:
    """A request that resolves to no id at all is a client error: answering 200
    with an empty `visible` would read to the caller as `nothing you asked for
    is visible`, which is a different fact."""
    response = api.post(VISIBLE_PERSONS, json_body={"person_ids": person_ids})
    assert response.status_code == 400, (
        f"should reject {case}: status={response.status_code} {response.text[:300]}"
    )


@pytest.mark.reliability
def test_more_ids_than_the_cap_is_a_400(api: ApiClient) -> None:
    """The request bounds the query — one bound parameter per id. The cap
    matches the analytics metric-results cap, which forwards a cleared request
    here whole."""
    over_cap = [str(uuid.uuid4()) for _ in range(_MAX_PERSON_IDS + 1)]

    response = api.post(VISIBLE_PERSONS, json_body={"person_ids": over_cap})
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("dev_lead", "development_ic", "sales_ic", "other_tenant_lead")
@pytest.mark.security
def test_the_roster_enumerates_exactly_what_the_filter_would_confirm(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """The enumeration obeys the boundary the filter obeys.

    Held to the same seeded pair as the POST case above: an enumeration that
    widened the set, or skipped the tenant intersection, would disclose who
    exists rather than merely who reports to whom.
    """
    report = stand_manifest.fixture("development_ic").uuid
    outsider = stand_manifest.fixture("sales_ic").uuid
    other_tenant = stand_manifest.fixture("other_tenant_lead").uuid

    response = lead_session.client.get(VISIBLE_PERSONS, params={"limit": 500})
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    listed = {str(item.person_id) for item in response.parse(VisiblePersonsPage).items}

    assert {lead_session.person.uuid, report} <= listed, (
        f"a lead must find themselves and their report: {sorted(listed)}"
    )
    assert outsider not in listed, f"{outsider} is outside the lead's org scope"
    assert other_tenant not in listed, (
        f"{other_tenant} belongs to another tenant and is not a candidate at all"
    )


@pytest.mark.requires_seed("dev_lead", "development_ic")
@pytest.mark.reliability
def test_a_cursor_walks_the_roster_without_repeating_or_skipping(
    lead_session: PersonaSession,
) -> None:
    """Paging one row at a time must visit each person exactly once.

    A page size of one makes every boundary a resume, so an off-by-one in the
    cursor shows up as a duplicate or a hole rather than hiding in a big page.
    """
    seen: list[str] = []
    cursor: str | None = None

    for _ in range(20):
        params: dict[str, object] = {"limit": 1}
        if cursor:
            params["cursor"] = cursor
        response = lead_session.client.get(VISIBLE_PERSONS, params=params)
        assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
        page = response.parse(VisiblePersonsPage)

        seen.extend(str(item.person_id) for item in page.items)
        cursor = page.next_cursor
        if cursor is None:
            break

    assert cursor is None, "the walk did not terminate within 20 pages"
    assert len(seen) == len(set(seen)), f"a person was served twice: {seen}"
    assert lead_session.person.uuid in seen, "the walk skipped the caller"


@pytest.mark.requires_seed("dev_lead", "development_ic")
@pytest.mark.reliability
def test_a_search_term_narrows_the_roster_to_the_person_it_names(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """Searching within the visible set, never across it.

    The term comes from the seeded report rather than being written here, so the
    case survives a reseed that renames them.
    """
    report = stand_manifest.fixture("development_ic")
    term = report.email.split("@")[0]

    response = lead_session.client.get(VISIBLE_PERSONS, params={"q": term})
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    listed = {str(item.person_id) for item in response.parse(VisiblePersonsPage).items}

    assert report.uuid in listed, f"searching {term!r} lost {report.email}: {sorted(listed)}"


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_a_cursor_is_refused_where_it_was_not_issued(lead_session: PersonaSession) -> None:
    """A position is bound to the query that ordered it.

    Resuming a browse position inside a narrowed search would skip or repeat
    rows silently, so the request is refused and the caller restarts the new
    list from its first page.
    """
    first = lead_session.client.get(VISIBLE_PERSONS, params={"limit": 1})
    assert first.status_code == 200, f"status={first.status_code} {first.text[:300]}"
    cursor = first.parse(VisiblePersonsPage).next_cursor
    assert cursor, "a cut page must carry a cursor for this case to mean anything"

    refused = lead_session.client.get(
        VISIBLE_PERSONS, params={"q": "nobody", "limit": 1, "cursor": cursor}
    )

    assert refused.status_code == 400, f"status={refused.status_code} {refused.text[:300]}"


@pytest.mark.reliability
def test_an_over_long_query_is_refused_rather_than_scanned(api_client: ApiClient) -> None:
    """The query bounds the scan: one LIKE probe per term over the journal."""
    response = api_client.get(VISIBLE_PERSONS, params={"q": "x" * 201})

    assert response.status_code in (400, 401), (
        f"status={response.status_code} {response.text[:300]}"
    )
