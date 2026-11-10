from __future__ import annotations

from collections.abc import Callable, Iterator, Sequence
from contextlib import contextmanager

import pytest
from insight_stand import (
    ADMIN_OPERATOR_FIXTURE,
    OTHER_TENANT_FIXTURE,
    ApiClient,
    ApiResponse,
    Manifest,
    PersonaSession,
    analytics_path,
    identity_path,
)

from .. import scratch
from ..analytics import query_window
from ..schemas import (
    BatchProfilesResponse,
    MeResponse,
    MetricResultsResponse,
    ProblemDocument,
    Profile,
    Subchart,
    SubchartForest,
    Visibility,
    VisibilityList,
    VisiblePersons,
    VisiblePersonsPage,
)
from ..schemas.identity import PeopleListItemResponse, PeopleListResponse
from .views import in_force, walk

ME = identity_path("/v1/me")
SUBCHART = identity_path("/v1/subchart")
VISIBLE_PERSONS = identity_path("/v1/visible-persons")
PEOPLE = identity_path("/v1/people")
PROFILES = identity_path("/v1/profiles")
PROFILE_BATCH = identity_path("/v1/profiles/batch")
VISIBILITY = identity_path("/v1/visibility")
METRIC_RESULTS = analytics_path("/v1/metric-results")

REACH_PERSONAS = ("dev_lead", "sales_lead", "development_ic", "ceo")

type Reports = dict[str, set[str]]


def org_chart(manifest: Manifest) -> Reports:
    leads = [person for person in manifest.personas if person.role == "lead" and person.team]
    lead_of = {lead.team: lead.uuid for lead in leads}
    if len(lead_of) != len(leads):
        pytest.skip(
            "a grown roster nests squads under its leads, which the manifest does not record"
        )

    ceo = manifest.fixture("ceo").uuid
    reports: Reports = {}
    for person in manifest.personas:
        if person.role == "lead" and person.team:
            reports.setdefault(ceo, set()).add(person.uuid)
        elif person.role == "ic" and person.team:
            reports.setdefault(lead_of[person.team], set()).add(person.uuid)
    return reports


def reach(reports: Reports, person_id: str) -> set[str]:
    found = {person_id}
    for report in reports.get(person_id, ()):
        found |= reach(reports, report)
    return found


def _ok(response: ApiResponse, what: str) -> ApiResponse:
    assert response.status_code == 200, f"{what}: {response.status_code} {response.text[:300]}"
    return response


def forest_ids(client: ApiClient) -> set[str]:
    forest = _ok(client.get(SUBCHART), "subchart").parse(SubchartForest)
    return {str(node.person_id) for root in forest.roots for node in walk(root)}


def listed_ids(client: ApiClient) -> set[str]:
    page = _ok(client.get(VISIBLE_PERSONS, params={"limit": 500}), "visible-persons")
    parsed = page.parse(VisiblePersonsPage)
    assert parsed.next_cursor is None, "the roster did not fit one 500-row page"
    return {str(item.person_id) for item in parsed.items}


def filtered_ids(client: ApiClient, candidates: Sequence[str]) -> set[str]:
    response = client.post(VISIBLE_PERSONS, json_body={"person_ids": list(candidates)})
    return {str(person_id) for person_id in _ok(response, "filter").parse(VisiblePersons).visible}


def _people_ids(client: ApiClient) -> set[str]:
    page = _ok(client.get(PEOPLE, params={"limit": 500}), "people").parse(PeopleListResponse)
    assert page.next_cursor is None, "the people list did not fit one 500-row page"
    return {str(item.person_id) for item in page.items}


def _batch_ids(client: ApiClient, candidates: Sequence[str]) -> set[str]:
    response = client.post(PROFILE_BATCH, json_body={"person_ids": list(candidates)})
    profiles = _ok(response, "profiles batch").parse(BatchProfilesResponse).profiles
    return {str(profile.person_id) for profile in profiles}


def _resolve(client: ApiClient, value_type: str, value: str) -> ApiResponse:
    return client.post(PROFILES, json_body={"value_type": value_type, "value": value})


def _metric_results(client: ApiClient, manifest: Manifest, person_id: str) -> ApiResponse:
    start, end = query_window(manifest)
    return client.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [person_id]},
            "period": {"from": start, "to": end},
            "metrics": [{"metric_key": "git.commits", "views": [{"view": "period"}]}],
        },
    )


@contextmanager
def visibility_grant(operator: ApiClient, viewer: str, viewed: str) -> Iterator[Visibility]:
    created = operator.post(
        VISIBILITY,
        json_body={
            "viewer_person_id": viewer,
            "viewed_person_id": viewed,
            "reason": "stand scenario",
        },
    )
    assert created.status_code == 201, f"grant: {created.status_code} {created.text[:300]}"

    grant = created.parse(Visibility)
    grant_id = scratch.track(
        VISIBILITY, "visibility_id", grant.visibility_id, params={"viewer": viewer}
    )
    try:
        yield grant
    finally:
        revoked = operator.delete(f"{VISIBILITY}/{grant_id}")
        assert revoked.status_code == 204, f"revoke: {revoked.status_code} {revoked.text[:300]}"


def _grants(operator: ApiClient, params: dict[str, str]) -> list[Visibility]:
    response = operator.get(VISIBILITY, params={**params, "limit": 500})
    return _ok(response, "visibility listing").parse(VisibilityList).items


@pytest.mark.parametrize("persona", REACH_PERSONAS)
@pytest.mark.requires_seed(*REACH_PERSONAS)
@pytest.mark.security
def test_every_read_of_who_a_persona_can_see_agrees_with_the_org_chart(
    persona: str,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    session = session_for(persona)
    client = session.client
    viewer = session.person.uuid
    reports = org_chart(stand_manifest)
    expected = reach(reports, viewer)
    roster = stand_manifest.personas
    everyone = [person.uuid for person in roster]

    me = _ok(client.get(ME), "me").parse(MeResponse)
    assert (str(me.person_id), str(me.insight_tenant_id)) == (viewer, stand_manifest.tenant)

    assert forest_ids(client) == (expected if viewer in reports else set()), "subchart forest"
    assert listed_ids(client) == expected, "visible-persons enumeration"
    assert filtered_ids(client, everyone) == expected, "visible-persons filter over the roster"
    assert _people_ids(client) == expected, "people list"
    assert _batch_ids(client, everyone) == expected, "profiles batch over the roster"

    answered: dict[str, tuple[int, ...]] = {}
    for person in roster:
        subtree = client.get(f"{SUBCHART}/{person.uuid}")
        detail = client.get(f"{PEOPLE}/{person.uuid}")
        by_email = _resolve(client, "email", person.email)
        by_id = _resolve(client, "person_id", person.uuid)
        responses = (subtree, detail, by_email, by_id)
        answered[person.email] = tuple(response.status_code for response in responses)

        if person.uuid not in expected:
            for response in responses:
                assert response.parse(ProblemDocument).status == response.status_code
            continue
        if answered[person.email] != (200, 200, 200, 200):
            continue

        root = subtree.parse(Subchart).root
        assert {str(node.person_id) for node in walk(root)} == reach(reports, person.uuid), (
            f"{persona}'s subtree rooted at {person.email}"
        )
        assert str(detail.parse(PeopleListItemResponse).person_id) == person.uuid
        assert str(by_email.parse(Profile).person_id) == person.uuid
        assert str(by_id.parse(Profile).person_id) == person.uuid

    assert answered == {
        person.email: (200, 200, 200, 200) if person.uuid in expected else (404, 404, 404, 404)
        for person in roster
    }, f"{persona}: (subchart, people, profile by email, profile by id) per person"


@pytest.mark.requires_seed(OTHER_TENANT_FIXTURE, "dev_lead")
@pytest.mark.security
def test_another_tenants_lead_sees_nothing_of_this_tenant(
    other_tenant_session: PersonaSession,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    outsider = other_tenant_session.client
    own = other_tenant_session.person.uuid
    target = session_for("dev_lead")
    everyone = [person.uuid for person in stand_manifest.personas]
    home = set(everyone) - {own}

    me = _ok(outsider.get(ME), "me").parse(MeResponse)
    assert str(me.person_id) == own
    assert str(me.insight_tenant_id) == stand_manifest.tenants.other != stand_manifest.tenant

    assert not forest_ids(outsider) & home, "subchart forest"
    assert not listed_ids(outsider) & home, "visible-persons enumeration"
    assert not _people_ids(outsider) & home, "people list"
    assert filtered_ids(outsider, everyone) == {own}, "visible-persons filter over both tenants"
    assert _batch_ids(outsider, everyone) == {own}, "profiles batch over both tenants"

    lookups = {
        "subchart": outsider.get(f"{SUBCHART}/{target.person.uuid}"),
        "people": outsider.get(f"{PEOPLE}/{target.person.uuid}"),
        "profile by email": _resolve(outsider, "email", target.person.email),
        "profile by id": _resolve(outsider, "person_id", target.person.uuid),
        "metric-results": _metric_results(outsider, stand_manifest, target.person.uuid),
    }
    assert {name: response.status_code for name, response in lookups.items()} == {
        "subchart": 404,
        "people": 404,
        "profile by email": 404,
        "profile by id": 404,
        "metric-results": 403,
    }
    for response in lookups.values():
        assert response.parse(ProblemDocument).status == response.status_code

    own_profile = _ok(_resolve(target.client, "email", target.person.email), "own profile")
    assert str(own_profile.parse(Profile).person_id) == target.person.uuid

    own_numbers = _metric_results(target.client, stand_manifest, target.person.uuid)
    _ok(own_numbers, "own metrics").parse(MetricResultsResponse)


@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "hr_ic", "dev_lead")
@pytest.mark.security
def test_a_visibility_grant_moves_the_grantees_org_chart_and_roster_until_revoked(
    admin_operator_session: PersonaSession,
    session_for: Callable[[str], PersonaSession],
    stand_manifest: Manifest,
) -> None:
    operator = admin_operator_session.client
    grantee = session_for("hr_ic")
    viewer = grantee.person.uuid
    viewed = stand_manifest.fixture("dev_lead").uuid
    reports = org_chart(stand_manifest)
    own = reach(reports, viewer)
    team = reach(reports, viewed)
    everyone = [person.uuid for person in stand_manifest.personas]

    assert forest_ids(grantee.client) == set()
    assert listed_ids(grantee.client) == own

    with visibility_grant(operator, viewer, viewed) as grant:
        grant_id = str(grant.visibility_id)
        assert in_force(grant)
        assert (str(grant.viewer_person_id), str(grant.viewed_person_id)) == (viewer, viewed)

        granted_forest = forest_ids(grantee.client)
        granted_listing = listed_ids(grantee.client)
        granted_filter = filtered_ids(grantee.client, everyone)
        by_viewer = _grants(operator, {"viewer": viewer})
        by_viewed = _grants(operator, {"viewed": viewed, "active": "true"})

    assert granted_forest == team, "the forest did not take on the granted lead's team"
    assert granted_listing == own | team, "the enumeration did not follow the grant"
    assert granted_filter == own | team, "the filter did not follow the grant"
    assert grant_id in {str(row.visibility_id) for row in by_viewer}
    assert {str(row.viewer_person_id) for row in by_viewer} == {viewer}
    assert grant_id in {str(row.visibility_id) for row in by_viewed}
    assert {str(row.viewed_person_id) for row in by_viewed} == {viewed}
    assert all(in_force(row) for row in by_viewed)

    assert forest_ids(grantee.client) == set(), "the forest kept the revoked grant"
    assert listed_ids(grantee.client) == own, "the enumeration kept the revoked grant"

    journal = {str(row.visibility_id): row for row in _grants(operator, {"viewer": viewer})}
    assert not in_force(journal[grant_id]), "the revoked grant is still in force"
    active = _grants(operator, {"viewed": viewed, "active": "true"})
    assert grant_id not in {str(row.visibility_id) for row in active}
