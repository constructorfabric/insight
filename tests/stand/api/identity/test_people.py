from __future__ import annotations

import pytest
from insight_stand import Manifest, PersonaSession, identity_path

from ..schemas.identity import PeopleListItemResponse, PeopleListResponse

PEOPLE = identity_path("/v1/people")


@pytest.mark.requires_seed("dev_lead", "development_ic", "sales_ic")
@pytest.mark.security
def test_people_lists_only_the_callers_visible_roster(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    report = stand_manifest.fixture("development_ic")
    outsider = stand_manifest.fixture("sales_ic")

    response = lead_session.client.get(PEOPLE, params={"limit": 500})

    assert (
        response.status_code == 200
    ), f"status={response.status_code} {response.text[:300]}"
    people = response.parse(PeopleListResponse).items
    by_id = {str(person.person_id): person for person in people}
    assert lead_session.person.uuid in by_id
    assert report.uuid in by_id
    assert outsider.uuid not in by_id


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.security
def test_people_detail_returns_the_visible_roster_profile(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    expected = stand_manifest.fixture("dev_lead")

    response = lead_session.client.get(f"{PEOPLE}/{expected.uuid}")

    assert (
        response.status_code == 200
    ), f"status={response.status_code} {response.text[:300]}"
    person = response.parse(PeopleListItemResponse)
    assert str(person.person_id) == expected.uuid
    assert person.email == expected.email
    assert person.display_name == expected.display_name
