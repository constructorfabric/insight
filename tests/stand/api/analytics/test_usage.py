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
policy does not reach them. `/v1/usage/summary` is the table's only reader, so
nothing else in the suite sees what accumulates — but this module has to defend
itself: the breakdowns are ranked top-N lists, so every read here asks for THIS
RUN's day only. Left on the default window, a stand with more than
`BREAKDOWN_LIMIT` single-view paths from earlier runs would tie-break the fresh
one out of `by_page`, and the wait below would fail for a reason that has nothing
to do with the code under test.

The 401 half is in `test_gateway.py`, swept over every operation at once.
"""

from __future__ import annotations

import json
from datetime import UTC, datetime

import pytest
from insight_stand import ApiClient, PersonaSession, analytics_path, wait_until
from insight_stand.api import JsonValue

from .. import scratch
from ..schemas import ProblemDocument, UsageConfigResponse, UsageSummaryResponse

EVENTS = analytics_path("/v1/usage/events")
CONFIG = analytics_path("/v1/usage/config")
SUMMARY = analytics_path("/v1/usage/summary")

#: A page in the shape the SPA sends: the person the screen is about is already
#: reduced to `:id` before it leaves the browser. Ingest stores the path as it
#: arrives, so this is both what is sent and what is read back.
BEACON_PATH = f"/ic/:id/{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}"


#: What the SDK labels its body. Not `application/json` — the transport speaks a
#: Kafka REST Proxy media type, and the extractor accepts it only because the
#: mime suffix is `+json`. Sending the real one is the point: a proxy or an
#: extractor change that stopped accepting it would take usage down silently.
SDK_CONTENT_TYPE = "application/vnd.kafka.json.v2+json"


def _beacon(path: str) -> JsonValue:
    """One SDK v2 body carrying one page view, in the wire form ingest takes.

    `meta` is empty here on purpose: the SDK only hoists shared fields when a
    batch holds more than one record, so a single beacon carries everything
    inline. The hoisted case is covered by the service's own unit tests.
    """
    return {
        "meta": {},
        "records": [
            {
                "name": "page_view",
                "context_session_id": f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}",
                "context_app_name": "insight-stand-tests",
                "context_app_version": "0",
                "data": {"path": path},
            }
        ],
    }


def _config(client: ApiClient) -> UsageConfigResponse:
    response = client.get(CONFIG)
    assert response.status_code == 200, f"config: {response.status_code} {response.text[:300]}"
    return response.parse(UsageConfigResponse)


def _today() -> str:
    """The server buckets by UTC day, so the run's own window is the UTC date."""
    return datetime.now(UTC).date().isoformat()


def _summary(client: ApiClient, since: str) -> UsageSummaryResponse:
    # `since` is captured before the write and `until` read now, so a run that
    # straddles UTC midnight still spans the day its own beacon landed on.
    response = client.get(SUMMARY, params={"since": since, "until": _today()})
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

    day = _today()
    accepted = lead_session.client.post(
        EVENTS,
        content=json.dumps(_beacon(BEACON_PATH)),
        headers={"Content-Type": SDK_CONTENT_TYPE},
    )
    assert accepted.status_code == 204, (
        f"ingest answered {accepted.status_code}, expected 204: {accepted.text[:300]}"
    )

    admin = admin_operator_session.client
    wait_until(
        lambda: BEACON_PATH in {page.path for page in _summary(admin, day).by_page},
        timeout_s=20,
        description=f"the page view at {BEACON_PATH} to reach the usage summary",
    )
    return _summary(admin, day)


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
    assert BEACON_PATH in pages, (
        f"the recorded page is absent from the summary; it lists {sorted(pages)[:20]}"
    )
    assert pages[BEACON_PATH].views >= 1
    assert summary_after_a_beacon.totals.page_views >= 1


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_a_beacon_carrying_no_session_is_not_a_visit(
    lead_session: PersonaSession, admin_operator_session: PersonaSession
) -> None:
    """A visit is a session; a record without one cannot be counted as either.

    Ingest stores a missing `context_session_id` as `''`, and every such row —
    across every person and every day — is the same value. Counted naively they
    fold into exactly one phantom visit that never grows and never leaves.
    """
    if not _config(lead_session.client).enabled:
        pytest.skip("this instance does not record usage")

    admin = admin_operator_session.client
    day = _today()
    before = _summary(admin, day).totals.visits

    path = f"/portal/{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-sessionless"
    sessionless: JsonValue = {
        "meta": {},
        "records": [{"name": "page_view", "data": {"path": path}}],
    }
    accepted = lead_session.client.post(
        EVENTS,
        content=json.dumps(sessionless),
        headers={"Content-Type": SDK_CONTENT_TYPE},
    )
    assert accepted.status_code == 204, accepted.text[:300]

    wait_until(
        lambda: path in {page.path for page in _summary(admin, day).by_page},
        timeout_s=20,
        description="the sessionless page view to reach the summary",
    )

    assert _summary(admin, day).totals.visits == before, (
        "a row with no session id registered a visit"
    )


def _figures_for(summary: UsageSummaryResponse, person_id: str) -> tuple[int, int]:
    """That person's (visits, page_views) in this summary. (0, 0) when absent."""
    for person in summary.by_person:
        if person.person_id == person_id:
            return person.visits, person.page_views
    return 0, 0


@pytest.mark.requires_seed("dev_lead", "admin_operator")
@pytest.mark.security
def test_the_sender_owns_the_visit_whoever_the_message_names(
    lead_session: PersonaSession, admin_operator_session: PersonaSession
) -> None:
    """#2573 scenario 9 — the session decides whose visit it is, not the body.

    The message names a second person everywhere a handler could be tempted to
    read an identity from: beside the record, inside its data, and in the meta
    the batch shares. The wire type declares none of these, and serde drops what
    it does not declare — which is the property under test, not an accident to
    rely on silently.
    """
    if not _config(lead_session.client).enabled:
        pytest.skip("this instance does not record usage")

    admin = admin_operator_session.client
    day = _today()
    sender = lead_session.person.uuid
    named = admin_operator_session.person.uuid
    assert sender != named, "the sender and the person named must differ"

    before = _summary(admin, day)
    before_sender = _figures_for(before, sender)
    before_named = _figures_for(before, named)

    tag = f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-impostor"
    path = f"/portal/{tag}"
    claimed: JsonValue = {
        "meta": {"person_id": named, "user_id": named},
        "records": [
            {
                "name": "page_view",
                "context_session_id": tag,
                "context_app_name": "insight-stand-tests",
                "context_app_version": "0",
                "person_id": named,
                "user_id": named,
                "email": admin_operator_session.email,
                "data": {
                    "path": path,
                    "person_id": named,
                    "email": admin_operator_session.email,
                },
            }
        ],
    }
    accepted = lead_session.client.post(
        EVENTS,
        content=json.dumps(claimed),
        headers={"Content-Type": SDK_CONTENT_TYPE},
    )
    assert accepted.status_code == 204, accepted.text[:300]

    wait_until(
        lambda: path in {page.path for page in _summary(admin, day).by_page},
        timeout_s=20,
        description="the page view claiming to be somebody else to reach the summary",
    )

    after = _summary(admin, day)
    assert _figures_for(after, sender) == (before_sender[0] + 1, before_sender[1] + 1), (
        f"the sender was not credited: {before_sender} -> {_figures_for(after, sender)}"
    )
    assert _figures_for(after, named) == before_named, (
        f"the person the message named gained activity they did not make: "
        f"{before_named} -> {_figures_for(after, named)}"
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
