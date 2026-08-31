"""`POST /v1/profiles` — resolve a person by email or source-native id.

    POST /v1/profiles   200 by email · 200 by person_id · 400 bad value_type
                        404 unknown email · out of scope · another tenant
                        400 value_type=id without a source

The only non-admin write on identity, and the second independent proof of the
identity chain in this suite. `/v1/subchart` proves it CALLER-derived — the
session resolves to a person. This proves it by LOOKUP — the address the seed
used resolves to the UUID the manifest recorded. Either could pass with the
other broken.

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

import pytest
from insight_stand import Manifest, PersonaSession, identity_path

from ..schemas import BatchProfilesResponse, ProblemDocument, Profile

PROFILES = identity_path("/v1/profiles")
PROFILE_BATCH = identity_path("/v1/profiles/batch")


@pytest.mark.reliability
def test_batch_profiles_returns_the_visible_requested_person(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    person = stand_manifest.fixture("dev_lead")

    response = lead_session.client.post(
        PROFILE_BATCH, json_body={"person_ids": [person.uuid]}
    )

    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    profiles = response.parse(BatchProfilesResponse).profiles
    assert [str(profile.person_id) for profile in profiles] == [person.uuid]


@pytest.mark.reliability
def test_resolve_by_email_200(lead_session: PersonaSession, stand_manifest: Manifest) -> None:
    """A seeded address resolves to the person the manifest names, in this tenant."""
    expected = stand_manifest.fixture("dev_lead")
    response = lead_session.client.post(
        PROFILES, json_body={"value_type": "email", "value": expected.email}
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    profile = response.parse(Profile)
    assert str(profile.person_id) == expected.uuid, (
        f"{expected.email} resolved to {profile.person_id}, but the manifest says {expected.uuid}"
    )
    assert str(profile.insight_tenant_id) == stand_manifest.tenant, (
        "the profile came back under a different tenant than the manifest declares"
    )


@pytest.mark.reliability
def test_resolve_by_email_404_unknown(lead_session: PersonaSession) -> None:
    response = lead_session.client.post(
        PROFILES, json_body={"value_type": "email", "value": "nobody@example.com"}
    )
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 404


@pytest.mark.reliability
def test_resolve_400_unknown_value_type(lead_session: PersonaSession) -> None:
    """`value_type` is a closed set, and an unknown one is rejected as an argument.

    A canonical 400 rather than one of Axum's plain-text extractor rejections:
    the body IS the request type, so the handler is reached and the complaint is
    about the value.
    """
    response = lead_session.client.post(
        PROFILES, json_body={"value_type": "not-a-value-type", "value": "x"}
    )
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.reliability
def test_resolve_by_id_400_without_a_source(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """`value_type: "id"` needs the source that issued the id.

    A conditional requirement — `insight_source_type` and `insight_source_id`
    are optional on the request type and mandatory for this one value_type — so
    it is exactly the kind of rule a schema cannot state and only a test can.
    Omitting them must fail rather than resolve against an arbitrary source.
    """
    response = lead_session.client.post(
        PROFILES,
        json_body={"value_type": "id", "value": stand_manifest.fixture("dev_lead").uuid},
    )
    assert response.status_code == 400, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.requires_seed("dev_lead", "sales_ic")
@pytest.mark.security
def test_a_person_outside_the_callers_scope_is_404_not_403(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """Roles are not visibility, and the refusal must not say which applied.

    A lead resolving somebody outside their subtree gets the same answer as for
    an address nobody holds. Anything else turns this endpoint into an oracle
    for "does <email> belong to somebody here?" — answerable by any
    authenticated caller, about anyone, without ever seeing a profile.

    Asserted as a PAIR with the unknown-email case, because the 404 alone would
    also pass if resolution had simply stopped working.
    """
    outsider = stand_manifest.fixture("sales_ic")

    hidden = lead_session.client.post(
        PROFILES, json_body={"value_type": "email", "value": outsider.email}
    )
    assert hidden.status_code == 404, (
        f"resolving {outsider.email}, who is outside the lead's scope, answered "
        f"{hidden.status_code} — anything but 404 discloses that they exist: "
        f"{hidden.text[:300]}"
    )

    unknown = lead_session.client.post(
        PROFILES, json_body={"value_type": "email", "value": "nobody@example.com"}
    )
    assert unknown.status_code == 404
    assert hidden.parse(ProblemDocument).title == unknown.parse(ProblemDocument).title, (
        "the out-of-scope and never-existed answers differ, so the difference is observable"
    )


@pytest.mark.requires_seed("dev_lead", "other_tenant_lead")
@pytest.mark.security
def test_an_email_in_another_tenant_is_404(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """A different boundary from the one above, and a harder one.

    The person outside the caller's scope is at least in their tenant; this one
    is not, so the lookup should not resolve them at all rather than resolving
    them and then refusing. The distinction is invisible in the response — both
    are 404 — which is exactly why it is worth a case of its own: a regression
    that widened the search to every tenant would still answer 404 here for as
    long as the visibility gate happened to hold.
    """
    other = stand_manifest.fixture("other_tenant_lead")
    response = lead_session.client.post(
        PROFILES, json_body={"value_type": "email", "value": other.email}
    )
    assert response.status_code == 404, (
        f"resolving {other.email}, who belongs to another tenant, answered "
        f"{response.status_code}: {response.text[:300]}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_person_id_and_email_are_two_spellings_of_one_identity(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """`value_type="person_id"` resolves the same profile the email does.

    The key the metrics runtime and the SPA use since the identity cutover
    (#2098). Asserting the two answers are EQUAL, rather than that each is a
    200, is what makes it a statement about identity rather than about two
    endpoints that both happen to work: a person is one record reachable by
    either spelling, not two records that agree today.
    """
    person = stand_manifest.fixture("dev_lead")

    by_id = lead_session.client.post(
        PROFILES, json_body={"value_type": "person_id", "value": person.uuid}
    )
    assert by_id.status_code == 200, f"by person_id: {by_id.status_code} {by_id.text[:300]}"

    by_email = lead_session.client.post(
        PROFILES, json_body={"value_type": "email", "value": person.email}
    )
    assert by_email.status_code == 200, f"by email: {by_email.status_code} {by_email.text[:300]}"

    assert by_id.json() == by_email.json(), (
        "the same person resolved through two keys returned two different profiles"
    )


@pytest.mark.requires_seed("dev_lead", "sales_ic")
@pytest.mark.security
def test_a_person_id_outside_the_callers_scope_is_404(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """Visibility gates the new key exactly as it gates the old one.

    Worth its own case rather than trusting the email version: a key added
    after the gate was written is precisely the kind of thing that reaches the
    lookup by a path the gate does not cover.
    """
    outsider = stand_manifest.fixture("sales_ic")
    response = lead_session.client.post(
        PROFILES, json_body={"value_type": "person_id", "value": outsider.uuid}
    )
    assert response.status_code == 404, (
        f"resolving {outsider.email} by person_id from outside their scope answered "
        f"{response.status_code}: {response.text[:300]}"
    )
