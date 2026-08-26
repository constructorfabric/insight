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
that happens to have been seeded.

Two tests do need recorded runs to mean anything, and they carry
`requires_ingestion` so a stand without a writer SKIPS them with a reason rather
than iterating an empty list and reporting a pass. The rest hold on any stand:
the shape validates, the gate holds, and an unknown connector is an empty history
rather than an error.

The absent-vs-zero distinction is not re-asserted here as if it were an
end-to-end fact — the wire exposes no flag to check it against, so it is proven
where it can be, over the mapping, in the analytics unit tests. What this module
adds is that the two surfaces which each resolve it independently agree.

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
    """The validating parse IS the assertion.

    The models are generated from the service's own published contract, so a
    successful parse means the service and its contract agree and a failure means
    they do not. Nothing is added below it: `history_available` is a fact about
    this stand rather than a requirement on it, and re-asserting a field Pydantic
    has already typed would only look like a second check.
    """
    assert health.connectors is not None


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_ingestion
@pytest.mark.reliability
def test_the_summary_and_the_history_agree_on_whether_a_delivery_was_measured(
    admin_operator_session: PersonaSession, health: ConnectorHealthResponse
) -> None:
    """Two independent surfaces must not disagree about an absent measurement.

    The distinction between "measured zero" and "not measured" is what separates
    the "reported records, nothing landed" finding from a sync nobody measured,
    and each surface resolves it with its own statement. A read path that lost
    the distinction in one of them would show a connector as delivering on one
    screen and as misdelivering on the other.

    Only meaningful where something records runs, so it carries the ingestion
    capability rather than passing silently over an empty list.
    """
    # Keyed on the mover's job, not on the connector: the summary resolves ONE
    # sync by claim precedence while the history lists every event, so "any
    # event was measured" answers a different question and can disagree with a
    # correct summary.
    measured = {
        row.connector: (row.last_sync.job_id, row.last_sync.rows_landed is not None)
        for row in health.connectors
        if row.last_sync is not None and row.last_sync.job_id is not None
    }
    assert measured, "the stand declares ingestion but recorded no sync to compare"

    for connector, (job_id, summary_measured) in measured.items():
        response = admin_operator_session.client.get(_runs_path(connector))
        assert response.status_code == 200, (
            f"{connector}: history answered {response.status_code}: {response.text[:300]}"
        )
        syncs = [
            event
            for event in response.parse(ConnectorRunsResponse).runs
            if event.event == "sync.completed" and event.job_id == job_id
        ]
        if not syncs:
            continue
        history_measured = any(event.rows_landed is not None for event in syncs)
        assert history_measured == summary_measured, (
            f"{connector}: the summary says measured={summary_measured} while its history "
            f"says {history_measured} — one of the two lost the absent-vs-zero distinction"
        )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.requires_ingestion
@pytest.mark.reliability
def test_a_syncs_trigger_is_never_asserted_as_manual_without_evidence(
    health: ConnectorHealthResponse,
) -> None:
    """`unclaimed` is unknown provenance, and the vocabulary must stay closed.

    A new word appearing here would let the page fall through to whatever its
    default branch says about a sync nobody claimed. The wire types `trigger` as
    a plain string, so nothing generated can hold this vocabulary — it mirrors
    the domain's own values.

    Carries the ingestion capability: on a stand with no writer there is no sync
    to inspect, and a silent pass would read exactly like a check that ran.
    """
    allowed = {"claimed", "out_of_band", "unclaimed"}
    triggers = [row.last_sync.trigger for row in health.connectors if row.last_sync]
    assert triggers, "the stand declares ingestion but recorded no sync to inspect"

    unknown = sorted(set(triggers) - allowed)
    assert not unknown, f"triggers outside the closed vocabulary: {unknown}, expected {allowed}"


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


@pytest.mark.security
def test_a_non_admin_is_refused_the_instance_wide_read(api: ApiClient) -> None:
    """No tenant-scoped variant exists, so the only honest answer is a refusal."""
    for path in (HEALTH, _runs_path(SOME_CONNECTOR)):
        response = api.get(path)
        assert response.status_code == 403, (
            f"{path} answered {response.status_code} for a non-admin: {response.text[:300]}"
        )
        assert response.parse(ProblemDocument).status == 403
