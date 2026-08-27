"""The `/v1/connector-health` pair on analytics — what each connector recorded.

    GET /v1/connector-health                     200 admin · 403 everybody else
    GET /v1/connector-health/{connector}/syncs   200 admin · 400 unparseable name ·
                                                 403 everybody else

Both are `.authenticated()` at the edge with the operator gate inside the
handler, the same shape `/v1/usage/summary` and the feedback listing use, so the
gateway sees two ordinary authenticated routes and the refusal is only
observable with a session.

The read is instance-wide on purpose: the schemas it reports on are not
partitioned by tenant, so there is no tenant-scoped variant to fall back to and
a non-admin is refused outright rather than served a narrowed view.

What this module does NOT assert is any figure. A compose stand runs no data
mover, so nothing writes the ledger there and every count is legitimately
absent; asserting one would pin the suite to a stand that happened to have been
seeded. What holds on any stand is the shape, the gate, and the two honesty
properties the page turns on — that an empty ledger says so instead of implying
health, and that an unknown connector is an empty window rather than an error.

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

import pytest
from insight_stand import PersonaSession, analytics_path

from ..operations import SOME_CONNECTOR
from ..schemas import ConnectorHealthResponse, ProblemDocument, SyncHistoryResponse

SUMMARY = analytics_path("/v1/connector-health")


def _syncs(connector: str) -> str:
    return analytics_path(f"/v1/connector-health/{connector}/syncs")


@pytest.fixture(scope="module")
def summary(admin_operator_session: PersonaSession) -> ConnectorHealthResponse:
    """Read the summary once: it is instance-wide, so every test sees the same
    answer and none of them races another into writing one."""
    response = admin_operator_session.client.get(SUMMARY)
    assert response.status_code == 200, (
        f"summary answered {response.status_code}: {response.text[:300]}"
    )
    return response.parse(ConnectorHealthResponse)


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_summary_validates_against_the_published_contract(
    summary: ConnectorHealthResponse,
) -> None:
    """A contract test, not a data test.

    `extra="forbid"` on the generated model means an undeclared field fails
    here, which is the drift worth catching: the page is written against the
    contract and the service is what actually serializes.
    """
    assert summary.as_of, "the answer must date itself even with nothing recorded"
    for row in summary.connectors:
        assert row.connector, "a row with no connector names nothing"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_an_unrecorded_install_says_so_rather_than_implying_health(
    summary: ConnectorHealthResponse,
) -> None:
    """A compose stand runs no mover, so this is the state it is actually in.

    The two fields have to agree: claiming history while naming no moment the
    mover was read would let the page present a picture nothing produced.
    """
    if summary.history_available:
        assert summary.checked_at is not None, (
            "history is claimed but no read is dated — the page would present "
            "facts it cannot place in time"
        )
        return

    assert summary.checked_at is None
    assert summary.typical_read_interval_ms is None, (
        "an interval measured from no reads is not a measurement"
    )
    assert summary.connectors == [], (
        "no history recorded, yet connectors are reported: "
        f"{[row.connector for row in summary.connectors]}"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_an_unknown_connector_is_an_empty_window_not_an_error(
    admin_operator_session: PersonaSession,
) -> None:
    """Asking about a connector with no recorded sync is a normal question.

    Answering 404 would make the page's own drill-down look broken on a
    connector that simply has not synced yet.
    """
    response = admin_operator_session.client.get(_syncs(SOME_CONNECTOR))
    assert response.status_code == 200, (
        f"answered {response.status_code}: {response.text[:300]}"
    )
    window = response.parse(SyncHistoryResponse)
    assert window.connector == SOME_CONNECTOR
    assert window.syncs == []
    assert window.window > 0, "the page needs the window's size to say it is one"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_name_the_route_cannot_parse_is_refused_as_a_bad_request(
    admin_operator_session: PersonaSession,
) -> None:
    """`404` would say the connector does not exist; it says the name is unusable.

    The distinction matters to whoever reads the refusal: one sends them looking
    for a missing connector, the other tells them the link is wrong.
    """
    response = admin_operator_session.client.get(_syncs("Not_A_Name"))
    assert response.status_code == 400, (
        f"answered {response.status_code}: {response.text[:300]}"
    )
    problem = response.parse(ProblemDocument)
    assert problem.status == 400


@pytest.mark.security
def test_a_lead_is_refused_both_surfaces(lead_session: PersonaSession) -> None:
    """An ordinary authenticated caller, not an anonymous one.

    The gate is inside the handler, so this is the only place it is observable —
    the edge admits the request and the refusal comes from the role check.
    """
    for path in (SUMMARY, _syncs(SOME_CONNECTOR)):
        response = lead_session.client.get(path)
        assert response.status_code == 403, (
            f"{path} answered {response.status_code} for a lead: {response.text[:300]}"
        )
        assert response.parse(ProblemDocument).status == 403


@pytest.mark.security
def test_a_realm_admin_without_the_operator_row_is_still_refused(
    realm_admin_session: PersonaSession,
) -> None:
    """The gate reads an active `admin` row, not the realm role.

    Worth its own case: a senior person's view of the organisation is not
    administrative authority over the install, and a gate that accepted the
    realm role would pass every other test here.
    """
    response = realm_admin_session.client.get(SUMMARY)
    assert response.status_code == 403, (
        f"answered {response.status_code}: {response.text[:300]}"
    )


@pytest.mark.requires_seed("other_tenant_lead")
@pytest.mark.security
def test_a_caller_from_another_tenant_is_refused(
    other_tenant_session: PersonaSession,
) -> None:
    """The surface is instance-wide, so there is no narrowed view to fall back
    to — a caller outside this install's operator role gets nothing."""
    response = other_tenant_session.client.get(SUMMARY)
    assert response.status_code == 403, (
        f"answered {response.status_code}: {response.text[:300]}"
    )
