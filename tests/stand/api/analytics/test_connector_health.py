"""The `/v1/connector-health` pair on analytics — what each connector recorded.

    GET /v1/connector-health                    200 admin · 403 everybody else
    GET /v1/connector-health/{connector}/runs   200 admin · 403 everybody else

Both are `.authenticated()` at the edge with the admin gate inside the handler,
the same shape `/v1/usage/summary` and the feedback listing use, so the gateway
sees two ordinary authenticated routes and the refusal is only observable with a
session.

The read is instance-wide on purpose: the schemas it reports on are not
partitioned by tenant, so there is no tenant-scoped variant to fall back to and
a non-admin must be refused outright rather than served a narrowed view.

What this module does NOT assert is any number. A compose stand runs neither the
mover nor the workflow layer, so nothing writes the run ledger there and every
figure is legitimately absent — asserting a count would pin the suite to a stand
that happens to have been seeded. What it asserts instead is the contract: the
shape validates, the gate holds, an unknown connector is an empty history rather
than an error, and absence never renders as a zero.

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, PersonaSession, analytics_path

from ..operations import SOME_CONNECTOR
from ..schemas import ConnectorHealthResponse, ConnectorRunsResponse, ProblemDocument

HEALTH = analytics_path("/v1/connector-health")


def _runs_path(connector: str) -> str:
    return analytics_path(f"/v1/connector-health/{connector}/runs")


@pytest.fixture(scope="module")
def health(admin_operator_session: PersonaSession) -> ConnectorHealthResponse:
    response = admin_operator_session.client.get(HEALTH)
    assert response.status_code == 200, (
        f"the health read answered {response.status_code}: {response.text[:300]}"
    )
    return response.parse(ConnectorHealthResponse)


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_health_read_serves_the_operator_a_valid_contract(
    health: ConnectorHealthResponse,
) -> None:
    """A validating parse IS the assertion: the models are generated from the
    service's own published contract, so a failure means the two disagree."""
    assert health.as_of, "the response does not say when it was assembled"
    # `history_available` is a fact about this stand, not a requirement on it: a
    # stand where nothing records answers False, and the page says so.
    assert isinstance(health.history_available, bool)


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_an_unmeasured_delivery_is_absent_rather_than_a_zero(
    health: ConnectorHealthResponse,
) -> None:
    """The distinction the whole pairing exists for.

    A sync the pipeline did not measure — swept, out-of-band, or backfilled —
    must report no measurement. A zero there is the "reported records, nothing
    landed" finding, so serialising absence as zero would fabricate it.
    """
    for row in health.connectors:
        sync = row.last_sync
        if sync is None or sync.rows_landed is not None:
            continue
        assert sync.rows_landed is None, (
            f"{row.connector}: an unmeasured delivery must stay absent, not become 0"
        )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_syncs_trigger_is_never_asserted_as_manual_without_evidence(
    health: ConnectorHealthResponse,
) -> None:
    """`unclaimed` is unknown provenance, and the vocabulary must stay closed.

    A new word appearing here would let the page fall through to whatever its
    default branch says about a sync nobody claimed.
    """
    allowed = {"claimed", "out_of_band", "unclaimed"}
    for row in health.connectors:
        if row.last_sync is None:
            continue
        assert row.last_sync.trigger in allowed, (
            f"{row.connector}: unknown trigger {row.last_sync.trigger!r}, expected one of {allowed}"
        )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_connector_nothing_recorded_is_an_empty_history_not_an_error(
    admin_operator_session: PersonaSession,
) -> None:
    """A pasted or stale link must not look broken.

    There is no 404 here by design: a connector nobody ever ran is a connector
    with no runs, which is exactly what a fresh install looks like.
    """
    response = admin_operator_session.client.get(_runs_path(SOME_CONNECTOR))
    assert response.status_code == 200, (
        f"an unknown connector answered {response.status_code}: {response.text[:300]}"
    )

    history = response.parse(ConnectorRunsResponse)
    assert history.connector == SOME_CONNECTOR
    assert history.runs == [], "an unknown connector cannot have recorded runs"


@pytest.mark.reliability
def test_a_non_admin_is_refused_the_instance_wide_read(api: ApiClient) -> None:
    """No tenant-scoped variant exists, so the only honest answer is a refusal."""
    for path in (HEALTH, _runs_path(SOME_CONNECTOR)):
        response = api.get(path)
        assert response.status_code == 403, (
            f"{path} answered {response.status_code} for a non-admin: {response.text[:300]}"
        )
        assert response.parse(ProblemDocument).status == 403
