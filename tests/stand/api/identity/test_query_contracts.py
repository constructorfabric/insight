"""The query parameters a consumer actually sets, and whether they are honoured.

    GET /v1/person-roles   ?person · ?role&active · ?limit
    GET /v1/visibility     ?viewer · ?viewed&active
    GET /v1/subchart/{id}  ?depth

Nothing here is visible to the coverage gate, and that is the point. Every case
below answers 200 whether or not the filter did anything — a listing that
ignores `?person=` returns everybody with the same status code as one that
applies it. Status-code coverage cannot tell those apart; only reading the rows
can, which is why this module exists alongside a gate that reports the suite as
complete without it.

The failure mode is always the same shape and always silent: a parameter that
is dropped rather than honoured widens an answer. On `/v1/person-roles?person=`
that means showing an operator every grant in the tenant when they asked about
one person; on `/v1/subchart?depth=` it means rendering a whole org under a
request for one level.

Each filter is asserted from BOTH sides — something that must appear and
something that must not — because a filter returning everything and a filter
returning the right thing agree on any test that only checks the former.
"""

from __future__ import annotations

from collections.abc import Callable

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, identity_path

from ..schemas import (
    PersonRoleList,
    RoleList,
    Subchart,
    SubchartNode,
    Visibility,
    VisibilityList,
)
from .views import in_force

PERSON_ROLES = identity_path("/v1/person-roles")
VISIBILITY = identity_path("/v1/visibility")


def _person_roles(client: ApiClient, query: str = "") -> PersonRoleList:
    response = client.get(f"{PERSON_ROLES}{query}")
    assert response.status_code == 200, f"{query}: {response.status_code} {response.text[:300]}"
    return response.parse(PersonRoleList)


def _visibility(client: ApiClient, query: str = "") -> VisibilityList:
    response = client.get(f"{VISIBILITY}{query}")
    assert response.status_code == 200, f"{query}: {response.status_code} {response.text[:300]}"
    return response.parse(VisibilityList)


def _people(node: SubchartNode) -> set[str]:
    """Every person id in a rendered subtree, root included."""
    found = {str(node.person_id)}
    for child in node.subordinates or []:
        found |= _people(child)
    return found


@pytest.mark.requires_seed("admin_operator", "dev_lead")
def test_person_roles_filtered_by_person_shows_only_that_person(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """The operator holds an ACTIVE grant; the dev lead holds none. Both halves.

    Without the empty half this passes for a listing that ignores the filter
    entirely, since the operator's own grant would be in the unfiltered answer
    too.

    `active=true` on the empty half is load-bearing rather than decorative. A
    revoke here is temporal — `DELETE` sets `valid_to` and the row stays — so
    an earlier test's granted-then-revoked assignment is still a row the
    unfiltered filter correctly returns. Asserting "no rows at all" would blame
    the filter for the audit trail doing its job.
    """
    operator = stand_manifest.fixture("admin_operator")
    lead = stand_manifest.fixture("dev_lead")
    client = admin_operator_session.client

    mine = _person_roles(client, f"?person={operator.uuid}")
    assert mine.items, "the admin operator's own grant is missing from a filter naming them"
    assert all(str(row.person_id) == operator.uuid for row in mine.items), (
        f"?person={operator.uuid} returned rows about somebody else: "
        f"{[str(r.person_id) for r in mine.items]}"
    )

    theirs = _person_roles(client, f"?person={lead.uuid}&active=true")
    assert [row for row in theirs.items if in_force(row)] == [], (
        f"?person={lead.uuid}&active=true returned {len(theirs.items)} grants in force "
        "for somebody who holds none"
    )
    assert all(str(row.person_id) == lead.uuid for row in theirs.items), (
        "the person filter returned rows about somebody else"
    )


@pytest.mark.requires_seed("admin_operator")
def test_person_roles_filtered_by_role_and_active_narrows_on_both(
    admin_operator_session: PersonaSession,
) -> None:
    """Two filters at once, and `active=true` means `valid_to is null`."""
    client = admin_operator_session.client
    catalogue = client.get(identity_path("/v1/roles"))
    assert catalogue.status_code == 200, f"roles: {catalogue.status_code}"
    roles = catalogue.parse(RoleList).items
    assert roles, "the role catalogue is empty"
    role_id = str(roles[0].role_id)

    rows = _person_roles(client, f"?role={role_id}&active=true").items
    assert all(str(row.role_id) == role_id for row in rows), "the role filter was not applied"
    assert all(in_force(row) for row in rows), (
        "active=true returned a revoked grant — valid_to is set on at least one row"
    )


@pytest.mark.requires_seed("admin_operator")
def test_person_roles_limit_caps_the_page(admin_operator_session: PersonaSession) -> None:
    """`limit=1` returns one row, whatever the tenant holds.

    Meaningful only because the unfiltered listing is non-empty — asserted
    first, so a limit that "works" by there being nothing to return cannot
    pass.
    """
    client = admin_operator_session.client
    assert _person_roles(client).items, "no grants at all — limit proves nothing here"

    page = _person_roles(client, "?limit=1")
    assert len(page.items) == 1, f"limit=1 returned {len(page.items)} rows"


@pytest.mark.requires_seed("admin_operator", "dev_lead", "sales_ic")
def test_visibility_filters_narrow_by_viewer_and_by_viewed(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """A grant this test creates, then finds through each filter, then removes.

    Creating it here rather than relying on a seeded one keeps the assertion
    exact: the row's viewer and viewed are known, so "the filter found it" and
    "the filter found something" are distinguishable.
    """
    client = admin_operator_session.client
    viewer = stand_manifest.fixture("dev_lead")
    viewed = stand_manifest.fixture("sales_ic")

    created = client.post(
        VISIBILITY,
        json_body={
            "viewer_person_id": viewer.uuid,
            "viewed_person_id": viewed.uuid,
            "reason": "stand query-contract fixture",
        },
    )
    assert created.status_code == 201, f"setup: {created.status_code} {created.text[:300]}"
    grant_id = str(created.parse(Visibility).visibility_id)

    try:
        by_viewer = _visibility(client, f"?viewer={viewer.uuid}").items
        assert any(str(row.visibility_id) == grant_id for row in by_viewer), (
            "the grant is absent from a filter naming its viewer"
        )
        assert all(str(row.viewer_person_id) == viewer.uuid for row in by_viewer), (
            "?viewer= returned a grant belonging to somebody else"
        )

        by_viewed = _visibility(client, f"?viewed={viewed.uuid}&active=true").items
        assert any(str(row.visibility_id) == grant_id for row in by_viewed), (
            "the grant is absent from a filter naming its target"
        )
        assert all(in_force(row) for row in by_viewed), "active=true returned a revoked grant"
    finally:
        client.delete(f"{VISIBILITY}/{grant_id}")


@pytest.mark.requires_seed("ceo", "dev_lead", "development_ic")
@pytest.mark.parametrize("depth", [0, 1])
def test_subchart_depth_cuts_the_descent_where_it_says(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest, depth: int
) -> None:
    """`depth=0` is the root alone; `depth=1` adds its reports and stops.

    The roster is three deep — ceo → dev_lead → development_ic — which is the
    minimum that can tell "one level" from "everything": at depth=1 the lead
    must appear and the IC must not, and a service ignoring the parameter
    passes any test that only looks for the lead.
    """
    ceo = stand_manifest.fixture("ceo")
    lead = stand_manifest.fixture("dev_lead")
    ic = stand_manifest.fixture("development_ic")

    session = session_for("ceo")
    response = session.client.get(identity_path(f"/v1/subchart/{ceo.uuid}?depth={depth}"))
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    rendered = _people(response.parse(Subchart).root)
    assert ceo.uuid in rendered, "the root itself is missing from its own subtree"
    assert ic.uuid not in rendered, (
        f"depth={depth} rendered a grandchild — the descent was not cut: {sorted(rendered)}"
    )
    if depth == 0:
        assert lead.uuid not in rendered, f"depth=0 rendered a subordinate: {sorted(rendered)}"
    else:
        assert lead.uuid in rendered, f"depth=1 dropped the root's own report: {sorted(rendered)}"
