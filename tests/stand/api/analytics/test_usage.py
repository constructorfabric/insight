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


# ---------------------------------------------------------------------------
# The attribution and counting rules.
#
# Everything above proves a beacon survives the round trip. These four ask what
# the recorded rows MEAN once an admin reads them back: what counts as one
# visit, who a visit is credited to, and whose report it may appear in. Each is
# a criterion of #2573 that nothing exercised before 2026-08-20.
# ---------------------------------------------------------------------------


def _page_view(path: str, session_id: str, **extra: JsonValue) -> JsonValue:
    """One page view in a named sitting, with room for fields ingest must ignore."""
    record: dict[str, JsonValue] = {
        "name": "page_view",
        "context_session_id": session_id,
        "context_app_name": "insight-stand-tests",
        "context_app_version": "0",
        "data": {"path": path},
    }
    record.update(extra)
    return record


def _send(session: PersonaSession, records: list[JsonValue]) -> None:
    """Post one SDK batch and require the 204 that means it was taken."""
    accepted = session.client.post(
        EVENTS,
        content=json.dumps({"meta": {}, "records": records}),
        headers={"Content-Type": SDK_CONTENT_TYPE},
    )
    assert accepted.status_code == 204, (
        f"ingest answered {accepted.status_code}, expected 204: {accepted.text[:300]}"
    )


def _tag(suffix: str) -> str:
    """A path no other run and no other test in this run can collide with."""
    return f"/portal/{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-{suffix}"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_one_sitting_of_five_pages_counts_as_one_visit(
    lead_session: PersonaSession, admin_operator_session: PersonaSession
) -> None:
    """A visit is a sitting, not a page load.

    The two numbers move on different axes and the whole headline rests on that:
    "how many visits" answers how often people came back, "pages viewed" how far
    they went once here. Counting a page load as a visit would inflate the first
    into a copy of the second, and no other test here separates them — the
    module above only ever sends one page view at a time.

    Five distinct paths in one named sitting, so the assertion can be exact in
    both directions rather than a lower bound.
    """
    if not _config(lead_session.client).enabled:
        pytest.skip("this instance does not record usage")

    admin = admin_operator_session.client
    day = _today()
    before = _summary(admin, day).totals

    sitting = f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-sitting"
    paths = [_tag(f"sitting-{index}") for index in range(5)]
    _send(lead_session, [_page_view(path, sitting) for path in paths])

    wait_until(
        lambda: paths[-1] in {page.path for page in _summary(admin, day).by_page},
        timeout_s=20,
        description="the last page view of the sitting to reach the summary",
    )

    after = _summary(admin, day)
    pages = {page.path: page for page in after.by_page}
    missing = [path for path in paths if path not in pages]
    assert not missing, f"the sitting lost page views: {missing}"
    assert [pages[path].views for path in paths] == [1] * 5

    assert after.totals.page_views - before.page_views == 5, (
        "five page views in one sitting did not add five to the pages figure"
    )
    assert after.totals.visits - before.visits == 1, (
        f"five page views in ONE sitting added {after.totals.visits - before.visits} "
        "visits; a visit is a sitting, so exactly one was expected"
    )


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.security
def test_a_person_named_in_the_body_is_ignored_and_the_sender_credited(
    lead_session: PersonaSession, admin_operator_session: PersonaSession
) -> None:
    """The identity comes off the session, never out of the message.

    The beacon is sent by a browser, so its body is caller-controlled: anybody
    signed in can put somebody else's person id in it. If ingest ever preferred
    that field, one employee could write page views onto a colleague's name in a
    report the organisation reads. The service's own unit tests copy the handed
    identity, but their fixture body carries no identity-shaped field at all, so
    a handler that started trusting one would still pass them.

    Three spellings, because the failure to catch is a future field name being
    honoured rather than any one of them being honoured today.
    """
    if not _config(lead_session.client).enabled:
        pytest.skip("this instance does not record usage")

    admin = admin_operator_session.client
    day = _today()
    sender = lead_session.person.uuid
    impostor = "00000000-0000-4000-8000-00000000dead"

    before = {person.person_id: person for person in _summary(admin, day).by_person}
    sender_before = before[sender].page_views if sender in before else 0

    path = _tag("forged-identity")
    _send(
        lead_session,
        [
            _page_view(
                path,
                f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-forged",
                context_person_id=impostor,
                person_id=impostor,
                user_id=impostor,
            )
        ],
    )

    wait_until(
        lambda: path in {page.path for page in _summary(admin, day).by_page},
        timeout_s=20,
        description="the forged-identity page view to reach the summary",
    )

    after = {person.person_id: person for person in _summary(admin, day).by_person}
    assert impostor not in after, (
        "a person id supplied in the request body reached the report as a visitor"
    )
    assert sender in after, "the sender is absent from the report they wrote to"
    assert after[sender].page_views == sender_before + 1, (
        "the page view was not credited to the session that sent it"
    )


@pytest.mark.requires_seed("admin_operator", "other_tenant_lead")
@pytest.mark.security
def test_another_organisations_activity_stays_out_of_this_report(
    other_tenant_session: PersonaSession, admin_operator_session: PersonaSession
) -> None:
    """One store holds every organisation's record of who read what.

    Usage is the first surface where one table carries every customer's people,
    screens and totals, and the only thing keeping them apart is the tenant bind
    on each read. Every other test in this module runs inside a single
    organisation, so that bind is unproven — dropping it from the reads would
    leave them all green while every admin saw every customer.

    The second organisation has no admin of its own on a seeded stand, so this
    asserts the half that matters to a customer: what they must NOT see.
    """
    if not _config(other_tenant_session.client).enabled:
        pytest.skip("this instance does not record usage")

    admin = admin_operator_session.client
    day = _today()

    path = _tag("other-tenant")
    _send(
        other_tenant_session,
        [_page_view(path, f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-other-tenant")],
    )

    # No wait: the row must never appear, so there is nothing to wait FOR. Give
    # the batched insert the same window the positive tests allow it, then read.
    wait_until(
        lambda: _summary(admin, day).totals.page_views >= 0,
        timeout_s=2,
        description="the insert window to pass",
    )

    report = _summary(admin, day)
    assert path not in {page.path for page in report.by_page}, (
        "a page view recorded by another organisation is listed in this admin's report"
    )
    assert other_tenant_session.person.uuid not in {
        person.person_id for person in report.by_person
    }, "a visitor from another organisation is listed in this admin's report"


#: Analytics' own address for the ingest route. The `/api/analytics` prefix
#: belongs to the gateway, and the caller below deliberately does not go there.
DIRECT_EVENTS = "/v1/usage/events"


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
@pytest.mark.xfail(
    strict=True,
    reason=(
        "#2573 AC-7: a bearer-carrying caller records usage against analytics and "
        "reaches the report as a nameless visitor, counted as a distinct one. The "
        "page count is right; the people table is not. Strict, so this flips the "
        "moment it is fixed."
    ),
)
def test_a_visit_that_names_nobody_is_counted_but_not_listed(
    analytics_service_client: ApiClient,
    lead_session: PersonaSession,
    admin_operator_session: PersonaSession,
) -> None:
    """A visit the product cannot tie to a person counts, and names nobody.

    Reaching that state needs a caller who is authenticated but is not a person,
    and the published contract supplies one: `/v1/usage/events` is declared under
    `bearerAuth`, so a service principal is a documented caller. The gateway
    refuses it — it is a browser BFF and looks for a session cookie — but the
    edge is not the only address analytics answers on.

    What the report must not do is invent a visitor out of it. The row carries a
    subject id that no identity row describes, so the people table has neither a
    name nor a handle to render, and the visitor count has nothing to count.
    """
    if not _config(lead_session.client).enabled:
        pytest.skip("this instance does not record usage")

    admin = admin_operator_session.client
    day = _today()
    before = _summary(admin, day).totals

    path = _tag("no-person")
    accepted = analytics_service_client.post(
        DIRECT_EVENTS,
        content=json.dumps(
            {
                "meta": {},
                "records": [
                    _page_view(path, f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-no-person")
                ],
            }
        ),
        headers={"Content-Type": SDK_CONTENT_TYPE},
    )
    assert accepted.status_code == 204, (
        f"analytics answered {accepted.status_code} to a service principal: {accepted.text[:300]}"
    )

    wait_until(
        lambda: path in {page.path for page in _summary(admin, day).by_page},
        timeout_s=20,
        description="the unattributable page view to reach the summary",
    )

    after = _summary(admin, day)
    assert after.totals.page_views - before.page_views == 1, (
        "a visit nobody can be named for was dropped from the page count"
    )

    nameless = [
        person
        for person in after.by_person
        if not person.display_name.strip() and not person.username.strip()
    ]
    assert not nameless, "the visitor list carries a row that names nobody: " + ", ".join(
        f"{person.person_id} ({person.page_views} pages)" for person in nameless
    )


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("admin_operator")
@pytest.mark.security
def test_the_edge_refuses_a_bearer_carrying_beacon(
    gateway_service_client: ApiClient,
    lead_session: PersonaSession,
    admin_operator_session: PersonaSession,
) -> None:
    """The gateway is a browser BFF, and usage ingest is behind it like the rest.

    Worth its own case because ingest answers 204 whatever becomes of the write:
    if bearer traffic ever reached it through the edge, rows nobody can
    attribute would start accumulating and nothing would say so. This is the
    half of that boundary the edge is responsible for.

    Sent to the GATEWAY deliberately. A bearer aimed at a service that does not
    serve the route answers 404, which satisfies "refused" while proving
    nothing about the edge.
    """
    if not _config(lead_session.client).enabled:
        pytest.skip("this instance does not record usage")

    admin = admin_operator_session.client
    day = _today()
    before = _summary(admin, day).totals

    path = _tag("edge-bearer")
    refused = gateway_service_client.post(
        EVENTS,
        content=json.dumps(
            {
                "meta": {},
                "records": [
                    _page_view(path, f"{scratch.SCRATCH_PREFIX}-{scratch.RUN_TAG}-edge-bearer")
                ],
            }
        ),
        headers={"Content-Type": SDK_CONTENT_TYPE},
    )

    assert refused.status_code == 401, (
        f"the edge answered {refused.status_code} to a bearer-carrying beacon, expected 401: "
        f"{refused.text[:300]}"
    )

    after = _summary(admin, day)
    assert path not in {page.path for page in after.by_page}, "a refused beacon was recorded anyway"
    assert after.totals.page_views == before.page_views, (
        f"the day's page count moved on a refused beacon: "
        f"{before.page_views} -> {after.totals.page_views}"
    )
