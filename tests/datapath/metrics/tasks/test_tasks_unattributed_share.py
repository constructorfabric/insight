"""Work that reaches no person is the organisation's unattributed share.

Gold keeps an unassigned issue with an empty account and email, and keeps an
unbound account's work under that account. The runtime resolves neither to a
person, so per-person values never carry them; the unattributed view sums exactly
those rows across the tenant, and the members' values plus it make the total.
Logged time follows the worklog's author, so dave's hours on an unassigned issue
stay his.
"""

from __future__ import annotations

from dataclasses import replace

import pytest
from insight_datapath.caller import StandCaller
from insight_datapath.metric_expect import MetricResponse, approx, one
from insight_datapath.spec_runner import SpecRun
from insight_stand.personas import PersonaSession

pytestmark = pytest.mark.fixture

SPEC = "tasks_unattributed_share"

DAVE = "dave@example.com"

APRIL = {"from": "2026-04-01", "to": "2026-04-30"}

SHARES = [
    pytest.param("tasks.closed", 2, 3, id="closed issues"),
    pytest.param("tasks.logged_hours", 8, 4, id="logged hours"),
]


def results(
    spec: SpecRun, metric_key: str, views: list[dict[str, str]], person: str = DAVE
) -> MetricResponse:
    return spec.call(
        {
            "url": "/v1/metric-results",
            "method": "POST",
            "body": {
                "entity": {"type": "person", "ids": [person]},
                "period": APRIL,
                "metrics": [{"metric_key": metric_key, "views": views}],
            },
        }
    )


@pytest.fixture
def as_admin(spec: SpecRun, operator_session: PersonaSession) -> tuple[SpecRun, str]:
    """The figure is organisation-wide, so the org-chart stand serves it to an admin only.

    The admin operator sits outside the org chart and sees nobody but itself, so its
    own person anchors the request: the view reads the tenant, not the entity.
    """
    return replace(spec, caller=StandCaller(operator_session.client)), operator_session.person.uuid


@pytest.mark.parametrize(("metric_key", "dave", "unattributed"), SHARES)
def test_a_person_carries_only_the_work_resolved_to_them(
    spec: SpecRun, metric_key: str, dave: float, unattributed: float
) -> None:
    """Neither the unbound account's nor the unassigned issues' work reaches dave."""
    r = results(spec, metric_key, [{"view": "period"}])
    assert r.status == 200
    r.row(metric_key, "period", entity_id=DAVE).equals(value=dave)


@pytest.mark.parametrize(("metric_key", "dave", "unattributed"), SHARES)
def test_the_unattributed_share_holds_exactly_what_reaches_no_person(
    spec: SpecRun, as_admin: tuple[SpecRun, str], metric_key: str, dave: float, unattributed: float
) -> None:
    """Dave's value plus the unattributed share is everything seeded."""
    lead_read = results(spec, metric_key, [{"view": "period"}])
    assert lead_read.status == 200
    lead_read.row(metric_key, "period", entity_id=DAVE).equals(value=dave)

    admin, anchor = as_admin
    r = results(admin, metric_key, [{"view": "unattributed"}], person=anchor)
    assert r.status == 200, r.payload
    assert r.unattributed(metric_key) == approx(unattributed), f"{metric_key}: unattributed share"


def test_a_lead_may_not_read_the_organisation_wide_share(spec: SpecRun) -> None:
    """Under the org-chart policy an ordinary lead's subtree owns no share of it."""
    r = results(spec, "tasks.closed", [{"view": "unattributed"}])
    assert r.status == 403, r.payload


def test_logged_hours_serve_every_view_the_catalogue_requires(spec: SpecRun) -> None:
    """Dave's 8 h by day; a pool of dave alone is below the peer minimum, so no quantiles."""
    r = results(
        spec,
        "tasks.logged_hours",
        [{"view": "period"}, {"view": "peer"}, {"view": "timeseries", "bucket": "day"}],
    )
    assert r.status == 200

    r.row("tasks.logged_hours", "period", entity_id=DAVE).equals(value=8)
    r.row("tasks.logged_hours", "peer", entity_id=DAVE).equals(
        target_value=8, p25=None, median=None, p75=None, min=None, max=None, n=1
    )
    points = one(r.series("tasks.logged_hours"), entity_id=DAVE)["points"]
    logged = {point["bucket_start"][:10]: point["value"] for point in points if point["value"]}
    assert logged == {"2026-04-07": approx(6), "2026-04-09": approx(2)}, logged
