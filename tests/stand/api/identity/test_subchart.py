"""The identity-resolution service, through the gateway.

Per `deploy/compose/gateway/routes.yaml`, `/api/identity` reaches
`identity-resolution:8082` — the Rust service. The .NET `insight-identity` that
used to answer here was removed upstream in favour of it (epic #1602) and the
gateway was repointed.

Organised by SERVICE, not by auth state, against one path constant per route.
The 401 half is not here: `test_gateway.py` sweeps it over every operation in
`operations.py` at once, since refusing an anonymous caller is the gateway's
uniform behaviour rather than anything identity does. The success cases below
are what make that sweep mean "refused" instead of "there was nothing there" —
the gateway answers 401 for paths that do not exist too.

NOT here: `/v1/persons/{email}`. The committed contract at
`docs/components/backend/identity-resolution/openapi.json` still declares it on
this service, but identity-resolution answers 404 — only
`/internal/persons/by-email/{email}` (service principals) survived the port.
Analytics no longer offers a person facade either; `POST /v1/profiles` is the
one way to resolve a person.

Every PERSON below was logged in by driving the deployed OIDC chain against
Keycloak; no token is minted anywhere in this suite. The one non-person caller
is the service principal in `test_internal.py`, which states how it is issued.
"""

from __future__ import annotations

from collections.abc import Callable

import pytest
from insight_stand import (
    ADMIN_ROLE,
    LEAD_ROLE,
    MEMBER_ROLE,
    Manifest,
    PersonaSession,
    identity_path,
)

from ..schemas import ProblemDocument, Subchart, SubchartForest
from ..scratch import UNKNOWN_ID
from .views import forest_emails, walk

#: Caller-derived org subchart — takes no person argument, so what comes back
#: identifies whoever the session belongs to. 401 anonymous (swept in
#: `test_gateway.py`), 200 with a session, and populated from the seeded org
#: chart.
SUBCHART = identity_path("/v1/subchart")


def _forest(session: PersonaSession) -> SubchartForest:
    """That persona's visible forest, parsed into `SubchartForest` so a malformed
    payload fails as a statement about the response rather than as a `TypeError`.
    """
    response = session.client.get(SUBCHART)
    assert response.status_code == 200, (
        f"{session.name} could not read {SUBCHART}: {response.status_code} {response.text[:300]}"
    )
    return response.parse(SubchartForest)


@pytest.mark.reliability
def test_subchart_is_200_with_a_session(lead_session: PersonaSession) -> None:
    """Same url the gateway sweep refuses anonymously; a session is the only
    difference — and the roots rule out "there was nothing there".
    """
    assert _forest(lead_session).roots, "the authenticated subchart carried no roots"


@pytest.mark.security
@pytest.mark.stand_smoke
def test_the_session_belongs_to_the_persona_who_logged_in(lead_session: PersonaSession) -> None:
    """A session that authenticates as somebody else is worse than none.

    Asserted through a CALLER-DERIVED endpoint on purpose: `/v1/subchart` takes
    no person argument, so the stack resolves the caller from the session
    alone. Finding this persona in the result — and finding the manifest's UUID
    on that node — is the whole chain confirming it landed on the intended
    human: Keycloak authenticated them, the authenticator mapped the token to a
    person, and identity found that person in the seeded roster.

    Marked `stand_smoke`: this is the post-deploy gate's "a seeded persona can
    actually log in" check. `lead_session` drives the real OIDC chain against
    Keycloak to obtain the session in the first place, so a login that cannot
    complete fails here at fixture setup, before this test's own assertion
    ever runs.
    """
    nodes = [node for root in _forest(lead_session).roots for node in walk(root)]
    mine = [node for node in nodes if node.email == lead_session.email]
    assert len(mine) == 1, (
        f"the caller-derived org chart for {lead_session.name} contains "
        f"{sorted(str(node.email) for node in nodes)}, which does not name "
        f"{lead_session.email} exactly once — the session resolved to someone else"
    )
    assert str(mine[0].person_id) == lead_session.person.uuid, (
        f"identity resolved {lead_session.email} to person_id {mine[0].person_id}, "
        f"but the manifest says {lead_session.person.uuid}"
    )


@pytest.mark.security
def test_org_visibility_scope_differs_by_persona(
    realm_admin_session: PersonaSession,
    lead_session: PersonaSession,
    member_session: PersonaSession,
) -> None:
    """One endpoint, three personas, three materially different answers.

    Scope is enforced by the deployed stack, not by the test.

    Note what is NOT being claimed. The scope comes from the seeded org chart,
    not from the caller's realm role — identity never reads the
    `insight-admin` / `insight-lead` / `insight-member` grants for this endpoint
    (its admin gate consults the `person_roles` table instead, and only the
    admin operator holds a row there; see `test_admin.py`). The
    realm-role assertions below are a precondition pinning down WHICH three
    personas are compared, not the mechanism under test.

    Relationships are asserted rather than exact counts, so a roster change
    moves the numbers without inventing a failure.
    """
    assert realm_admin_session.has_realm_role(ADMIN_ROLE)
    assert lead_session.has_realm_role(LEAD_ROLE) and not lead_session.has_realm_role(ADMIN_ROLE)
    assert member_session.has_realm_role(MEMBER_ROLE)
    assert realm_admin_session.email != lead_session.email, (
        "the realm admin and the lead resolved to the same persona"
    )

    admin_view = forest_emails(_forest(realm_admin_session))
    lead_view = forest_emails(_forest(lead_session))
    member_view = forest_emails(_forest(member_session))

    assert member_view == set(), (
        f"a plain member sees {sorted(member_view)} in the org chart; expected nothing"
    )
    assert lead_view, f"{lead_session.name} is a lead but sees nobody in the org chart"
    assert len(admin_view) > len(lead_view), (
        f"{realm_admin_session.name} sees {len(admin_view)} people and {lead_session.name} "
        f"(lead) sees {len(lead_view)} — the senior view must be strictly wider"
    )
    assert lead_view <= admin_view, (
        f"{lead_session.name} sees {sorted(lead_view - admin_view)}, which the admin does not"
    )


@pytest.mark.requires_seed("dev_lead", "sales_lead")
@pytest.mark.security
def test_two_leads_of_different_teams_see_different_people(
    session_for: Callable[[str], PersonaSession],
) -> None:
    """Same role, same endpoint, different answers.

    Holding the realm role constant isolates the check to per-person scoping:
    if both leads saw the same set, visibility would be role-shaped only and
    the org chart would be leaking across teams.
    """
    dev, sales = session_for("dev_lead"), session_for("sales_lead")
    assert dev.person.team != sales.person.team

    dev_view, sales_view = forest_emails(_forest(dev)), forest_emails(_forest(sales))

    assert dev_view and sales_view, "expected both leads to see somebody"
    assert dev_view != sales_view, (
        f"both leads see the same people ({sorted(dev_view)}) — visibility is not per-person"
    )
    assert not (dev_view & sales_view), (
        f"leads of different teams share {sorted(dev_view & sales_view)}"
    )


# ---------------------------------------------------------------------------
# The by-person route
# ---------------------------------------------------------------------------

SUBCHART_OF = identity_path("/v1/subchart")


def _subtree(session: PersonaSession, person_uuid: str) -> Subchart:
    response = session.client.get(f"{SUBCHART_OF}/{person_uuid}")
    assert response.status_code == 200, (
        f"{session.name} could not read the subtree of {person_uuid}: "
        f"{response.status_code} {response.text[:300]}"
    )
    return response.parse(Subchart)


@pytest.mark.reliability
def test_subchart_of_self_is_200(lead_session: PersonaSession) -> None:
    """Distinct from the forest: this route takes an explicit person.

    Asking for oneself is the case that cannot be confused with anything else —
    the root that comes back must be the caller.
    """
    subtree = _subtree(lead_session, lead_session.person.uuid)
    assert str(subtree.root.person_id) == lead_session.person.uuid
    assert subtree.root.email == lead_session.email


@pytest.mark.reliability
def test_subchart_of_a_visible_report_is_200(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """A lead can root the tree at somebody they can see."""
    report = stand_manifest.fixture("development_ic")
    assert str(_subtree(lead_session, report.uuid).root.person_id) == report.uuid


@pytest.mark.security
def test_subchart_of_someone_out_of_scope_is_404_not_403(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """Out of scope is indistinguishable from not existing. That is the point.

    A 403 would confirm the person exists — turning this endpoint into an
    oracle for "is <uuid> somebody in this company?" for any authenticated
    caller. Answering 404, byte for byte the same as for a uuid nobody holds, is
    what stops it leaking membership.

    Verified as a pair on purpose: asserting the 404 alone would also pass if
    the route had simply stopped working, so the in-scope 200 above and the
    unknown-uuid 404 below bracket it.
    """
    outsider = stand_manifest.fixture("sales_ic")
    response = lead_session.client.get(f"{SUBCHART_OF}/{outsider.uuid}")
    assert response.status_code == 404, (
        f"a lead asking for {outsider.email}, who is outside their scope, got "
        f"{response.status_code} — anything but 404 discloses that the person exists: "
        f"{response.text[:300]}"
    )

    unknown = lead_session.client.get(f"{SUBCHART_OF}/{UNKNOWN_ID}")
    assert unknown.status_code == 404
    assert response.parse(ProblemDocument).title == unknown.parse(ProblemDocument).title, (
        "the out-of-scope and never-existed answers differ, so the difference is observable"
    )


@pytest.mark.reliability
def test_subchart_of_an_unknown_person_is_404(lead_session: PersonaSession) -> None:
    response = lead_session.client.get(f"{SUBCHART_OF}/{UNKNOWN_ID}")
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"
    assert response.parse(ProblemDocument).status == 404
