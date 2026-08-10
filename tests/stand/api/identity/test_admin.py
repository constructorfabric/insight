"""The admin-gated half of identity-resolution.

Thirteen operations sit behind `require_admin`, which resolves the caller from
the gateway JWT and requires an active `admin` row in `identity.person_roles`.
It never reads the `insight-admin` REALM role — so the CEO, who holds that role,
is refused exactly like everybody else. The seed grants the row to one account:
the admin operator.

That account is deliberately **outside the organisation** — no team, no edge in
the org chart in either direction. It therefore contributes no activity data,
sees nobody in `/v1/subchart`, and cannot move a metric or a visibility
assertion. `test_operator_sees_nobody_in_the_org_chart` below is what keeps that
true: if the operator ever acquires an org edge, it fails here rather than as a
mysterious drift in an unrelated test.

The 401 half is in `test_gateway.py`, swept over every operation at once.


"""

from __future__ import annotations

import pytest
from insight_stand import ADMIN_ROLE, ApiClient, Manifest, PersonaSession, identity_path
from pydantic import BaseModel

from .. import scratch
from ..schemas import (
    PersonRole,
    PersonRoleList,
    ProblemDocument,
    Role,
    RoleList,
    SeedOperationList,
    SubchartForest,
    SyncOperationList,
    Visibility,
    VisibilityList,
)
from .views import forest_emails, in_force

#: Each admin listing with the model that describes it. Parametrising over the
#: pair rather than over the path alone is what makes the 200 cases assert a
#: SHAPE and not merely a status — every declared field of every item is checked
#: on the way through.
ADMIN_LISTINGS_WITH_MODELS = (
    ("/v1/roles", RoleList),
    ("/v1/person-roles", PersonRoleList),
    ("/v1/visibility", VisibilityList),
    ("/v1/persons-seed", SeedOperationList),
    ("/v1/persons-sync", SyncOperationList),
)


def _roles(client: ApiClient) -> RoleList:
    response = client.get(identity_path("/v1/roles"))
    assert response.status_code == 200, f"roles: {response.status_code} {response.text[:300]}"
    return response.parse(RoleList)


def _admin_role_id(client: ApiClient) -> str:
    """The `admin` role's id, read from the catalogue rather than hardcoded.

    The row is created by the identity migrations, so its id is not this
    repository's to know.
    """
    catalogue = _roles(client)
    for role in catalogue.items:
        if role.name == "admin":
            return str(role.role_id)
    raise AssertionError(
        f"no 'admin' role in the catalogue: {[role.name for role in catalogue.items]}"
    )


def _forest(session: PersonaSession) -> SubchartForest:
    response = session.client.get(identity_path("/v1/subchart"))
    assert response.status_code == 200, f"subchart: {response.status_code} {response.text[:300]}"
    return response.parse(SubchartForest)


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.parametrize(
    ("path", "model"), ADMIN_LISTINGS_WITH_MODELS, ids=lambda v: getattr(v, "__name__", v)
)
def test_admin_listing_is_200_for_the_operator(
    admin_operator_session: PersonaSession, path: str, model: type[BaseModel]
) -> None:
    """The grant works, on every admin route, and the payload is the shape claimed.

    Validating rather than shrugging at the body is most of this test's value on
    the empty journals: `GET /v1/persons-seed` returning `{"items": []}` and
    returning `{}` are different answers, and only the model can tell them apart.
    """
    response = admin_operator_session.client.get(identity_path(path))
    assert response.status_code == 200, (
        f"{path} answered {response.status_code} to the admin operator: {response.text[:300]}"
    )
    response.parse(model)


@pytest.mark.requires_seed("admin_operator", "ceo")
@pytest.mark.parametrize("path", [path for path, _ in ADMIN_LISTINGS_WITH_MODELS])
def test_admin_listing_is_403_for_a_realm_admin_without_the_grant(
    realm_admin_session: PersonaSession, path: str
) -> None:
    """Holding `insight-admin` in the realm is NOT administrative authority.

    The sharpest statement of what the gate reads. This persona carries the
    realm's admin role in its token and is still refused, because the gate
    consults `person_roles` and nothing else. A regression that started trusting
    the token's roles would open the admin API to the CEO, and only this test
    would notice.
    """
    assert realm_admin_session.has_realm_role(ADMIN_ROLE)
    response = realm_admin_session.client.get(identity_path(path))
    assert response.status_code == 403, (
        f"{path} answered {response.status_code} to {realm_admin_session.name}, who holds "
        f"{ADMIN_ROLE} in the realm but no person_roles grant: {response.text[:300]}"
    )
    problem = response.parse(ProblemDocument)
    assert problem.status == 403
    assert problem.detail, f"{path}: the refusal carries no detail a caller can act on"


#: The two admin listings that also serve a per-operation detail route. The
#: other three (`/v1/roles`, `/v1/person-roles`, `/v1/visibility`) have no GET
#: by id — their by-id verb is DELETE, covered by the round-trips below.
JOURNALS_WITH_A_DETAIL_ROUTE = ("/v1/persons-seed", "/v1/persons-sync")


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.parametrize("journal", JOURNALS_WITH_A_DETAIL_ROUTE)
def test_an_unknown_journal_entry_is_404_for_the_operator(
    admin_operator_session: PersonaSession, journal: str
) -> None:
    """The detail route reached, past the gate, by somebody who may use it.

    Both journals are empty on a seeded stand — no seed or sync has run — so a
    404 for an id nobody holds is the only answer these routes can give, and
    the only way to show they exist at all. Without it the two are touched by
    nothing but the anonymous sweep, which proves the EDGE refuses a stranger
    and nothing whatever about the route behind it.
    """
    response = admin_operator_session.client.get(identity_path(f"{journal}/{scratch.UNKNOWN_ID}"))
    assert response.status_code == 404, (
        f"{journal}/<unknown> answered {response.status_code} to the admin operator: "
        f"{response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 404


@pytest.mark.requires_seed("admin_operator", "ceo")
@pytest.mark.parametrize("journal", JOURNALS_WITH_A_DETAIL_ROUTE)
def test_a_journal_entry_is_403_without_the_grant(
    realm_admin_session: PersonaSession, journal: str
) -> None:
    """And the detail route reads the same grant its listing does.

    The pair that makes the 404 above mean something: alone it is equally
    consistent with the route being open to anyone authenticated. Ordered as
    the service checks — admin first, existence second — so a caller without
    the grant cannot tell an id that exists from one that does not.
    """
    response = realm_admin_session.client.get(identity_path(f"{journal}/{scratch.UNKNOWN_ID}"))
    assert response.status_code == 403, (
        f"{journal}/<unknown> answered {response.status_code} to {realm_admin_session.name}, "
        f"who holds {ADMIN_ROLE} in the realm but no person_roles grant — a 404 here would "
        f"leak that the gate ran after the lookup: {response.text[:300]}"
    )


@pytest.mark.requires_seed("admin_operator")
def test_the_roles_catalogue_contains_the_admin_role(
    admin_operator_session: PersonaSession,
) -> None:
    """The role the operator holds is the one the catalogue lists.

    Closes the loop between the seed and the API: the grant was written against
    a `roles` row looked up by name, and this is that row coming back out.
    """
    names = {role.name for role in _roles(admin_operator_session.client).items}
    assert "admin" in names, f"the roles catalogue does not contain 'admin': {sorted(names)}"


@pytest.mark.requires_seed("admin_operator")
def test_operator_sees_nobody_in_the_org_chart(
    admin_operator_session: PersonaSession,
) -> None:
    """The operator is in the roster but not in the organisation.

    An empty forest is the observable form of that. It is asserted rather than
    assumed because the isolation is what makes the account safe to add: the
    moment the operator gains an org edge it starts appearing in other people's
    views and contributing to metrics, and every scope assertion in the suite
    quietly changes meaning.
    """
    response = admin_operator_session.client.get(identity_path("/v1/subchart"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    body = response.json()
    assert isinstance(body, dict)
    assert body.get("roots") == [], (
        "the admin operator has an org-chart edge — it is supposed to sit outside "
        f"the organisation entirely: {response.text[:300]}"
    )


# ---------------------------------------------------------------------------
# Round trips
# ---------------------------------------------------------------------------
#
# Each one creates through the API and deletes what it created, per the policy
# in `scratch.py`. Asserting the full create -> list -> delete -> gone cycle in
# ONE test is deliberate: a create case that leaves the row behind and a delete
# case that runs against a row it did not make are the two ways this kind of
# coverage rots, and a single cycle can do neither.


@pytest.mark.requires_seed("admin_operator")
def test_role_create_list_delete_round_trip(admin_operator_session: PersonaSession) -> None:
    """`POST` 201 → the catalogue lists it → `DELETE` 204 → it is gone."""
    client = admin_operator_session.client
    name = scratch.scratch_name("role")

    created = client.post(identity_path("/v1/roles"), json_body={"name": name})
    assert created.status_code == 201, f"create role: {created.status_code} {created.text[:300]}"
    role = created.parse(Role)
    assert role.name == name

    assert name in {r.name for r in _roles(client).items}, (
        "the created role is not in the catalogue"
    )

    deleted = client.delete(identity_path(f"/v1/roles/{role.role_id}"))
    assert deleted.status_code == 204, f"delete role: {deleted.status_code} {deleted.text[:300]}"

    assert name not in {r.name for r in _roles(client).items}, (
        "the role is still listed after a 204 delete"
    )

    gone = client.delete(identity_path(f"/v1/roles/{role.role_id}"))
    assert gone.status_code == 404, (
        f"deleting an already-deleted role answered {gone.status_code}, expected 404: "
        f"{gone.text[:300]}"
    )


@pytest.mark.requires_seed("admin_operator", "dev_lead")
def test_person_role_grant_and_revoke_round_trip(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """Granting a real role to a real seeded person, then revoking it.

    The role granted is `admin` itself — the one role the stand is guaranteed to
    have, since the seed looked it up by name to grant the operator.

    Note the asymmetry with the role catalogue above: a role is REMOVED by its
    delete and stops being listed, while an assignment is REVOKED and stays,
    carrying a `valid_to`. Both are asserted as they behave.
    """
    client = admin_operator_session.client
    subject = stand_manifest.fixture("dev_lead")
    role_id = _admin_role_id(client)

    created = client.post(
        identity_path("/v1/person-roles"),
        json_body={"person_id": subject.uuid, "role_id": role_id, "reason": "stand round trip"},
    )
    assert created.status_code == 201, f"grant role: {created.status_code} {created.text[:300]}"
    assignment = created.parse(PersonRole)
    assert str(assignment.person_id) == subject.uuid
    assert in_force(assignment), f"a fresh assignment is already revoked: {assignment}"
    assignment_id = scratch.track(
        identity_path("/v1/person-roles"), "person_role_id", str(assignment.person_role_id)
    )

    listed = client.get(identity_path("/v1/person-roles")).parse(PersonRoleList)
    assert assignment_id in {str(item.person_role_id) for item in listed.items}, (
        "the grant is not listed"
    )

    revoked = client.delete(identity_path(f"/v1/person-roles/{assignment_id}"))
    assert revoked.status_code == 204, f"revoke: {revoked.status_code} {revoked.text[:300]}"

    journal = client.get(identity_path("/v1/person-roles")).parse(PersonRoleList)
    after = [item for item in journal.items if str(item.person_role_id) == assignment_id]
    assert len(after) == 1, f"the revoked assignment vanished from the journal: {after}"
    assert not in_force(after[0]), f"the assignment is still in force after a 204 revoke: {after[0]}"


@pytest.mark.requires_seed("admin_operator", "dev_lead")
def test_a_visibility_grant_changes_what_the_grantee_can_see(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """The one assertion that proves visibility is APPLIED, not merely stored.

    Every other scope test reads visibility that the seed created, so it can only
    show that *some* rule is in force. This one moves the rule mid-test and
    watches the answer follow: the operator sees nobody, is granted sight of a
    lead, sees that lead's team, and sees nobody again once the grant is revoked.

    The operator is the ideal grantee precisely because it starts empty — an
    empty-to-populated transition needs no reasoning about pre-existing scope.

    A LEAD is the viewed person rather than the CEO for a structural reason: the
    subchart is built from `org_chart` edges, and the CEO has no parent edge, so
    the CEO can never appear as a node in anybody's forest. Granting sight of a
    lead makes the viewed person themselves visible, which is the stronger claim
    — "you can see who you were granted" rather than "somebody appeared".
    """
    client = admin_operator_session.client
    viewed = stand_manifest.fixture("dev_lead")

    before = forest_emails(_forest(admin_operator_session))
    assert before == set(), (
        f"the operator already sees {sorted(before)} — this test needs it to start with "
        "an empty forest, so an earlier grant leaked"
    )

    created = client.post(
        identity_path("/v1/visibility"),
        json_body={
            "viewer_person_id": admin_operator_session.person.uuid,
            "viewed_person_id": viewed.uuid,
            "reason": "stand round trip",
        },
    )
    assert created.status_code == 201, f"create grant: {created.status_code} {created.text[:300]}"
    grant = created.parse(Visibility)
    assert str(grant.viewer_person_id) == admin_operator_session.person.uuid
    assert in_force(grant), f"a fresh grant is already revoked: {grant}"
    grant_id = scratch.track(
        identity_path("/v1/visibility"), "visibility_id", str(grant.visibility_id)
    )

    try:
        visible = forest_emails(_forest(admin_operator_session))
        assert viewed.email in visible, (
            f"after being granted sight of {viewed.email} the operator sees "
            f"{sorted(visible)} — the grant was stored but is not applied"
        )
    finally:
        revoked = client.delete(identity_path(f"/v1/visibility/{grant_id}"))
        assert revoked.status_code == 204, f"revoke: {revoked.status_code} {revoked.text[:300]}"

    after = forest_emails(_forest(admin_operator_session))
    assert after == set(), (
        f"the operator still sees {sorted(after)} after the grant was revoked — "
        "revocation is not applied"
    )
