from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Final

import pytest
from insight_stand import ADMIN_OPERATOR_FIXTURE, ApiClient, JsonValue, PersonaSession, v3_path

from ..schemas.v3 import Problem
from ..schemas.v3_untyped import Dataset, Drilldown, Named
from .fed_records import RECORDS, _create, _expect, _run
from .fed_records import fed_dataset as fed_dataset
from .fed_records import ingest as ingest
from .fed_records import removals as removals

_ADMIN_ONLY: Final[str] = "admin role required for this operation"
_PAGE_ROWS: Final[int] = 2


@dataclass(frozen=True)
class Drilled:
    figure: Named
    rows: Named
    stat: Named
    dashboard: Named


def _walk(client: ApiClient, path: str) -> list[Drilldown]:
    pages: list[Drilldown] = []
    cursor: str | None = None

    while True:
        body: dict[str, JsonValue] = {
            "sort": {"key": "lines", "direction": "asc"},
            "limit": _PAGE_ROWS,
        }
        if cursor is not None:
            body["cursor"] = cursor

        page = _expect(client.post(path, json_body=body), 200).parse(Drilldown)
        pages.append(page)

        cursor = page.next_cursor
        if cursor is None:
            return pages
        assert len(pages) <= len(RECORDS), f"{path} kept handing out cursors past every record"


def _row_key(values: dict[str, JsonValue]) -> tuple[str, JsonValue, JsonValue]:
    return str(values["day"])[:10], values["author"], values["lines"]


@pytest.fixture
def drilled(v3_admin: ApiClient, fed_dataset: Dataset, removals: list[str]) -> Drilled:
    figure = _create(
        v3_admin,
        removals,
        "metrics",
        {
            "dataset": fed_dataset.name,
            "fields": [{"agg": "count", "type": "int", "as_name": "records"}],
        },
    )
    rows = _create(
        v3_admin,
        removals,
        "metrics",
        {
            "dataset": fed_dataset.name,
            "fields": [
                {"field": "day", "type": "string", "as_name": "day"},
                {"field": "author", "type": "string", "as_name": "author"},
                {"field": "lines", "type": "int", "as_name": "lines"},
            ],
        },
    )
    stat = _create(
        v3_admin,
        removals,
        "widgets",
        {"type": "stat", "metric": figure.id, "value": "records", "detail": rows.id},
    )
    dashboard = _create(
        v3_admin,
        removals,
        "dashboards",
        {"title": "Records", "items": [{"widget": stat.id}]},
    )

    return Drilled(figure=figure, rows=rows, stat=stat, dashboard=dashboard)


@pytest.mark.reliability
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE)
def test_drilling_into_a_widget_pages_every_row_behind_its_figure_exactly_once(
    v3_admin: ApiClient, drilled: Drilled
) -> None:
    figure = _run(v3_admin, v3_path(f"/v1/metrics/{drilled.figure.id}/run"))
    assert figure.rows == [[len(RECORDS)]]

    by_metric = _walk(v3_admin, v3_path(f"/v1/metrics/{drilled.rows.id}/drilldown"))
    by_widget = _walk(
        v3_admin,
        v3_path(f"/v1/dashboards/{drilled.dashboard.id}/widgets/{drilled.stat.id}/drilldown"),
    )

    assert [(page.selection, page.rows) for page in by_widget] == [
        (page.selection, page.rows) for page in by_metric
    ]
    assert {page.selection.metric for page in by_widget} == {drilled.rows.id}

    rows = [row.values for page in by_metric for row in page.rows]
    assert all(len(page.rows) <= _PAGE_ROWS for page in by_metric)
    assert len(rows) == figure.rows[0][0]
    assert [row["lines"] for row in rows] == sorted(record.lines for record in RECORDS)
    assert sorted(_row_key(row) for row in rows) == sorted(record.key() for record in RECORDS)


@pytest.mark.security
@pytest.mark.requires_seed(ADMIN_OPERATOR_FIXTURE, "dev_lead")
def test_a_dashboard_viewer_sees_the_figure_but_may_not_drill_into_the_rows_behind_it(
    v3_admin: ApiClient, drilled: Drilled, session_for: Callable[[str], PersonaSession]
) -> None:
    viewer = session_for("dev_lead")
    dashboard_path = v3_path(f"/v1/dashboards/{drilled.dashboard.id}")
    grant = f"{dashboard_path}/grants/{viewer.person.uuid}"

    _expect(v3_admin.put(grant, json_body={"level": "viewer"}), 204)
    try:
        figure = _run(viewer.client, f"{dashboard_path}/widgets/{drilled.stat.id}/run")
        assert figure.rows == [[len(RECORDS)]]

        for drilldown in (
            f"{dashboard_path}/widgets/{drilled.stat.id}/drilldown",
            v3_path(f"/v1/metrics/{drilled.rows.id}/drilldown"),
        ):
            refused = _expect(
                viewer.client.post(drilldown, json_body={"limit": _PAGE_ROWS}), 403
            ).parse(Problem)
            assert refused.context["reason"] == _ADMIN_ONLY
    finally:
        v3_admin.delete(grant)
