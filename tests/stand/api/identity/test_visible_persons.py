"""`POST /v1/visible-persons` — filtering person ids to what the caller may see.

    POST /v1/visible-persons   200 · 415 wrong-ct

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
"""

from __future__ import annotations

import uuid

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, identity_path

from ..schemas import VisiblePersons

#: `visible_persons.rs::MAX_PERSON_IDS` — one bound parameter per id.
_MAX_PERSON_IDS = 1000

VISIBLE_PERSONS = identity_path("/v1/visible-persons")

#: Nobody holds this. Present so the answer is shown to drop an id it cannot
#: resolve, rather than echoing back whatever it was handed — which is what a
#: wildcard grant did before #2098.
UNKNOWN_PERSON_ID = "01900000-0000-7000-8000-000000000000"


@pytest.mark.requires_seed("dev_lead", "development_ic", "sales_ic", "other_tenant_lead")
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
def test_a_request_naming_nobody_is_a_400(api: ApiClient, person_ids: list[str], case: str) -> None:
    """A request that resolves to no id at all is a client error: answering 200
    with an empty `visible` would read to the caller as `nothing you asked for
    is visible`, which is a different fact."""
    response = api.post(VISIBLE_PERSONS, json_body={"person_ids": person_ids})
    assert response.status_code == 400, (
        f"should reject {case}: status={response.status_code} {response.text[:300]}"
    )


def test_more_ids_than_the_cap_is_a_400(api: ApiClient) -> None:
    """The request bounds the query — one bound parameter per id. The cap
    matches the analytics metric-results cap, which forwards a cleared request
    here whole."""
    over_cap = [str(uuid.uuid4()) for _ in range(_MAX_PERSON_IDS + 1)]

    response = api.post(VISIBLE_PERSONS, json_body={"person_ids": over_cap})
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"
