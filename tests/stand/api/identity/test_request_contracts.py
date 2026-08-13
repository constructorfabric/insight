"""Malformed input, refused before identity looks anything up.

    {id} routes        400 when the segment is not a UUID
    list query params  400 when a value cannot be parsed as its type
    admin deletes      404 for an id nobody holds · 403 without the grant

Three families the rest of this directory leaves out, gathered because they are
properties of the ROUTE TABLE rather than of any handler — the same reasoning
as `analytics/test_request_contracts.py`, and the same benefit: the table is
the assertion, so a route added to `operations.py` and not here is one absence
in one place rather than a test nobody wrote.

Every case is addressed by the ADMIN OPERATOR where the route is admin-gated.
That is deliberate and it costs a stronger claim: a non-admin caller would also
show that the parse happens before the gate, but a failure would then be
ambiguous between "the gate ran first" and "the parse does not reject". Asking
as somebody entitled to an answer leaves only one thing being tested.

Nothing here mutates: the 400s fail before any write, and the 404s name ids
that do not exist.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, PersonaSession, identity_path

from .. import scratch

#: `{id}`-taking routes, admin-gated unless noted. `/v1/subchart/{id}` is the
#: one an ordinary session reaches, so it is asked for by a lead below.
MALFORMED_ID_ADMIN_ROUTES: tuple[tuple[str, str], ...] = (
    ("DELETE", f"/v1/roles/{scratch.NON_UUID}"),
    ("DELETE", f"/v1/person-roles/{scratch.NON_UUID}"),
    ("DELETE", f"/v1/visibility/{scratch.NON_UUID}"),
    ("GET", f"/v1/persons-seed/{scratch.NON_UUID}"),
    ("GET", f"/v1/persons-sync/{scratch.NON_UUID}"),
)

#: Query values that cannot be the type their parameter declares. `limit=abc`
#: is not a number, `person=` is not a uuid, `active=maybe` is not a boolean —
#: each one a filter the caller believes took effect if it is silently dropped.
MALFORMED_QUERY_ADMIN_ROUTES: tuple[str, ...] = (
    "/v1/person-roles?limit=abc",
    "/v1/persons-seed?limit=abc",
    "/v1/persons-sync?limit=abc",
    "/v1/person-roles?person=not-a-uuid",
    "/v1/visibility?active=maybe",
)

#: `/v1/subchart` is `.authenticated()`, not admin-gated.
MALFORMED_SUBCHART_QUERIES: tuple[str, ...] = (
    "/v1/subchart?depth=-1",
    "/v1/subchart?valid_at=not-a-date",
)

#: The admin deletes, which share a shape: gate, then look up, then remove.
ADMIN_DELETES: tuple[str, ...] = (
    "/v1/roles",
    "/v1/person-roles",
    "/v1/visibility",
)


def _id(value: str) -> str:
    return value


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.parametrize(("method", "suffix"), MALFORMED_ID_ADMIN_ROUTES, ids=_id)
@pytest.mark.reliability
def test_a_non_uuid_path_segment_is_400(
    admin_operator_session: PersonaSession, method: str, suffix: str
) -> None:
    """Rejected by the path parser, before identity resolves anything."""
    response = admin_operator_session.client.request(method, identity_path(suffix))
    assert response.status_code == 400, (
        f"{method} {suffix} answered {response.status_code} for a segment that is not a "
        f"UUID: {response.text[:300]}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_a_non_uuid_subchart_id_is_400(lead_session: PersonaSession) -> None:
    """`/v1/subchart/{id}` too, and it is worth stating separately.

    Its sibling contract is that an id OUTSIDE the caller's scope answers 404
    rather than 403, so as not to disclose that the person exists
    (`test_subchart.py`). An unparseable id is a different thing entirely — it
    names nobody, so there is nothing to withhold, and answering 404 here would
    fold a client mistake into the non-disclosure story.
    """
    response = lead_session.client.get(identity_path(f"/v1/subchart/{scratch.NON_UUID}"))
    assert response.status_code == 400, (
        f"an unparseable subchart id answered {response.status_code}: {response.text[:300]}"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.parametrize("suffix", MALFORMED_QUERY_ADMIN_ROUTES, ids=_id)
@pytest.mark.reliability
def test_a_query_value_of_the_wrong_type_is_400(
    admin_operator_session: PersonaSession, suffix: str
) -> None:
    """A filter that cannot be parsed is refused, never ignored.

    The failure mode this prevents is silent: a listing that drops an
    unparseable `person=` and returns everything looks exactly like a listing
    that applied the filter and found everything.
    """
    response = admin_operator_session.client.get(identity_path(suffix))
    assert response.status_code == 400, (
        f"{suffix} answered {response.status_code} to a value of the wrong type: "
        f"{response.text[:300]}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.parametrize("suffix", MALFORMED_SUBCHART_QUERIES, ids=_id)
@pytest.mark.reliability
def test_a_subchart_query_value_that_cannot_be_honoured_is_400(
    lead_session: PersonaSession, suffix: str
) -> None:
    """A negative depth and an unparseable instant, both refused.

    `depth=-1` parses as a number and is still impossible to honour, which is
    why it is here rather than left to the type: a tree of negative depth has
    no meaning, and returning the whole forest instead would be a different
    answer to a different question.
    """
    response = lead_session.client.get(identity_path(suffix))
    assert response.status_code == 400, (
        f"{suffix} answered {response.status_code}: {response.text[:300]}"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.parametrize("collection", ADMIN_DELETES, ids=_id)
@pytest.mark.reliability
def test_deleting_something_nobody_holds_is_404(
    admin_operator_session: PersonaSession, collection: str
) -> None:
    """Past the gate, and the row genuinely is not there."""
    response = admin_operator_session.client.delete(
        identity_path(f"{collection}/{scratch.UNKNOWN_ID}")
    )
    assert response.status_code == 404, (
        f"DELETE {collection}/<unknown> answered {response.status_code} to the admin "
        f"operator: {response.text[:300]}"
    )


@pytest.mark.requires_seed("admin_operator", "ceo")
@pytest.mark.parametrize("collection", ADMIN_DELETES, ids=_id)
@pytest.mark.security
def test_deleting_without_the_grant_is_403(
    realm_admin_session: PersonaSession, collection: str
) -> None:
    """And the gate comes first, so the refusal discloses nothing.

    The pair that makes the 404 above mean something. Ordered as the service
    checks — grant, then existence — so a caller without it cannot use the
    delete route to learn which ids are real.
    """
    response = realm_admin_session.client.delete(
        identity_path(f"{collection}/{scratch.UNKNOWN_ID}")
    )
    assert response.status_code == 403, (
        f"DELETE {collection}/<unknown> answered {response.status_code} to a caller "
        f"holding the realm role but no grant — a 404 would leak that the gate ran "
        f"after the lookup: {response.text[:300]}"
    )


#: The admin WRITES. Each needs a body the extractor ACCEPTS, so that what the
#: 403 below proves is the grant check and not a parse failure.
ADMIN_WRITES: tuple[tuple[str, str], ...] = (
    ("POST", "/v1/roles"),
    ("POST", "/v1/person-roles"),
    ("POST", "/v1/visibility"),
)

#: Shapes each route's extractor accepts. The IDS name nobody — a refusal must
#: not depend on the referenced rows existing, and an ungranted caller must be
#: turned away before anything is looked up either way.
_VALID_BODIES: dict[str, dict[str, str]] = {
    "/v1/roles": {"name": "stand-never-created"},
    "/v1/person-roles": {"person_id": scratch.UNKNOWN_ID, "role_id": scratch.UNKNOWN_ID},
    "/v1/visibility": {
        "viewer_person_id": scratch.UNKNOWN_ID,
        "viewed_person_id": scratch.UNKNOWN_ID,
    },
}


def _valid_body(suffix: str) -> dict[str, str]:
    return _VALID_BODIES[suffix]


@pytest.mark.requires_seed("admin_operator", "ceo")
@pytest.mark.parametrize(("method", "suffix"), ADMIN_WRITES, ids=_id)
@pytest.mark.security
def test_creating_without_the_grant_is_403(
    realm_admin_session: PersonaSession, method: str, suffix: str
) -> None:
    """The body is validated BEFORE the grant is checked — asserted as it is.

    An ungranted caller sending an unparseable body gets 400 `request body did
    not match the expected schema`, not 403: Axum runs a handler's extractors
    before the handler, and the gate lives inside it. So a caller with no grant
    can learn a route's rough shape — a small disclosure, unavoidable at this
    layer, and not the one that matters: the DATA ordering is pinned by the
    403-before-404 on the deletes above. This sends a body the extractor
    ACCEPTS, which is what makes the 403 prove the grant check.
    """
    response = realm_admin_session.client.request(
        method, identity_path(suffix), json_body=dict(_valid_body(suffix))
    )
    assert response.status_code == 403, (
        f"{method} {suffix} answered {response.status_code} to a caller holding the "
        f"realm role but no grant — a well-formed request from an ungranted caller "
        f"must be refused: {response.text[:300]}"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_an_unknown_journal_id_is_404_on_both_journals(
    admin_operator_session: PersonaSession,
) -> None:
    """`persons-sync/{id}`, alongside the persons-seed case in `test_admin.py`."""
    response = admin_operator_session.client.get(
        identity_path(f"/v1/persons-sync/{scratch.UNKNOWN_ID}")
    )
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.security
def test_an_unauthenticated_caller_never_reaches_any_of_this(api_client: ApiClient) -> None:
    """The whole file's premise, stated once.

    Every case above asks as somebody. That only means anything because the
    same urls are closed to nobody — proven per operation by `test_gateway.py`,
    and spot-checked here so this module carries its own reason for using a
    session at all.
    """
    response = api_client.delete(identity_path(f"/v1/roles/{scratch.NON_UUID}"))
    assert response.status_code == 401, (
        f"an anonymous caller reached a malformed-id refusal ({response.status_code}) — "
        f"the edge must close first: {response.text[:300]}"
    )
