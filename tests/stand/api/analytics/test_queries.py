"""The `/v1/queries` path group on analytics — saved queries and running them.

    GET    /v1/queries              200 list · excludes deleted
    POST   /v1/queries              201 · 400 not-a-read
    GET    /v1/queries/{id}         200 · 404 unknown · 404 deleted
    PUT    /v1/queries/{id}         200 · 400 not-a-read · 404 unknown
    DELETE /v1/queries/{id}         204 · 404 unknown
    POST   /v1/queries/{id}/run     200 · 400 unbound param · 404 · 415 wrong-ct

The non-uuid and wrong-media-type halves are in `test_request_contracts.py`,
swept over every route at once; `/run` keeps its own 415 because it is the
route where an OPTIONAL body makes silently ignoring one plausible.

`/run` is the one that earns its place here rather than in the rig. It goes
gateway → analytics → ClickHouse in one request, so a green run means the whole
chain is wired: the session survived the edge, the tenant came out of the JWT,
and the query engine answered. The saved SQL returns a single deterministic row,
`{"one": 1}`, so the result can be asserted exactly instead of "something came
back".

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, Manifest, analytics_path

from ..schemas import RunResponse, SavedQuery, SavedQueryListResponse
from ..scratch import SCRATCH_QUERY_REF, UNKNOWN_ID, create_saved_query, scratch_name

QUERIES = analytics_path("/v1/queries")


def _query_path(query_id: object, suffix: str = "") -> str:
    return analytics_path(f"/v1/queries/{query_id}{suffix}")


def _saved(api: ApiClient) -> set[str]:
    """Every saved-query name the listing reports, validated on the way through."""
    response = api.get(QUERIES)
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    return {item.name for item in response.parse(SavedQueryListResponse).items}


@pytest.mark.reliability
def test_list_queries_200(api: ApiClient, scratch_saved_query: SavedQuery) -> None:
    assert scratch_saved_query.name in _saved(api)


@pytest.mark.reliability
def test_saved_query_create_run_update_delete_round_trip(api: ApiClient) -> None:
    """One cycle: create → read → run → update → delete → gone.

    Asserted as a cycle rather than as six independent cases because that is
    what makes each half honest — a create that leaks its row and a delete that
    runs against a row it did not make are the two ways this coverage rots, and
    a single cycle can do neither.
    """
    created = create_saved_query(api, "roundtrip")
    query_id = created.id

    fetched = api.get(_query_path(query_id))
    assert fetched.status_code == 200, f"read back: {fetched.status_code} {fetched.text[:300]}"
    assert fetched.parse(SavedQuery).sql == SCRATCH_QUERY_REF

    ran = api.post(_query_path(query_id, "/run"), json_body={})
    assert ran.status_code == 200, f"run: {ran.status_code} {ran.text[:300]}"
    assert ran.parse(RunResponse).rows == [{"one": 1}], (
        f"the saved SQL should return exactly one deterministic row: {ran.text[:300]}"
    )

    updated = api.put(
        _query_path(query_id),
        json_body={
            "name": created.name,
            "description": "updated by the stand suite",
            "sql": SCRATCH_QUERY_REF,
        },
    )
    assert updated.status_code == 200, f"update: {updated.status_code} {updated.text[:300]}"

    deleted = api.delete(_query_path(query_id))
    assert deleted.status_code == 204, f"delete: {deleted.status_code} {deleted.text[:300]}"

    assert api.get(_query_path(query_id)).status_code == 404
    assert created.name not in _saved(api), "a hard-deleted saved query is still listed"


@pytest.mark.reliability
def test_get_query_404_unknown(api: ApiClient) -> None:
    response = api.get(_query_path(UNKNOWN_ID))
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.reliability
def test_update_query_404_unknown(api: ApiClient) -> None:
    response = api.put(
        _query_path(UNKNOWN_ID),
        json_body={"name": "absent", "description": "x", "sql": SCRATCH_QUERY_REF},
    )
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.reliability
def test_delete_query_404_unknown(api: ApiClient) -> None:
    response = api.delete(_query_path(UNKNOWN_ID))
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.reliability
def test_run_query_404_unknown(api: ApiClient) -> None:
    response = api.post(_query_path(UNKNOWN_ID, "/run"), json_body={})
    assert response.status_code == 404, f"status={response.status_code} {response.text[:300]}"


@pytest.mark.reliability
def test_run_query_415_wrong_content_type(api: ApiClient, scratch_saved_query: SavedQuery) -> None:
    """`/run` takes an OPTIONAL body, and still refuses one it cannot read.

    Optional is the reason to assert it separately from the create case above:
    a route that may be called with no body at all is the one where "ignore what
    I cannot parse" is a plausible implementation, and ignoring it would mean
    running the query with silently discarded parameters.

    An existing query on purpose — a 404 would satisfy a status-only assertion
    for the wrong reason, since the media type is checked before the lookup.
    """
    response = api.post(
        _query_path(scratch_saved_query.id, "/run"),
        content="{}",
        headers={"Content-Type": "text/plain"},
    )
    assert response.status_code == 415, f"status={response.status_code} {response.text[:300]}"


# ---------------------------------------------------------------------------
# The SQL gate, and what `/run` binds
# ---------------------------------------------------------------------------


@pytest.mark.parametrize(
    "statement",
    ["DROP TABLE metrics", "INSERT INTO metrics VALUES (1)"],
    ids=["drop", "insert"],
)
@pytest.mark.security
def test_a_statement_that_is_not_a_read_is_refused_on_create(
    api: ApiClient, statement: str
) -> None:
    """The single-SELECT gate, at the point a query is stored.

    The most consequential validation on this surface: a saved query is SQL a
    caller gets to run later, so anything that lands here runs with the
    service's own ClickHouse credentials.
    """
    response = api.post(QUERIES, json_body={"name": scratch_name("bad-sql"), "sql": statement})
    assert response.status_code == 400, (
        f"{statement!r} was accepted as a saved query ({response.status_code}): "
        f"{response.text[:300]}"
    )


@pytest.mark.security
def test_an_update_revalidates_the_sql(api: ApiClient, scratch_saved_query: SavedQuery) -> None:
    """And again on update — a stored query that passed once can be rewritten.

    Validating only on create would leave the gate trivially bypassable: store
    a `SELECT`, then PUT anything.
    """
    response = api.put(
        _query_path(scratch_saved_query.id), json_body={"sql": "INSERT INTO metrics VALUES (1)"}
    )
    assert response.status_code == 400, (
        f"a non-read statement was accepted on update ({response.status_code}): "
        f"{response.text[:300]}"
    )


@pytest.mark.reliability
def test_a_deleted_query_leaves_the_listing_and_the_id_stops_resolving(
    api: ApiClient,
) -> None:
    """Hard delete, asserted from both directions.

    The listing and the by-id read can rot independently — a delete that
    unlinks the row from the listing while leaving it readable by id is a
    plausible half-implementation, and only checking both catches it.
    """
    query = create_saved_query(api, "deleted")
    assert query.name in _saved(api)
    assert api.delete(_query_path(query.id)).status_code == 204

    assert query.name not in _saved(api), "a deleted saved query is still listed"
    gone = api.get(_query_path(query.id))
    assert gone.status_code == 404, (
        f"a deleted saved query still reads back ({gone.status_code}): {gone.text[:300]}"
    )


@pytest.mark.security
def test_run_binds_the_tenant_from_the_session_not_the_request(
    api: ApiClient, stand_manifest: Manifest
) -> None:
    """`{tenant}` comes out of the signed session, and a caller cannot supply it.

    The assertion this suite exists to be able to make. In the rig the tenant
    comes from a token the test minted, so the value proves only that the code
    read its own input; here it has travelled Keycloak login → gateway JWT →
    analytics → ClickHouse, and comes back as a row. A regression that let a
    client name its own tenant would read every other tenant's data through a
    saved query.
    """
    query = create_saved_query(
        api, "tenant-param", sql="SELECT {tenant:String} AS tenant FROM system.one"
    )
    try:
        response = api.post(_query_path(query.id, "/run"), json_body={"tenant": "not-my-tenant"})
        assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
        assert response.parse(RunResponse).rows == [{"tenant": stand_manifest.tenant}], (
            "the tenant bound into the query is not the session's — a caller-supplied "
            "value reached the parameter"
        )
    finally:
        api.delete(_query_path(query.id))


@pytest.mark.reliability
def test_run_binds_a_named_parameter_from_the_body(api: ApiClient) -> None:
    query = create_saved_query(
        api, "period-param", sql="SELECT {period:String} AS period FROM system.one"
    )
    try:
        response = api.post(_query_path(query.id, "/run"), json_body={"period": "2026-Q1"})
        assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
        assert response.parse(RunResponse).rows == [{"period": "2026-Q1"}]
    finally:
        api.delete(_query_path(query.id))


@pytest.mark.reliability
def test_running_with_a_parameter_left_unbound_is_400_not_500(api: ApiClient) -> None:
    """An unbound parameter is the caller's mistake, and must be reported as one.

    ClickHouse raises UNKNOWN_QUERY_PARAMETER, which reaches the client as a
    bare 500 unless the service classifies it. The difference matters to whoever
    is holding the failure: a 500 says the product broke, a 400 says the request
    was incomplete and names what is missing.
    """
    query = create_saved_query(
        api, "missing-param", sql="SELECT {period:String} AS period FROM system.one"
    )
    try:
        response = api.post(_query_path(query.id, "/run"), json_body={})
        assert response.status_code == 400, (
            f"an unbound query parameter answered {response.status_code} rather than "
            f"classifying the caller's omission: {response.text[:300]}"
        )
    finally:
        api.delete(_query_path(query.id))
