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

It also asserts the shape of the cost family as a whole: which quantities are
served, and that the one called overage is the amount the vendor billed rather
than the excess over the ceiling that caps it.

Every expectation is derived from what the stand serves — the date and the
billing month come out of the evidence itself — so the suite keeps working when
the seed is anchored to another date.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Callable, Mapping
from typing import Final

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, analytics_path

from ..schemas import MetricDefinitionListResponse, MetricResultsResponse, PeriodView
from . import query_window

METRIC_RESULTS = analytics_path("/v1/metric-results")
METRIC_DEFINITIONS = analytics_path("/v1/metric-definitions")
DRILLDOWN = analytics_path("/v1/metric-drilldown")

MONEY = "ai.extra_usage_cost"
DAILY_MONEY = "ai.daily_approximate_extra_usage_cost"
SEAT_COST = "ai.seat_cost"
UTILISATION = "ai.extra_usage_utilisation"
USAGE_PRICED = "ai.cost"

#: The subject the registry files a cost figure under.
COST_SUBJECT = "cost"

#: Every figure served under that subject, and the quantity each one reports.
#: Pinned as a set rather than counted: a sixth arrival either names a quantity
#: none of these reports — add it here, with that quantity — or it is a second
#: reading of one of them, which is what the test below exists to catch.
COST_FIGURES: Final[Mapping[str, str]] = {
    USAGE_PRICED: "consumption priced at vendor rates, which nobody was billed for",
    SEAT_COST: "the seat's own fee, taken from the vendor invoice",
    MONEY: "the usage the vendor billed on top of that fee",
    DAILY_MONEY: "the same billed usage, spread over the days it was spent",
    UTILISATION: "how close that spend is to the ceiling that blocks the seat",
}


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


@pytest.mark.reliability
def test_the_cost_family_serves_one_figure_per_quantity(api: ApiClient) -> None:
    """A second reading of the same money is the failure this catches.

    Two quantities sit one word apart here: what the vendor bills once a seat
    exhausts the usage its fee included, and the excess over the ceiling that
    caps that spend. They differ by orders of magnitude, both answer to
    "overage", and serving both at once leaves a reader no way to tell which of
    them is the money.
    """
    response = api.get(METRIC_DEFINITIONS)
    assert response.status_code == 200, (
        f"definitions: status={response.status_code} {response.text[:200]}"
    )

    served = {
        metric.metric_key
        for metric in response.parse(MetricDefinitionListResponse).metrics
        if metric.subject == COST_SUBJECT and metric.origin == "builtin" and metric.is_enabled
    }

    assert served == set(COST_FIGURES), (
        f"the enabled cost family is {sorted(served)} and this suite knows "
        f"{sorted(COST_FIGURES)}. A key added here has to report a quantity none of the "
        "others does; a second reading of the spend past a seat's fee is the one this "
        "forbids."
    )


@pytest.mark.requires_seed("dev_lead")
@pytest.mark.reliability
def test_the_billed_amount_is_not_the_excess_over_the_ceiling(
    session_for: Callable[[str], PersonaSession], stand_manifest: Manifest
) -> None:
    """Which of the two overage quantities is served, told from a single row.

    A seat-month row carries both the amount and the ceiling that amount is
    judged against, so a seat that spent something and stayed under its ceiling
    separates them with no second request: the vendor's billed amount is
    positive there, while the excess over the ceiling is exactly zero.
    """
    session = session_for("dev_lead")
    start, end = query_window(stand_manifest)

    rows = _evidence(session.client, session.person.uuid, start, end)
    assert rows, "no seat-month evidence over the seeded window"

    spending_under_a_ceiling = [
        row
        for row in rows
        if row.get("ceiling_usd") is not None
        and 0 < float(row["value"]) < float(row["ceiling_usd"])  # type: ignore[arg-type]
    ]

    assert spending_under_a_ceiling, (
        f"none of the {len(rows)} seat-months spends a non-zero amount below its ceiling, so "
        "nothing here tells the billed amount apart from the excess over the ceiling — the "
        "seed stopped covering the case this asserts"
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
    """The evidence row's own shape: the month it bills for, and the ceiling beside it.

    A seat-month is dated at the first day of the month it bills for, so the
    row's date and its billing month are one fact stated twice. A row where they
    differ has taken its date from the day it was read instead, which moves with
    the sync schedule. The ceiling sits beside the amount because the amount has
    nothing to be judged against without it.
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
        # Equal, not merely ordered: the row is dated AT the month it bills for,
        # so any difference means the date came from the day of the read.
        assert billing_month == _dt.date.fromisoformat(str(row["date"])), (
            f"a row dated {row['date']} says it bills for {billing_month}"
        )
