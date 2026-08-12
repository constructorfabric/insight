"""What the admin writes refuse before they write anything.

    POST   /v1/roles            400 empty name · 400 name too long
    POST   /v1/person-roles     400 nil person · 400 nil role · 400 reason too long
    DELETE /v1/person-roles/{id} 400 reason too long
    POST   /v1/visibility       400 nil viewer · 400 nil viewed · 400 reason too long
    DELETE /v1/visibility/{id}  400 reason too long

Every case is refused BEFORE the lookup, which is what makes the whole module
non-mutating: the deletes address ids nobody holds, and never reach them.

The nil-UUID cases are the ones worth reading twice. A nil id is well-formed —
it parses, it is the right type, and it means nothing. On `/v1/visibility` the
distinction is load-bearing: an ABSENT `viewed_person_id` grants sight of the
viewer's whole subtree, so a present-but-nil one is not a smaller version of
that request, it is a different request that happens to look like it. Accepting
it would create a grant nobody asked for.

Asked as the admin operator throughout — the caller entitled to be here — so a
400 is the validator's answer and not the gate's. `test_request_contracts.py`
owns the gate's side of these same routes.
"""

from __future__ import annotations

from collections.abc import Mapping

import pytest
from insight_stand import Manifest, PersonaSession, identity_path

from .. import scratch
from ..schemas import ProblemDocument, RoleList

# Quality vector of this module's tests.
pytestmark = pytest.mark.reliability

#: Both validators cap `reason` at 500 characters.
TOO_LONG_REASON = "x" * 501

#: Well-formed, right type, names nobody.
NIL_UUID = "00000000-0000-0000-0000-000000000000"

#: The name cap is 64.
TOO_LONG_NAME = "r" * 65


def _a_role_id(session: PersonaSession) -> str:
    """Any role from the global catalogue — which one is irrelevant here.

    Read rather than hardcoded for the same reason `test_admin.py` reads it:
    the rows come from the identity migrations, so their ids are not this
    repository's to know.
    """
    response = session.client.get(identity_path("/v1/roles"))
    assert response.status_code == 200, f"roles: {response.status_code} {response.text[:300]}"
    catalogue = response.parse(RoleList).items
    assert catalogue, "the role catalogue is empty — did the identity migrations run?"
    return str(catalogue[0].role_id)


def _refused(session: PersonaSession, method: str, suffix: str, body: Mapping[str, str]) -> None:
    response = session.client.request(method, identity_path(suffix), json_body=dict(body))
    assert response.status_code == 400, (
        f"{method} {suffix} accepted {body!r} ({response.status_code}) rather than "
        f"refusing it: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.parametrize("name", ["", TOO_LONG_NAME], ids=["empty", "too-long"])
def test_a_role_name_outside_the_bounds_is_refused(
    admin_operator_session: PersonaSession, name: str
) -> None:
    """An unnamed role is unusable and an over-long one is a truncated lie.

    The catalogue is global — `_admin_role_id` in `test_admin.py` finds the
    `admin` row by name — so a nameless entry is not merely untidy, it is
    unaddressable by the only handle the API offers.
    """
    _refused(admin_operator_session, "POST", "/v1/roles", {"name": name})


@pytest.mark.requires_seed("admin_operator", "dev_lead")
def test_a_person_role_grant_naming_nobody_is_refused(
    admin_operator_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """Nil on either side of the grant, and both must be refused.

    They fail for different reasons, and a validator that checks only one is
    exactly the bug worth catching — a grant of a real role to nobody, or of
    nothing to a real person, both persist happily if unchecked.
    """
    person = stand_manifest.fixture("dev_lead")
    role_id = _a_role_id(admin_operator_session)

    _refused(
        admin_operator_session,
        "POST",
        "/v1/person-roles",
        {"person_id": NIL_UUID, "role_id": role_id},
    )
    _refused(
        admin_operator_session,
        "POST",
        "/v1/person-roles",
        {"person_id": person.uuid, "role_id": NIL_UUID},
    )


@pytest.mark.requires_seed("admin_operator", "dev_lead", "development_ic")
@pytest.mark.parametrize("nil_field", ["viewer_person_id", "viewed_person_id"])
def test_a_visibility_grant_naming_nobody_is_refused(
    admin_operator_session: PersonaSession, stand_manifest: Manifest, nil_field: str
) -> None:
    """And the nil VIEWED case is not the same as leaving it out.

    Omitting `viewed_person_id` is a whole-subtree grant, a documented and much
    broader thing. A nil value is therefore not a degenerate version of the
    same request — treating it as one would silently widen a grant from one
    person to everybody the viewer can see.
    """
    grant = {
        "viewer_person_id": stand_manifest.fixture("dev_lead").uuid,
        "viewed_person_id": stand_manifest.fixture("development_ic").uuid,
    }
    grant[nil_field] = NIL_UUID

    _refused(admin_operator_session, "POST", "/v1/visibility", grant)


@pytest.mark.requires_seed("admin_operator", "dev_lead", "development_ic")
@pytest.mark.parametrize(
    ("method", "suffix", "extra"),
    [
        ("POST", "/v1/person-roles", "grant"),
        ("POST", "/v1/visibility", "grant"),
        ("DELETE", f"/v1/person-roles/{scratch.UNKNOWN_ID}", "revoke"),
        ("DELETE", f"/v1/visibility/{scratch.UNKNOWN_ID}", "revoke"),
    ],
    ids=["person-role-grant", "visibility-grant", "person-role-revoke", "visibility-revoke"],
)
def test_an_over_long_reason_is_refused_on_every_route_that_takes_one(
    admin_operator_session: PersonaSession,
    stand_manifest: Manifest,
    method: str,
    suffix: str,
    extra: str,
) -> None:
    """The audit trail's own field, capped consistently on all four routes.

    `reason` is what an operator writes for whoever reads the grant later, so
    an unchecked cap is a column that silently truncates the only explanation
    of why somebody can see something. The two REVOKE cases matter most: the
    check runs before the lookup, which is what lets them address an id nobody
    holds and still assert the validator rather than a 404.
    """
    body: dict[str, str] = {"reason": TOO_LONG_REASON}
    if extra == "grant" and "person-roles" in suffix:
        body["person_id"] = stand_manifest.fixture("dev_lead").uuid
        body["role_id"] = _a_role_id(admin_operator_session)
    elif extra == "grant":
        body["viewer_person_id"] = stand_manifest.fixture("dev_lead").uuid
        body["viewed_person_id"] = stand_manifest.fixture("development_ic").uuid

    _refused(admin_operator_session, method, suffix, body)
