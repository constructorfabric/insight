"""What a seat-month metric promises that no other test in this suite asserts.

`ai.extra_usage_cost` and `ai.extra_usage_utilisation` are monthly facts about a
paid seat, and three of their properties are decided outside the metric value the
drilldown sweep reconciles:

* a seat with no ceiling has no ratio at all, and must not read as `0` — the room
  under a ceiling nobody set was never purchased;
* a seat-month is dated at the FIRST DAY of the month it bills for, so a window
  holding that day returns the month in full and a window inside the month
  returns nothing rather than a slice;
* the row therefore has to name the month it bills for, and the ceiling the
  amount is judged against, or a reader cannot place either.

Every expectation is derived from what the stand serves — the date and the
billing month come out of the evidence itself — so the suite keeps working when
the seed is anchored to another date.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Callable, Mapping

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, analytics_path

from ..schemas import MetricResultsResponse, PeriodView
from . import query_window

METRIC_RESULTS = analytics_path("/v1/metric-results")
DRILLDOWN = analytics_path("/v1/metric-drilldown")

MONEY = "ai.extra_usage_cost"
UTILISATION = "ai.extra_usage_utilisation"
USAGE_PRICED = "ai.cost"


def _period_values(
    client: ApiClient, person: str, keys: list[str], start: str, end: str
) -> dict[str, float | None]:
    """Each metric's period value for one person, keyed by metric."""
    response = client.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [person]},
            "period": {"from": start, "to": end},
            "metrics": [{"metric_key": key, "views": [{"view": "period"}]} for key in keys],
        },
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    values: dict[str, float | None] = {}
    for metric in response.parse(MetricResultsResponse).metrics:
        for view in metric.root.views:
            assert isinstance(view.root, PeriodView), (
                f"asked for the period view and got {type(view.root).__name__}"
            )
            for value in view.root.values:
                if value.entity_id == person:
                    values[metric.root.metric_key] = value.value
    return values


def _evidence(client: ApiClient, person: str, start: str, end: str) -> list[Mapping[str, object]]:
    """The money metric's evidence rows over a window, newest page first."""
    response = client.post(
        DRILLDOWN,
        json_body={
            "metric_key": MONEY,
            "entity": {"type": "person", "id": person},
            "period": {"from": start, "to": end},
            "filters": [],
            "display_dimensions": [],
            "limit": 100,
        },
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"
    return [row["values"] for row in response.json()["rows"]]


@pytest.mark.requires_seed("support_lead")
@pytest.mark.reliability
def test_a_seat_with_no_ceiling_reports_money_and_withholds_utilisation(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    """Honest-NULL over the wire: no ceiling means no ratio, never a zero.

    A zero would read as "used none of its allowance" for a seat that has no
    allowance to use. The support roster is the seeded set of unassigned seats
    (`generators/ai.py`), which is what makes this case reachable at all.
    """
    session = session_for("support_lead")
    start, end = query_window(stand_manifest)

    values = _period_values(session.client, session.person.uuid, [MONEY, UTILISATION], start, end)

    assert values.get(MONEY) is not None, (
        f"an unassigned seat reported no extra usage at all: {values}"
    )
    assert values.get(UTILISATION) is None, (
        f"a seat with no ceiling answered {values.get(UTILISATION)} for utilisation"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_a_window_holding_the_month_start_returns_the_whole_billing_month(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    """A monthly fact, not a daily accrual — and that is observable from outside.

    The date and the amount come from the evidence, so this asserts the dating
    rule rather than a seeded number: a one-day window on the month's first day
    returns the month in full, and a window inside the same month returns
    nothing.
    """
    session = session_for("dev_lead")
    start, end = query_window(stand_manifest)

    rows = _evidence(session.client, session.person.uuid, start, end)
    assert rows, "no seat-month evidence over the seeded window"
    row = rows[0]
    anchor = _dt.date.fromisoformat(str(row["date"]))
    month_total = float(row["value"])  # type: ignore[arg-type]

    assert anchor.day == 1, f"a seat-month row is dated {anchor}, which is not a month start"

    on_the_day = _period_values(
        session.client, session.person.uuid, [MONEY], anchor.isoformat(), anchor.isoformat()
    )
    assert on_the_day.get(MONEY) == pytest.approx(month_total), (
        f"a window of the month's first day alone answered {on_the_day.get(MONEY)}, not the "
        f"month's {month_total}"
    )

    # Days 2..21 of the same month — every month has them, and the row is not there.
    inside = _period_values(
        session.client,
        session.person.uuid,
        [MONEY],
        (anchor + _dt.timedelta(days=1)).isoformat(),
        (anchor + _dt.timedelta(days=20)).isoformat(),
    )
    assert inside.get(MONEY) is None, (
        f"a window inside the billing month answered {inside.get(MONEY)}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_the_usage_priced_and_the_billed_key_are_served_side_by_side(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    """One response, two figures, each from its own source and neither blended.

    `ai.cost` prices consumption at vendor rates; `ai.extra_usage_cost` is what
    the vendor billed on top of the seat fee. Serving the pair is the intended
    use — the two are never added.
    """
    session = session_for("dev_lead")
    start, end = query_window(stand_manifest)

    values = _period_values(session.client, session.person.uuid, [USAGE_PRICED, MONEY], start, end)

    assert values.get(USAGE_PRICED) is not None and values.get(MONEY) is not None, (
        f"the pair did not both answer: {values}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_the_seat_tier_breakdown_adds_up_to_the_period_value(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    """A dimension whose parts do not sum to the whole is a dimension nobody can act on."""
    session = session_for("dev_lead")
    start, end = query_window(stand_manifest)

    response = session.client.post(
        METRIC_RESULTS,
        json_body={
            "entity": {"type": "person", "ids": [session.person.uuid]},
            "period": {"from": start, "to": end},
            "metrics": [
                {
                    "metric_key": MONEY,
                    "views": [
                        {"view": "period"},
                        {"view": "breakdown", "dimensions": ["seat_tier"]},
                    ],
                }
            ],
        },
    )
    assert response.status_code == 200, f"status={response.status_code} {response.text[:300]}"

    views = response.json()["metrics"][0]["views"]
    period = next(view for view in views if view["view"] == "period")["values"][0]["value"]
    breakdown = next(view for view in views if view["view"] == "breakdown")["values"]

    assert breakdown, "the seat-tier breakdown carried no rows"
    assert sum(value["value"] for value in breakdown) == pytest.approx(period), (
        f"the breakdown sums to {sum(value['value'] for value in breakdown)}, the period says "
        f"{period}"
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_a_seat_month_row_names_its_billing_month_and_its_ceiling(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    """The row's own date is the day it was read, so the month has to be a column.

    Without it a month read late is indistinguishable from the next month's, and
    the amount has nothing to be judged against.
    """
    session = session_for("dev_lead")
    start, end = query_window(stand_manifest)

    rows = _evidence(session.client, session.person.uuid, start, end)
    assert rows, "no seat-month evidence over the seeded window"

    for row in rows:
        assert "billing_month" in row, f"the billing month is absent from {sorted(row)}"
        assert "ceiling_usd" in row, f"the ceiling is absent from {sorted(row)}"

        billing_month = _dt.date.fromisoformat(str(row["billing_month"]))
        assert billing_month.day == 1, f"the billing month is not a month start: {billing_month}"
        # A month is read while it runs or after it closes, never before it
        # opens — the one ordering that holds however late the read was.
        assert billing_month <= _dt.date.fromisoformat(str(row["date"])), (
            f"a row dated {row['date']} bills for {billing_month}, which had not started"
        )
