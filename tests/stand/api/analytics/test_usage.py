"""The `/v1/usage/*` path group on analytics — adoption monitoring.

    POST /v1/usage/events    204 · the SPA's beacon, any signed-in caller
    GET  /v1/usage/config    200 · whether this instance records at all
    GET  /v1/usage/summary   200 for the admin operator · 400 malformed day ·
                             403 for everybody else

`/v1/usage/summary` is the first admin-gated operation analytics serves. The gate
is inside the handler, not at the edge: it asks identity `/v1/me` for an active
`admin` row — the same grant the identity admin API reads, and the seed grants it
to the admin operator alone. So the gateway sees three ordinary authenticated
routes, and the refusal is only observable with a session.

Ingest is the one place this suite writes rows it cannot remove. Usage events are
append-only and no operation deletes them, so `scratch.py`'s create-then-delete
policy does not reach them. Two things keep that safe: `/v1/usage/summary` is the
table's only reader, so nothing else in the suite can see what accumulates; and
the assertions below look for one run-tagged path rather than for a total, so a
stand that already holds events from earlier runs changes no outcome.

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, PersonaSession, analytics_path, wait_until
from insight_stand.api import JsonValue

from .. import scratch
from ..schemas import ProblemDocument, UsageConfigResponse, UsageSummaryResponse

EVENTS = analytics_path("/v1/usage/events")
CONFIG = analytics_path("/v1/usage/config")
SUMMARY = analytics_path("/v1/usage/summary")

#: A page whose middle segment is a person id — the shape `/ic/{id}/personal`
#: has in the product. The id is the suite's unclaimed stand-in, so asserting it
#: never reaches storage says something about scrubbing rather than about
#: whoever happens to be seeded.
BEACON_PATH = f"/ic/{scratch.UNKNOWN_ID}/{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}"

#: The same page after the server replaces the identifying segment.
STORED_PATH = f"/ic/:id/{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}"


def _beacon(path: str) -> JsonValue:
    """One SDK envelope carrying one page view, in the wire form ingest takes."""
    return {
        "records": [
            {
                "value": {
                    "name": "page_view",
                    "context_session_id": f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}",
                    "context_app_name": "insight-stand-tests",
                    "context_app_version": "0",
                    "data": {"path": path},
                }
            }
        ]
    }


def _config(client: ApiClient) -> UsageConfigResponse:
    response = client.get(CONFIG)
    assert response.status_code == 200, f"config: {response.status_code} {response.text[:300]}"
    return response.parse(UsageConfigResponse)


def _summary(client: ApiClient) -> UsageSummaryResponse:
    response = client.get(SUMMARY)
    assert response.status_code == 200, f"summary: {response.status_code} {response.text[:300]}"
    return response.parse(UsageSummaryResponse)


@pytest.fixture(scope="module")
def summary_after_a_beacon(
    lead_session: PersonaSession, admin_operator_session: PersonaSession
) -> UsageSummaryResponse:
    """Ingest one page view as an ordinary caller, then read it back as admin.

    Module-scoped so the run adds one undeletable row rather than one per test,
    and so the two assertions below describe the same recorded event instead of
    racing each other to write one.

    Polled rather than read straight through: the insert is batched server-side,
    so "accepted" and "visible" are two moments, and a bare read would be a
    flake waiting for a slow flush.
    """
    if not _config(lead_session.client).enabled:
        pytest.skip("this instance does not record usage, so no beacon can reach the summary")

    accepted = lead_session.client.post(EVENTS, json_body=_beacon(BEACON_PATH))
    assert accepted.status_code == 204, (
        f"ingest answered {accepted.status_code}, expected 204: {accepted.text[:300]}"
    )

    admin = admin_operator_session.client
    wait_until(
        lambda: STORED_PATH in {page.path for page in _summary(admin).by_page},
        timeout_s=20,
        description=f"the page view at {STORED_PATH} to reach the usage summary",
    )
    return _summary(admin)


@pytest.mark.reliability
def test_usage_config_is_readable_by_any_signed_in_caller(api: ApiClient) -> None:
    """The SPA reads this before it starts the SDK, so it cannot be admin-only.

    A regression that put the config behind the summary's gate would leave every
    non-admin session unable to tell collection-off from a broken endpoint, and
    the product would silently stop reporting for everyone but admins.
    """
    assert isinstance(_config(api).enabled, bool)


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_beacon_is_recorded_and_reaches_the_summary(
    summary_after_a_beacon: UsageSummaryResponse,
) -> None:
    """The whole point of the feature, end to end on the deployed path.

    Ingest accepting a beacon proves nothing on its own — the handler answers 204
    whatever happens to the write, deliberately, so that a tracking failure never
    surfaces in the product. Only the read model can show the event was stored,
    which is why the 204 and this assertion belong to one test.
    """
    pages = {page.path: page for page in summary_after_a_beacon.by_page}
    assert STORED_PATH in pages, (
        f"the recorded page is absent from the summary; it lists {sorted(pages)[:20]}"
    )
    assert pages[STORED_PATH].views >= 1
    assert summary_after_a_beacon.totals.page_views >= 1


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.security
def test_a_stored_page_never_carries_the_person_it_is_about(
    summary_after_a_beacon: UsageSummaryResponse,
) -> None:
    """A path names a screen; the id in it is scrubbed before the row is written.

    Without this, adoption counting quietly becomes a record of who opened whose
    profile — the same rows, read a different way. The scrub happens in the SPA
    and again at ingest, and this asserts the outcome of the second one, which is
    the only one a client cannot skip.
    """
    stored = [page.path for page in summary_after_a_beacon.by_page]
    assert STORED_PATH in stored
    assert not [path for path in stored if scratch.UNKNOWN_ID in path], (
        "a person id reached the usage table verbatim, so page paths identify "
        f"who was looked at: {[path for path in stored if scratch.UNKNOWN_ID in path]}"
    )


@pytest.mark.security
def test_the_summary_is_refused_without_the_admin_grant(api: ApiClient) -> None:
    """Admin-only is enforced by the service, not by hiding the nav entry.

    The SPA hides the page from non-admins, which is a courtesy and not a
    boundary: anybody signed in can address the url. This is the assertion that
    the boundary exists at all.
    """
    response = api.get(SUMMARY)
    assert response.status_code == 403, (
        f"the usage summary answered {response.status_code} to a caller holding no "
        f"admin grant: {response.text[:300]}"
    )
    problem = response.parse(ProblemDocument)
    assert problem.status == 403
    assert problem.detail, "the refusal carries no detail a caller can act on"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_malformed_day_is_refused_rather_than_queried(
    admin_operator_session: PersonaSession,
) -> None:
    """The window is parsed before ClickHouse is asked anything.

    `since` and `until` are caller-supplied and reach a query, so the failure to
    avoid is one where an unparseable value is carried far enough to become a
    500 — or worse, part of a statement.
    """
    response = admin_operator_session.client.get(SUMMARY, params={"since": "not-a-date"})
    assert response.status_code == 400, (
        f"a malformed `since` answered {response.status_code}: {response.text[:300]}"
    )
    assert response.parse(ProblemDocument).status == 400
